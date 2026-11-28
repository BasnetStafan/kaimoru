//! OAuth 2.0 protected-resource metadata (RFC 9728) for the
//! MCP server — Phase 4.
//!
//! An MCP client that receives a 401 from `/mcp` follows the
//! `resource_metadata` pointer in the challenge to this
//! document to learn which authorization server issues the
//! tokens this store accepts, and which scopes exist. The
//! document is served only while OAuth is configured (both
//! `MCP_OAUTH_ISSUER` and `MCP_OAUTH_AUDIENCE` set); while
//! inactive both paths answer 404, so a deployment that
//! never enabled OAuth does not advertise an authorization
//! server it does not use.
//!
//! Mounted with the crawler documents in routes.rs: a
//! public, stateless document — no session stack, no CSRF
//! layer — under the maintenance gate, logged on the
//! storefront surface.

use axum::Router;
use axum::http::HeaderMap;
use axum::response::{IntoResponse, Json, Response};
use axum::routing::get;
use serde_json::json;
use sqlx::PgPool;

use crate::config::CONFIG;
use crate::services::api_key_service;

/// The resource name the metadata advertises.
const RESOURCE_NAME: &str = "Kaimoru MCP";

/// The protected-resource metadata routes: the bare
/// well-known path and the resource-suffixed form the MCP
/// authorization specification points clients at. Both
/// serve the same document.
pub fn router() -> Router<PgPool> {
    Router::new()
        .route("/.well-known/oauth-protected-resource", get(metadata))
        .route("/.well-known/oauth-protected-resource/mcp", get(metadata))
}

/// Serves the metadata document, or 404 while OAuth is
/// inactive. The resource URL and the authorization server
/// come from the deployment's own configuration, so the
/// document can never point at a server the resource
/// server would not accept tokens from.
async fn metadata(headers: HeaderMap) -> Response {
    let Some(issuer) = CONFIG.mcp_oauth_issuer() else {
        return axum::http::StatusCode::NOT_FOUND.into_response();
    };
    if CONFIG.mcp_oauth_audience().is_none() {
        return axum::http::StatusCode::NOT_FOUND.into_response();
    }
    let base = super::sitemap::base_url(&headers);
    Json(json!({
        "resource": format!("{base}/mcp"),
        "authorization_servers": [issuer],
        "scopes_supported": api_key_service::SCOPES,
        "bearer_methods_supported": ["header"],
        "resource_name": RESOURCE_NAME,
    }))
    .into_response()
}
