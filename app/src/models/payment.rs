//! Payment models (KOMOJU integration).
//!
//! Payment state is stored, separate from fulfilment state: an
//! order says where the parcel is, a payment says whether the
//! money moved. The transition function here is pure, so the
//! state machine's rules — monotonic moves, a failed attempt
//! never closing a payment, a capture landing after cancellation
//! being flagged rather than applied — are testable without a
//! database or a network.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Payment status: created with the order, KOMOJU session not
/// settled yet. Konbini instructions issued also rest here —
/// `authorized` at KOMOJU still means no money has moved.
pub const STATUS_PENDING: &str = "pending_payment";
/// Payment status: the money is captured.
pub const STATUS_PAID: &str = "paid";
/// Payment status: the last attempt failed. NOT terminal: the
/// customer may retry on KOMOJU's page, so a later capture is
/// still legal from here.
pub const STATUS_FAILED: &str = "failed";
/// Payment status: the payment window lapsed unpaid.
pub const STATUS_EXPIRED: &str = "expired";
/// Payment status: cancelled before capture.
pub const STATUS_CANCELLED: &str = "cancelled";
/// Payment status: fully refunded.
pub const STATUS_REFUNDED: &str = "refunded";
/// Payment status: partly refunded, the rest still captured.
/// Reached when part of a captured payment is refunded, in the
/// admin or from the provider dashboard.
pub const STATUS_PARTIALLY_REFUNDED: &str = "partially_refunded";

/// How the customer pays, chosen at checkout. The KOMOJU values
/// are KOMOJU's own `payment_types` slugs, so the slug is what
/// the session request sends.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PaymentMethod {
    /// Credit card via KOMOJU.
    CreditCard,
    /// Convenience-store payment via KOMOJU (asynchronous).
    Konbini,
    /// PayPay via KOMOJU.
    PayPay,
    /// Cash on delivery: the store's original method, no provider.
    #[default]
    Cod,
    /// The dummy method of the original human-approved agent
    /// checkout: no provider, no money, born
    /// settled. Retired from the flow in the settlement rebuild — the
    /// agent settlement now pays through the ordinary
    /// methods — and kept so the dummy orders placed under
    /// it still render with their method, provider and
    /// status. It is never offered and never accepted as a
    /// posted choice.
    AgentTest,
}

impl PaymentMethod {
    /// Parses a posted method value; `None` for anything the
    /// checkout never offered (a forged value settles nothing).
    pub fn from_slug(slug: &str) -> Option<Self> {
        match slug {
            "credit_card" => Some(Self::CreditCard),
            "konbini" => Some(Self::Konbini),
            "paypay" => Some(Self::PayPay),
            "cod" => Some(Self::Cod),
            "agent_test" => Some(Self::AgentTest),
            _ => None,
        }
    }

