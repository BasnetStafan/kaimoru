//! The KOMOJU client: the only module that talks to KOMOJU.
//!
//! All KOMOJU JSON shapes live here and are translated into the
//! [`super::payment_gateway`] models at this boundary; the rest of the
//! app never sees a provider payload. Authentication is HTTP
//! Basic with the secret key as username and an empty password,
//! per KOMOJU's model. The key comes from the environment and is
//! never logged; response bodies are never logged whole.

use std::sync::{Arc, OnceLock};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde_json::{Value, json};

use crate::config::CONFIG;
use crate::services::payment_gateway::{
    CreateSessionRequest, CreatedSession, GatewayError, PaymentGateway, ProviderPayment,
    SessionState,
};

/// The resolved KOMOJU wiring for this process: the gateway when
/// the integration is fully configured, plus the pieces the
/// webhook and the return URL need.
pub struct KomojuSettings {
    gateway: Option<Arc<dyn PaymentGateway>>,
    webhook_secret: Option<String>,
    public_base_url: Option<String>,
}

/// Returns the process-wide KOMOJU settings, resolved once.
///
/// Fail-closed by construction: the gateway exists only when
/// the typed `komoju_enabled` flag is on AND every secret the
/// flow needs is in the environment. Enabled but misconfigured
/// logs one clear error and yields no gateway, so checkout
/// serves cash on delivery only and the store stays up.
pub fn settings() -> &'static KomojuSettings {
    static SETTINGS: OnceLock<KomojuSettings> = OnceLock::new();
    SETTINGS.get_or_init(resolve_settings)
}

/// Resolves the settings from config and environment (once).
fn resolve_settings() -> KomojuSettings {
    let unavailable = KomojuSettings {
        gateway: None,
        webhook_secret: None,
        public_base_url: None,
    };
    if !CONFIG.komoju_enabled {
        return unavailable;
    }
    let (Ok(secret_key), Ok(webhook_secret), Ok(public_base_url)) = (
        CONFIG.komoju_secret_key().ok_or(()),
        CONFIG.komoju_webhook_secret().ok_or(()),
        CONFIG.public_base_url().ok_or(()),
    ) else {
        // One clear line, naming what is missing without ever
        // printing a value: this is the misconfiguration the spec
        // says must degrade to COD, never take the store down.
        let missing: Vec<&str> = [
            ("KOMOJU_SECRET_KEY", CONFIG.komoju_secret_key()),
            ("KOMOJU_WEBHOOK_SECRET", CONFIG.komoju_webhook_secret()),
            ("PUBLIC_BASE_URL", CONFIG.public_base_url()),
        ]
        .iter()
        .filter(|(_, value)| value.is_none())
        .map(|(name, _)| *name)
        .collect();
        tracing::error!(
            missing = missing.join(", "),
            "komoju_enabled is true but KOMOJU configuration is incomplete; \
             checkout serves cash on delivery only"
        );
        return unavailable;
    };
    let gateway = match KomojuGateway::new(CONFIG.komoju_api_base_url(), secret_key) {
        Ok(gateway) => Arc::new(gateway) as Arc<dyn PaymentGateway>,
        Err(e) => {
            tracing::error!(
                "KOMOJU client could not be built: {e}; checkout serves cash on delivery only"
            );
            return unavailable;
        }
    };
    KomojuSettings {
        gateway: Some(gateway),
        webhook_secret: Some(webhook_secret),
        public_base_url: Some(public_base_url.trim_end_matches('/').to_string()),
    }
}

impl KomojuSettings {
    /// The gateway, when the integration is fully configured.
    pub fn gateway(&self) -> Option<Arc<dyn PaymentGateway>> {
        self.gateway.clone()
    }

    /// The webhook HMAC secret, when configured.
    pub fn webhook_secret(&self) -> Option<&str> {
        self.webhook_secret.as_deref()
    }

    /// The return URL KOMOJU sends the customer's browser to,
    /// when a public base URL is configured.
    pub fn return_url(&self) -> Option<String> {
        self.public_base_url
            .as_ref()
            .map(|base| format!("{base}/checkout/payment/return"))
    }
}

/// The real gateway: KOMOJU's Sessions / Payments API over
/// reqwest (rustls), every call timed out.
pub struct KomojuGateway {
    /// The HTTP client (pooled, rustls).
    client: reqwest::Client,
    /// API base URL, no trailing slash.
    base_url: String,
    /// The secret key (Basic-auth username). Never logged.
    secret_key: String,
}

impl KomojuGateway {
    /// Builds a gateway for one API base URL and secret key.
    ///
    /// # Errors
    ///
    /// Returns [`GatewayError`] when the HTTP client cannot be
    /// built (a TLS initialisation failure).
    pub fn new(base_url: String, secret_key: String) -> Result<Self, GatewayError> {
        let client = reqwest::Client::builder()
            .connect_timeout(std::time::Duration::from_secs(5))
            .timeout(std::time::Duration::from_secs(20))
            .build()
            .map_err(|e| GatewayError::Unexpected(e.to_string()))?;
        Ok(Self {
            client,
            base_url: base_url.trim_end_matches('/').to_string(),
            secret_key,
        })
    }

