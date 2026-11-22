//! The Stripe client: the only module that talks to Stripe.
//!
//! All Stripe shapes live here and are translated into the
//! [`super::payment_gateway`] models at this boundary; the rest
//! of the app never sees a provider payload. Stripe differs from
//! KOMOJU in transport, and both differences are owned here:
//! requests are form-encoded (not JSON) and authenticate with a
//! Bearer secret key. Amounts are integer yen end to end — JPY
//! is a zero-decimal currency at Stripe, so a yen amount is
//! sent and read as-is; any minor-unit (×100) conversion here
//! would be a bug by definition. The key comes from the
//! environment and is never logged; response bodies are never
//! logged whole.

use std::sync::{Arc, OnceLock};

use async_trait::async_trait;
use chrono::DateTime;
use serde_json::Value;

use crate::config::CONFIG;
use crate::models::PaymentMethod;
use crate::services::payment_gateway::{
    CreateSessionRequest, CreatedSession, GatewayError, PaymentGateway, ProviderPayment,
    SessionState,
};

/// The resolved Stripe wiring for this process: the gateway
/// when the integration is fully configured, plus the pieces
/// the webhook and the return URL need.
pub struct StripeSettings {
    gateway: Option<Arc<dyn PaymentGateway>>,
    secret_for_webhooks: Option<String>,
    public_base_url: Option<String>,
}

/// Returns the process-wide Stripe settings, resolved once.
///
/// Fail-closed by construction, mirroring the KOMOJU settings:
/// the gateway exists only when the typed `stripe_enabled`
/// flag is on AND every secret the flow needs is in the
/// environment. Enabled but misconfigured logs one clear error
/// and yields no gateway, so checkout serves cash on delivery
/// only and the store stays up.
pub fn settings() -> &'static StripeSettings {
    static SETTINGS: OnceLock<StripeSettings> = OnceLock::new();
    SETTINGS.get_or_init(resolve_settings)
}

/// Resolves the settings from config and environment (once).
fn resolve_settings() -> StripeSettings {
    let unavailable = StripeSettings {
        gateway: None,
        secret_for_webhooks: None,
        public_base_url: None,
    };
    if !CONFIG.stripe_enabled {
        return unavailable;
    }
    let (Ok(secret_key), Ok(signing_secret), Ok(public_base_url)) = (
        CONFIG.stripe_secret_key().ok_or(()),
        CONFIG.stripe_webhook_secret().ok_or(()),
        CONFIG.public_base_url().ok_or(()),
    ) else {
        // One clear line, naming what is missing without ever
        // printing a value.
        let missing: Vec<&str> = [
            ("STRIPE_SECRET_KEY", CONFIG.stripe_secret_key()),
            ("STRIPE_WEBHOOK_SECRET", CONFIG.stripe_webhook_secret()),
            ("PUBLIC_BASE_URL", CONFIG.public_base_url()),
        ]
        .iter()
        .filter(|(_, value)| value.is_none())
        .map(|(name, _)| *name)
        .collect();
        tracing::error!(
            missing = missing.join(", "),
            "stripe_enabled is true but Stripe configuration is incomplete; \
             checkout serves cash on delivery only"
        );
        return unavailable;
    };
    let gateway = match StripeGateway::new(CONFIG.stripe_api_base_url(), secret_key) {
        Ok(gateway) => Arc::new(gateway) as Arc<dyn PaymentGateway>,
        Err(e) => {
            tracing::error!(
                "Stripe client could not be built: {e}; checkout serves cash on delivery only"
            );
            return unavailable;
        }
    };
    StripeSettings {
        gateway: Some(gateway),
        secret_for_webhooks: Some(signing_secret),
        public_base_url: Some(public_base_url.trim_end_matches('/').to_string()),
    }
}

impl StripeSettings {
    /// The gateway, when the integration is fully configured.
    pub fn gateway(&self) -> Option<Arc<dyn PaymentGateway>> {
        self.gateway.clone()
    }

    /// The webhook signing secret, when configured.
    pub fn signing_secret(&self) -> Option<&str> {
        self.secret_for_webhooks.as_deref()
    }

    /// The return URL Stripe sends the customer's browser to,
    /// when a public base URL is configured.
    pub fn return_url(&self) -> Option<String> {
        self.public_base_url
            .as_ref()
            .map(|base| format!("{base}/checkout/payment/return"))
    }
}