    /// The stored/requested slug for this method.
    pub fn slug(&self) -> &'static str {
        match self {
            Self::CreditCard => "credit_card",
            Self::Konbini => "konbini",
            Self::PayPay => "paypay",
            Self::Cod => "cod",
            Self::AgentTest => "agent_test",
        }
    }

    /// The customer-facing method name.
    pub fn label(&self) -> &'static str {
        match self {
            Self::CreditCard => "クレジットカード",
            Self::Konbini => "コンビニ決済",
            Self::PayPay => "PayPay",
            Self::Cod => "代金引換",
            Self::AgentTest => "エージェント決済（テスト）",
        }
    }

    /// The note shown under the method on the delivery step.
    pub fn note(&self, fees: crate::models::FeeSettings, gateway_name: &str) -> String {
        match self {
            Self::CreditCard => format!("{gateway_name}の画面でカード情報を入力します。"),
            Self::Konbini => {
                "ご注文後に支払い番号を発行します。期限までにお支払いください。".to_string()
            }
            Self::PayPay => format!("{gateway_name}の画面でPayPayの承認を行います。"),
            Self::Cod => format!(
                "代引き手数料 ¥{} が加算されます。お届け時に現金でお支払いください。",
                fees.cod_fee_yen
            ),
            Self::AgentTest => "テスト用の決済です。実際の請求は発生しません。".to_string(),
        }
    }

    /// The confirm page's place-order button for this method.
    pub fn place_button_label(&self) -> &'static str {
        match self {
            Self::Konbini => "支払い番号を発行する",
            Self::CreditCard | Self::PayPay => "支払いへ進む",
            Self::Cod | Self::AgentTest => "注文を確定する",
        }
    }

    /// Whether a hosted gateway settles this method (the
    /// three hosted methods; cash on delivery and the agent
    /// dummy settle elsewhere — here, and nowhere).
    pub fn is_hosted(&self) -> bool {
        matches!(self, Self::CreditCard | Self::Konbini | Self::PayPay)
    }

    /// Whether this method is the agent checkout's dummy:
    /// the one method the normal payment step must never
    /// accept as a posted choice.
    pub fn is_agent_test(&self) -> bool {
        matches!(self, Self::AgentTest)
    }

    /// The status the payment row is born with. Every real
    /// method starts unsettled and settles later; the dummy
    /// starts settled, because its settlement — the owner's
    /// approval plus the token check — already happened
    /// before placement ran.
    pub fn initial_status(&self) -> &'static str {
        match self {
            Self::AgentTest => STATUS_PAID,
            _ => STATUS_PENDING,
        }
    }

    /// The status the payment row is born with for an order of
    /// `amount_yen`: [`Self::initial_status`], except that a
    /// hosted method on a ¥0 order is born settled — no session
    /// will ever exist to settle it later.
    pub fn initial_status_for(&self, amount_yen: i64) -> &'static str {
        if self.is_hosted() && amount_yen == 0 {
            STATUS_PAID
        } else {
            self.initial_status()
        }
    }

    /// Whether this method settles an order of `amount_yen`
    /// without a gateway session: cash on delivery and the
    /// agent dummy never use one, and a ¥0 order has nothing to
    /// collect through one — the gateway cannot open a session
    /// for ¥0, so placement completes the order in place
    /// instead.
    pub fn settles_without_session(&self, amount_yen: i64) -> bool {
        !self.is_hosted() || amount_yen == 0
    }

    /// Whether this method is convenience-store payment: the
    /// asynchronous method whose shipment waits for the money,
    /// which the confirm step's delivery note depends on.
    pub fn is_konbini(&self) -> bool {
        matches!(self, Self::Konbini)
    }

    /// The highest total this method accepts, when the provider
    /// publishes one the checkout can check before redirecting.
    /// PayPay's limit is per customer per day, which only KOMOJU
    /// can enforce, so it is not listed here.
    pub fn max_total_yen(&self) -> Option<i64> {
        match self {
            Self::Konbini => Some(300_000),
            _ => None,
        }
    }
}

/// One order's payment row: the money half of the order.
#[derive(Clone, sqlx::FromRow)]
pub struct Payment {
    /// Surrogate ID.
    pub id: i64,
    /// The order this payment settles (one payment per order).
    pub order_id: i64,
    /// `komoju`, `stripe`, `cod` or `agent` (the original dummy).
    pub provider: String,
    /// Method slug (`credit_card`, `konbini`, `paypay`, `cod`).
    pub method: String,
    /// State machine status (the `STATUS_*` values).
    pub status: String,
    /// The order total recorded on this payment, in yen.
    pub amount_yen: i64,
    /// Refunded so far, in yen.
    pub amount_refunded_yen: i64,
    /// Currency code, always `JPY` in V1.
    pub currency: String,
    /// KOMOJU session ID, once a session exists.
    pub komoju_session_id: Option<String>,
    /// KOMOJU payment ID, once KOMOJU has created the payment.
    pub komoju_payment_id: Option<String>,
    /// Stripe Checkout session ID, once a session exists.
    pub stripe_session_id: Option<String>,
    /// Stripe PaymentIntent ID, once Stripe has created it.
    pub stripe_payment_id: Option<String>,
    /// The external order number actually sent (retries suffix
    /// it; correlation never relies on it alone).
    pub external_order_num: Option<String>,
    /// Konbini payment deadline, as KOMOJU reported it.
    pub payment_deadline: Option<DateTime<Utc>>,
    /// KOMOJU's hosted instructions page (konbini).
    pub instructions_url: Option<String>,
    /// Last provider `payment_details`, kept so the instructions
    /// render without an API call.
    pub payment_details: Option<serde_json::Value>,
    /// Why this payment needs a human's eyes (amount mismatch,
    /// capture after cancellation), when flagged.
    pub flag_note: Option<String>,
    /// When the money was captured.
    pub captured_at: Option<DateTime<Utc>>,
    /// When the confirmation mail was claimed/sent; the claim
    /// that makes the mail exactly-once across webhook, return
    /// and sweep.
    pub confirmation_mail_sent_at: Option<DateTime<Utc>>,
    /// A full refund has landed but its compensation
    /// (restock + points reversal) has not completed: set
    /// in the transaction that applies the refund, cleared
    /// by the compensation itself, and retried by the
    /// payment sweep while it stands.
    pub compensation_pending: bool,
    /// When the row was created.
    pub created_at: DateTime<Utc>,
    /// When the row last changed.
    pub updated_at: DateTime<Utc>,
}

