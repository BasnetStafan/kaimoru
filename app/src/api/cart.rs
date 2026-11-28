//! The agent API's cart endpoints: ensure the caller's cart,
//! read it, and add items to it. The handlers sit on the
//! member session (see [`crate::routes`]) — the cart belongs
//! to whoever's session calls, member or guest — and answer
//! JSON only.

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use sqlx::PgPool;
use tower_sessions::Session;

use crate::services::api_cart_service::{self, AddError};

use super::{api_error, internal_error};

/// The body of an add-item request.
#[derive(Deserialize)]
pub struct AddItemRequest {
    /// Product to add; must be in the public catalog.
    pub product_id: i64,
    /// Combination to add. Required when the product has
    /// several; omit for a plain product.
    pub variant_id: Option<i64>,
    /// Units to add, 1..=999,999.
    pub quantity: i32,
}

/// `POST /api/carts` — ensures the caller's cart exists and
/// returns it. Idempotent: calling it again returns the same
/// cart.
pub async fn ensure_cart(State(pool): State<PgPool>, session: Session) -> Response {
    match api_cart_service::ensure_cart(&pool, &session).await {
        Ok(cart) => Json(cart).into_response(),
        Err(e) => {
            tracing::error!("api cart ensure failed: {e}");
            internal_error()
        }
    }
}

/// `GET /api/carts/current` — the caller's cart, read back.
/// Creates nothing: a caller who never added reads an empty
/// cart.
pub async fn get_current(State(pool): State<PgPool>, session: Session) -> Response {
    match api_cart_service::read_cart(&pool, &session).await {
        Ok(cart) => Json(cart).into_response(),
        Err(e) => {
            tracing::error!("api cart read failed: {e}");
            internal_error()
        }
    }
}

/// `POST /api/carts/current/items` — adds units of a product
/// (and combination) to the caller's cart and returns the
/// cart read back, so a capped add shows what actually
/// landed.
pub async fn add_item(
    State(pool): State<PgPool>,
    session: Session,
    Json(request): Json<AddItemRequest>,
) -> Response {
    match api_cart_service::add_item(
        &pool,
        &session,
        request.product_id,
        request.variant_id,
        request.quantity,
    )
    .await
    {
        Ok(cart) => Json(cart).into_response(),
        Err(AddError::ProductNotFound) => api_error(StatusCode::NOT_FOUND, "product_not_found"),
        Err(AddError::VariantNotFound) => api_error(StatusCode::NOT_FOUND, "variant_not_found"),
        Err(AddError::VariantRequired) => api_error(StatusCode::BAD_REQUEST, "variant_required"),
        Err(AddError::InvalidQuantity) => api_error(StatusCode::BAD_REQUEST, "invalid_quantity"),
        Err(AddError::SoldOut) => api_error(StatusCode::CONFLICT, "sold_out"),
        Err(AddError::Internal(e)) => {
            tracing::error!("api cart add failed: {e}");
            internal_error()
        }
    }
}

/// Routes for the cart endpoints.
pub fn router() -> Router<PgPool> {
    Router::new()
        .route("/api/carts", post(ensure_cart))
        .route("/api/carts/current", get(get_current))
        .route("/api/carts/current/items", post(add_item))
}
