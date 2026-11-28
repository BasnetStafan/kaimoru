//! Blog controller (CMS blog spec, docs/specs/CMS_SPEC.md).
//!
//! The shop's articles: the index, one post, the category
//! archives. Every read goes through the blog service's
//! liveness rule, so a draft, a scheduled post and an
//! unknown slug answer the same styled 404. The RSS feed
//! handler lives here too, but its router mounts with the
//! crawler documents in routes.rs (no session, no CSRF),
//! like the sitemap.

use axum::{
    Router,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
    routing::get,
};
use serde::Deserialize;
use sqlx::PgPool;
use tower_sessions::Session;

use super::{header_for, render};
use crate::models::PostView;
use crate::services::blog_service;
use crate::util::pagination::total_pages;
use crate::views::storefront::blog::{BlogCard, BlogIndexView, BlogPostView};
use crate::views::storefront::errors::PageNotFoundView;

/// Builds the blog router.
pub fn router() -> Router<PgPool> {
    Router::new()
        .route("/blog", get(index))
        .route("/blog/category/{slug}", get(category_archive))
        .route("/blog/{slug}", get(post_page))
}

/// List query: the page number only — the blog has no
/// other knobs.
#[derive(Deserialize, Default)]
pub struct BlogQuery {
    /// Page number.
    page: Option<String>,
}

/// Shapes one live post into an index card.
fn to_card(view: &PostView) -> BlogCard {
    BlogCard {
        url: view.post.url_path(),
        cover_image_path: view.post.cover_thumb_path(),
        date_label: view.post.published_long_label().unwrap_or_default(),
        category_name: view.category_name.clone(),
        category_url: view
            .category_slug
            .as_ref()
            .map(|slug| format!("/blog/category/{slug}")),
        title: view.post.title.clone(),
        excerpt: view.post.excerpt_or_derived(),
    }
}

/// The shared index tail: load one page of live posts and
/// render the index template for it (the archive passes
/// its category; the plain index passes `None` twice).
async fn index_response(
    pool: &PgPool,
    session: &Session,
    category_id: Option<i64>,
    category_name: Option<String>,
    page_base: String,
    query: &BlogQuery,
) -> Response {
    let header = header_for(pool, session, false).await;
    let per_page = crate::config::CONFIG.search_per_page;
    let page_no: i64 = query
        .page
        .as_deref()
        .and_then(|raw| raw.parse().ok())
        .unwrap_or(1)
        .max(1);
    let offset = page_no.saturating_sub(1).saturating_mul(per_page);
    match blog_service::live_posts(pool, category_id, per_page, offset).await {
        Ok((views, total)) => {
            let pages = total_pages(total, per_page).max(1);
            render(BlogIndexView {
                header,
                category_name,
                cards: views.iter().map(to_card).collect(),
                total,
                page: page_no.min(pages),
                total_pages: pages,
                page_base,
            })
        }
        Err(e) => {
            tracing::error!("blog index load failed: {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, "Internal Server Error").into_response()
        }
    }
}

/// GET /blog: the index of live posts, newest first.
async fn index(
    session: Session,
    State(pool): State<PgPool>,
    Query(query): Query<BlogQuery>,
) -> Response {
    index_response(&pool, &session, None, None, "/blog?".to_string(), &query).await
}

/// GET /blog/category/{slug}: one category's archive. An
/// unknown category answers the styled 404; a known one
/// with no live posts renders the empty archive.
async fn category_archive(
    session: Session,
    State(pool): State<PgPool>,
    Path(slug): Path<String>,
    Query(query): Query<BlogQuery>,
) -> Response {
    let category = match blog_service::category_by_slug(&pool, &slug).await {
        Ok(Some(category)) => category,
        Ok(None) => return not_found(&pool, &session).await,
        Err(e) => {
            tracing::error!("blog category load failed: {e}");
            return (StatusCode::INTERNAL_SERVER_ERROR, "Internal Server Error").into_response();
        }
    };
    let base = format!("/blog/category/{}?", category.slug);
    index_response(
        &pool,
        &session,
        Some(category.id),
        Some(category.name.clone()),
        base,
        &query,
    )
    .await
}

/// GET /blog/{slug}: one live post. Draft, scheduled and
/// unknown slugs answer the same styled 404.
async fn post_page(
    session: Session,
    State(pool): State<PgPool>,
    Path(slug): Path<String>,
) -> Response {
    let loaded = blog_service::live_post(&pool, &slug).await;
    let header = header_for(&pool, &session, false).await;
    match loaded {
        Ok(Some(view)) => {
            // The meta description's fallback chain: the
            // post's own, else its excerpt (stored or
            // derived), else the shop default the header
            // already carries.
            let meta_description = if !view.post.meta_description.trim().is_empty() {
                view.post.meta_description.clone()
            } else {
                let excerpt = view.post.excerpt_or_derived();
                if excerpt.is_empty() {
                    header.shop.meta_description().to_string()
                } else {
                    excerpt
                }
            };
            render(BlogPostView {
                header,
                title: view.post.title.clone(),
                date_label: view.post.published_long_label().unwrap_or_default(),
                author_name: view.author_name.clone(),
                category_name: view.category_name.clone(),
                category_url: view
                    .category_slug
                    .as_ref()
                    .map(|slug| format!("/blog/category/{slug}")),
                cover_image_path: view.post.cover_medium_path(),
                body_html: blog_service::render_body(&view.post),
                meta_description,
            })
        }
        Ok(None) => {
            let view = PageNotFoundView { header };
            (StatusCode::NOT_FOUND, render(view)).into_response()
        }
        Err(e) => {
            tracing::error!("blog post load failed: {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, "Internal Server Error").into_response()
        }
    }
}

/// The styled 404 response (the archive's unknown-category
/// tail).
async fn not_found(pool: &PgPool, session: &Session) -> Response {
    let header = header_for(pool, session, false).await;
    let view = PageNotFoundView { header };
    (StatusCode::NOT_FOUND, render(view)).into_response()
}

/// Builds the feed router: the one blog route that does
/// NOT ride the page stack. routes.rs merges it into the
/// crawler-documents mount (no session, no CSRF, the
/// maintenance gate), like the sitemap it resembles.
pub fn feed_router() -> Router<PgPool> {
    Router::new().route("/blog/feed.xml", get(feed))
}

/// GET /blog/feed.xml: the RSS 2.0 feed of the newest live
/// posts. An hour of shared caching, like the sitemap's:
/// the document changes only when a post does.
async fn feed(State(pool): State<PgPool>, headers: HeaderMap) -> Response {
    let base = super::sitemap::base_url(&headers);
    match blog_service::feed_xml(&pool, &base).await {
        Ok(xml) => (
            StatusCode::OK,
            [
                (header::CONTENT_TYPE, "application/rss+xml; charset=utf-8"),
                (header::CACHE_CONTROL, "public, max-age=3600"),
            ],
            xml,
        )
            .into_response(),
        Err(e) => {
            tracing::error!("blog feed generation failed: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}
