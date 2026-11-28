//! MCP OAuth gate: validates externally issued access
//! tokens on the `/mcp` route and hands the tool layer a
//! proven identity.
//!
//! The gate is deliberately thin. It runs only on the MCP
//! route, and only for a bearer token that is JWT-shaped
//! while OAuth is configured: the token is validated by
//! [`mcp_oauth_service`], and on success its subject and
//! scopes are stamped onto the request as the internal
//! headers below, which the transport carries into the
//! tool handlers' request parts. Everything else — no
//! header, an `kmcp_` API key, OAuth inactive — passes
//! through untouched, so the API-key behaviour of
//! Phases 2–3 is exactly what it was.
//!
//! The internal headers are stripped from every incoming
//! request first, unconditionally: they are this gate's
//! output, never a client's input, so a client that sends
//! them itself is treated as if it had not.

use axum::extract::Request;
use axum::http::{HeaderValue, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

use crate::api::mcp::{AUTH_ERROR_CODE, AUTH_ERROR_MESSAGE};
use crate::config::CONFIG;
use crate::services::api_key_service::TOKEN_MARKER;
use crate::services::mcp_oauth_service;

/// The internal header carrying a validated token's
/// subject to the tool layer. Set by this gate only,
/// after validation; stripped from every request on the
/// way in.
pub const OAUTH_SUBJECT_HEADER: &str = "x-mcp-oauth-subject";

/// The internal header carrying a validated token's
/// scope list (space-separated, as granted) to the tool
/// layer. Set and stripped with [`OAUTH_SUBJECT_HEADER`].
pub const OAUTH_SCOPES_HEADER: &str = "x-mcp-oauth-scopes";

/// Middleware: validates a JWT-shaped bearer token when
/// OAuth is active, short-circuiting an invalid one with
/// the RFC 9728 challenge; passes everything else
/// through with the internal headers stripped.
pub async fn gate(req: Request, next: Next) -> Response {
    let (mut parts, body) = req.into_parts();
    // Stripped first, always: a client-supplied identity
    // header must never survive into the tool layer.
    parts.headers.remove(OAUTH_SUBJECT_HEADER);
    parts.headers.remove(OAUTH_SCOPES_HEADER);

    let bearer = parts
        .headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "));
    if let Some(token) = bearer
        && is_jwt_shaped(token)
        && CONFIG.mcp_oauth_active()
    {
        match mcp_oauth_service::validate_access_token(token).await {
            Ok(identity) => {
                // The validated identity rides internal
                // headers into the transport's request
                // parts. A subject that cannot be a header
                // value is not a subject this gate passes:
                // fail closed, as if validation had failed.
                let subject = HeaderValue::from_str(&identity.subject);
                let scopes = HeaderValue::from_str(&identity.scopes.join(" "));
                match (subject, scopes) {
                    (Ok(subject), Ok(scopes)) => {
                        parts.headers.insert(OAUTH_SUBJECT_HEADER, subject);
                        parts.headers.insert(OAUTH_SCOPES_HEADER, scopes);
                    }
                    _ => return unauthorized(&parts),
                }
            }
            Err(()) => return unauthorized(&parts),
        }
    }
    next.run(Request::from_parts(parts, body)).await
}

/// Whether a bearer token has the JWT shape — exactly two
/// segment separators — and is not one of our API keys.
/// API keys are the tool layer's business; this gate only
/// ever looks at tokens that claim to be JWTs.
fn is_jwt_shaped(token: &str) -> bool {
    !token.starts_with(TOKEN_MARKER) && token.matches('.').count() == 2
}

/// The 401 for an invalid access token: the RFC 9728
/// `WWW-Authenticate` challenge pointing at this store's
/// protected-resource metadata, and a JSON-RPC error body
/// carrying the tool layer's one authentication message —
/// the same words a bad API key earns, so the two
/// credential kinds refuse identically.
fn unauthorized(parts: &axum::http::request::Parts) -> Response {
    let resource_metadata = format!(
        "{}/.well-known/oauth-protected-resource",
        public_base(parts)
    );
    let challenge =
        format!("Bearer error=\"invalid_token\", resource_metadata=\"{resource_metadata}\"");
    (
        StatusCode::UNAUTHORIZED,
        [(header::WWW_AUTHENTICATE, challenge)],
        axum::Json(serde_json::json!({
            "jsonrpc": "2.0",
            "id": null,
            "error": { "code": AUTH_ERROR_CODE, "message": AUTH_ERROR_MESSAGE },
        })),
    )
        .into_response()
}

/// The store's public base for the challenge URL: the
/// configured `PUBLIC_BASE_URL` when one exists, otherwise
/// the origin this request arrived on (the sitemap's
/// base rule, so every public document names the store
/// the same way).
fn public_base(parts: &axum::http::request::Parts) -> String {
    if let Some(base) = CONFIG.public_base_url() {
        return base.trim_end_matches('/').to_string();
    }
    let scheme = parts
        .headers
        .get("x-forwarded-proto")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("http");
    let host = parts
        .headers
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
        .map(ToString::to_string)
        .or_else(|| parts.uri.authority().map(|authority| authority.to_string()))
        .unwrap_or_else(|| "localhost".to_string());
    format!("{scheme}://{host}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jwt_shape_is_two_separators_and_not_a_key() {
        assert!(is_jwt_shaped("aaa.bbb.ccc"));
        assert!(!is_jwt_shaped("aaa.bbb"));
        assert!(!is_jwt_shaped("aaa.bbb.ccc.ddd"));
        assert!(!is_jwt_shaped("kmcp_abcdef012345_abcd"));
        assert!(!is_jwt_shaped("plain-token"));
    }
}
