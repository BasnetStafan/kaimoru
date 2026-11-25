//! The payment gateway boundary.
//!
//! Everything the store needs from a payment provider, as plain
//! models: services speak this trait, never HTTP. [`KomojuGateway`]
//! is the real implementation; [`FakeGateway`] is the scripted
//! stand-in the tests drive, so no test needs a KOMOJU account
//! or a network (the trait-plus-fake shape used across the
//! services).
//!
//! [`KomojuGateway`]: super::komoju_client::KomojuGateway

use std::collections::HashMap;
use std::sync::Mutex;

use async_trait::async_trait;
use chrono::{DateTime, Utc};

use crate::models::PaymentMethod;

/// What creating a session needs: amounts are integer yen,
/// computed server-side from the placed order.
#[derive(Clone, Debug)]
pub struct CreateSessionRequest {
    /// The order total to charge, in yen.
    pub amount_yen: i64,
    /// The buyer's email (the member's, or the guest's form one).
    pub email: String,
    /// The chosen method; the session offers exactly this one.
    pub method: PaymentMethod,
    /// Where KOMOJU sends the browser back afterwards.
    pub return_url: String,
    /// The external order number for this attempt (unique per
    /// session; retries suffix it).
    pub external_order_num: String,
}

/// A created session: where to send the customer.
#[derive(Clone, Debug)]
pub struct CreatedSession {
    /// The provider's session ID.
    pub session_id: String,
    /// The hosted payment page URL.
    pub session_url: String,
}

/// The provider's view of one payment, translated at the client
/// boundary: amounts as integer yen, statuses as the provider's
/// raw words (the state machine in the payment model owns the
/// mapping — an unknown word must never invent a local state).
#[derive(Clone, Debug)]
pub struct ProviderPayment {
    /// The provider's payment ID.
    pub id: String,
    /// The provider's status word (`authorized`, `captured`, ...).
    pub status: String,
    /// The amount before tax in yen (the payment's `amount`).
    pub amount_yen: i64,
    /// The charged total in yen (the payment's `total`): the
    /// figure amount integrity checks against our order total.
    pub total_yen: i64,
    /// Currency code.
    pub currency: String,
    /// The external order number the session carried.
    pub external_order_num: Option<String>,
    /// The session this payment belongs to, when reported.
    pub session_id: Option<String>,
    /// Konbini payment deadline, when issued.
    pub payment_deadline: Option<DateTime<Utc>>,
    /// The hosted instructions page (konbini).
    pub instructions_url: Option<String>,
    /// Method-specific details (store chain, slip numbers).
    pub payment_details: Option<serde_json::Value>,
    /// When the money was captured.
    pub captured_at: Option<DateTime<Utc>>,
    /// Refunded so far, in yen.
    pub amount_refunded_yen: i64,
}

/// A session as fetched back: its own status word is advisory
/// only (KOMOJU's docs spell it two ways); the attached payment
/// object is the truth the sync path reads.
#[derive(Clone, Debug)]
pub struct SessionState {
    /// The session's status word, uninterpreted.
    pub status: String,
    /// The attached payment, once KOMOJU has created one.
    pub payment: Option<ProviderPayment>,
}

/// Why a gateway call did not produce provider truth.
#[derive(Debug)]
pub enum GatewayError {
    /// The provider could not be reached (DNS, timeout, reset).
    Transport(String),
    /// The provider answered with an error status.
    Api { status: u16, message: String },
    /// The answer could not be understood (shape drift).
    Unexpected(String),
}

impl std::fmt::Display for GatewayError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Transport(message) => write!(f, "payment provider unreachable: {message}"),
            Self::Api { status, message } => {
                write!(f, "payment provider error {status}: {message}")
            }
            Self::Unexpected(message) => write!(f, "payment provider answer unreadable: {message}"),
        }
    }
}

impl std::error::Error for GatewayError {}

/// The payment provider's operations. Amounts are integer yen;
/// `None` for a refund/capture amount means the full amount.
#[async_trait]
pub trait PaymentGateway: Send + Sync {
    /// Creates a hosted-page session for one order total.
    async fn create_session(
        &self,
        request: &CreateSessionRequest,
    ) -> Result<CreatedSession, GatewayError>;

    /// Fetches one session and its attached payment.
    async fn fetch_session(&self, session_id: &str) -> Result<SessionState, GatewayError>;

    /// Fetches one payment directly.
    async fn fetch_payment(&self, payment_id: &str) -> Result<ProviderPayment, GatewayError>;

