//! Machine-facing endpoints that are not pages: the payment
//! webhook receivers and the agent-readable API. These routers
//! carry no session stack and no CSRF layer — the webhook's
//! caller authenticates by signature, and the agent API's read
//! endpoints are public and hold no per-visitor state.

pub mod cart;
pub mod catalog;
pub mod komoju_webhook;
pub mod mail_tracking;
pub mod postal_codes;
pub mod stripe_webhook;
pub mod ucp_profile;

use axum::Router;
use sqlx::PgPool;

/// Combines the provider-facing routers (webhook receivers).
pub fn router() -> Router<PgPool> {
    Router::new()
        .merge(komoju_webhook::router())
        .merge(stripe_webhook::router())
}

/// The agent-readable API: public discovery and catalog reads
/// for software agents.
pub fn agent_router() -> Router<PgPool> {
    Router::new()
        .merge(ucp_profile::router())
        .merge(catalog::router())
}

/// The campaign open/click tracking endpoints. Public
/// and token-keyed; [`crate::routes`] mounts them with
/// no session stack and outside the maintenance gate.
pub fn tracking_router() -> Router<PgPool> {
    Router::new().merge(mail_tracking::router())
}

/// The postal code lookup the storefront's address forms
/// call. Public reference data, no per-visitor state.
pub fn postal_router() -> Router<PgPool> {
    Router::new().merge(postal_codes::router())
}

/// The agent cart API. Unlike the read endpoints above it
/// carries per-visitor state, so [`crate::routes`] assembles
/// it with the member session layer (but no CSRF layer).
pub fn cart_router() -> Router<PgPool> {
    Router::new().merge(cart::router())
}

/// A machine-readable error body: agents and callers branch
/// on the `error` code, never on a rendered page. One shared
/// shape for every machine-facing module in this tree.
#[derive(serde::Serialize)]
pub(crate) struct ApiError {
    /// Stable machine-readable error code.
    error: &'static str,
}

/// Answers an error status with its stable code.
pub(crate) fn api_error(
    status: axum::http::StatusCode,
    code: &'static str,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    (status, axum::Json(ApiError { error: code })).into_response()
}

/// Answers a generic 500; the detail stays in the server log.
pub(crate) fn internal_error() -> axum::response::Response {
    api_error(
        axum::http::StatusCode::INTERNAL_SERVER_ERROR,
        "internal_error",
    )
}
