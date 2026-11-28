//! Blog embed shortcodes (CMS blog embeds, CMS_SPEC §8).
//!
//! A post body may carry `[[product:123]]` and
//! `[[category:45]]` tokens. They are author-typed text,
//! never markup: the render pipeline first renders the body
//! exactly as it always did (Markdown through
//! [`crate::util::markdown`], HTML as sanitised at save),
//! then [`mark_embeds`] rewrites every well-formed token in
//! the rendered HTML into an inert placeholder comment, and
//! the blog service later splices the app-rendered card
//! markup into those placeholders, per request. Because the
//! expansion is the app's own rendering — never the
//! author's — the sanitizer story is untouched, and because
//! the placeholder is a comment, a surface that never
//! expands it shows nothing at all.
//!
//! The grammar is deliberately tiny: `[[`, the kind word
//! (`product` or `category`), `:`, one or more ASCII digits,
//! `]]`. Anything else — empty digits, letters, spaces, a
//! number too large for an id — is not a token and stays
//! literal text, so an author writing about the syntax (or
//! mistyping it) sees their own words, not a vanished hole.

/// Which catalogue a shortcode points into.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum EmbedKind {
    /// A single product: `[[product:<id>]]`.
    Product,
    /// A product category showcase: `[[category:<id>]]`.
    Category,
}

impl EmbedKind {
    /// The kind word as written in a token and a placeholder.
    fn word(self) -> &'static str {
        match self {
            EmbedKind::Product => "product",
            EmbedKind::Category => "category",
        }
    }
}

/// One well-formed shortcode occurrence: a kind and the id
/// it names. Produced by [`mark_embeds`]'s scan of a body
/// and by [`embed_refs`]' scan of marked HTML.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct EmbedRef {
    /// The catalogue the token points into.
    pub kind: EmbedKind,
    /// The named row's id.
    pub id: i64,
}

/// The placeholder a token becomes in rendered HTML: an
/// HTML comment naming the embed, e.g.
/// `<!--blog-embed:product:123-->`. A comment is inert in
/// every surface that never expands it (and invisible in
/// the page), carries no session data, and is unique enough
/// never to collide with author text.
pub fn placeholder(kind: EmbedKind, id: i64) -> String {
    format!("<!--blog-embed:{}:{}-->", kind.word(), id)
}

/// Scans `text` for well-formed shortcode tokens, returning
/// each occurrence in order. The one tokeniser: marking,
/// stripping and (through the placeholders) expansion all
/// read a body through the grammar defined here.
fn scan_tokens(text: &str) -> Vec<(usize, usize, EmbedRef)> {
    let bytes = text.as_bytes();
    let mut found = Vec::new();
    let mut i = 0;
    while i + 1 < bytes.len() {
        if bytes[i] == b'['
            && bytes[i + 1] == b'['
            && let Some((end, embed)) = parse_token_at(text, i)
        {
            found.push((i, end, embed));
            i = end;
            continue;
        }
        i += 1;
    }
    found
}

/// Parses one token at byte offset `start` (which points at
/// `[[`): the kind word, a colon, ASCII digits, `]]`.
/// Returns the offset just past the token and the ref, or
/// `None` when the text at `start` is not a well-formed
/// token.
fn parse_token_at(text: &str, start: usize) -> Option<(usize, EmbedRef)> {
    let rest = &text[start + 2..];
    let (kind, rest) = if let Some(r) = rest.strip_prefix("product:") {
        (EmbedKind::Product, r)
    } else {
        let r = rest.strip_prefix("category:")?;
        (EmbedKind::Category, r)
    };
    let digit_len = rest.bytes().take_while(|b| b.is_ascii_digit()).count();
    if digit_len == 0 {
        return None;
    }
    let (digits, tail) = rest.split_at(digit_len);
    if !tail.starts_with("]]") {
        return None;
    }
    // A digit run too large for an id is not a token: it
    // stays the author's literal text like any other
    // malformed shape.
    let id: i64 = digits.parse().ok()?;
    // The token's full length: `[[` + the kind word + `:`
    // + the digits + `]]`.
    let end = start + 2 + kind.word().len() + 1 + digit_len + 2;
    Some((end, EmbedRef { kind, id }))
}

/// Rewrites every well-formed shortcode token in `html`
/// into its placeholder comment (see [`placeholder`]).
/// Malformed shapes stay literal. Applied to rendered body
/// HTML before it enters the blog render cache, so the
/// cache only ever holds placeholders — never product data.
pub fn mark_embeds(html: &str) -> String {
    let tokens = scan_tokens(html);
    if tokens.is_empty() {
        return html.to_string();
    }
    let mut out = String::with_capacity(html.len());
    let mut cursor = 0;
    for (start, end, embed) in tokens {
        out.push_str(&html[cursor..start]);
        out.push_str(&placeholder(embed.kind, embed.id));
        cursor = end;
    }
    out.push_str(&html[cursor..]);
    out
}