impl Payment {
    /// The typed method of this payment.
    pub fn method_kind(&self) -> PaymentMethod {
        PaymentMethod::from_slug(&self.method).unwrap_or(PaymentMethod::Cod)
    }

    /// The method as customers and admins read it.
    pub fn method_label(&self) -> &'static str {
        self.method_kind().label()
    }

    /// The gateway's own payment ID for this row, whichever
    /// provider owns it — the one ID refunds, captures and
    /// cancels address.
    pub fn provider_payment_id(&self) -> Option<&str> {
        match self.provider.as_str() {
            "stripe" => self.stripe_payment_id.as_deref(),
            _ => self.komoju_payment_id.as_deref(),
        }
    }

    /// The gateway's hosted-session ID for this row, whichever
    /// provider owns it — the return handler correlates on it.
    pub fn provider_session_id(&self) -> Option<&str> {
        match self.provider.as_str() {
            "stripe" => self.stripe_session_id.as_deref(),
            _ => self.komoju_session_id.as_deref(),
        }
    }

    /// The payment status in the shared wording of the spec:
    /// the one label the admin list, admin detail and mypage
    /// all render, so the screens can never disagree.
    pub fn status_label(&self) -> &'static str {
        status_label_for_status(&self.status)
    }

    /// Whether the money is in: the shipping guard's question.
    pub fn is_paid(&self) -> bool {
        self.status == STATUS_PAID || self.status == STATUS_PARTIALLY_REFUNDED
    }

    /// Whether a hosted gateway owns this payment (KOMOJU
    /// or Stripe): the provider word on the row decides, so
    /// a payment taken on a rail stays that rail's payment
    /// even if the rail is later unconfigured. Cash on
    /// delivery and the agent dummy are not hosted.
    pub fn is_hosted(&self) -> bool {
        matches!(self.provider.as_str(), "komoju" | "stripe")
    }

    /// Whether money was ever taken for this payment:
    /// paid now, partly refunded, or fully refunded all
    /// mean money moved. The coupon-release rule's
    /// dividing line (Order Cancellation spec 5.4).
    pub fn money_settled(&self) -> bool {
        self.is_paid() || self.status == STATUS_REFUNDED
    }

    /// Whether the payment can still settle or die on its own
    /// (the states the sweep and sync paths work on).
    pub fn is_open(&self) -> bool {
        self.status == STATUS_PENDING || self.status == STATUS_FAILED
    }

    /// The amount formatted for display.
    pub fn formatted_amount(&self) -> String {
        crate::pricing::display_price(self.amount_yen)
    }

    /// The refunded amount formatted for display.
    pub fn formatted_refunded(&self) -> String {
        crate::pricing::display_price(self.amount_refunded_yen)
    }

    /// The konbini deadline in the long Japanese form, JST.
    pub fn deadline_long(&self) -> Option<String> {
        self.payment_deadline
            .map(super::jst::jst_long_datetime_label)
    }

    /// The convenience store chain's Japanese name, from the
    /// provider payment details, when KOMOJU has issued them.
    pub fn store_label(&self) -> Option<&'static str> {
        let store = self.payment_details.as_ref()?.get("store")?.as_str()?;
        Some(match store {
            "seven-eleven" => "セブン-イレブン",
            "family-mart" => "ファミリーマート",
            "lawson" => "ローソン",
            "ministop" => "ミニストップ",
            "daily-yamazaki" => "デイリーヤマザキ",
            "seicomart" => "セイコーマート",
            _ => "コンビニ",
        })
    }
}

