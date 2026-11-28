//! Blog post model (CMS blog spec, docs/specs/CMS_SPEC.md).
//!
//! A post is the pages CMS's dated, authored sibling: the
//! same body discipline (Markdown source rendered by
//! [`crate::util::markdown`], or HTML sanitised by
//! [`crate::util::html_sanitize`] before it was stored), plus
//! a publication instant, an author, an excerpt, a cover
//! image and a category.
//!
//! Liveness is a rule, not a column: a post is live iff it
//! is published AND its `published_at` has arrived. The rule
//! is defined once here ([`Post::is_live`]); every storefront
//! query carries the same predicate, so the index, the post
//! page, the archive, the feed and the sitemap can never
//! disagree about what is public. Scheduling is a future
//! `published_at` — no worker, no third status.

use chrono::{DateTime, Utc};

use super::jst::jst;

/// One row of `post_categories`.
#[derive(Clone, sqlx::FromRow)]
pub struct PostCategory {
    /// Surrogate ID.
    pub id: i64,
    /// Display name.
    pub name: String,
    /// URL identifier of `/blog/category/{slug}`.
    /// Unique across categories.
    pub slug: String,
    /// Creation instant.
    pub created_at: DateTime<Utc>,
    /// Last save instant.
    pub updated_at: DateTime<Utc>,
}

impl PostCategory {
    /// The storefront archive path.
    pub fn url_path(&self) -> String {
        format!("/blog/category/{}", self.slug)
    }
}

/// The derived publication state of a post: the stored
/// `status` read together with `published_at` against the
/// current instant. Admin surfaces show this, never the raw
/// status alone — a published post whose date has not
/// arrived is a reservation, not a live article.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PostState {
    /// Not published: invisible everywhere on the storefront.
    Draft,
    /// Published, but the publication instant is still ahead
    /// (or missing): scheduled, equally invisible.
    Scheduled,
    /// Published and the instant has arrived: public.
    Live,
}

impl PostState {
    /// The state's Japanese label for the admin list.
    pub fn label(&self) -> &'static str {
        match self {
            PostState::Draft => "下書き",
            PostState::Scheduled => "予約中",
            PostState::Live => "公開中",
        }
    }
}

/// One row of `posts`.
#[derive(Clone, sqlx::FromRow)]
pub struct Post {
    /// Surrogate ID.
    pub id: i64,
    /// URL identifier: the last segment of `/blog/{slug}`.
    /// Immutable after creation.
    pub slug: String,
    /// Post title, shown as the page heading.
    pub title: String,
    /// Body source text: Markdown, or sanitised HTML when
    /// [`Self::format`] is `html`.
    pub body_md: String,
    /// How [`Self::body_md`] is interpreted: `markdown` or
    /// `html`.
    pub format: String,
    /// Operator-written summary for cards and the feed;
    /// empty derives one from the body at render
    /// ([`Post::excerpt_or_derived`]).
    pub excerpt: String,
    /// Per-post meta description; empty falls back at render.
    pub meta_description: String,
    /// Cover image path (an uploaded image's served path),
    /// when one is set.
    pub cover_image_path: Option<String>,
    /// The post's category, when categorised.
    pub category_id: Option<i64>,
    /// The admin who created the post: its byline.
    pub author_admin_id: i64,
    /// `draft` or `published`.
    pub status: String,
    /// When the post goes (or went) public; a published post
    /// always carries one once saved through the admin.
    pub published_at: Option<DateTime<Utc>>,
    /// Creation instant.
    pub created_at: DateTime<Utc>,
    /// Last save instant.
    pub updated_at: DateTime<Utc>,
}

impl Post {
    /// Whether the body is HTML rather than Markdown.
    pub fn is_html(&self) -> bool {
        self.format == "html"
    }

    /// Whether the storefront may serve this post at
    /// `now`: published, with a publication instant that
    /// has arrived. THE liveness rule — the storefront
    /// repositories carry this same predicate in SQL.
    pub fn is_live(&self, now: DateTime<Utc>) -> bool {
        self.status == "published" && self.published_at.is_some_and(|at| at <= now)
    }