/// The real gateway: Stripe's Checkout Sessions and
/// PaymentIntents API over reqwest (rustls), every call
/// timed out.
pub struct StripeGateway {
    /// The HTTP client (pooled, rustls).
    client: reqwest::Client,
    /// API base URL, no trailing slash.
    base_url: String,
    /// The secret key (Bearer token). Never logged.
    secret_key: String,
}

impl StripeGateway {
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
    /// answer. POSTs are form-encoded (Stripe's API shape);
    /// GETs carry no body.
    async fn send(
        &self,
        method: reqwest::Method,
        path: &str,
        form: Option<Vec<(String, String)>>,
        expand: &[&str],
    ) -> Result<Value, GatewayError> {
        let mut request = self
            .client
            .request(method, format!("{}{path}", self.base_url))
            .bearer_auth(&self.secret_key);
        if !expand.is_empty() {
            let params: Vec<(&str, &str)> = expand.iter().map(|item| ("expand[]", *item)).collect();
            request = request.query(&params);
        }
        if let Some(form) = form {
            request = request.form(&form);
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
            // Stripe's error shape is {error: {message}}; surface
            // that message, or a short excerpt — never the whole
            // body.
            let message = serde_json::from_str::<Value>(&text)
                .ok()
                .and_then(|v| {
                    v.get("error")
                        .and_then(|e| e.get("message"))
                        .and_then(Value::as_str)
                        .map(str::to_string)
                })
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

/// The Stripe payment-method word for one of our methods: the
/// session is restricted to exactly the method the customer
/// already chose on our page, mirroring the KOMOJU session.
fn stripe_method_word(method: PaymentMethod) -> &'static str {
    match method {
        PaymentMethod::Konbini => "konbini",
        PaymentMethod::PayPay => "paypay",
        _ => "card",
    }
}

/// Builds the form body for Checkout Session creation. JPY is
/// zero-decimal: `unit_amount` is the order total in plain yen.
fn session_form(request: &CreateSessionRequest) -> Vec<(String, String)> {
    let return_target = format!("{}?session_id={{CHECKOUT_SESSION_ID}}", request.return_url);
    let pairs: Vec<(&str, String)> = vec![
        ("mode", "payment".to_string()),
        (
            "payment_method_types[0]",
            stripe_method_word(request.method).to_string(),
        ),
        ("success_url", return_target.clone()),
        ("cancel_url", return_target),
        ("client_reference_id", request.external_order_num.clone()),
        ("customer_email", request.email.clone()),
        ("line_items[0][quantity]", "1".to_string()),
        ("line_items[0][price_data][currency]", "jpy".to_string()),
        (
            "line_items[0][price_data][unit_amount]",
            request.amount_yen.to_string(),
        ),
        (
            "line_items[0][price_data][product_data][name]",
            "ご注文のお支払い".to_string(),
        ),
        (
            "metadata[external_order_num]",
            request.external_order_num.clone(),
        ),
        (
            "payment_intent_data[metadata][external_order_num]",
            request.external_order_num.clone(),
        ),
    ];
    pairs
        .into_iter()
        .map(|(key, value)| (key.to_string(), value))
        .collect()
}

/// Translates one PaymentIntent object into the gateway model.
///
/// The charge must ride expanded (`latest_charge`): the
/// refunded total and the capture instant live on it, and a
/// sighting built without them would regress the row's
/// refunded total to zero at the apply path. Konbini voucher
/// details ride in `next_action.konbini_display_details` while
/// the payment is outstanding.
fn parse_payment_intent(
    value: &Value,
    session_id: Option<String>,
) -> Result<ProviderPayment, GatewayError> {
    let get_str = |key: &str| value.get(key).and_then(Value::as_str);
    let get_i64 = |key: &str| value.get(key).and_then(Value::as_i64).unwrap_or(0);
    let status = get_str("status").unwrap_or_default().to_string();
    let charge = value.get("latest_charge").filter(|c| c.is_object());
    let amount_refunded_yen = charge
        .and_then(|c| c.get("amount_refunded"))
        .and_then(Value::as_i64)
        .unwrap_or(0);
    let captured_at = if status == "succeeded" {
        charge
            .and_then(|c| c.get("created"))
            .and_then(Value::as_i64)
            .and_then(|secs| DateTime::from_timestamp(secs, 0))
    } else {
        None
    };
    let konbini = value
        .get("next_action")
        .filter(|action| {
            action.get("type").and_then(Value::as_str) == Some("konbini_display_details")
        })
        .and_then(|action| action.get("konbini_display_details"));
    let (payment_deadline, instructions_url, payment_details) = match konbini {
        Some(details) => (
            details
                .get("expires_at")
                .and_then(Value::as_i64)
                .and_then(|secs| DateTime::from_timestamp(secs, 0)),
            details
                .get("hosted_voucher_url")
                .and_then(Value::as_str)
                .map(str::to_string),
            Some(details.clone()),
        ),
        None => (None, None, None),
    };
    Ok(ProviderPayment {
        id: get_str("id")
            .ok_or_else(|| GatewayError::Unexpected("payment intent without id".to_string()))?
            .to_string(),
        status,
        amount_yen: get_i64("amount"),
        total_yen: get_i64("amount"),
        currency: get_str("currency").unwrap_or_default().to_uppercase(),
        external_order_num: value
            .get("metadata")
            .and_then(|m| m.get("external_order_num"))
            .and_then(Value::as_str)
            .map(str::to_string),
        session_id,
        payment_deadline,
        instructions_url,
        payment_details,
        captured_at,
        amount_refunded_yen,
    })
}

/// Translates one Checkout Session object: its own status word
/// is advisory only; the expanded PaymentIntent is the truth.
fn parse_session(value: &Value) -> Result<SessionState, GatewayError> {
    let session_id = value.get("id").and_then(Value::as_str).map(str::to_string);
    let payment = match value.get("payment_intent") {
        Some(intent) if intent.is_object() => {
            let mut sighting = parse_payment_intent(intent, session_id.clone())?;
            // The session carries the reference when the intent's
            // own metadata lacks it.
            if sighting.external_order_num.is_none() {
                sighting.external_order_num = value
                    .get("client_reference_id")
                    .and_then(Value::as_str)
                    .map(str::to_string);
            }
            Some(sighting)
        }
        _ => None,
    };
    Ok(SessionState {
        status: value
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        payment,
    })
}

#[async_trait]
impl PaymentGateway for StripeGateway {
    async fn create_session(
        &self,
        request: &CreateSessionRequest,
    ) -> Result<CreatedSession, GatewayError> {
        let answer = self
            .send(
                reqwest::Method::POST,
                "/v1/checkout/sessions",
                Some(session_form(request)),
                &[],
            )
            .await?;
        let session_id = answer
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| GatewayError::Unexpected("session without id".to_string()))?;
        let session_url = answer
            .get("url")
            .and_then(Value::as_str)
            .ok_or_else(|| GatewayError::Unexpected("session without url".to_string()))?;
        Ok(CreatedSession {
            session_id: session_id.to_string(),
            session_url: session_url.to_string(),
        })
    }

    async fn fetch_session(&self, session_id: &str) -> Result<SessionState, GatewayError> {
        let answer = self
            .send(
                reqwest::Method::GET,
                &format!("/v1/checkout/sessions/{session_id}"),
                None,
                &["payment_intent", "payment_intent.latest_charge"],
            )
            .await?;
        parse_session(&answer)
    }

    async fn fetch_payment(&self, payment_id: &str) -> Result<ProviderPayment, GatewayError> {
        let answer = self
            .send(
                reqwest::Method::GET,
                &format!("/v1/payment_intents/{payment_id}"),
                None,
                &["latest_charge"],
            )
            .await?;
        parse_payment_intent(&answer, None)
    }

    async fn capture(
        &self,
        payment_id: &str,
        amount_yen: Option<i64>,
    ) -> Result<ProviderPayment, GatewayError> {
        let form: Vec<(String, String)> = match amount_yen {
            Some(amount) => vec![("amount_to_capture".to_string(), amount.to_string())],
            None => Vec::new(),
        };
        let answer = self
            .send(
                reqwest::Method::POST,
                &format!("/v1/payment_intents/{payment_id}/capture"),
                Some(form),
                &["latest_charge"],
            )
            .await?;
        parse_payment_intent(&answer, None)
    }

    async fn cancel(&self, payment_id: &str) -> Result<ProviderPayment, GatewayError> {
        let answer = self
            .send(
                reqwest::Method::POST,
                &format!("/v1/payment_intents/{payment_id}/cancel"),
                Some(Vec::new()),
                &["latest_charge"],
            )
            .await?;
        parse_payment_intent(&answer, None)
    }

    async fn refund(
        &self,
        payment_id: &str,
        amount_yen: Option<i64>,
    ) -> Result<ProviderPayment, GatewayError> {
        // The refund object is not the payment: create the
        // refund, then read the PaymentIntent back so the
        // sighting carries the refunded total the apply path
        // checks. (Against real Stripe a refund settles
        // asynchronously; the webhook and the sweep converge
        // the row when it lands.)
        let mut form: Vec<(String, String)> =
            vec![("payment_intent".to_string(), payment_id.to_string())];
        if let Some(amount) = amount_yen {
            form.push(("amount".to_string(), amount.to_string()));
        }
        self.send(reqwest::Method::POST, "/v1/refunds", Some(form), &[])
            .await?;
        self.fetch_payment(payment_id).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn session_form_charges_plain_yen_never_minor_units() {
        let request = CreateSessionRequest {
            amount_yen: 3_530,
            email: "guest@example.jp".to_string(),
            method: PaymentMethod::CreditCard,
            return_url: "https://shop.example/checkout/payment/return".to_string(),
            external_order_num: "AS-20261122-0001".to_string(),
        };
        let form = session_form(&request);
        let get = |key: &str| form.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str());
        // JPY is zero-decimal: 3530 yen rides as "3530".
        assert_eq!(get("line_items[0][price_data][unit_amount]"), Some("3530"));
        assert_eq!(get("line_items[0][price_data][currency]"), Some("jpy"));
        assert_eq!(get("payment_method_types[0]"), Some("card"));
        assert_eq!(
            get("success_url"),
            Some("https://shop.example/checkout/payment/return?session_id={CHECKOUT_SESSION_ID}")
        );
        assert_eq!(get("client_reference_id"), Some("AS-20261122-0001"));
        assert_eq!(
            get("payment_intent_data[metadata][external_order_num]"),
            Some("AS-20261122-0001")
        );
    }

    #[test]
    fn succeeded_intent_translates_with_charge_truth() {
        let value = json!({
            "id": "pi_123",
            "status": "succeeded",
            "amount": 3530,
            "currency": "jpy",
            "metadata": {"external_order_num": "AS-20261122-0001"},
            "latest_charge": {
                "id": "ch_123",
                "created": 1763712000,
                "amount_refunded": 0
            }
        });
        let payment =
            parse_payment_intent(&value, Some("cs_123".to_string())).expect("intent parses");
        assert_eq!(payment.id, "pi_123");
        assert_eq!(payment.status, "succeeded");
        assert_eq!(payment.total_yen, 3_530);
        // Stripe answers lowercase; the row stores `JPY`.
        assert_eq!(payment.currency, "JPY");
        assert_eq!(payment.amount_refunded_yen, 0);
        assert!(payment.captured_at.is_some());
        assert_eq!(payment.session_id.as_deref(), Some("cs_123"));
    }

    #[test]
    fn konbini_intent_carries_voucher_details() {
        let value = json!({
            "id": "pi_456",
            "status": "requires_action",
            "amount": 1430,
            "currency": "jpy",
            "metadata": {"external_order_num": "AS-20261122-0002"},
            "next_action": {
                "type": "konbini_display_details",
                "konbini_display_details": {
                    "expires_at": 1764057600,
                    "hosted_voucher_url": "https://payments.stripe.com/konbini/instructions/test",
                    "payment_code": "12345678901234"
                }
            },
            "latest_charge": {"id": "ch_456", "created": 1763712000, "amount_refunded": 0}
        });
        let payment = parse_payment_intent(&value, None).expect("intent parses");
        assert_eq!(payment.status, "requires_action");
        assert_eq!(
            payment.instructions_url.as_deref(),
            Some("https://payments.stripe.com/konbini/instructions/test")
        );
        let deadline = payment.payment_deadline.expect("deadline");
        assert_eq!(deadline.timestamp(), 1_764_057_600);
        assert!(payment.captured_at.is_none());
    }

    #[test]
    fn refunded_total_comes_from_the_charge() {
        let value = json!({
            "id": "pi_789",
            "status": "succeeded",
            "amount": 3530,
            "currency": "jpy",
            "metadata": {},
            "latest_charge": {"id": "ch_789", "created": 1763712000, "amount_refunded": 3530}
        });
        let payment = parse_payment_intent(&value, None).expect("intent parses");
        assert_eq!(payment.amount_refunded_yen, 3_530);
    }
}