/// Returns the 支払い状況 label for an order: for a KOMOJU
/// payment the payment row is the truth and its status wording
/// is used; for cash on delivery (and for pre-payments orders
/// with no row) the label keeps its long-standing derivation
/// from the fulfilment status. One function for the admin list,
/// the admin detail and mypage, so the two axes can never be
/// worded two ways on different screens.
pub fn payment_status_label_for(order_status: &str, payment: Option<&Payment>) -> String {
    let (provider, status) = match payment {
        Some(payment) => (
            Some(payment.provider.as_str()),
            Some(payment.status.as_str()),
        ),
        None => (None, None),
    };
    payment_status_label_for_parts(order_status, provider, status)
}

/// The admin badge tone class for the same label
/// [`payment_status_label_for_parts`] computes, so the order
/// list's payment cell reads as a state, not a footnote: paid
/// is the teal "done" tone, awaiting the amber "watch" tone,
/// refunded a cool neutral, failed/expired the muted red, and
/// a cancelled order's payment the plain badge.
pub fn payment_badge_class_for_parts(
    order_status: &str,
    provider: Option<&str>,
    payment_status: Option<&str>,
) -> &'static str {
    if provider == Some("komoju") || provider == Some("stripe") || provider == Some("agent") {
        return match payment_status {
            Some(STATUS_PAID) => "pay-paid",
            Some(STATUS_REFUNDED) | Some(STATUS_PARTIALLY_REFUNDED) => "pay-refunded",
            Some(STATUS_FAILED) | Some(STATUS_EXPIRED) => "pay-failed",
            Some(STATUS_CANCELLED) => "pay-muted",
            _ => "pay-awaiting",
        };
    }
    // Cash on delivery: the money state follows the delivery.
    match order_status {
        "delivered" => "pay-paid",
        "cancelled" => "pay-muted",
        _ => "pay-awaiting",
    }
}

/// The unified label from bare parts — list rows carry the
/// provider and status without the whole payment row. KOMOJU
/// payments read their status wording; everything else (a COD
/// row, or no row at all) derives from the fulfilment status
/// exactly as the store always has.
pub fn payment_status_label_for_parts(
    order_status: &str,
    provider: Option<&str>,
    status: Option<&str>,
) -> String {
    match (provider, status) {
        (Some("komoju"), Some(status))
        | (Some("stripe"), Some(status))
        | (Some("agent"), Some(status)) => status_label_for_status(status).to_string(),
        _ => super::order::payment_status_label(order_status).to_string(),
    }
}

/// The KOMOJU payment-status wording for a bare status value —
/// the mapping [`Payment::status_label`] renders, shared with
/// list rows that carry the status without the whole row.
pub fn status_label_for_status(status: &str) -> &'static str {
    match status {
        STATUS_PAID => "支払い済み",
        STATUS_FAILED => "支払い失敗",
        STATUS_EXPIRED => "支払い期限切れ",
        STATUS_CANCELLED => "キャンセル",
        STATUS_REFUNDED => "返金済み",
        STATUS_PARTIALLY_REFUNDED => "一部返金",
        _ => "入金待ち",
    }
}

/// What the state machine decided about one provider sighting.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Transition {
    /// Move to (or refresh) this status.
    Apply(&'static str),
    /// Recorded and deliberately not applied: a backwards or
    /// unknown move against the monotonic rules.
    Ignore,
    /// Money arrived after the order was already closed
    /// (cancelled or expired): never applied, always flagged
    /// for a human — silently accepting it would strand the
    /// customer's money.
    FlagLateCapture,
}

/// Maps a KOMOJU payment status onto our vocabulary. An unknown
/// spelling maps to the current status (a refresh of where we
/// already are): KOMOJU's docs have disagreed with its API
/// before, so an unrecognised word must never invent a state.
fn desired_status(
    current: &str,
    provider_status: &str,
    amount_refunded_yen: i64,
    amount_yen: i64,
) -> &'static str {
    match provider_status {
        // KOMOJU's `authorized` and Stripe's pre-capture /
        // awaiting-action words all mean the same thing here:
        // the money is promised or awaited, not yet in.
        "authorized"
        | "requires_payment_method"
        | "requires_confirmation"
        | "requires_action"
        | "requires_capture"
        | "processing" => STATUS_PENDING,
        // Stripe spells its cancellation with one `l`.
        "canceled" => STATUS_CANCELLED,
        "succeeded" | "captured" => {
            if amount_yen > 0 && amount_refunded_yen >= amount_yen {
                STATUS_REFUNDED
            } else if amount_refunded_yen > 0 {
                STATUS_PARTIALLY_REFUNDED
            } else {
                STATUS_PAID
            }
        }
        "refunded" => STATUS_REFUNDED,
        "cancelled" => STATUS_CANCELLED,
        "expired" => STATUS_EXPIRED,
        "failed" => STATUS_FAILED,
        _ => match current {
            STATUS_PAID => STATUS_PAID,
            STATUS_FAILED => STATUS_FAILED,
            STATUS_EXPIRED => STATUS_EXPIRED,
            STATUS_CANCELLED => STATUS_CANCELLED,
            STATUS_REFUNDED => STATUS_REFUNDED,
            STATUS_PARTIALLY_REFUNDED => STATUS_PARTIALLY_REFUNDED,
            _ => STATUS_PENDING,
        },
    }
}