    /// Sends one authenticated request and parses the JSON
    /// answer. The secret travels as the Basic-auth username
    /// with an empty password, KOMOJU's model.
    async fn send(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<Value>,
    ) -> Result<Value, GatewayError> {
        let mut request = self
            .client
            .request(method, format!("{}{path}", self.base_url))
            .basic_auth(&self.secret_key, Some(""));
        if let Some(body) = body {
            request = request.json(&body);
        }
        let response = request
            .send()
            .await
            .map_err(|e| GatewayError::Transport(e.to_string()))?;
        let status = response.status();
        let text = response
            .text()
            .await
            .map_err(|e| GatewayError::Transport(e.to_string()))?;
        if !status.is_success() {
            // The provider's error shape is not contract; surface
            // a short excerpt only, never the whole body.
            let message = serde_json::from_str::<Value>(&text)
                .ok()
                .and_then(|v| v.get("message").and_then(Value::as_str).map(str::to_string))
                .unwrap_or_else(|| text.chars().take(200).collect());
            return Err(GatewayError::Api {
                status: status.as_u16(),
                message,
            });
        }
        serde_json::from_str(&text)
            .map_err(|e| GatewayError::Unexpected(format!("unparseable answer: {e}")))
    }
}

/// Translates a provider payment object carried outside the
/// API responses — a webhook event's `data` — into the gateway
/// model. The webhook's own module owns the event envelope;
/// the payment object inside it is the same shape the API
/// returns, translated by the same code.
pub fn parse_payment_object(value: &Value) -> Result<ProviderPayment, GatewayError> {
    parse_payment(value)
}

/// Translates one KOMOJU payment object into the gateway model.
///
/// Amount integrity note: KOMOJU models a payment as
/// `amount + tax = total`. The store charges tax-inclusive
/// Japanese prices, so session creation passes the order total
/// as `amount` with an explicit `tax` of 0, making `total` the
/// order total itself; the service layer checks `total` against
/// the payment row on every sighting and quarantines a mismatch.
fn parse_payment(value: &Value) -> Result<ProviderPayment, GatewayError> {
    let get_str = |key: &str| value.get(key).and_then(Value::as_str);
    let get_i64 = |key: &str| value.get(key).and_then(Value::as_i64).unwrap_or(0);
    let details = value.get("payment_details").cloned();
    // The instructions URL lives inside payment_details for the
    // asynchronous methods (KOMOJU's konbini shape).
    let instructions_url = details
        .as_ref()
        .and_then(|d| d.get("instructions_url"))
        .and_then(Value::as_str)
        .map(str::to_string);
    Ok(ProviderPayment {
        id: get_str("id")
            .ok_or_else(|| GatewayError::Unexpected("payment without id".to_string()))?
            .to_string(),
        status: get_str("status").unwrap_or_default().to_string(),
        amount_yen: get_i64("amount"),
        total_yen: value
            .get("total")
            .and_then(Value::as_i64)
            .unwrap_or_else(|| get_i64("amount")),
        currency: get_str("currency").unwrap_or_default().to_string(),
        external_order_num: get_str("external_order_num").map(str::to_string),
        session_id: get_str("session_id").map(str::to_string),
        payment_deadline: get_str("payment_deadline").and_then(parse_komoju_time),
        instructions_url,
        payment_details: details,
        captured_at: get_str("captured_at").and_then(parse_komoju_time),
        amount_refunded_yen: get_i64("amount_refunded"),
    })
}

/// Parses a KOMOJU timestamp: RFC 3339 when it carries a time,
/// or a bare date, which for a payment deadline means the end of
/// that Japan day (the slip is voided at 23:59:59 JST).
fn parse_komoju_time(text: &str) -> Option<DateTime<Utc>> {
    if let Ok(parsed) = DateTime::parse_from_rfc3339(text) {
        return Some(parsed.with_timezone(&Utc));
    }
    let date = chrono::NaiveDate::parse_from_str(text, "%Y-%m-%d").ok()?;
    let jst = chrono::FixedOffset::east_opt(9 * 3600)?;
    let end_of_day = date
        .and_hms_opt(23, 59, 59)?
        .and_local_timezone(jst)
        .single()?;
    Some(end_of_day.with_timezone(&Utc))
}

