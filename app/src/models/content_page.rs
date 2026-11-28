//! Content page model (CMS spec C-2).
//!
//! The shop's long-form words as database rows: system pages
//! (guide, terms, privacy, legal, faq) seeded by migration and
//! custom pages the operator creates. A body is Markdown
//! source, rendered by [`crate::util::markdown`], or HTML the admin
//! wrote directly — sanitised by [`crate::util::html_sanitize`]
//! before it was stored, so stored HTML is always safe.
//!
//! A page also carries a type (CMS_SPEC §9): the `layout`
//! column separates standard pages from landing pages —
//! the same words machinery under two storefront shapes.

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
    /// Page type (CMS_SPEC §9): `standard` (the historic
    /// full-chrome page) or `landing` (minimal chrome at
    /// `/lp/{slug}`). Set at creation; never changes.
    pub layout: String,
    /// Whether the sitemap may list this page. Standard
    /// pages default to listed; a landing page is listed
    /// only when its operator opts in.
    pub in_sitemap: bool,
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

    /// Whether this page is a landing page (CMS_SPEC §9):
    /// minimal chrome, served under `/lp/`.
    pub fn is_landing(&self) -> bool {
        self.layout == "landing"
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
    /// routes, landing pages live under `/lp/`, and the
    /// remaining custom pages under `/pages/`. Slugs are
    /// unique per layout, so the two custom namespaces may
    /// share one slug string without colliding.
    pub fn url_path(&self) -> String {
        if self.is_system() {
            format!("/{}", self.slug)
        } else if self.is_landing() {
            format!("/lp/{}", self.slug)
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

    /// The page type's Japanese badge for the admin lists
    /// (CMS_SPEC §9): a landing page says so whatever its
    /// kind; every other page keeps its kind label.
    pub fn type_label(&self) -> &'static str {
        if self.is_landing() {
            "ランディングページ"
        } else {
            self.kind_label()
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
    /// Page type: `standard` or `landing` (CMS_SPEC §9).
    pub layout: &'a str,
    /// Whether the sitemap may list the page.
    pub in_sitemap: bool,
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

#[cfg(test)]
mod tests {
    use super::*;

    /// A bare page with the routing-relevant fields set by
    /// each test.
    fn page(kind: &str, layout: &str, slug: &str) -> ContentPage {
        ContentPage {
            id: 1,
            slug: slug.to_string(),
            kind: kind.to_string(),
            title: "題".to_string(),
            body_md: String::new(),
            format: "markdown".to_string(),
            meta_description: String::new(),
            status: "published".to_string(),
            layout: layout.to_string(),
            in_sitemap: true,
            created_at: DateTime::from_timestamp(0, 0).expect("epoch"),
            updated_at: DateTime::from_timestamp(0, 0).expect("epoch"),
        }
    }

    #[test]
    fn url_path_routes_each_type_to_its_namespace() {
        assert_eq!(page("system", "standard", "guide").url_path(), "/guide");
        assert_eq!(
            page("custom", "standard", "about").url_path(),
            "/pages/about"
        );
        assert_eq!(
            page("custom", "landing", "spring-sale").url_path(),
            "/lp/spring-sale"
        );
    }

    #[test]
    fn type_label_names_landing_pages_and_keeps_kind_labels() {
        assert_eq!(
            page("custom", "landing", "x").type_label(),
            "ランディングページ"
        );
        assert_eq!(page("system", "standard", "x").type_label(), "固定ページ");
        assert_eq!(page("custom", "standard", "x").type_label(), "自由ページ");
    }
}
