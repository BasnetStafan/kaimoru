//! Category index controller.
//! Serves the full category list behind the top page
//! nav's see-more link: the nav shows the config number
//! of categories, this page shows them all.

use axum::{Router, extract::State, response::Response, routing::get};
use sqlx::PgPool;
use tower_sessions::Session;

use crate::services::product_service;
use crate::views::storefront::category_index::CategoryIndexView;

/// Routes owned by the category index controller.
pub fn router() -> Router<PgPool> {
    Router::new().route("/categories", get(index_handler))
}

/// Handles GET /categories by rendering every navigation
/// category (active, holding public products — the nav's
/// own universe) as a link to its product list.
async fn index_handler(session: Session, State(pool): State<PgPool>) -> Response {
    let header = super::header_for(&pool, &session, false).await;
    let categories = product_service::categories(&pool).await.unwrap_or_default();

    super::render(CategoryIndexView { header, categories })
}
