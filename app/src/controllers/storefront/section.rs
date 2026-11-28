//! Section listing controller.
//! Serves the full listing behind a top page section's
//! see-more link: the row shows the config count, this
//! page shows everything, paged by the search page's
//! config size — the store's one paging size.

use std::collections::HashMap;

use axum::{
    Router,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::get,
};
use sqlx::PgPool;
use tower_sessions::Session;

use crate::services::{home_content_service, product_service};
use crate::views::storefront::errors::PageNotFoundView;
use crate::views::storefront::section::SectionView;

/// The sections that have a full listing page: the rows
/// whose see-more link leads here.
const LISTABLE_KEYS: [&str; 4] = ["new_arrivals", "ranking", "event", "staff_picks"];

/// Routes owned by the section listing controller.
pub fn router() -> Router<PgPool> {
    Router::new().route("/sections/{key}", get(index_handler))
}

/// Handles GET /sections/{key} by rendering the section's
/// products in its own order, one page at a time. An
/// unknown key is the styled 404, and an out-of-range page
/// clamps to the last one.
async fn index_handler(
    Path(key): Path<String>,
    Query(params): Query<HashMap<String, String>>,
    session: Session,
    State(pool): State<PgPool>,
) -> Response {
    let header = super::header_for(&pool, &session, false).await;
    if !LISTABLE_KEYS.contains(&key.as_str()) {
        return (
            StatusCode::NOT_FOUND,
            super::render(PageNotFoundView { header }),
        )
            .into_response();
    }

    let total = match product_service::section_total(&pool, &key).await {
        Ok(total) => total,
        Err(e) => {
            tracing::error!("Failed to count section listing: {e}");
            return (StatusCode::INTERNAL_SERVER_ERROR, "Internal Server Error").into_response();
        }
    };
    let total_pages =
        (total + product_service::section_page_size() - 1) / product_service::section_page_size();
    let page = params
        .get("page")
        .and_then(|value| value.parse::<i64>().ok())
        .unwrap_or(1)
        .clamp(1, total_pages.max(1));

    let content = home_content_service::load(&pool).await;
    let heading = content
        .sections
        .iter()
        .find(|section| section.section_key == key)
        .map(|section| section.display_heading())
        .unwrap_or_else(|| crate::models::default_heading(&key).to_string());

    let (products, _) = match product_service::section_page(&pool, &key, page, &content.pins).await
    {
        Ok(values) => values,
        Err(e) => {
            tracing::error!("Failed to load section listing: {e}");
            return (StatusCode::INTERNAL_SERVER_ERROR, "Internal Server Error").into_response();
        }
    };
    let viewer = crate::session::current_user(&pool, &session)
        .await
        .map(|user| user.id);
    let products = super::product_cards(&pool, viewer, products).await;

    super::render(SectionView {
        header,
        heading,
        products,
        page,
        total_pages,
        key,
    })
}