    /// The derived state at `now` (see [`PostState`]).
    pub fn state_at(&self, now: DateTime<Utc>) -> PostState {
        if self.status != "published" {
            PostState::Draft
        } else if self.is_live(now) {
            PostState::Live
        } else {
            PostState::Scheduled
        }
    }

    /// The storefront path.
    pub fn url_path(&self) -> String {
        format!("/blog/{}", self.slug)
    }

    /// The summary cards and the feed show: the stored
    /// excerpt when the operator wrote one, else one derived
    /// from the body text.
    pub fn excerpt_or_derived(&self) -> String {
        let stored = self.excerpt.trim();
        if !stored.is_empty() {
            return stored.to_string();
        }
        derive_excerpt(&self.body_md, &self.format)
    }

    /// The card-sized derivative of the cover image, when
    /// one is set: the resized copy the index cards serve.
    /// The path is derived by [`crate::util::image_paths`]
    /// — the product images' rule — so a blog card never
    /// ships a full-size original.
    pub fn cover_thumb_path(&self) -> Option<String> {
        self.cover_image_path
            .as_deref()
            .map(crate::util::image_paths::thumb_path)
    }

    /// The detail-sized derivative of the cover image,
    /// when one is set: the resized copy the post page's
    /// main view serves.
    pub fn cover_medium_path(&self) -> Option<String> {
        self.cover_image_path
            .as_deref()
            .map(crate::util::image_paths::medium_path)
    }

    /// The publication date for cards and the post page
    /// (`YYYY年M月D日`, JST); `None` while unpublished or
    /// unscheduled.
    pub fn published_long_label(&self) -> Option<String> {
        self.published_at.map(|at| {
            at.with_timezone(&jst())
                .format("%Y年%-m月%-d日")
                .to_string()
        })
    }

    /// The publication stamp for the admin list
    /// (`YYYY/MM/DD HH:MM`, JST); `-` while none is set.
    pub fn published_label(&self) -> String {
        self.published_at
            .map(super::jst::jst_datetime_label)
            .unwrap_or_else(|| "-".to_string())
    }

    /// The last-update date for the admin list (`YYYY/MM/DD`).
    pub fn updated_date_label(&self) -> String {
        super::jst::jst_date_label(self.updated_at)
    }
}

/// The fields of a post being created, grouped for the
/// repository's insert (the content pages' `NewContentPage`
/// reasoning: the group keeps the create shape in one named
/// place).
pub struct NewPost<'a> {
    /// URL identifier, unique across posts.
    pub slug: &'a str,
    /// Post title.
    pub title: &'a str,
    /// Body source text.
    pub body_md: &'a str,
    /// How `body_md` is interpreted.
    pub format: &'a str,
    /// Operator-written summary; may be empty.
    pub excerpt: &'a str,
    /// Per-post meta description; may be empty.
    pub meta_description: &'a str,
    /// Cover image path, when set.
    pub cover_image_path: Option<&'a str>,
    /// The post's category, when categorised.
    pub category_id: Option<i64>,
    /// The creating admin (the byline).
    pub author_admin_id: i64,
    /// `draft` or `published`.
    pub status: &'a str,
    /// Publication instant, when set.
    pub published_at: Option<DateTime<Utc>>,
}

/// A post plus the names the admin list shows beside it:
/// its category, its author, and the admin who recorded its
/// newest revision when one exists.
#[derive(sqlx::FromRow)]
pub struct PostSummary {
    /// The post row, flattened from the query.
    #[sqlx(flatten)]
    pub post: Post,
    /// Category display name; `None` when uncategorised.
    pub category_name: Option<String>,
    /// The author's display name (the byline).
    pub author_name: String,
    /// Display name of the admin who recorded the newest
    /// revision, when one exists.
    pub last_editor_name: Option<String>,
}

/// A post plus the names the storefront shows: its category
/// (name and slug, for the archive link) and its author's
/// display name. The one read shape of the index, the post
/// page, the archive and the feed.
#[derive(Clone, sqlx::FromRow)]
pub struct PostView {
    /// The post row, flattened from the query.
    #[sqlx(flatten)]
    pub post: Post,
    /// Category display name; `None` when uncategorised.
    pub category_name: Option<String>,
    /// Category slug for the archive link.
    pub category_slug: Option<String>,
    /// The author's display name (the byline).
    pub author_name: String,
}