    /// Captures an authorized payment (manual-capture flows;
    /// V1 charges with capture "auto", so this is the trait's
    /// room to switch, not a V1 code path).
    async fn capture(
        &self,
        payment_id: &str,
        amount_yen: Option<i64>,
    ) -> Result<ProviderPayment, GatewayError>;

    /// Voids a not-yet-captured payment (an unpaid konbini slip,
    /// an authorization): the customer can no longer pay it.
    async fn cancel(&self, payment_id: &str) -> Result<ProviderPayment, GatewayError>;

    /// Refunds a captured payment, fully when no amount is given.
    async fn refund(
        &self,
        payment_id: &str,
        amount_yen: Option<i64>,
    ) -> Result<ProviderPayment, GatewayError>;
}

/// A scripted in-memory gateway for tests: sessions and payments
/// are minted locally, and the test moves them between states
/// with [`FakeGateway::set_payment_status`] — the konbini arc
/// (authorized, then captured at the store) is two calls.
#[derive(Default)]
pub struct FakeGateway {
    state: Mutex<FakeState>,
}

/// The fake's mutable world.
#[derive(Default)]
struct FakeState {
    /// Sessions by ID.
    sessions: HashMap<String, SessionState>,
    /// Payments by ID.
    payments: HashMap<String, ProviderPayment>,
    /// Next minted ID.
    sequence: u64,
    /// When set, session creation fails with this message —
    /// the outage drill for the retry path.
    create_failure: Option<String>,
}

impl FakeGateway {
    /// Creates an empty fake.
    pub fn new() -> Self {
        Self::default()
    }

    /// Makes session creation fail until cleared (provider
    /// outage): every attempt fails, not just the next.
    pub fn fail_creation(&self, message: &str) {
        self.state.lock().expect("fake gateway lock").create_failure = Some(message.to_string());
    }

    /// Clears a scripted creation failure.
    pub fn clear_failure(&self) {
        self.state.lock().expect("fake gateway lock").create_failure = None;
    }

    /// Moves one payment to a new provider status word, the way
    /// an outside event (a store payment, an expiry) would.
    pub fn set_payment_status(&self, payment_id: &str, status: &str) {
        let mut state = self.state.lock().expect("fake gateway lock");
        if let Some(payment) = state.payments.get_mut(payment_id) {
            payment.status = status.to_string();
            if status == "captured" && payment.captured_at.is_none() {
                payment.captured_at = Some(Utc::now());
            }
        }
        for session in state.sessions.values_mut() {
            if let Some(payment) = &mut session.payment
                && payment.id == payment_id
            {
                payment.status = status.to_string();
                if status == "captured" && payment.captured_at.is_none() {
                    payment.captured_at = Some(Utc::now());
                }
            }
        }
    }

    /// How many sessions creation produced (the double-submit
    /// question: retries included).
    pub fn session_count(&self) -> usize {
        self.state.lock().expect("fake gateway lock").sessions.len()
    }
}

#[async_trait]
impl PaymentGateway for FakeGateway {
    async fn create_session(
        &self,
        request: &CreateSessionRequest,
    ) -> Result<CreatedSession, GatewayError> {
        let mut state = self.state.lock().expect("fake gateway lock");
        if let Some(message) = &state.create_failure {
            return Err(GatewayError::Transport(message.clone()));
        }
        state.sequence += 1;
        let n = state.sequence;
        let session_id = format!("fake-session-{n}");
        let payment_id = format!("fake-payment-{n}");
        // Card and PayPay settle on the spot with capture "auto";
        // konbini issues instructions and waits (authorized).
        let (status, deadline, instructions_url, details) = match request.method {
            PaymentMethod::Konbini => (
                "authorized",
                Some(Utc::now() + chrono::Duration::days(3)),
                Some(format!("https://fake.invalid/instructions/{payment_id}")),
                Some(serde_json::json!({"type": "konbini", "store": "seven-eleven"})),
            ),
            _ => ("captured", None, None, None),
        };
        let payment = ProviderPayment {
            id: payment_id,
            status: status.to_string(),
            amount_yen: request.amount_yen,
            total_yen: request.amount_yen,
            currency: "JPY".to_string(),
            external_order_num: Some(request.external_order_num.clone()),
            session_id: Some(session_id.clone()),
            payment_deadline: deadline,
            instructions_url,
            payment_details: details,
            captured_at: (status == "captured").then(Utc::now),
            amount_refunded_yen: 0,
        };
        state.payments.insert(payment.id.clone(), payment.clone());
        state.sessions.insert(
            session_id.clone(),
            SessionState {
                status: "completed".to_string(),
                payment: Some(payment),
            },
        );
        Ok(CreatedSession {
            session_url: format!("https://fake.invalid/sessions/{session_id}"),
            session_id,
        })
    }

