//! The postal code lookup endpoint the storefront's address
//! forms call: a 7-digit code in, the addresses it covers
//! out.
//!
//! The endpoint is public and read-only: the data is Japan
//! Post's published reference data, and it holds no
//! per-visitor state, so it mounts with no session stack.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use serde::Serialize;
use sqlx::PgPool;

use super::api_error;

use crate::models::PostalAddress;
use crate::services::postal_code_service;

/// One address in the lookup response.
#[derive(Serialize)]
struct AddressJson {
    /// Prefecture name, e.g. 東京都.
    prefecture: String,
    /// City, ward, town or village name, e.g. 千代田区.
    city: String,
    /// Town name; empty when the code resolves at city level.
    town: String,
}

impl From<PostalAddress> for AddressJson {
    fn from(address: PostalAddress) -> Self {
        Self {
            prefecture: address.prefecture,
            city: address.city,
            town: address.town,
        }
    }
}

/// The lookup response: the code echoed back, plus one entry
/// per town the code covers.
#[derive(Serialize)]
struct LookupJson {
    /// The 7-digit postal code that was looked up.
    postal_code: String,
    /// Every address the code resolves to.
    addresses: Vec<AddressJson>,
}

/// `GET /api/postal-codes/{code}` — the addresses one code covers.
pub async fn lookup(State(pool): State<PgPool>, Path(code): Path<String>) -> Response {
    if !postal_code_service::is_valid_postal_code(&code) {
        return api_error(StatusCode::BAD_REQUEST, "invalid_postal_code");
    }
    match postal_code_service::lookup(&pool, &code).await {
        Ok(addresses) if !addresses.is_empty() => Json(LookupJson {
            postal_code: code,
            addresses: addresses.into_iter().map(AddressJson::from).collect(),
        })
        .into_response(),
        Ok(_) => api_error(StatusCode::NOT_FOUND, "address_not_found"),
        Err(e) => {
            tracing::error!("postal lookup {code} failed: {e}");
            api_error(StatusCode::INTERNAL_SERVER_ERROR, "internal_error")
        }
    }
}

/// Routes for the postal code lookup.
pub fn router() -> Router<PgPool> {
    Router::new().route("/api/postal-codes/{code}", get(lookup))
}