/// Removes every well-formed shortcode token from plain
/// text (excerpts, feed descriptions): a token is an
/// instruction to the renderer, not words, so it must never
/// leak into a text surface. Malformed shapes are the
/// author's literal words and stay.
pub fn strip_shortcodes(text: &str) -> String {
    let tokens = scan_tokens(text);
    if tokens.is_empty() {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len());
    let mut cursor = 0;
    for (start, end, _) in tokens {
        out.push_str(&text[cursor..start]);
        cursor = end;
    }
    out.push_str(&text[cursor..]);
    out
}

/// The distinct embeds a marked HTML body waits on, in
/// first-appearance order: the expansion's shopping list.
/// Duplicates collapse here (each placeholder string is
/// spliced everywhere it appears), kinds stay separate so
/// the caller can batch one query per kind.
pub fn embed_refs(marked_html: &str) -> Vec<EmbedRef> {
    let mut refs: Vec<EmbedRef> = Vec::new();
    let mut rest = marked_html;
    while let Some(pos) = rest.find("<!--blog-embed:") {
        let tail = &rest[pos..];
        let Some(end) = tail.find("-->") else {
            break;
        };
        let inner = &tail["<!--blog-embed:".len()..end];
        if let Some((word, digits)) = inner.split_once(':')
            && let Ok(id) = digits.parse::<i64>()
        {
            let kind = match word {
                "product" => Some(EmbedKind::Product),
                "category" => Some(EmbedKind::Category),
                _ => None,
            };
            if let Some(kind) = kind {
                let embed = EmbedRef { kind, id };
                if !refs.contains(&embed) {
                    refs.push(embed);
                }
            }
        }
        rest = &tail[end + 3..];
    }
    refs
}

/// Removes every embed placeholder from marked HTML,
/// leaving the surrounding body untouched: the expansion's
/// failure posture, where a catalogue read error costs the
/// cards, never the article.
pub fn strip_placeholders(marked_html: &str) -> String {
    let mut out = marked_html.to_string();
    for embed in embed_refs(marked_html) {
        out = out.replace(&placeholder(embed.kind, embed.id), "");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marks_product_and_category_tokens() {
        let html = "<p>前 [[product:12]] 中 [[category:3]] 後</p>";
        let marked = mark_embeds(html);
        assert_eq!(
            marked,
            "<p>前 <!--blog-embed:product:12--> 中 <!--blog-embed:category:3--> 後</p>"
        );
    }

    #[test]
    fn marks_duplicates_and_adjacent_tokens() {
        let marked = mark_embeds("[[product:1]][[product:1]] と [[product:2]]");
        assert_eq!(
            marked,
            "<!--blog-embed:product:1--><!--blog-embed:product:1--> と <!--blog-embed:product:2-->"
        );
    }

    #[test]
    fn malformed_tokens_stay_literal() {
        for text in [
            "[[product:]]",
            "[[product:abc]]",
            "[[product: 12]]",
            "[[product:12 ]]",
            "[[product:12]",
            "[[products:12]]",
            "[[product:99999999999999999999999999]]",
            "[[カテゴリ:3]]",
        ] {
            assert_eq!(mark_embeds(text), text, "must stay literal: {text}");
            assert_eq!(strip_shortcodes(text), text, "must stay literal: {text}");
        }
    }

    #[test]
    fn strip_removes_wellformed_tokens_only() {
        assert_eq!(
            strip_shortcodes("おすすめ [[product:5]] です [[product:x]]"),
            "おすすめ  です [[product:x]]"
        );
    }

    #[test]
    fn refs_are_distinct_in_first_appearance_order() {
        let marked =
            mark_embeds("[[category:2]] [[product:9]] [[category:2]] [[product:9]] [[product:3]]");
        assert_eq!(
            embed_refs(&marked),
            vec![
                EmbedRef {
                    kind: EmbedKind::Category,
                    id: 2
                },
                EmbedRef {
                    kind: EmbedKind::Product,
                    id: 9
                },
                EmbedRef {
                    kind: EmbedKind::Product,
                    id: 3
                },
            ]
        );
    }

    #[test]
    fn strip_placeholders_removes_every_occurrence() {
        let marked = mark_embeds("a [[product:1]] b [[product:1]] c");
        assert_eq!(strip_placeholders(&marked), "a  b  c");
    }

    #[test]
    fn plain_text_passes_through_unchanged() {
        let text = "<p>トークンのない本文です。[リンク](https://example.jp)</p>";
        assert_eq!(mark_embeds(text), text);
        assert!(embed_refs(text).is_empty());
    }
}
