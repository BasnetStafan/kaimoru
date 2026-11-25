//! The KOMOJU webhook receiver: POST /webhooks/komoju.
//!
//! The discipline (payment spec §6):
//!
//! - The hex HMAC-SHA256 signature over the RAW body is
//!   verified BEFORE the body is parsed. A re-encoded body —
//!   same JSON, different bytes — fails, as it must.
//! - Without a configured webhook secret the route answers
//!   404: an unconfigured integration does not expose the
//!   endpoint at all.
//! - Signature failures and unparseable bodies are 400s:
//!   KOMOJU does not retry those, and neither should be
//!   discoverable as valid by probing.
//! - Everything after verification is the payment service's:
//!   the event is recorded in the ledger keyed by the event id
//!   IN THE BODY (the `X-Komoju-ID` header is a per-delivery
//!   tracing number only — redeliveries mint new ones), then
//!   applied through the one status path. A body with no event
//!   id has no dedupe key: it is a 400 and is never recorded.
//! - A transient failure (the database) is a 500, so KOMOJU
//!   redelivers on its retry schedule; a processed or
//!   deliberately-ignored event is a fast 200.
//!
//! The body is capped: a payment event is a few kilobytes, so
//! 256 KiB is generous and bounds what a flood can make us hold.

use axum::{
    Router,
    body::Bytes,
    extract::{DefaultBodyLimit, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::post,
};
use sqlx::PgPool;

use crate::services::komoju_client;
use crate::services::payment_service;
use crate::util::hmac_util;

/// The largest webhook body accepted.
const MAX_BODY_BYTES: usize = 256 * 1024;

/// Routes owned by the webhook receiver.
pub fn router() -> Router<PgPool> {
    Router::new().route(
        "/webhooks/komoju",
        post(receive).layer(DefaultBodyLimit::max(MAX_BODY_BYTES)),
    )
}

/// Handles one delivery.
async fn receive(State(pool): State<PgPool>, headers: HeaderMap, body: Bytes) -> Response {
    // Unconfigured integrations do not expose the endpoint at
    // all: without a webhook secret there is nothing legitimate
    // to verify against, so the route reads as absent.
    let Some(secret) = komoju_client::settings().webhook_secret() else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let signature = headers
        .get("X-Komoju-Signature")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("");
    if !hmac_util::verify_hmac_sha256_hex(secret.as_bytes(), &body, signature) {
        // Never log the body or the attempted signature.
        tracing::warn!("KOMOJU webhook rejected: signature mismatch");
        return StatusCode::BAD_REQUEST.into_response();
    }
    // The signature is valid: the body may now be parsed.
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(&body) else {
        tracing::warn!("KOMOJU webhook rejected: body is not JSON");
        return StatusCode::BAD_REQUEST.into_response();
    };
    let Some(event_id) = value.get("id").and_then(serde_json::Value::as_str) else {
        // Without the body's event id there is no dedupe key:
        // not an event we can safely process.
        tracing::warn!("KOMOJU webhook rejected: event without id");
        return StatusCode::BAD_REQUEST.into_response();
    };
    let event_type = value
        .get("type")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");
    let delivery_id = headers
        .get("X-Komoju-ID")
        .and_then(|value| value.to_str().ok());
    // `ping` events carry an empty data object; a parse failure
    // here just means "no payment sighting", which the service
    // records and acknowledges like any non-payment event.
    let sighting = value
        .get("data")
        .and_then(|data| komoju_client::parse_payment_object(data).ok());
    match payment_service::handle_webhook_event(
        &pool,
        event_id,
        delivery_id,
        event_type,
        &value,
        sighting.as_ref(),
    )
    .await
    {
        Ok(_) => StatusCode::OK.into_response(),
        Err(e) => {
            // Transient by assumption: KOMOJU redelivers, and
            // the ledger makes the redelivery safe.
            tracing::error!(event_id, "KOMOJU webhook handling failed: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}
