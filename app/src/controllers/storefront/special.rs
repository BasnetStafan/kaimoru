//! Special feature controller.
//! Serves the 特集 landing page: the page the top page's
//! feature banner has always promised with its 特集を見る
//! link, which used to anchor to the new arrivals instead
//! of going anywhere. The page's products are the featured
//! section's pins; while the operator has pinned nothing,
//! the staff picks stand in — the banner's own words are
//! "the products the staff chose", so the page is never a
//! dead end.

use axum::{Router, extract::State, response::Response, routing::get};
use sqlx::PgPool;
use tower_sessions::Session;

use crate::services::{home_content_service, product_service};
use crate::views::storefront::special::SpecialView;

/// Routes owned by the special feature controller.
pub fn router() -> Router<PgPool> {
    Router::new().route("/special", get(index_handler))
}

/// Handles GET /special by loading the featured section's
/// heading and products and rendering the feature page.
async fn index_handler(session: Session, State(pool): State<PgPool>) -> Response {
    let content = home_content_service::load(&pool).await;
    let featured = content
        .sections
        .iter()
        .find(|section| section.section_key == "featured");
    let heading = featured
        .map(|section| section.display_heading())
        .unwrap_or_else(|| crate::models::default_heading("featured").to_string());
    let description = featured
        .map(|section| section.display_description())
        .unwrap_or_else(|| crate::models::default_description("featured").to_string());

    let mut products = product_service::pinned_products(&pool, &content.pins, "featured")
        .await
        .unwrap_or_default();
    if products.is_empty() {
        products = product_service::staff_picks(&pool)
            .await
            .unwrap_or_default();
    }

    let header = super::header_for(&pool, &session, false).await;
    let viewer = crate::session::current_user(&pool, &session)
        .await
        .map(|user| user.id);
    let products = super::product_cards(&pool, viewer, products).await;

    super::render(SpecialView {
        header,
        heading,
        description,
        products,
    })
}