/// Decides the transition for one provider sighting of a payment.
///
/// Transitions are monotonic: a settled state never regresses,
/// the terminal states never reopen, and `failed` is the one
/// non-terminal resting state — a capture after a failed attempt
/// is the normal retry path, not a backwards move.
pub fn next_status(
    current: &str,
    provider_status: &str,
    amount_refunded_yen: i64,
    amount_yen: i64,
) -> Transition {
    let desired = desired_status(current, provider_status, amount_refunded_yen, amount_yen);
    // A capture arriving after the order was closed is the named
    // race: flag it, never apply it, whichever closed state the
    // payment rests in.
    if desired == STATUS_PAID && (current == STATUS_CANCELLED || current == STATUS_EXPIRED) {
        return Transition::FlagLateCapture;
    }
    match current {
        STATUS_PENDING | STATUS_FAILED => Transition::Apply(desired),
        STATUS_PAID => match desired {
            STATUS_PAID | STATUS_PARTIALLY_REFUNDED | STATUS_REFUNDED => Transition::Apply(desired),
            _ => Transition::Ignore,
        },
        STATUS_PARTIALLY_REFUNDED => match desired {
            STATUS_PARTIALLY_REFUNDED | STATUS_REFUNDED => Transition::Apply(desired),
            _ => Transition::Ignore,
        },
        // Terminal states accept only a refresh of themselves.
        _ => {
            if desired == current {
                Transition::Apply(desired)
            } else {
                Transition::Ignore
            }
        }
    }
}

/// Returns the session's external order number for one creation
/// attempt: the plain order number first, then a numeric suffix,
/// because KOMOJU requires the value to be unique per session
/// and a retry is a new session.
pub fn external_order_num_for(order_number: &str, attempt: u32) -> String {
    if attempt <= 1 {
        order_number.to_string()
    } else {
        format!("{order_number}-{attempt}")
    }
}

/// One recorded webhook delivery: the diary row the receiver
/// writes before it applies anything. The raw body stays in the
/// database only — it is never rendered and never logged.
#[derive(Clone, sqlx::FromRow)]
pub struct WebhookEvent {
    /// The event `id` from the signed body: unique per event
    /// and the deduplication key.
    pub event_id: String,
    /// The per-delivery id from the header — a redelivery mints
    /// a new one, so it is recorded for tracing only.
    pub delivery_id: Option<String>,
    /// The event type, for example `payment.captured`.
    pub event_type: String,
    /// The payment the event resolved to, when it did.
    pub payment_id: Option<i64>,
    /// When the delivery arrived.
    pub received_at: DateTime<Utc>,
    /// Whether this delivery changed a payment status.
    pub applied: bool,
}

