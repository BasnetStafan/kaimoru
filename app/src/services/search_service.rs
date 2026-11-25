//! Search service.
//! Owns query normalisation, the sort allowlist, and the suggest
//! rate limit. Knows nothing about HTTP or templates.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

use sqlx::PgPool;

use crate::error::AppError;
use crate::messages::NON_NEGATIVE_INTEGER_MESSAGE;
use crate::repositories::search_repository::{self, SearchSort};
use crate::repositories::tag_repository;
use crate::util::like_pattern::escape_like;
use crate::util::rate_limit_knobs::SWEEP_THRESHOLD;

/// The search outcome type, re-exported so controllers never
/// reach into the repository for it.
pub use crate::repositories::search_repository::SearchOutcome as Outcome;

/// The search inputs, re-exported so controllers build them
/// without reaching into the repository.
pub use crate::repositories::search_repository::SearchArgs;

/// Runs one catalog search (the results page's data).
pub async fn search(pool: &PgPool, args: SearchArgs<'_>) -> Result<Outcome, AppError> {
    Ok(search_repository::search(pool, args).await?)
}

/// Returns the display label for a tag slug — the search page's
/// active-filter chip. `None` for an unknown slug: the filter
/// then simply matches nothing, and no chip is shown.
pub async fn tag_label(pool: &PgPool, slug: &str) -> Result<Option<String>, AppError> {
    Ok(tag_repository::find_by_slug(pool, slug)
        .await?
        .map(|t| t.label))
}

/// Product names for the suggestion fragment.
pub async fn suggest_names(
    pool: &PgPool,
    pattern: &str,
    limit: i64,
) -> Result<Vec<String>, AppError> {
    Ok(search_repository::suggest(pool, pattern, limit).await?)
}

/// Maximum suggestions returned for one keystroke query.
pub const SUGGEST_LIMIT: i64 = 8;
/// Queries are capped at this many characters before splitting.
const MAX_QUERY_CHARS: usize = 50;
/// At most this many terms are searched; extra terms are dropped.
const MAX_TERMS: usize = 5;
/// Suggest requests allowed per IP inside the window.
const SUGGEST_LIMIT_PER_WINDOW: usize = 30;
/// The suggest rate limit window.
const SUGGEST_WINDOW: Duration = Duration::from_secs(60);

/// The full-width forms of the half-width katakana block
/// U+FF61..=U+FF9D, in code-point order. The two sound marks
/// (U+FF9E/U+FF9F) are not in the table: they merge into the
/// preceding kana instead of standing alone.
const HALFWIDTH_KATAKANA: &str = "。「」、・ヲァィゥェォャュョッーアイウエオカキクケコサシスセソタチツテトナニヌネノハヒフヘホマミムメモヤユヨラリルレロワン";

/// Normalizes the width of a raw search query to the forms the
/// catalog text is stored in: full-width ASCII (U+FF01..=U+FF5E)
/// becomes half-width, a full-width space becomes a plain space,
/// and half-width katakana becomes full-width katakana, with a
/// half-width sound mark merged into the kana before it when
/// that kana has a voiced form. Both the results page and the
/// suggestions run this before any cap or split, so the two can
/// never disagree about what a query means.
pub fn normalize_query_width(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for c in raw.chars() {
        match c {
            '\u{FF01}'..='\u{FF5E}' => {
                out.push(char::from_u32(c as u32 - 0xFEE0).unwrap_or(c));
            }
            '\u{3000}' => out.push(' '),
            '\u{FF61}'..='\u{FF9D}' => {
                let index = (c as u32 - 0xFF61) as usize;
                if let Some(full) = HALFWIDTH_KATAKANA.chars().nth(index) {
                    out.push(full);
                }
            }
            '\u{FF9E}' | '\u{FF9F}' => push_sound_mark(&mut out, c),
            _ => out.push(c),
        }
    }
    out
}

/// Merges a half-width sound mark (dakuten U+FF9E, handakuten
/// U+FF9F) into the last emitted kana when that kana has the
/// marked form; a mark with nothing to merge into becomes the
/// standalone mark character instead.
fn push_sound_mark(out: &mut String, mark: char) {
    let marked = out.chars().next_back().and_then(|last| {
        if mark == '\u{FF9E}' {
            voiced_kana(last)
        } else {
            semi_voiced_kana(last)
        }
    });
    match marked {
        Some(kana) => {
            out.pop();
            out.push(kana);
        }
        None => out.push(if mark == '\u{FF9E}' {
            '\u{309B}'
        } else {
            '\u{309C}'
        }),
    }
}

