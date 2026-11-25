//! The Stripe webhook receiver: POST /webhooks/stripe.
//!
//! The same discipline as the KOMOJU receiver, with Stripe's
//! signature shape:
//!
//! - The `Stripe-Signature` header carries `t=<unix>,v1=<hex
//!   HMAC-SHA256>` over `"<t>.<raw body>"`, verified BEFORE the
//!   body is parsed, with Stripe's 5-minute timestamp tolerance
//!   as replay protection. Any one matching `v1` value accepts
//!   (Stripe sends several during secret rotation).
//! - Without a configured signing secret the route answers
//!   404: an unconfigured integration does not expose the
//!   endpoint at all.
//! - Signature failures and unparseable bodies are 400s.
//! - The event `id` in the body is the ledger dedupe key,
//!   exactly as for KOMOJU; a body with no event id is a 400.
//! - A sighting is NEVER built from the payload: the event
//!   names an object, and the handler fetches that object's
//!   truth from the API before applying it. A bare
//!   PaymentIntent payload carries no refunded total, and a
//!   payload-built sighting could regress a refund to zero.
//!   A failed fetch is a 500, so Stripe redelivers.
//! - A transient failure (the database) is a 500; a processed
//!   or deliberately-ignored event is a fast 200.
//!
//! The body is capped at 256 KiB, like the KOMOJU receiver.

