//! The agent API's catalog endpoints: the public product
//! list and product detail as JSON.

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use serde::Deserialize;
use sqlx::PgPool;

use crate::services::api_catalog_service;

use super::{api_error, internal_error};

/// Query parameters for the catalog list.
#[derive(Deserialize)]
pub struct ListParams {
    /// 1-based page number; absent means page 1.
    pub page: Option<i64>,
}

/// The 404 every unknown or non-public product id answers:
/// the API does not reveal which ids exist as drafts.
fn not_found() -> Response {
    api_error(StatusCode::NOT_FOUND, "product_not_found")
}

/// `GET /api/products` — one page of the public catalog.
pub async fn list_products(
    State(pool): State<PgPool>,
    Query(params): Query<ListParams>,
) -> Response {
    match api_catalog_service::list_products(&pool, params.page.unwrap_or(1)).await {
        Ok(page) => Json(page).into_response(),
        Err(e) => {
            tracing::error!("catalog list failed: {e}");
            internal_error()
        }
    }
}

/// `GET /api/products/{id}` — one public product.
pub async fn get_product(State(pool): State<PgPool>, Path(raw_id): Path<String>) -> Response {
    // Parsed by hand (the app-wide convention) so a malformed
    // id answers the API's JSON 404, not a framework 400.
    let Ok(id) = raw_id.parse::<i64>() else {
        return not_found();
    };
    match api_catalog_service::get_product(&pool, id).await {
        Ok(Some(product)) => Json(product).into_response(),
        Ok(None) => not_found(),
        Err(e) => {
            tracing::error!("catalog product {id} failed: {e}");
            internal_error()
        }
    }
}

/// Routes for the catalog endpoints.
pub fn router() -> Router<PgPool> {
    Router::new()
        .route("/api/products", get(list_products))
        .route("/api/products/{id}", get(get_product))
}