/// Returns the dakuten (voiced) form of a full-width katakana,
/// or `None` when the kana has none.
fn voiced_kana(kana: char) -> Option<char> {
    match kana {
        'カ' => Some('ガ'),
        'キ' => Some('ギ'),
        'ク' => Some('グ'),
        'ケ' => Some('ゲ'),
        'コ' => Some('ゴ'),
        'サ' => Some('ザ'),
        'シ' => Some('ジ'),
        'ス' => Some('ズ'),
        'セ' => Some('ゼ'),
        'ソ' => Some('ゾ'),
        'タ' => Some('ダ'),
        'チ' => Some('ヂ'),
        'ツ' => Some('ヅ'),
        'テ' => Some('デ'),
        'ト' => Some('ド'),
        'ハ' => Some('バ'),
        'ヒ' => Some('ビ'),
        'フ' => Some('ブ'),
        'ヘ' => Some('ベ'),
        'ホ' => Some('ボ'),
        'ウ' => Some('ヴ'),
        _ => None,
    }
}

/// Returns the handakuten (semi-voiced) form of a full-width
/// katakana, or `None` when the kana has none.
fn semi_voiced_kana(kana: char) -> Option<char> {
    match kana {
        'ハ' => Some('パ'),
        'ヒ' => Some('ピ'),
        'フ' => Some('プ'),
        'ヘ' => Some('ペ'),
        'ホ' => Some('ポ'),
        _ => None,
    }
}

/// Normalizes a raw search string into ILIKE patterns, one per term.
///
/// The width is normalized first (see [`normalize_query_width`]),
/// the input is then capped at 50 characters and 5 terms, and
/// LIKE wildcards in the input are escaped so a customer typing
/// "%" searches for a literal percent. Each returned pattern is
/// `%term%`, ready to bind.
pub fn normalize_patterns(raw: &str) -> Vec<String> {
    let normalized = normalize_query_width(raw);
    let capped: String = normalized.trim().chars().take(MAX_QUERY_CHARS).collect();
    capped
        .split_whitespace()
        .take(MAX_TERMS)
        .map(|term| format!("%{}%", escape_like(term)))
        .collect()
}

/// Builds the single pattern for name suggestions, or `None` when
/// the input is under the 2-character minimum (the client also
/// waits for 2 characters; the server never trusts that it did).
pub fn suggest_pattern(raw: &str) -> Option<String> {
    let normalized = normalize_query_width(raw);
    let term = normalized.trim();
    if term.chars().count() < 2 {
        return None;
    }
    let capped: String = term.chars().take(MAX_QUERY_CHARS).collect();
    Some(format!("%{}%", escape_like(&capped)))
}

/// One parsed price-range bound pair: the bounds to filter
/// by (`None` for an open end or a bound that failed
/// validation) and the per-field errors to show. An invalid
/// bound never reaches the query — the search runs without
/// that bound and the field error explains why.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PriceRange {
    /// Lowest price to keep, in yen.
    pub min: Option<i64>,
    /// Highest price to keep, in yen.
    pub max: Option<i64>,
    /// Error for the lower-bound field.
    pub min_error: Option<String>,
    /// Error for the upper-bound field.
    pub max_error: Option<String>,
}

/// Parses one raw price bound: blank is an open end, a
/// non-negative integer is a bound, anything else is a
/// field error (the bound is dropped, never guessed at).
fn parse_price_bound(raw: Option<&str>) -> (Option<i64>, Option<String>) {
    let Some(text) = raw.map(str::trim).filter(|t| !t.is_empty()) else {
        return (None, None);
    };
    match text.parse::<i64>() {
        Ok(value) if value >= 0 => (Some(value), None),
        _ => (None, Some(NON_NEGATIVE_INTEGER_MESSAGE.to_string())),
    }
}

/// Validates the price-range parameters as a pair. Each
/// bound stands on its own (see `parse_price_bound`); two
/// valid bounds in the wrong order invalidate the range as
/// a whole — the error lands on the upper field and neither
/// bound filters.
pub fn parse_price_range(min_raw: Option<&str>, max_raw: Option<&str>) -> PriceRange {
    let (min, min_error) = parse_price_bound(min_raw);
    let (max, max_error) = parse_price_bound(max_raw);
    if let (Some(lo), Some(hi)) = (min, max)
        && lo > hi
    {
        return PriceRange {
            min: None,
            max: None,
            min_error,
            max_error: Some("上限は下限以上の金額を入力してください。".to_string()),
        };
    }
    PriceRange {
        min,
        max,
        min_error,
        max_error,
    }
}

/// Maps the request's sort parameter onto the allowlist.
/// Unknown values fall back to the default order — the parameter
/// is never passed through to SQL.
pub fn parse_sort(param: Option<&str>) -> (SearchSort, &'static str) {
    match param {
        Some("new") => (SearchSort::Newest, "new"),
        Some("price_asc") => (SearchSort::PriceAsc, "price_asc"),
        Some("price_desc") => (SearchSort::PriceDesc, "price_desc"),
        Some("rating") => (SearchSort::Rating, "rating"),
        _ => (SearchSort::Recommended, "recommended"),
    }
}

