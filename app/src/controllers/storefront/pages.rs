//! Info controller.
//! The long-form pages linked from the footer — guide, terms,
//! privacy, the legal notice and FAQ — plus operator-created
//! pages under `/pages/{slug}` (CMS spec C-2). All of them load
//! from `content_pages` and render through one template; a
//! draft or unknown slug answers the styled 404,
//! indistinguishably. System pages resolve only at the short
//! URLs above: under `/pages/` they answer the same 404, so
//! no page ever has two URLs. Only `/withdrawn` stays a
//! static template: it is the farewell page of the withdrawal
//! flow, not shop content.

use axum::{
    Router,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::get,
};
use sqlx::PgPool;
use tower_sessions::Session;

use super::{header_for, render};
use crate::error::AppError;
use crate::models::ContentPage;
use crate::services::content_page_service;
use crate::views::storefront::content_page::ContentView;
use crate::views::storefront::errors::PageNotFoundView;
use crate::views::storefront::withdrawn::WithdrawnView;

/// Builds the info router: the DB-backed long-form pages and
/// the one static farewell page.
pub fn router() -> Router<PgPool> {
    Router::new()
        .route("/guide", get(guide_page))
        .route("/terms", get(terms_page))
        .route("/privacy", get(privacy_page))
        .route("/legal", get(legal_page))
        .route("/faq", get(faq_page))
        .route("/pages/{slug}", get(custom_page))
        .route("/withdrawn", get(withdrawn_page))
}

/// GET /guide: shipping fees and payment methods.
async fn guide_page(session: Session, State(pool): State<PgPool>) -> Response {
    content_response(&pool, &session, "guide").await
}

/// GET /terms: the terms of service.
async fn terms_page(session: Session, State(pool): State<PgPool>) -> Response {
    content_response(&pool, &session, "terms").await
}

/// GET /privacy: the privacy policy.
async fn privacy_page(session: Session, State(pool): State<PgPool>) -> Response {
    content_response(&pool, &session, "privacy").await
}

/// GET /legal: the 特定商取引法 notice.
async fn legal_page(session: Session, State(pool): State<PgPool>) -> Response {
    content_response(&pool, &session, "legal").await
}

/// GET /faq: the FAQ page, once the operator publishes the
/// seeded draft; until then it 404s like any draft.
async fn faq_page(session: Session, State(pool): State<PgPool>) -> Response {
    content_response(&pool, &session, "faq").await
}

/// GET /pages/{slug}: an operator-created page — custom
/// pages only, so a system slug here answers the styled 404,
/// never the page a second time.
async fn custom_page(
    session: Session,
    State(pool): State<PgPool>,
    Path(slug): Path<String>,
) -> Response {
    custom_content_response(&pool, &session, &slug).await
}

/// The system pages' tail: load the published page by slug.
async fn content_response(pool: &PgPool, session: &Session, slug: &str) -> Response {
    let page = content_page_service::published_page(pool, slug).await;
    page_response(pool, session, page).await
}

/// The `/pages/{slug}` tail: the same, through the
/// custom-pages-only lookup.
async fn custom_content_response(pool: &PgPool, session: &Session, slug: &str) -> Response {
    let page = content_page_service::published_custom_page(pool, slug).await;
    page_response(pool, session, page).await
}

/// Renders a loaded page, or answers the styled 404 — the
/// same response for an unknown slug, a draft and a system
/// page under `/pages/`, so neither the page's existence nor
/// its state leaks.
async fn page_response(
    pool: &PgPool,
    session: &Session,
    page: Result<Option<ContentPage>, AppError>,
) -> Response {
    let header = header_for(pool, session, false).await;
    match page {
        Ok(Some(page)) => render(ContentView {
            header,
            title: page.title.clone(),
            body_html: content_page_service::render_body(&page),
            updated_label: Some(page.updated_long_label()),
        }),
        Ok(None) => {
            let view = PageNotFoundView { header };
            (StatusCode::NOT_FOUND, render(view)).into_response()
        }
        Err(e) => {
            tracing::error!("content page load failed: {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, "Internal Server Error").into_response()
        }
    }
}

/// GET /withdrawn: the public farewell page shown after a
/// member withdrawal completes. Static by design — it carries
/// no per-member data and is not operator content.
async fn withdrawn_page(session: Session, State(pool): State<PgPool>) -> Response {
    render(WithdrawnView {
        header: header_for(&pool, &session, false).await,
    })
}