#[async_trait]
impl PaymentGateway for KomojuGateway {
    async fn create_session(
        &self,
        request: &CreateSessionRequest,
    ) -> Result<CreatedSession, GatewayError> {
        // Exactly one payment type: the customer already chose
        // on our page, so KOMOJU's page confirms that one method
        // instead of offering the whole account. Capture "auto"
        // (the V1 decision): a pre-shipment cancellation is a
        // true refund, which the admin refund flow covers.
        // KOMOJU's `tax` is ADDED to `amount` (its payment
        // objects read total = amount + tax), while this shop's
        // amounts are tax-INCLUSIVE: sending the computed tax
        // portion here would charge it twice. The field stays 0
        // and the order's tax breakdown lives on the order
        // itself (order_tax_breakdowns) — a recorded limitation
        // of the integration, not missing tax handling.
        let body = json!({
            "amount": request.amount_yen,
            "tax": 0,
            "currency": "JPY",
            "return_url": request.return_url,
            "email": request.email,
            "payment_types": [request.method.slug()],
            "default_locale": "ja",
            "payment_data": {
                "external_order_num": request.external_order_num,
                "capture": "auto",
            },
        });
        let answer = self
            .send(reqwest::Method::POST, "/api/v1/sessions", Some(body))
            .await?;
        let session_id = answer
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| GatewayError::Unexpected("session without id".to_string()))?;
        let session_url = answer
            .get("session_url")
            .and_then(Value::as_str)
            .ok_or_else(|| GatewayError::Unexpected("session without session_url".to_string()))?;
        Ok(CreatedSession {
            session_id: session_id.to_string(),
            session_url: session_url.to_string(),
        })
    }

    async fn fetch_session(&self, session_id: &str) -> Result<SessionState, GatewayError> {
        let answer = self
            .send(
                reqwest::Method::GET,
                &format!("/api/v1/sessions/{session_id}"),
                None,
            )
            .await?;
        // Key off the payment object, never the session status
        // string alone: KOMOJU's own guide spells that string
        // two ways ("complete" / "completed").
        let payment = match answer.get("payment") {
            Some(payment) if !payment.is_null() => Some(parse_payment(payment)?),
            _ => None,
        };
        Ok(SessionState {
            status: answer
                .get("status")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            payment,
        })
    }

    async fn fetch_payment(&self, payment_id: &str) -> Result<ProviderPayment, GatewayError> {
        let answer = self
            .send(
                reqwest::Method::GET,
                &format!("/api/v1/payments/{payment_id}"),
                None,
            )
            .await?;
        parse_payment(&answer)
    }

    async fn capture(
        &self,
        payment_id: &str,
        amount_yen: Option<i64>,
    ) -> Result<ProviderPayment, GatewayError> {
        let body = match amount_yen {
            Some(amount) => json!({ "amount": amount }),
            None => json!({}),
        };
        let answer = self
            .send(
                reqwest::Method::POST,
                &format!("/api/v1/payments/{payment_id}/capture"),
                Some(body),
            )
            .await?;
        parse_payment(&answer)
    }

    async fn cancel(&self, payment_id: &str) -> Result<ProviderPayment, GatewayError> {
        let answer = self
            .send(
                reqwest::Method::POST,
                &format!("/api/v1/payments/{payment_id}/cancel"),
                Some(json!({})),
            )
            .await?;
        parse_payment(&answer)
    }

    async fn refund(
        &self,
        payment_id: &str,
        amount_yen: Option<i64>,
    ) -> Result<ProviderPayment, GatewayError> {
        let body = match amount_yen {
            Some(amount) => json!({ "amount": amount }),
            None => json!({}),
        };
        let answer = self
            .send(
                reqwest::Method::POST,
                &format!("/api/v1/payments/{payment_id}/refund"),
                Some(body),
            )
            .await?;
        parse_payment(&answer)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn payment_objects_translate_at_the_boundary() {
        let value = json!({
            "id": "pay_123",
            "status": "authorized",
            "amount": 4740,
            "tax": 0,
            "total": 4740,
            "currency": "JPY",
            "external_order_num": "AS-20261103-0001",
            "payment_deadline": "2026-11-06",
            "payment_details": {
                "type": "konbini",
                "store": "seven-eleven",
                "instructions_url": "https://komoju.com/ja/instructions/pay_123"
            },
            "amount_refunded": 0
        });
        let payment = parse_payment(&value).expect("payment parses");
        assert_eq!(payment.id, "pay_123");
        assert_eq!(payment.total_yen, 4_740);
        assert_eq!(
            payment.instructions_url.as_deref(),
            Some("https://komoju.com/ja/instructions/pay_123")
        );
        // A bare deadline date is the end of that Japan day.
        let deadline = payment.payment_deadline.expect("deadline");
        assert_eq!(deadline.to_rfc3339(), "2026-11-06T14:59:59+00:00");
    }

    #[test]
    fn rfc3339_timestamps_parse_as_given() {
        let parsed = parse_komoju_time("2026-11-03T01:02:03Z").expect("parses");
        assert_eq!(parsed.to_rfc3339(), "2026-11-03T01:02:03+00:00");
        assert!(parse_komoju_time("not a date").is_none());
    }
}