impl WebhookEvent {
    /// When the delivery arrived, in the admin list's format, JST.
    pub fn received_display(&self) -> String {
        super::jst::jst_datetime_label(self.received_at)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn card_capture_moves_pending_to_paid() {
        assert_eq!(
            next_status(STATUS_PENDING, "captured", 0, 5_000),
            Transition::Apply(STATUS_PAID)
        );
    }

    #[test]
    fn konbini_authorized_stays_pending() {
        assert_eq!(
            next_status(STATUS_PENDING, "authorized", 0, 5_000),
            Transition::Apply(STATUS_PENDING)
        );
    }

    #[test]
    fn failed_attempt_is_not_terminal() {
        // The failed attempt records, and the retry's capture
        // still applies from there (TC-02's core rule).
        assert_eq!(
            next_status(STATUS_PENDING, "failed", 0, 5_000),
            Transition::Apply(STATUS_FAILED)
        );
        assert_eq!(
            next_status(STATUS_FAILED, "captured", 0, 5_000),
            Transition::Apply(STATUS_PAID)
        );
    }

    #[test]
    fn paid_never_regresses() {
        for provider in ["authorized", "failed", "expired", "cancelled"] {
            assert_eq!(
                next_status(STATUS_PAID, provider, 0, 5_000),
                Transition::Ignore,
                "provider status {provider} must not move a paid payment"
            );
        }
    }

    #[test]
    fn capture_after_cancel_is_flagged_never_applied() {
        assert_eq!(
            next_status(STATUS_CANCELLED, "captured", 0, 5_000),
            Transition::FlagLateCapture
        );
        assert_eq!(
            next_status(STATUS_EXPIRED, "captured", 0, 5_000),
            Transition::FlagLateCapture
        );
    }

    #[test]
    fn refunds_progress_from_captured_amounts() {
        assert_eq!(
            next_status(STATUS_PAID, "captured", 2_000, 5_000),
            Transition::Apply(STATUS_PARTIALLY_REFUNDED)
        );
        assert_eq!(
            next_status(STATUS_PARTIALLY_REFUNDED, "captured", 5_000, 5_000),
            Transition::Apply(STATUS_REFUNDED)
        );
        assert_eq!(
            next_status(STATUS_PAID, "refunded", 5_000, 5_000),
            Transition::Apply(STATUS_REFUNDED)
        );
    }

    #[test]
    fn unknown_provider_status_never_invents_a_state() {
        assert_eq!(
            next_status(STATUS_PENDING, "some_future_word", 0, 5_000),
            Transition::Apply(STATUS_PENDING)
        );
        assert_eq!(
            next_status(STATUS_PAID, "some_future_word", 0, 5_000),
            Transition::Apply(STATUS_PAID)
        );
    }

    #[test]
    fn terminal_states_only_refresh_themselves() {
        assert_eq!(
            next_status(STATUS_EXPIRED, "expired", 0, 5_000),
            Transition::Apply(STATUS_EXPIRED)
        );
        assert_eq!(
            next_status(STATUS_EXPIRED, "cancelled", 0, 5_000),
            Transition::Ignore
        );
        assert_eq!(
            next_status(STATUS_REFUNDED, "expired", 0, 5_000),
            Transition::Ignore
        );
    }

    #[test]
    fn external_order_numbers_suffix_from_the_second_attempt() {
        assert_eq!(
            external_order_num_for("AS-20261103-0001", 1),
            "AS-20261103-0001"
        );
        assert_eq!(
            external_order_num_for("AS-20261103-0001", 2),
            "AS-20261103-0001-2"
        );
        assert_eq!(
            external_order_num_for("AS-20261103-0001", 3),
            "AS-20261103-0001-3"
        );
    }

    #[test]
    fn method_slugs_round_trip() {
        for method in [
            PaymentMethod::CreditCard,
            PaymentMethod::Konbini,
            PaymentMethod::PayPay,
            PaymentMethod::Cod,
            PaymentMethod::AgentTest,
        ] {
            assert_eq!(PaymentMethod::from_slug(method.slug()), Some(method));
        }
        assert_eq!(PaymentMethod::from_slug("bitcoin"), None);
        assert_eq!(PaymentMethod::default(), PaymentMethod::Cod);
    }

    #[test]
    fn zero_yen_komoju_payment_is_born_settled_without_a_session() {
        for method in [
            PaymentMethod::CreditCard,
            PaymentMethod::Konbini,
            PaymentMethod::PayPay,
        ] {
            assert_eq!(method.initial_status_for(0), STATUS_PAID);
            assert!(method.settles_without_session(0));
            assert_eq!(method.initial_status_for(1), STATUS_PENDING);
            assert!(!method.settles_without_session(1));
        }
        // The non-KOMOJU methods never use a session at any
        // amount, and their birth statuses are unchanged.
        assert_eq!(PaymentMethod::Cod.initial_status_for(0), STATUS_PENDING);
        assert!(PaymentMethod::Cod.settles_without_session(0));
        assert_eq!(PaymentMethod::AgentTest.initial_status_for(0), STATUS_PAID);
        assert!(PaymentMethod::AgentTest.settles_without_session(5_000));
    }
}