use axum::{
    Router,
    body::Bytes,
    extract::{DefaultBodyLimit, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::post,
};
use sqlx::PgPool;

use crate::services::payment_gateway::{GatewayError, PaymentGateway, ProviderPayment};
use crate::services::payment_service;
use crate::services::stripe_client;
use crate::util::hmac_util;

/// The largest webhook body accepted.
const MAX_BODY_BYTES: usize = 256 * 1024;

/// Stripe's replay tolerance for the signature timestamp.
const SIGNATURE_TOLERANCE_SECS: i64 = 300;

/// Routes owned by the webhook receiver.
pub fn router() -> Router<PgPool> {
    Router::new().route(
        "/webhooks/stripe",
        post(receive).layer(DefaultBodyLimit::max(MAX_BODY_BYTES)),
    )
}

/// Verifies one `Stripe-Signature` header against the raw
/// body: timestamp inside the tolerance window and at least
/// one `v1` value matching the HMAC of `"<t>.<body>"`.
fn signature_valid(secret: &str, header: &str, body: &[u8], now_unix: i64) -> bool {
    let mut timestamp: Option<i64> = None;
    let mut candidates: Vec<&str> = Vec::new();
    for part in header.split(',') {
        let part = part.trim();
        if let Some(value) = part.strip_prefix("t=") {
            timestamp = value.parse::<i64>().ok();
        } else if let Some(value) = part.strip_prefix("v1=") {
            candidates.push(value);
        }
    }
    let Some(timestamp) = timestamp else {
        return false;
    };
    if (now_unix - timestamp).abs() > SIGNATURE_TOLERANCE_SECS {
        return false;
    }
    let mut payload = timestamp.to_string().into_bytes();
    payload.push(b'.');
    payload.extend_from_slice(body);
    candidates
        .iter()
        .any(|candidate| hmac_util::verify_hmac_sha256_hex(secret.as_bytes(), &payload, candidate))
}

/// Fetches the provider truth one event points at. Events
/// about objects we do not track resolve to `None` (recorded
/// and acknowledged); a fetch failure is the caller's 500.
///
/// `payment_failed` events are translated to the state
/// machine's `failed` word when the intent rests back at
/// `requires_payment_method`: the event itself is the failure
/// signal, and the intent's resting word alone would read as
/// a still-open payment.
async fn fetch_sighting(
    gateway: &std::sync::Arc<dyn PaymentGateway>,
    event_type: &str,
    value: &serde_json::Value,
) -> Result<Option<ProviderPayment>, GatewayError> {
    let object = value.get("data").and_then(|data| data.get("object"));
    let object_id = object
        .and_then(|o| o.get("id"))
        .and_then(serde_json::Value::as_str);
    let mut sighting = match event_type {
        "checkout.session.completed"
        | "checkout.session.async_payment_succeeded"
        | "checkout.session.async_payment_failed"
        | "checkout.session.expired" => match object_id {
            Some(session_id) => gateway.fetch_session(session_id).await?.payment,
            None => None,
        },
        "payment_intent.succeeded"
        | "payment_intent.payment_failed"
        | "payment_intent.canceled"
        | "payment_intent.processing"
        | "payment_intent.requires_action" => match object_id {
            Some(payment_id) => Some(gateway.fetch_payment(payment_id).await?),
            None => None,
        },
        "charge.refunded" | "refund.updated" => {
            match object
                .and_then(|o| o.get("payment_intent"))
                .and_then(serde_json::Value::as_str)
            {
                Some(payment_id) => Some(gateway.fetch_payment(payment_id).await?),
                None => None,
            }
        }
        _ => None,
    };
    if (event_type == "payment_intent.payment_failed"
        || event_type == "checkout.session.async_payment_failed")
        && let Some(sighting) = sighting.as_mut()
        && sighting.status == "requires_payment_method"
    {
        sighting.status = "failed".to_string();
    }
    Ok(sighting)
}

/// Handles one delivery.
async fn receive(State(pool): State<PgPool>, headers: HeaderMap, body: Bytes) -> Response {
    // Unconfigured integrations do not expose the endpoint at
    // all: without a signing secret there is nothing legitimate
    // to verify against, so the route reads as absent.
    let Some(secret) = stripe_client::settings().signing_secret() else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let Some(gateway) = stripe_client::settings().gateway() else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let signature = headers
        .get("Stripe-Signature")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("");
    if !signature_valid(secret, signature, &body, chrono::Utc::now().timestamp()) {
        // Never log the body or the attempted signature.
        tracing::warn!("Stripe webhook rejected: signature mismatch or stale timestamp");
        return StatusCode::BAD_REQUEST.into_response();
    }
    // The signature is valid: the body may now be parsed.
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(&body) else {
        tracing::warn!("Stripe webhook rejected: body is not JSON");
        return StatusCode::BAD_REQUEST.into_response();
    };
    let Some(event_id) = value.get("id").and_then(serde_json::Value::as_str) else {
        // Without the body's event id there is no dedupe key:
        // not an event we can safely process.
        tracing::warn!("Stripe webhook rejected: event without id");
        return StatusCode::BAD_REQUEST.into_response();
    };
    let event_type = value
        .get("type")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");
    let sighting = match fetch_sighting(&gateway, event_type, &value).await {
        Ok(sighting) => sighting,
        Err(e) => {
            // The event is real but its truth could not be
            // fetched: 500, so Stripe redelivers.
            tracing::error!(event_id, "Stripe webhook sighting fetch failed: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };
    match payment_service::handle_webhook_event(
        &pool,
        event_id,
        None,
        event_type,
        &value,
        sighting.as_ref(),
    )
    .await
    {
        Ok(_) => StatusCode::OK.into_response(),
        Err(e) => {
            // Transient by assumption: Stripe redelivers, and
            // the ledger makes the redelivery safe.
            tracing::error!(event_id, "Stripe webhook handling failed: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sign(secret: &str, timestamp: i64, body: &[u8]) -> String {
        let mut payload = timestamp.to_string().into_bytes();
        payload.push(b'.');
        payload.extend_from_slice(body);
        let hex = crate::util::hmac_util::hmac_sha256_hex(secret.as_bytes(), &[payload.as_slice()]);
        format!("t={timestamp},v1={hex}")
    }

    #[test]
    fn valid_signature_accepts_within_tolerance() {
        let body = br#"{"id":"evt_1","type":"ping"}"#;
        let header = sign("whsec_test", 1_000_000, body);
        assert!(signature_valid("whsec_test", &header, body, 1_000_100));
    }

    #[test]
    fn stale_timestamp_rejects_even_with_a_valid_hmac() {
        let body = br#"{"id":"evt_1","type":"ping"}"#;
        let header = sign("whsec_test", 1_000_000, body);
        assert!(!signature_valid("whsec_test", &header, body, 1_000_301));
    }

    #[test]
    fn reencoded_body_and_wrong_secret_reject() {
        let body = br#"{"id":"evt_1","type":"ping"}"#;
        let header = sign("whsec_test", 1_000_000, body);
        let pretty = b"{\n  \"id\": \"evt_1\",\n  \"type\": \"ping\"\n}";
        assert!(!signature_valid("whsec_test", &header, pretty, 1_000_000));
        assert!(!signature_valid("whsec_other", &header, body, 1_000_000));
        assert!(!signature_valid("whsec_test", "garbage", body, 1_000_000));
    }
}