/// One row of `post_revisions`: the state of a post *before*
/// the save that recorded it, so any earlier state can be
/// restored. Capped at the newest 50 per post. A snapshot
/// holds words only — title, body, format, meta, excerpt —
/// never the status, category, cover or schedule.
#[derive(Clone, sqlx::FromRow)]
pub struct PostRevision {
    /// Surrogate ID.
    pub id: i64,
    /// The post this snapshot belongs to.
    pub post_id: i64,
    /// Title before the recording save.
    pub title: String,
    /// Body before the recording save.
    pub body_md: String,
    /// The replaced state's body format (`markdown`/`html`).
    pub format: String,
    /// Meta description before the recording save.
    pub meta_description: String,
    /// Excerpt before the recording save.
    pub excerpt: String,
    /// The admin whose save or restore recorded this snapshot.
    pub edited_by: i64,
    /// When the snapshot was recorded.
    pub created_at: DateTime<Utc>,
}

impl PostRevision {
    /// The recording instant for the revisions list
    /// (`YYYY/MM/DD HH:MM`, JST).
    pub fn created_label(&self) -> String {
        super::jst::jst_datetime_label(self.created_at)
    }
}

/// A revision plus its recording admin's display name, for
/// the revisions list.
#[derive(sqlx::FromRow)]
pub struct PostRevisionSummary {
    /// The revision row, flattened from the query.
    #[sqlx(flatten)]
    pub revision: PostRevision,
    /// Display name of the admin who recorded the snapshot.
    pub editor_name: String,
}

/// How many characters a derived excerpt keeps before the
/// ellipsis: a card summary, not the article.
pub const DERIVED_EXCERPT_MAX_CHARS: usize = 120;

/// Derives a plain-text excerpt from a body: markup is
/// stripped, whitespace collapsed, and the result truncated
/// to [`DERIVED_EXCERPT_MAX_CHARS`] with an ellipsis when it
/// ran longer. The stored-excerpt path never comes here —
/// this is the fallback for a post whose operator wrote no
/// summary.
pub fn derive_excerpt(body_md: &str, format: &str) -> String {
    let text = if format == "html" {
        strip_html(body_md)
    } else {
        strip_markdown(body_md)
    };
    truncate_excerpt(&text)
}

/// Collapses every whitespace run to one space and trims
/// the ends: the shared tail of both strippers.
fn collapse_whitespace(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Truncates already-plain text to the derived-excerpt
/// length, appending `…` when anything was cut.
fn truncate_excerpt(text: &str) -> String {
    if text.chars().count() <= DERIVED_EXCERPT_MAX_CHARS {
        return text.to_string();
    }
    let head: String = text.chars().take(DERIVED_EXCERPT_MAX_CHARS).collect();
    format!("{}…", head.trim_end())
}

/// Strips HTML to its text: tags drop out, the five common
/// entities decode, whitespace collapses. The input is a
/// sanitised stored body, so no script text survives to be
/// excerpted in the first place.
fn strip_html(source: &str) -> String {
    let mut out = String::with_capacity(source.len());
    let mut in_tag = false;
    for ch in source.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => {
                in_tag = false;
                out.push(' ');
            }
            _ if !in_tag => out.push(ch),
            _ => {}
        }
    }
    let decoded = out
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&amp;", "&");
    collapse_whitespace(&decoded)
}

/// Strips Markdown to its text: heading / quote / list
/// markers drop off line starts, links and images reduce to
/// their words, emphasis and code markers drop, whitespace
/// collapses.
fn strip_markdown(source: &str) -> String {
    let mut lines = Vec::new();
    for line in source.lines() {
        let mut text = line.trim();
        while let Some(rest) = text.strip_prefix('#') {
            text = rest.trim_start();
        }
        if let Some(rest) = text.strip_prefix('>') {
            text = rest.trim_start();
        }
        for marker in ["- ", "* ", "+ "] {
            if let Some(rest) = text.strip_prefix(marker) {
                text = rest;
                break;
            }
        }
        lines.push(strip_inline_markdown(text));
    }
    collapse_whitespace(&lines.join(" "))
}