/// Suggest request times per IP, for the rate limit.
static SUGGEST_LOG: LazyLock<Mutex<HashMap<String, Vec<Instant>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Whether one more suggest request from this IP is allowed, and
/// records it when it is. In-memory like the login throttle:
/// disposable by design, a restart simply resets the allowance.
pub fn suggest_allowed(ip: &str) -> bool {
    let now = Instant::now();
    let mut log = SUGGEST_LOG
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if log.len() >= SWEEP_THRESHOLD && !log.contains_key(ip) {
        log.retain(|_, times| {
            times.retain(|t| now.duration_since(*t) < SUGGEST_WINDOW);
            !times.is_empty()
        });
    }
    let entry = log.entry(ip.to_string()).or_default();
    entry.retain(|t| now.duration_since(*t) < SUGGEST_WINDOW);
    if entry.len() >= SUGGEST_LIMIT_PER_WINDOW {
        return false;
    }
    entry.push(now);
    true
}

#[cfg(test)]
mod tests {
    use super::{normalize_patterns, normalize_query_width, parse_sort, suggest_pattern};

    #[test]
    fn terms_become_contains_patterns() {
        assert_eq!(normalize_patterns("rust"), ["%rust%"]);
        assert_eq!(normalize_patterns("rust 本"), ["%rust%", "%本%"]);
        assert_eq!(normalize_patterns("rust　本"), ["%rust%", "%本%"]);
    }

    #[test]
    fn like_wildcards_in_input_are_escaped() {
        assert_eq!(normalize_patterns("100%"), ["%100\\%%"]);
        assert_eq!(normalize_patterns("a_b"), ["%a\\_b%"]);
    }

    #[test]
    fn term_count_is_capped() {
        assert_eq!(normalize_patterns("a b c d e f g").len(), 5);
    }

    #[test]
    fn suggest_needs_two_characters() {
        assert_eq!(suggest_pattern("a"), None);
        assert_eq!(suggest_pattern(" ab "), Some("%ab%".to_string()));
    }

    #[test]
    fn fullwidth_ascii_becomes_halfwidth() {
        assert_eq!(normalize_query_width("Ｔシャツ"), "Tシャツ");
        assert_eq!(normalize_query_width("１００％"), "100%");
        assert_eq!(normalize_query_width("Ｔ　シャツ"), "T シャツ");
    }

    #[test]
    fn halfwidth_katakana_becomes_fullwidth() {
        assert_eq!(normalize_query_width("ｽﾃｯｶｰ"), "ステッカー");
        assert_eq!(normalize_query_width("ﾏｸﾞｶｯﾌﾟ"), "マグカップ");
    }

    #[test]
    fn sound_marks_merge_into_the_preceding_kana() {
        assert_eq!(normalize_query_width("ｶﾞｰﾃﾞﾝ"), "ガーデン");
        assert_eq!(normalize_query_width("ﾊﾟﾝ"), "パン");
        assert_eq!(normalize_query_width("ｳﾞｧ"), "ヴァ");
        // A mark with no kana to merge into stays a mark.
        assert_eq!(normalize_query_width("a\u{FF9E}"), "a\u{309B}");
    }

    #[test]
    fn patterns_use_the_normalized_width() {
        assert_eq!(normalize_patterns("Ｔシャツ"), ["%Tシャツ%"]);
        assert_eq!(normalize_patterns("ｽﾃｯｶｰ"), ["%ステッカー%"]);
        assert_eq!(suggest_pattern("Ｔシャツ"), Some("%Tシャツ%".to_string()));
        assert_eq!(suggest_pattern("ｽﾃｯｶｰ"), Some("%ステッカー%".to_string()));
    }

    #[test]
    fn price_range_bounds_parse_and_validate() {
        use super::parse_price_range;
        let open = parse_price_range(None, None);
        assert_eq!(open.min, None);
        assert_eq!(open.max, None);
        assert_eq!(open.min_error, None);
        let ranged = parse_price_range(Some("1000"), Some(" 3000 "));
        assert_eq!(ranged.min, Some(1000));
        assert_eq!(ranged.max, Some(3000));
        assert_eq!(ranged.max_error, None);
        // A bad bound drops alone; the good one still filters.
        let one_bad = parse_price_range(Some("-5"), Some("3000"));
        assert_eq!(one_bad.min, None);
        assert!(one_bad.min_error.is_some());
        assert_eq!(one_bad.max, Some(3000));
        let junk = parse_price_range(Some("abc"), Some("1.5"));
        assert!(junk.min_error.is_some());
        assert!(junk.max_error.is_some());
        // Reversed bounds invalidate the whole range.
        let reversed = parse_price_range(Some("5000"), Some("1000"));
        assert_eq!(reversed.min, None);
        assert_eq!(reversed.max, None);
        assert!(reversed.max_error.is_some());
    }

    #[test]
    fn unknown_sort_falls_back_to_recommended() {
        assert_eq!(parse_sort(Some("price_asc")).1, "price_asc");
        assert_eq!(parse_sort(Some("bogus")).1, "recommended");
        assert_eq!(parse_sort(None).1, "recommended");
    }
}
