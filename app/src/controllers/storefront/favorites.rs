//! Favourites controller.
//! The heart toggle endpoint and the member's favourites page.
//! Member-only throughout: the [`CurrentUser`] extractor has
//! already bounced guests to the login page before a handler
//! runs, and the templates render guests a login link instead
//! of this endpoint's form, so a guest never posts here.

use axum::{
    Router,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde::Deserialize;
use sqlx::PgPool;
use tower_sessions::Session;

use super::{header_for, render};
use crate::error::AppError;
use crate::services::favorite_service;
use crate::session::CurrentUser;
use crate::views::storefront::favorites::{FavoriteToggleView, FavoritesPageView};

/// Routes owned by the favourites controller.
pub fn router() -> Router<PgPool> {
    Router::new()
        .route("/favorites/{product_id}/toggle", post(toggle))
        .route("/mypage/favorites", get(favorites_page))
}

/// Form posted by a toggle: only the presentation flag rides
/// along, so the swap answer matches the surface it came from
/// (the labelled buybox button or the icon-only card overlay).
#[derive(Deserialize)]
pub struct ToggleForm {
    /// "1" from the buybox form; absent from card forms.
    #[serde(default)]
    labelled: Option<String>,
}

/// POST /favorites/{product_id}/toggle: flips the member's
/// mark and answers with the toggle fragment, which htmx
/// swaps in place of the submitted form.
async fn toggle(
    CurrentUser(user): CurrentUser,
    State(pool): State<PgPool>,
    Path(raw_id): Path<String>,
    axum::Form(form): axum::Form<ToggleForm>,
) -> Response {
    let labelled = form.labelled.as_deref() == Some("1");
    let Ok(product_id) = raw_id.parse::<i64>() else {
        return (StatusCode::NOT_FOUND, "Not Found").into_response();
    };
    match favorite_service::toggle(&pool, user.id, product_id).await {
        Ok(is_favorite) => render(FavoriteToggleView {
            product_id,
            is_favorite,
            labelled,
            member: true,
        }),
        Err(AppError::NotFound) => (StatusCode::NOT_FOUND, "Not Found").into_response(),
        Err(e) => {
            tracing::error!(
                user_id = user.id,
                product_id,
                "favourite toggle failed: {e}"
            );
            (StatusCode::INTERNAL_SERVER_ERROR, "Internal Server Error").into_response()
        }
    }
}

/// GET /mypage/favorites: the member's favourited products as
/// cards, newest mark first.
async fn favorites_page(
    CurrentUser(user): CurrentUser,
    session: Session,
    State(pool): State<PgPool>,
) -> Response {
    let products = match favorite_service::favorite_products(&pool, user.id).await {
        Ok(products) => products,
        Err(e) => {
            tracing::error!(user_id = user.id, "favourites page failed: {e}");
            return (StatusCode::INTERNAL_SERVER_ERROR, "Internal Server Error").into_response();
        }
    };
    let cards = super::product_cards(&pool, Some(user.id), products).await;
    render(FavoritesPageView {
        header: header_for(&pool, &session, false).await,
        cards,
    })
}
