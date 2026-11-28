//! Storefront content page reads (CMS spec C-2).
//!
//! One read path for every long-form page: the system pages at
//! their historic routes and custom pages under `/pages/`. A
//! draft and an unknown slug return the same `None`, which is
//! what makes their 404s indistinguishable. The
//! `/pages/` read filters to custom pages: a system page lives
//! at its historic route only, never at a duplicate URL.
//!
//! Landing pages (CMS_SPEC §9) are the one content-page type
//! whose bodies expand embed shortcodes: they are campaign
//! surfaces built to sell, so [`render_landing_body`] runs the
//! blog's two-phase embed expansion over the rendered body.
//! Standard pages never pass through it — their shortcodes
//! stay literal text, exactly as before.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

use chrono::{DateTime, Utc};
use sqlx::PgPool;

use crate::error::AppError;
use crate::models::ContentPage;
use crate::repositories::content_page_repository;
use crate::services::blog_service;
use crate::util::markdown;
use crate::util::shortcode;

/// Returns the published page for a slug, or `None` when the
/// slug is unknown or the page is still a draft.
pub async fn published_page(pool: &PgPool, slug: &str) -> Result<Option<ContentPage>, AppError> {
    content_page_repository::find_published_by_slug(pool, slug).await
}

/// Returns the published custom page for a slug, or `None`
/// when the slug is unknown, still a draft, or names a system
/// page — `/pages/{slug}` resolves operator-created pages only.
pub async fn published_custom_page(
    pool: &PgPool,
    slug: &str,
) -> Result<Option<ContentPage>, AppError> {
    content_page_repository::find_published_custom_by_slug(pool, slug).await
}

/// Returns the published landing page for a slug, or `None`
/// when the slug is unknown, still a draft, or names a
/// standard page — `/lp/{slug}` resolves landing pages only,
/// and the identical `None` keeps their 404s
/// indistinguishable from a standard page's.
pub async fn published_landing_page(
    pool: &PgPool,
    slug: &str,
) -> Result<Option<ContentPage>, AppError> {
    content_page_repository::find_published_landing_by_slug(pool, slug).await
}

/// Rendered bodies by page ID, each entry stamped with the
/// `updated_at` it was rendered from. Every write path stamps
/// a fresh `updated_at`, so an entry matches only while the
/// row is unchanged — a save invalidates exactly, with no TTL
/// and nothing to clear by hand.
/// One cached rendering: the `updated_at` of the row it
/// was rendered from and the HTML itself.
type RenderEntry = (DateTime<Utc>, String);

static RENDER_CACHE: LazyLock<Mutex<HashMap<i64, RenderEntry>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Renders a page's body to HTML, served from the render
/// cache while the row is unchanged. A Markdown body goes
/// through [`crate::util::markdown`]; an HTML body is served as
/// stored — the save path sanitised it, and that invariant is
/// what makes serving it unescaped safe. The admin preview is
/// not a caller — it renders the operator's unsaved body
/// directly — so unsaved text is never cached and never
/// served from here.
pub fn render_body(page: &ContentPage) -> String {
    if let Ok(cache) = RENDER_CACHE.lock()
        && let Some((rendered_from, html)) = cache.get(&page.id)
        && *rendered_from == page.updated_at
    {
        return html.clone();
    }
    let html = if page.is_html() {
        page.body_md.clone()
    } else {
        markdown::render_markdown(&page.body_md)
    };
    if let Ok(mut cache) = RENDER_CACHE.lock() {
        cache.insert(page.id, (page.updated_at, html.clone()));
    }
    html
}

/// Renders a landing page's body with its embed shortcodes
/// expanded (CMS_SPEC §9): the cached standard render first
/// (no product data, no form — see [`render_body`]), then
/// token marking, then the blog service's per-request
/// expansion, which splices the catalogue cards in. The
/// two-phase split is the blog's own (§8): the cache stays
/// session-free, and the CSRF middleware gives each
/// visitor's forms their own token on the way out.
pub async fn render_landing_body(pool: &PgPool, page: &ContentPage) -> String {
    let marked = shortcode::mark_embeds(&render_body(page));
    blog_service::expand_embeds(pool, &marked).await
}