/// Reduces one line's inline Markdown: `[words](url)` and
/// `![words](url)` keep only their words; backticks and
/// emphasis asterisks / underscores drop.
fn strip_inline_markdown(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < chars.len() {
        // A link or image: keep the bracketed words, skip
        // the parenthesised destination.
        if chars[i] == '['
            && let Some(close) = find_char(&chars, i + 1, ']')
            && chars.get(close + 1) == Some(&'(')
            && let Some(end) = find_char(&chars, close + 2, ')')
        {
            out.extend(&chars[i + 1..close]);
            i = end + 1;
            continue;
        }
        match chars[i] {
            '`' | '*' | '_' => {}
            ch => out.push(ch),
        }
        i += 1;
    }
    out
}

/// The index of `target` in `chars` at or after `from`.
fn find_char(chars: &[char], from: usize, target: char) -> Option<usize> {
    (from..chars.len()).find(|&i| chars[i] == target)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fixed "now": 2026-11-28 12:00 UTC.
    fn now() -> DateTime<Utc> {
        DateTime::from_timestamp(1_795_867_200, 0).expect("valid timestamp")
    }

    /// A post in the given state ingredients.
    fn post(status: &str, published_at: Option<DateTime<Utc>>) -> Post {
        Post {
            id: 1,
            slug: "hello".to_string(),
            title: "題".to_string(),
            body_md: String::new(),
            format: "markdown".to_string(),
            excerpt: String::new(),
            meta_description: String::new(),
            cover_image_path: None,
            category_id: None,
            author_admin_id: 1,
            status: status.to_string(),
            published_at,
            created_at: now(),
            updated_at: now(),
        }
    }

    #[test]
    fn liveness_needs_published_and_arrived_date() {
        let past = now() - chrono::Duration::hours(1);
        let future = now() + chrono::Duration::hours(1);
        assert!(post("published", Some(past)).is_live(now()));
        assert!(!post("published", Some(future)).is_live(now()));
        assert!(!post("published", None).is_live(now()));
        assert!(!post("draft", Some(past)).is_live(now()));
        assert!(!post("draft", None).is_live(now()));
    }

    #[test]
    fn state_labels_follow_the_derived_state() {
        let past = now() - chrono::Duration::hours(1);
        let future = now() + chrono::Duration::hours(1);
        assert_eq!(post("draft", Some(past)).state_at(now()), PostState::Draft);
        assert_eq!(
            post("published", Some(future)).state_at(now()),
            PostState::Scheduled
        );
        assert_eq!(
            post("published", None).state_at(now()),
            PostState::Scheduled
        );
        assert_eq!(
            post("published", Some(past)).state_at(now()),
            PostState::Live
        );
        assert_eq!(PostState::Draft.label(), "下書き");
        assert_eq!(PostState::Scheduled.label(), "予約中");
        assert_eq!(PostState::Live.label(), "公開中");
    }

    #[test]
    fn stored_excerpt_wins_over_derivation() {
        let mut p = post("published", Some(now()));
        p.excerpt = "  書いた要約  ".to_string();
        p.body_md = "本文は別にある".to_string();
        assert_eq!(p.excerpt_or_derived(), "書いた要約");
    }

    #[test]
    fn excerpt_derives_from_markdown_text() {
        let body = "## 見出し\n\n- 項目A\n- 項目B\n\n[案内](/guide) を見てください。";
        assert_eq!(
            derive_excerpt(body, "markdown"),
            "見出し 項目A 項目B 案内 を見てください。"
        );
    }

    #[test]
    fn excerpt_derives_from_html_text() {
        let body = "<h2>見出し</h2><p>本文 &amp; 補足</p><script>x</script>";
        // The script TAG drops; its text stays a word — a
        // stored body is sanitised at save, so a script never
        // reaches derivation in production.
        assert_eq!(derive_excerpt(body, "html"), "見出し 本文 & 補足 x");
    }

    #[test]
    fn excerpt_truncates_with_ellipsis() {
        let body = "あ".repeat(200);
        let derived = derive_excerpt(&body, "markdown");
        assert_eq!(derived.chars().count(), DERIVED_EXCERPT_MAX_CHARS + 1);
        assert!(derived.ends_with('…'));
        let exact = "あ".repeat(DERIVED_EXCERPT_MAX_CHARS);
        assert_eq!(derive_excerpt(&exact, "markdown"), exact);
    }
}