    async fn fetch_session(&self, session_id: &str) -> Result<SessionState, GatewayError> {
        self.state
            .lock()
            .expect("fake gateway lock")
            .sessions
            .get(session_id)
            .cloned()
            .ok_or_else(|| GatewayError::Unexpected(format!("unknown session {session_id}")))
    }

    async fn fetch_payment(&self, payment_id: &str) -> Result<ProviderPayment, GatewayError> {
        self.state
            .lock()
            .expect("fake gateway lock")
            .payments
            .get(payment_id)
            .cloned()
            .ok_or_else(|| GatewayError::Unexpected(format!("unknown payment {payment_id}")))
    }

    async fn capture(
        &self,
        payment_id: &str,
        _amount_yen: Option<i64>,
    ) -> Result<ProviderPayment, GatewayError> {
        self.set_payment_status(payment_id, "captured");
        self.fetch_payment(payment_id).await
    }

    async fn cancel(&self, payment_id: &str) -> Result<ProviderPayment, GatewayError> {
        self.set_payment_status(payment_id, "cancelled");
        self.fetch_payment(payment_id).await
    }

    async fn refund(
        &self,
        payment_id: &str,
        amount_yen: Option<i64>,
    ) -> Result<ProviderPayment, GatewayError> {
        let mut state = self.state.lock().expect("fake gateway lock");
        let payment = state
            .payments
            .get_mut(payment_id)
            .ok_or_else(|| GatewayError::Unexpected(format!("unknown payment {payment_id}")))?;
        payment.amount_refunded_yen += amount_yen.unwrap_or(payment.total_yen);
        if payment.amount_refunded_yen >= payment.total_yen {
            payment.status = "refunded".to_string();
        }
        Ok(payment.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(method: PaymentMethod) -> CreateSessionRequest {
        CreateSessionRequest {
            amount_yen: 4_740,
            email: "guest@example.jp".to_string(),
            method,
            return_url: "http://localhost/checkout/payment/return".to_string(),
            external_order_num: "AS-20261103-0001".to_string(),
        }
    }

    #[tokio::test]
    async fn fake_card_session_captures_immediately() {
        let fake = FakeGateway::new();
        let session = fake
            .create_session(&request(PaymentMethod::CreditCard))
            .await
            .expect("session");
        let state = fake
            .fetch_session(&session.session_id)
            .await
            .expect("state");
        let payment = state.payment.expect("payment attached");
        assert_eq!(payment.status, "captured");
        assert_eq!(payment.total_yen, 4_740);
    }

    #[tokio::test]
    async fn fake_konbini_waits_then_captures() {
        let fake = FakeGateway::new();
        let session = fake
            .create_session(&request(PaymentMethod::Konbini))
            .await
            .expect("session");
        let state = fake
            .fetch_session(&session.session_id)
            .await
            .expect("state");
        let payment = state.payment.expect("payment attached");
        assert_eq!(payment.status, "authorized");
        assert!(payment.payment_deadline.is_some());
        assert!(payment.instructions_url.is_some());

        fake.set_payment_status(&payment.id, "captured");
        let state = fake
            .fetch_session(&session.session_id)
            .await
            .expect("state");
        assert_eq!(state.payment.expect("payment").status, "captured");
    }

    #[tokio::test]
    async fn fake_creation_failure_is_scriptable() {
        let fake = FakeGateway::new();
        fake.fail_creation("provider down");
        assert!(
            fake.create_session(&request(PaymentMethod::PayPay))
                .await
                .is_err()
        );
        fake.clear_failure();
        assert!(
            fake.create_session(&request(PaymentMethod::PayPay))
                .await
                .is_ok()
        );
    }

    #[tokio::test]
    async fn fake_refund_accumulates_to_full() {
        let fake = FakeGateway::new();
        let session = fake
            .create_session(&request(PaymentMethod::CreditCard))
            .await
            .expect("session");
        let payment_id = fake
            .fetch_session(&session.session_id)
            .await
            .expect("state")
            .payment
            .expect("payment")
            .id;
        let refunded = fake.refund(&payment_id, None).await.expect("refund");
        assert_eq!(refunded.status, "refunded");
        assert_eq!(refunded.amount_refunded_yen, 4_740);
    }
}
