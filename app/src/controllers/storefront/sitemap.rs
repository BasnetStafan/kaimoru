//! Sitemap and robots.txt controller.
//! Serves the two crawler documents. They mount outside the
//! page stack (see routes.rs): crawlers carry no session, and
//! a sitemap fetch must not create one.

use axum::{
    Router,
    extract::State,
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
    routing::get,
};
use sqlx::PgPool;

use crate::config::CONFIG;
use crate::services::sitemap_service;

/// Both documents change only when the catalog does, so an
/// hour of shared caching keeps crawler traffic off the
/// database without letting the documents go stale.
const CRAWLER_CACHE: &str = "public, max-age=3600";

/// Routes owned by the sitemap controller.
pub fn router() -> Router<PgPool> {
    Router::new()
        .route("/sitemap.xml", get(sitemap_xml))
        .route("/robots.txt", get(robots_txt))
}

/// Handles GET /sitemap.xml with the sitemap for the store's
/// public base URL.
async fn sitemap_xml(State(pool): State<PgPool>, headers: HeaderMap) -> Response {
    let base = base_url(&headers);
    match sitemap_service::sitemap_xml(&pool, &base).await {
        Ok(xml) => (
            StatusCode::OK,
            [
                (header::CONTENT_TYPE, "application/xml; charset=utf-8"),
                (header::CACHE_CONTROL, CRAWLER_CACHE),
            ],
            xml,
        )
            .into_response(),
        Err(e) => {
            tracing::error!("sitemap generation failed: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

/// Handles GET /robots.txt; the document is static apart
/// from the base URL, so it cannot fail.
async fn robots_txt(headers: HeaderMap) -> Response {
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "text/plain; charset=utf-8"),
            (header::CACHE_CONTROL, CRAWLER_CACHE),
        ],
        sitemap_service::render_robots(&base_url(&headers)),
    )
        .into_response()
}

/// The store's public base URL: the configured
/// `PUBLIC_BASE_URL` when set (the deployed store, and the
/// same value the payment return URLs use), else the origin
/// this request arrived on, so a local server advertises
/// URLs that resolve to itself.
fn base_url(headers: &HeaderMap) -> String {
    if let Some(base) = CONFIG.public_base_url() {
        return base.trim_end_matches('/').to_string();
    }
    let scheme = headers
        .get("x-forwarded-proto")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("http");
    let host = headers
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("localhost");
    format!("{scheme}://{host}")
}
