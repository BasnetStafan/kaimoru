//! Content page model (CMS spec C-2).
//!
//! The shop's long-form words as database rows: system pages
//! (guide, terms, privacy, legal, faq) seeded by migration and
//! custom pages the operator creates. A body is Markdown
//! source, rendered by [`crate::util::markdown`], or HTML the admin
//! wrote directly — sanitised by [`crate::util::html_sanitize`]
//! before it was stored, so stored HTML is always safe.

use chrono::{DateTime, Utc};

use super::jst::jst;

/// One row of `content_pages`.
#[derive(Clone, sqlx::FromRow)]
pub struct ContentPage {
    /// Surrogate ID.
    pub id: i64,
    /// URL identifier: the system pages' routes (`/guide`, …)
    /// and the last segment of `/pages/{slug}` for custom ones.
    /// Immutable after creation.
    pub slug: String,
    /// `system` (seeded, undeletable) or `custom` (operator-made).
    pub kind: String,
    /// Page title, shown as the page heading.
    pub title: String,
    /// Body source text: Markdown, or sanitised HTML when
    /// [`Self::format`] is `html`.
    pub body_md: String,
    /// How [`Self::body_md`] is interpreted: `markdown` or
    /// `html`.
    pub format: String,
    /// Per-page meta description; empty falls back at render.
    pub meta_description: String,
    /// `draft` or `published`; drafts 404 on the storefront.
    pub status: String,
    /// Creation instant.
    pub created_at: DateTime<Utc>,
    /// Last save instant.
    pub updated_at: DateTime<Utc>,
}

impl ContentPage {
    /// Whether this page was seeded by migration (undeletable,
    /// slug-locked).
    pub fn is_system(&self) -> bool {
        self.kind == "system"
    }

    /// Whether the storefront serves this page.
    pub fn is_published(&self) -> bool {
        self.status == "published"
    }

    /// Whether the body is HTML rather than Markdown.
    pub fn is_html(&self) -> bool {
        self.format == "html"
    }

    /// The storefront path: system pages keep their historic
    /// routes, custom pages live under `/pages/`.
    pub fn url_path(&self) -> String {
        if self.is_system() {
            format!("/{}", self.slug)
        } else {
            format!("/pages/{}", self.slug)
        }
    }

    /// The kind's Japanese label for the admin list.
    pub fn kind_label(&self) -> &'static str {
        if self.is_system() {
            "固定ページ"
        } else {
            "自由ページ"
        }
    }

    /// The status's Japanese label for the admin list.
    pub fn status_label(&self) -> &'static str {
        if self.is_published() {
            "公開"
        } else {
            "下書き"
        }
    }

    /// The last-update date for the admin list (`YYYY/MM/DD`).
    pub fn updated_date_label(&self) -> String {
        super::jst::jst_date_label(self.updated_at)
    }

    /// The 最終更新日 line for the storefront page
    /// (`YYYY年M月D日`).
    pub fn updated_long_label(&self) -> String {
        self.updated_at
            .with_timezone(&jst())
            .format("%Y年%-m月%-d日")
            .to_string()
    }
}

/// The fields of a page being created, grouped for the
/// repository's insert: seven loose values in a signature
/// drift too easily, and the group keeps the create shape in
/// one named place.
pub struct NewContentPage<'a> {
    /// URL identifier, unique across pages.
    pub slug: &'a str,
    /// `system` (seeded) or `custom` (operator-made).
    pub kind: &'a str,
    /// Page title.
    pub title: &'a str,
    /// Body source text.
    pub body_md: &'a str,
    /// How `body_md` is interpreted: `markdown` or `html`.
    pub format: &'a str,
    /// Per-page meta description.
    pub meta_description: &'a str,
    /// `draft` or `published`.
    pub status: &'a str,
}

/// A content page plus the name of the admin who last saved it
/// (from the newest revision), for the admin list. `None` for a
/// page nobody has saved since seeding.
#[derive(sqlx::FromRow)]
pub struct ContentPageSummary {
    /// The page row, flattened from the query.
    #[sqlx(flatten)]
    pub page: ContentPage,
    /// Display name of the admin who recorded the newest
    /// revision, when one exists.
    pub last_editor_name: Option<String>,
}

/// One row of `content_revisions`: the state of a page *before*
/// the save that recorded it, so any earlier state can be
/// restored. Capped at the newest 50 per page.
#[derive(Clone, sqlx::FromRow)]
pub struct ContentRevision {
    /// Surrogate ID.
    pub id: i64,
    /// The page this snapshot belongs to.
    pub page_id: i64,
    /// Title before the recording save.
    pub title: String,
    /// Body before the recording save.
    pub body_md: String,
    /// The replaced state's body format (`markdown`/`html`).
    pub format: String,
    /// Meta description before the recording save.
    pub meta_description: String,
    /// The admin whose save or restore recorded this snapshot.
    pub edited_by: i64,
    /// When the snapshot was recorded.
    pub created_at: DateTime<Utc>,
}

impl ContentRevision {
    /// The recording instant for the revisions list
    /// (`YYYY/MM/DD HH:MM`, JST).
    pub fn created_label(&self) -> String {
        super::jst::jst_datetime_label(self.created_at)
    }
}

/// A revision plus its recording admin's display name, for the
/// revisions list.
#[derive(sqlx::FromRow)]
pub struct ContentRevisionSummary {
    /// The revision row, flattened from the query.
    #[sqlx(flatten)]
    pub revision: ContentRevision,
    /// Display name of the admin who recorded the snapshot.
    pub editor_name: String,
}
