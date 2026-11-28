//! Payment service.
//!
//! The business rules of getting paid, around the
//! [`PaymentGateway`] boundary: session creation with
//! compensation, the one status-application path every trigger
//! shares (webhook, return, admin sync, sweep), refunds, and
//! the background sweep that keeps unsettled payments honest.
//!
//! Invariants this module holds:
//!
//! - A network call never runs inside a database transaction.
//!   Placement commits first; the session is created after, and
//!   creation failure leaves the order standing for recovery.
//! - Every provider sighting — webhook, return fetch, admin
//!   sync, sweep — is applied by [`apply_provider_payment`]
//!   alone, under one lock order (order, then payment), through
//!   the monotonic state machine in the payment model.
//! - The confirmation mail is claimed in the apply transaction
//!   and sent after commit by whichever path won the claim:
//!   exactly once, whoever delivers the capture first.

use std::sync::Arc;

use chrono::{DateTime, Duration, Utc};
use sqlx::PgPool;

use crate::config::CONFIG;
use crate::error::AppError;
use crate::messages::PAYMENT_INFO_MISSING_MESSAGE;
use crate::models::payment::{self, Transition};
use crate::models::{Payment, PaymentMethod};
use crate::repositories::{order_repository, payment_repository};
use crate::services::komoju_client;
use crate::services::order_service;
use crate::services::payment_gateway::{GatewayError, PaymentGateway, ProviderPayment};
use crate::services::stripe_client;

/// Attempts at creating one session before checkout gives up:
/// each attempt is a new session with a suffixed external order
/// number, so a retry never collides at the provider.
const SESSION_CREATE_ATTEMPTS: u32 = 3;
/// Unsettled payments younger than this are left alone by the
/// sweep: the customer may still be on the payment page.
const SWEEP_STALE_MINUTES: i64 = 15;
/// How long a card/PayPay payment may sit unsettled before the
/// sweep abandons it.
/// Konbini is exempt: KOMOJU's deadline governs those.
const ABANDONED_TTL_HOURS: i64 = 24;

/// The gateway registered for one provider word, when that
/// integration is fully configured in this process. Dispatch
/// is by the payment ROW's provider, never by method: two
/// gateways can both take a card, and an in-flight payment
/// keeps settling through its own gateway after the active
/// rail flips.
pub fn gateway_for(provider: &str) -> Option<Arc<dyn PaymentGateway>> {
    match provider {
        "komoju" => komoju_client::settings().gateway(),
        "stripe" => stripe_client::settings().gateway(),
        _ => None,
    }
}

/// The browser return URL for one provider's hosted page,
/// when that integration is configured.
fn return_url_for(provider: &str) -> Option<String> {
    match provider {
        "komoju" => komoju_client::settings().return_url(),
        "stripe" => stripe_client::settings().return_url(),
        _ => None,
    }
}

/// The provider word of the active hosted rail — the gateway
/// whose methods checkout offers. One rail at a time, chosen
/// by `hosted_gateway` in the config; a selected rail that is
/// not configured offers nothing (cash on delivery only)
/// rather than silently falling back to the other gateway.
pub fn active_provider() -> Option<&'static str> {
    let selected: &'static str = match CONFIG.hosted_gateway.as_str() {
        "stripe" => "stripe",
        _ => "komoju",
    };
    gateway_for(selected).map(|_| selected)
}

/// Whether checkout can offer hosted methods right now: the
/// selected rail is configured, enabled, and complete. When
/// false the store is cash-on-delivery only — by design, not
/// by error.
pub fn hosted_available() -> bool {
    active_provider().is_some()
}

/// The provider word a newly placed payment row records for
/// a method: the built-in rows name themselves; a hosted
/// method names the configured rail (its availability was
/// already proven when the method was offered).
pub fn provider_for_method(method: PaymentMethod) -> &'static str {
    match method {
        PaymentMethod::Cod => "cod",
        PaymentMethod::AgentTest => "agent",
        _ => match CONFIG.hosted_gateway.as_str() {
            "stripe" => "stripe",
            _ => "komoju",
        },
    }
}

/// The customer-facing name of a payment provider, for the
/// Japanese operator messages that name the gateway.
pub fn provider_display_name(provider: &str) -> &'static str {
    match provider {
        "stripe" => "Stripe",
        "komoju" => "KOMOJU",
        _ => "決済サービス",
    }
}

/// The active rail's customer-facing name, for the method
/// notes that name the gateway the shopper will pay through.
pub fn active_gateway_name() -> &'static str {
    provider_display_name(provider_for_method(PaymentMethod::CreditCard))
}

/// The methods the payment step offers: cash on delivery
/// always, the hosted methods when the selected rail is live.
pub fn offered_methods() -> Vec<PaymentMethod> {
    if hosted_available() {
        vec![
            PaymentMethod::CreditCard,
            PaymentMethod::Konbini,
            PaymentMethod::PayPay,
            PaymentMethod::Cod,
        ]
    } else {
        vec![PaymentMethod::Cod]
    }
}

/// Logs which world this process runs in (called once at boot):
/// payments available, or payments disabled. The fail-closed
/// case — enabled but misconfigured — logs nothing here: the
/// settings resolver reports that error itself, and no gateway
/// is built.
pub fn log_startup_state() {
    if let Some(provider) = active_provider() {
        tracing::info!(provider, "hosted payments are available at checkout");
    } else if !CONFIG.komoju_enabled && CONFIG.hosted_gateway != "stripe" {
        tracing::info!("KOMOJU payments disabled; checkout offers cash on delivery only");
    }
}

/// Creates the KOMOJU session for a placed order and returns the
/// hosted-page URL to redirect the customer to.
///
/// Compensation on total failure: the order and its unsettled
/// payment row stand (stock stays held, the coupon stays used) —
/// the customer keeps the order in their history with a
/// still-pending payment, the sweep keeps reconciling it, and
/// nothing is silently rolled back or silently left chargeable.
///
/// # Errors
///
/// Returns a `StartSessionError`: unavailable when the
/// integration is off, the last [`GatewayError`] when every
/// attempt failed, or the failure that kept a created session
/// from being recorded and applied.
pub async fn start_session(pool: &PgPool, order_id: i64) -> Result<String, StartSessionError> {
    // The row's provider picks the gateway: an order placed
    // under one rail keeps opening its session there even if
    // the active rail has since flipped.
    let payment = payment_repository::find_by_order_id(pool, order_id)
        .await?
        .ok_or(StartSessionError::OrderGone)?;
    let gateway = gateway_for(&payment.provider).ok_or(StartSessionError::Unavailable)?;
    start_session_with(pool, &gateway, order_id).await
}

/// [`start_session`] with the gateway given — the seam the
/// tests use to drive a fake provider.
pub async fn start_session_with(
    pool: &PgPool,
    gateway: &Arc<dyn PaymentGateway>,
    order_id: i64,
) -> Result<String, StartSessionError> {
    let order = order_repository::find_by_id(pool, order_id)
        .await?
        .ok_or(StartSessionError::OrderGone)?;
    let payment = payment_repository::find_by_order_id(pool, order_id)
        .await?
        .ok_or(StartSessionError::OrderGone)?;
    let return_url = return_url_for(&payment.provider).ok_or(StartSessionError::OrderGone)?;
    let order_number = order.order_number();
    let mut last_error: Option<GatewayError> = None;
    for attempt in 1..=SESSION_CREATE_ATTEMPTS {
        let external_order_num = payment::external_order_num_for(&order_number, attempt);
        let request = crate::services::payment_gateway::CreateSessionRequest {
            amount_yen: payment.amount_yen,
            email: order.email.clone(),
            method: payment.method_kind(),
            return_url: return_url.clone(),
            external_order_num: external_order_num.clone(),
        };
        match gateway.create_session(&request).await {
            Ok(created) => {
                // Record the session in its own transaction —
                // the provider call is already over, so nothing
                // here holds a transaction across the network.
                let mut tx = pool.begin().await?;
                payment_repository::set_session(
                    &mut *tx,
                    payment.id,
                    &created.session_id,
                    &external_order_num,
                )
                .await?;
                tx.commit().await?;
                // A konbini session is born with its payment
                // already authorized at the provider: apply that
                // sighting through the one path, so instructions
                // and the mail behave identically however they
                // are first learned.
                match gateway.fetch_session(&created.session_id).await {
                    Ok(state) => {
                        if let Some(sighting) = state.payment {
                            apply_provider_payment(pool, &sighting).await?;
                        }
                    }
                    Err(e) => {
                        // Not silent: the born-authorized
                        // sighting (and its instructions mail)
                        // now waits for a later sweep or webhook.
                        tracing::warn!(order_id, "payment session re-read failed: {e}");
                    }
                }
                return Ok(created.session_url);
            }
            Err(e) => {
                tracing::warn!(order_id, attempt, "payment session creation failed: {e}");
                last_error = Some(e);
            }
        }
    }
    Err(StartSessionError::Gateway(
        last_error.expect("at least one attempt ran"),
    ))
}

/// Why a session could not be started after placement.
#[derive(Debug)]
pub enum StartSessionError {
    /// The integration is not available in this process.
    Unavailable,
    /// Every creation attempt failed at the provider.
    Gateway(GatewayError),
    /// The order or its payment row is not there (never in
    /// practice — placement created both in one transaction).
    OrderGone,
    /// Recording the session failed.
    Db(sqlx::Error),
    /// Applying the session's first sighting failed.
    Apply(AppError),
}

impl From<sqlx::Error> for StartSessionError {
    fn from(e: sqlx::Error) -> Self {
        Self::Db(e)
    }
}

impl From<AppError> for StartSessionError {
    fn from(e: AppError) -> Self {
        Self::Apply(e)
    }
}

impl std::fmt::Display for StartSessionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unavailable => write!(f, "KOMOJU is not available"),
            Self::Gateway(e) => write!(f, "{e}"),
            Self::OrderGone => write!(f, "order or payment row missing at session start"),
            Self::Db(e) => write!(f, "database failure at session start: {e}"),
            Self::Apply(e) => write!(f, "applying the first sighting failed: {e}"),
        }
    }
}

/// What one status application did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ApplyOutcome {
    /// The payment the sighting resolved to.
    pub payment_id: i64,
    /// Whether a status was written (a refresh counts).
    pub applied: bool,
    /// Whether the sighting was quarantined for a human instead
    /// (amount mismatch, money after closure).
    pub flagged: bool,
}

/// Applies one provider sighting to its payment: THE status
/// path. Webhook deliveries, return fetches, admin syncs and
/// sweep reconciliations all funnel through here, so a capture
/// means the same thing however it is learned.
///
/// Returns `None` when the sighting matches no payment we know —
/// recorded by the webhook caller, impossible by construction
/// for the fetch-based callers.
///
/// # Errors
///
/// Returns [`AppError`] on database failure; the webhook maps
/// that to a 500 so KOMOJU redelivers.
pub async fn apply_provider_payment(
    pool: &PgPool,
    sighting: &ProviderPayment,
) -> Result<Option<ApplyOutcome>, AppError> {
    // Correlate outside the transaction: payment ID first (the
    // strongest key), then the session it came from, then the
    // external order number actually sent.
    let found = match payment_repository::find_by_provider_payment_id(pool, &sighting.id).await? {
        Some(payment) => Some(payment),
        None => match &sighting.session_id {
            Some(session_id) => payment_repository::find_by_session_id(pool, session_id).await?,
            None => None,
        },
    };
    let found = match found {
        Some(payment) => Some(payment),
        None => match &sighting.external_order_num {
            Some(number) => payment_repository::find_by_external_order_num(pool, number).await?,
            None => None,
        },
    };
    let Some(found) = found else {
        return Ok(None);
    };

    // The points expiry window for the reversal this flow
    // may run, read before the transaction opens (the A6
    // settings pattern).
    let points_expiry_days = crate::services::shop_settings_service::load(pool)
        .await
        .points_settings()
        .expiry_days;
    let mut tx = pool.begin().await?;
    // One lock order everywhere: order, then payment.
    let order = order_repository::lock_by_id(&mut tx, found.order_id)
        .await?
        .ok_or(AppError::NotFound)?;
    let payment = payment_repository::lock_by_id(&mut tx, found.id)
        .await?
        .ok_or(AppError::NotFound)?;

    // Amount integrity before any transition: a sighting whose
    // total or currency is not the order's is quarantined —
    // flagged for a human, never applied, never auto-refunded.
    if sighting.total_yen != payment.amount_yen || sighting.currency != payment.currency {
        payment_repository::set_flag(
            &mut *tx,
            payment.id,
            &format!(
                "金額または通貨が注文と一致しませんでした（{} total: {} {} / 注文: {} {}）。確認が必要です。",
                provider_display_name(&payment.provider),
                sighting.total_yen, sighting.currency, payment.amount_yen, payment.currency
            ),
        )
        .await?;
        tx.commit().await?;
        tracing::error!(
            payment_id = payment.id,
            order_id = order.id,
            "payment sighting quarantined: amount or currency mismatch"
        );
        return Ok(Some(ApplyOutcome {
            payment_id: payment.id,
            applied: false,
            flagged: true,
        }));
    }

    let transition = payment::next_status(
        &payment.status,
        &sighting.status,
        sighting.amount_refunded_yen,
        payment.amount_yen,
    );
    match transition {
        Transition::Ignore => {
            // Recorded by the caller (the webhook ledger); the
            // state machine deliberately leaves the row alone.
            Ok(Some(ApplyOutcome {
                payment_id: payment.id,
                applied: false,
                flagged: false,
            }))
        }
        Transition::FlagLateCapture => {
            // TC-14: money arrived after the order was closed.
            // Flag it for a human refund decision; do not move
            // the status, do not touch the order.
            payment_repository::set_flag(
                &mut *tx,
                payment.id,
                &format!(
                    "キャンセル・期限切れの後に{}から入金が確認されました。返金対応が必要です。",
                    provider_display_name(&payment.provider)
                ),
            )
            .await?;
            tx.commit().await?;
            tracing::error!(
                payment_id = payment.id,
                order_id = order.id,
                "late capture after order closure; flagged for manual refund"
            );
            Ok(Some(ApplyOutcome {
                payment_id: payment.id,
                applied: false,
                flagged: true,
            }))
        }
        Transition::Apply(new_status) => {
            let was_paid = payment.is_paid();
            let became_paid = matches!(
                new_status,
                payment::STATUS_PAID | payment::STATUS_PARTIALLY_REFUNDED
            ) && !was_paid;
            let instructions_arrived =
                sighting.instructions_url.is_some() && payment.instructions_url.is_none();
            let terminal_without_capture = matches!(
                new_status,
                payment::STATUS_EXPIRED | payment::STATUS_CANCELLED
            );
            // Compensation first, while the row still reads
            // unsettled (it flips the payment to cancelled as
            // part of the cancellation); the apply below then
            // writes the provider's true closing word over it.
            if terminal_without_capture {
                order_service::cancel_order_in_tx(&mut tx, &order, true, points_expiry_days)
                    .await?;
            }
            payment_repository::apply_state(
                &mut *tx,
                payment.id,
                &payment_repository::ProviderStateWrite {
                    status: new_status,
                    provider_payment_id: Some(sighting.id.clone()),
                    payment_deadline: sighting.payment_deadline,
                    instructions_url: sighting.instructions_url.clone(),
                    payment_details: sighting.payment_details.clone(),
                    captured_at: sighting.captured_at,
                    amount_refunded_yen: sighting.amount_refunded_yen,
                },
            )
            .await?;
            // A sighting that completes a refund creates the
            // compensation obligation in this same
            // transaction: the marker
            // is what the refund path and the sweep drive
            // the restock + points reversal from, so a crash
            // between the refund landing and the compensation
            // can no longer strand either.
            if payment.amount_yen > 0 && sighting.amount_refunded_yen >= payment.amount_yen {
                payment_repository::set_compensation_pending(&mut *tx, payment.id, true).await?;
            }
            // The exactly-once claim, in this transaction: only
            // the path that wins it sends the mail, after commit.
            let mail_claimed = if became_paid {
                payment_repository::claim_confirmation_mail(&mut *tx, payment.id).await?
            } else {
                false
            };
            tx.commit().await?;
            if mail_claimed {
                spawn_payment_confirmation_mail(pool.clone(), order.id);
            }
            if instructions_arrived && payment.method_kind() == PaymentMethod::Konbini {
                spawn_konbini_instructions_mail(pool.clone(), payment.id);
            }
            Ok(Some(ApplyOutcome {
                payment_id: payment.id,
                applied: true,
                flagged: false,
            }))
        }
    }
}

/// Sends the payment confirmation for a settled KOMOJU order,
/// spawned like every mail. This is NOT the checkout
/// controller's confirmation: that one is the cash-on-delivery
/// wording (pay at the door), while a captured KOMOJU order
/// needs the paid wording — the claim in the apply transaction
/// is what makes this exactly-once, this helper only sends.
fn spawn_payment_confirmation_mail(pool: PgPool, order_id: i64) {
    tokio::spawn(async move {
        let sent: Result<(), String> = async {
            let order = order_service::find_order(&pool, order_id)
                .await
                .map_err(|e| e.to_string())?
                .ok_or("order vanished before payment confirmation mail".to_string())?;
            let lines = order_service::order_lines(&pool, order_id)
                .await
                .map_err(|e| e.to_string())?;
            let breakdown = crate::repositories::order_repository::tax_breakdowns(&pool, order_id)
                .await
                .unwrap_or_default();
            let method_label = payment_repository::find_by_order_id(&pool, order_id)
                .await
                .ok()
                .flatten()
                .and_then(|p| crate::models::PaymentMethod::from_slug(&p.method))
                .map(|m| m.label())
                .unwrap_or("");
            crate::mail::send_payment_confirmation(&pool, &order, &lines, &breakdown, method_label)
                .await
                .map_err(|e| e.to_string())
        }
        .await;
        if let Err(message) = sent {
            tracing::error!(order_id, "payment confirmation mail failed: {message}");
        }
    });
}

/// Sends the konbini instructions mail, spawned like every mail:
/// loads the order, its lines and the payment's instructions,
/// and logs a failure against the payment ID only.
fn spawn_konbini_instructions_mail(pool: PgPool, payment_id: i64) {
    tokio::spawn(async move {
        let sent: Result<(), String> = async {
            let payment = payment_repository::find_by_id(&pool, payment_id)
                .await
                .map_err(|e| e.to_string())?
                .ok_or("payment vanished before instructions mail".to_string())?;
            let order = order_service::find_order(&pool, payment.order_id)
                .await
                .map_err(|e| e.to_string())?
                .ok_or("order vanished before instructions mail".to_string())?;
            let lines = order_service::order_lines(&pool, payment.order_id)
                .await
                .map_err(|e| e.to_string())?;
            let breakdown =
                crate::repositories::order_repository::tax_breakdowns(&pool, payment.order_id)
                    .await
                    .unwrap_or_default();
            crate::mail::send_konbini_instructions(&pool, &order, &lines, &breakdown, &payment)
                .await
                .map_err(|e| e.to_string())
        }
        .await;
        if let Err(message) = sent {
            tracing::error!(payment_id, "konbini instructions mail failed: {message}");
        }
    });
}

/// Fetches the provider's current truth for one payment and
/// applies it. The return handler, the admin sync and the sweep
/// all reconcile this way; a payment the provider has not
/// created yet simply has nothing to apply yet.
///
/// # Errors
///
/// Returns [`GatewayError`] from the fetch, or [`AppError`] from
/// applying what it said.
pub async fn sync_from_provider(
    pool: &PgPool,
    gateway: &Arc<dyn PaymentGateway>,
    payment: &Payment,
) -> Result<Option<ApplyOutcome>, SyncError> {
    let sighting = if let Some(payment_id) = payment.provider_payment_id() {
        Some(gateway.fetch_payment(payment_id).await?)
    } else if let Some(session_id) = payment.provider_session_id() {
        gateway.fetch_session(session_id).await?.payment
    } else {
        None
    };
    match sighting {
        Some(sighting) => Ok(apply_provider_payment(pool, &sighting).await?),
        None => Ok(None),
    }
}

/// Why a provider sync did not settle.
#[derive(Debug)]
pub enum SyncError {
    /// The provider fetch failed.
    Gateway(GatewayError),
    /// Applying the answer failed.
    Db(AppError),
}

impl From<GatewayError> for SyncError {
    fn from(e: GatewayError) -> Self {
        Self::Gateway(e)
    }
}

impl From<AppError> for SyncError {
    fn from(e: AppError) -> Self {
        Self::Db(e)
    }
}

impl From<sqlx::Error> for SyncError {
    fn from(e: sqlx::Error) -> Self {
        Self::Db(AppError::from(e))
    }
}

impl std::fmt::Display for SyncError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Gateway(e) => write!(f, "{e}"),
            Self::Db(e) => write!(f, "{e}"),
        }
    }
}

/// Handles one verified webhook event: records it in the ledger
/// first (a duplicate event id is a redelivery — acknowledged,
/// never re-applied), then routes it.
///
/// The receiver has already verified the signature over the raw
/// body; this function owns everything after that.
///
/// # Errors
///
/// Returns [`AppError`] on database failure, which the receiver
/// maps to a 500 so KOMOJU redelivers on its own schedule.
pub async fn handle_webhook_event(
    pool: &PgPool,
    event_id: &str,
    delivery_id: Option<&str>,
    event_type: &str,
    raw_body: &serde_json::Value,
    sighting: Option<&ProviderPayment>,
) -> Result<WebhookOutcome, AppError> {
    let first_delivery =
        payment_repository::record_event(pool, event_id, delivery_id, event_type, raw_body).await?;
    if !first_delivery && payment_repository::event_applied(pool, event_id).await? {
        return Ok(WebhookOutcome::Duplicate);
    }
    if !first_delivery {
        // The first delivery was recorded but its apply step
        // failed (it answered 500, so the provider retried):
        // this redelivery runs the apply path below instead of
        // being acknowledged as a duplicate.
        tracing::info!(
            event_id,
            "webhook redelivery of an unapplied event; applying now"
        );
    }
    // `ping` deliveries and events without a payment object are
    // recorded and acknowledged: the ledger is their answer.
    let Some(sighting) = sighting else {
        payment_repository::mark_event_outcome(pool, event_id, None, false).await?;
        return Ok(WebhookOutcome::RecordedOnly);
    };
    match apply_provider_payment(pool, sighting).await? {
        Some(outcome) => {
            payment_repository::mark_event_outcome(
                pool,
                event_id,
                Some(outcome.payment_id),
                outcome.applied,
            )
            .await?;
            Ok(WebhookOutcome::Applied(outcome))
        }
        None => {
            // A real event about a payment we cannot correlate:
            // recorded, acknowledged (redelivery cannot fix it),
            // and loud in the log for a human.
            tracing::warn!(
                event_id,
                provider_payment_id = sighting.id.as_str(),
                "webhook event matched no payment; recorded only"
            );
            payment_repository::mark_event_outcome(pool, event_id, None, false).await?;
            Ok(WebhookOutcome::RecordedOnly)
        }
    }
}

/// What one webhook delivery amounted to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WebhookOutcome {
    /// The event id was already recorded and applied: a redelivery.
    Duplicate,
    /// Recorded, nothing to apply (ping, unparseable, unmatched).
    RecordedOnly,
    /// Recorded and routed through the status path.
    Applied(ApplyOutcome),
}

/// Runs a full refund's compensation — the guarded
/// restock and the points reversal — and clears the
/// payment's pending marker, all in one transaction.
/// Both halves are idempotent by their own guards (the
/// order's restock stamp, the points reversal stamp), so
/// re-driving a half-finished compensation is safe: this
/// is called by the refund paths right after the refund
/// lands and by the sweep for every payment still flagged.
async fn compensate_full_refund(
    pool: &PgPool,
    payment_id: i64,
    order_id: i64,
) -> Result<(), AppError> {
    // The points expiry window for the reversal this flow
    // may run, read before the transaction opens (the A6
    // settings pattern).
    let points_expiry_days = crate::services::shop_settings_service::load(pool)
        .await
        .points_settings()
        .expiry_days;
    let mut tx = pool.begin().await?;
    crate::services::order_service::restock_order_in_tx(&mut tx, order_id, "refund").await?;
    let order = crate::repositories::order_repository::lock_by_id(&mut tx, order_id).await?;
    match &order {
        Some(order) => {
            if let Some(user_id) = order.user_id {
                crate::services::point_service::reverse_order_in_tx(
                    &mut tx,
                    user_id,
                    order_id,
                    points_expiry_days,
                )
                .await?;
            }
        }
        None => {
            // The order row is gone (member-withdrawal
            // cleanup or an inconsistent restore): neither
            // half can run and the points state cannot be
            // reconstructed from here. Say so loudly and
            // clear the flag anyway:
            // leaving it set would retry a compensation
            // that can never succeed on every sweep pass
            // forever, and clearing it silently — the old
            // behaviour — stranded the reversal with no
            // trace at all.
            tracing::error!(
                payment_id,
                order_id,
                "refund compensation skipped: the order no longer exists; \
                 clearing the pending flag without compensating"
            );
        }
    }
    payment_repository::set_compensation_pending(&mut *tx, payment_id, false).await?;
    tx.commit().await?;
    Ok(())
}

/// Refunds a settled card/PayPay payment, fully or partially,
/// and applies the provider's answer through the one path.
///
/// Konbini refunds deliberately have no in-admin flow in V1:
/// they run through KOMOJU's refund request from the dashboard,
/// so no bank details are ever collected or stored here.
///
/// # Errors
///
/// Returns [`RefundError::NotEligible`] with the operator-facing
/// reason when the payment cannot be refunded from here,
/// [`RefundError::Gateway`] when the provider refused (no state
/// changes then), or [`RefundError::Db`] on database failure.
pub async fn refund(
    pool: &PgPool,
    gateway: &Arc<dyn PaymentGateway>,
    order_id: i64,
    amount_yen: Option<i64>,
    expected_refunded_yen: Option<i64>,
) -> Result<Payment, RefundError> {
    let payment = payment_repository::find_by_order_id(pool, order_id)
        .await?
        .ok_or_else(|| RefundError::NotEligible(PAYMENT_INFO_MISSING_MESSAGE.to_string()))?;
    // A paid ¥0 payment — an order points covered in full,
    // whatever the method — has nothing for a gateway to
    // give back: no session, no
    // KOMOJU payment id, nothing to charge the refund
    // against. It refunds in place, full reversal only:
    // the row is marked refunded and the same compensation
    // as a cash full refund runs.
    if payment.amount_yen == 0 && payment.is_paid() {
        if let Some(expected) = expected_refunded_yen
            && expected != payment.amount_refunded_yen
        {
            return Err(RefundError::NotEligible(
                "返金の状態が既に変更されています。最新の状態を確認してください。".to_string(),
            ));
        }
        if let Some(amount) = amount_yen
            && amount != 0
        {
            return Err(RefundError::NotEligible(
                "ポイント全額払いの返金は全額返金のみ可能です。".to_string(),
            ));
        }
        let mut tx = pool.begin().await?;
        payment_repository::apply_state(
            &mut *tx,
            payment.id,
            &payment_repository::ProviderStateWrite {
                status: payment::STATUS_REFUNDED,
                provider_payment_id: None,
                payment_deadline: None,
                instructions_url: None,
                payment_details: None,
                captured_at: None,
                amount_refunded_yen: 0,
            },
        )
        .await?;
        payment_repository::set_compensation_pending(&mut *tx, payment.id, true).await?;
        tx.commit().await?;
        if let Err(e) = compensate_full_refund(pool, payment.id, order_id).await {
            tracing::error!(
                payment_id = payment.id,
                order_id,
                "¥0 refund compensation failed; left flagged for the sweep: {e}"
            );
        }
        return payment_repository::find_by_id(pool, payment.id)
            .await?
            .ok_or_else(|| RefundError::NotEligible(PAYMENT_INFO_MISSING_MESSAGE.to_string()));
    }
    if !payment.is_hosted() || !payment.is_paid() {
        return Err(RefundError::NotEligible(format!(
            "入金済みの{}決済だけが返金できます。",
            provider_display_name(&payment.provider)
        )));
    }
    if payment.method_kind() == PaymentMethod::Konbini {
        return Err(RefundError::NotEligible(format!(
            "コンビニ決済の返金は{}ダッシュボードから返金リクエストで行ってください。",
            provider_display_name(&payment.provider)
        )));
    }
    let Some(provider_payment_id) = payment.provider_payment_id().map(str::to_string) else {
        return Err(RefundError::NotEligible(format!(
            "{}の支払いIDが記録されていません。",
            provider_display_name(&payment.provider)
        )));
    };
    // Staleness guard (back-office rule): the admin form
    // carries the refunded total it was opened with; if
    // another refund has landed since, refuse instead of
    // silently refunding against a balance the operator
    // never saw.
    if let Some(expected) = expected_refunded_yen
        && expected != payment.amount_refunded_yen
    {
        return Err(RefundError::NotEligible(
            "返金の状態が既に変更されています。最新の状態を確認してください。".to_string(),
        ));
    }
    let remaining = payment.amount_yen - payment.amount_refunded_yen;
    let amount = amount_yen.unwrap_or(remaining);
    if amount <= 0 || amount > remaining {
        return Err(RefundError::NotEligible(format!(
            "返金額は1円以上、残り{}円以下で指定してください。",
            crate::pricing::display_price(remaining)
        )));
    }
    let sighting = gateway
        .refund(&provider_payment_id, Some(amount))
        .await
        .map_err(RefundError::Gateway)?;
    apply_provider_payment(pool, &sighting).await?;
    let updated = payment_repository::find_by_id(pool, payment.id)
        .await?
        .ok_or_else(|| RefundError::NotEligible(PAYMENT_INFO_MISSING_MESSAGE.to_string()))?;
    // A FULL refund returns the order's stock and reverses
    // its points (a partial refund keeps the goods with the
    // customer, so it never restocks). The apply above set
    // the pending marker in its transaction; running the
    // compensation here clears it. A failure no longer
    // strands the order: the refund
    // itself has landed, so it is logged at error level and
    // the sweep re-drives the compensation from the marker.
    if updated.amount_refunded_yen >= updated.amount_yen && updated.amount_yen > 0 {
        if let Err(e) = compensate_full_refund(pool, payment.id, order_id).await {
            tracing::error!(
                payment_id = payment.id,
                order_id,
                "refund compensation failed; left flagged for the sweep: {e}"
            );
        }
        let refreshed = payment_repository::find_by_id(pool, payment.id).await?;
        return Ok(refreshed.unwrap_or(updated));
    }
    Ok(updated)
}

/// Why a refund did not happen.
#[derive(Debug)]
pub enum RefundError {
    /// The payment is not refundable from the back office; the
    /// message is operator-facing and safe to show.
    NotEligible(String),
    /// The provider refused or could not be reached; nothing
    /// changed.
    Gateway(GatewayError),
    /// A database failure.
    Db(AppError),
}

impl From<sqlx::Error> for RefundError {
    fn from(e: sqlx::Error) -> Self {
        Self::Db(AppError::from(e))
    }
}

impl From<AppError> for RefundError {
    fn from(e: AppError) -> Self {
        Self::Db(e)
    }
}

impl std::fmt::Display for RefundError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotEligible(message) => write!(f, "{message}"),
            Self::Gateway(e) => write!(f, "{e}"),
            Self::Db(e) => write!(f, "{e}"),
        }
    }
}

/// Expires one still-open payment in its own transaction: the
/// provider-side window has closed (konbini deadline passed, or
/// the card/PayPay abandonment TTL), so the order is cancelled
/// with the payment compensation (stock and coupon released).
async fn expire_payment(pool: &PgPool, payment_id: i64) -> Result<(), AppError> {
    // The points expiry window for the reversal this flow
    // may run, read before the transaction opens (the A6
    // settings pattern).
    let points_expiry_days = crate::services::shop_settings_service::load(pool)
        .await
        .points_settings()
        .expiry_days;
    let mut tx = pool.begin().await?;
    let Some(found) = payment_repository::find_by_id(pool, payment_id).await? else {
        return Ok(());
    };
    let order = order_repository::lock_by_id(&mut tx, found.order_id)
        .await?
        .ok_or(AppError::NotFound)?;
    let payment = payment_repository::lock_by_id(&mut tx, payment_id)
        .await?
        .ok_or(AppError::NotFound)?;
    if !payment.is_open() {
        return Ok(());
    }
    // Compensation first (it flips the payment to cancelled);
    // the expiry word is written over it after, as in the apply
    // path.
    order_service::cancel_order_in_tx(&mut tx, &order, true, points_expiry_days).await?;
    payment_repository::apply_state(
        &mut *tx,
        payment.id,
        &payment_repository::ProviderStateWrite {
            status: payment::STATUS_EXPIRED,
            provider_payment_id: None,
            payment_deadline: None,
            instructions_url: None,
            payment_details: None,
            captured_at: None,
            amount_refunded_yen: payment.amount_refunded_yen,
        },
    )
    .await?;
    tx.commit().await?;
    tracing::info!(
        payment_id,
        order_id = order.id,
        "payment expired; order cancelled"
    );
    Ok(())
}

/// Runs one reconciliation pass over every open KOMOJU payment:
///
/// - konbini past its deadline: sync once (a last-moment store
///   payment still counts), then expire if it is still open;
/// - anything unsettled and stale: sync from the provider, so a
///   lost webhook is a delay, not a divergence;
/// - card/PayPay unsettled past the abandonment TTL: void at
///   the provider when there is a payment to void, then expire.
///
/// One payment's failure never stops the pass.
/// The advisory-lock key of one payment sweep pass: a fixed
/// constant, defined exactly once, so every process agrees
/// on which lock serialises the sweep. Session-level and
/// held on one dedicated connection for the whole pass, so
/// a crashed pass frees it with its connection.
const PAYMENT_SWEEP_LOCK_KEY: i64 = 741_001;

pub async fn sweep_once(pool: &PgPool) {
    // Multi-instance guard (the background-task house rule):
    // the sweep holds no atomic claim over its work — it
    // lists open payments and calls the provider — so one
    // pass runs at a time across every process sharing this
    // database. Not acquiring the lock is a normal outcome,
    // logged at debug and skipped quietly.
    let mut conn = match pool.acquire().await {
        Ok(conn) => conn,
        Err(e) => {
            tracing::error!("payment sweep could not acquire a connection: {e}");
            return;
        }
    };
    let locked = match sqlx::query_scalar::<_, bool>("SELECT pg_try_advisory_lock($1)")
        .bind(PAYMENT_SWEEP_LOCK_KEY)
        .fetch_one(&mut *conn)
        .await
    {
        Ok(locked) => locked,
        Err(e) => {
            tracing::error!("payment sweep could not take the sweep lock: {e}");
            return;
        }
    };
    if !locked {
        tracing::debug!("payment sweep skipped: another instance holds the sweep lock");
        return;
    }
    // Refund-compensation retries run first and need no
    // gateway (a ¥0 or COD compensation never talks to
    // KOMOJU): every payment still flagged gets its restock
    // + points reversal re-driven under this same lock, so
    // a compensation that failed beside its refund heals
    // here instead of stranding.
    match payment_repository::list_compensation_pending(pool, 50).await {
        Ok(flagged) => {
            for payment in flagged {
                if let Err(e) = compensate_full_refund(pool, payment.id, payment.order_id).await {
                    tracing::error!(
                        payment_id = payment.id,
                        order_id = payment.order_id,
                        "sweep refund-compensation retry failed: {e}"
                    );
                }
            }
        }
        Err(e) => {
            tracing::error!("payment sweep could not list pending compensations: {e}");
        }
    }
    let mut any_gateway = false;
    for provider in ["komoju", "stripe"] {
        let Some(gateway) = gateway_for(provider) else {
            continue;
        };
        any_gateway = true;
        sweep_pass(pool, &gateway, provider).await;
    }
    if any_gateway {
        // The ops status panel reads the sweep's last run
        // from this record; a failed write is logged,
        // never fatal.
        if let Err(e) =
            crate::repositories::ops_repository::record_job_run(pool, "payment_sweep", "完了").await
        {
            tracing::warn!("payment sweep could not record its run: {e}");
        }
    }
    // Explicit unlock; the connection's close would free it
    // anyway, but releasing here returns the connection to
    // the pool already unlocked. A failed unlock leaves the
    // session-scoped lock on the connection: close it rather
    // than returning it to the pool, where it would stall
    // later sweeps.
    if let Err(e) = sqlx::query("SELECT pg_advisory_unlock($1)")
        .bind(PAYMENT_SWEEP_LOCK_KEY)
        .execute(&mut *conn)
        .await
    {
        tracing::error!("payment sweep unlock failed; closing the lock connection: {e}");
        let _ = conn.close().await;
    }
}

/// One reconciliation pass, run holding the sweep lock (see
/// [`sweep_once`]).
async fn sweep_pass(pool: &PgPool, gateway: &Arc<dyn PaymentGateway>, provider: &str) {
    let open = match payment_repository::list_open_hosted(pool, provider).await {
        Ok(open) => open,
        Err(e) => {
            tracing::error!("payment sweep could not list open payments: {e}");
            return;
        }
    };
    let now = Utc::now();
    for payment in open {
        if let Err(e) = sweep_one(pool, gateway, &payment, now).await {
            // A provider 404 (the provider has no record of
            // the payment) is an expected sighting for aged
            // sessions, not an operational error: at error
            // level its volume buried real failures
            //.
            if matches!(e, SyncError::Gateway(GatewayError::Api { status: 404, .. })) {
                tracing::warn!(
                    payment_id = payment.id,
                    "payment sweep: provider has no record of this payment: {e}"
                );
            } else {
                tracing::error!(
                    payment_id = payment.id,
                    "payment sweep failed for one payment: {e}"
                );
            }
        }
    }
}

/// Reconciles one open payment (see [`sweep_once`]).
async fn sweep_one(
    pool: &PgPool,
    gateway: &Arc<dyn PaymentGateway>,
    payment: &Payment,
    now: DateTime<Utc>,
) -> Result<(), SyncError> {
    let method = payment.method_kind();
    // Konbini: the provider's deadline governs.
    if method == PaymentMethod::Konbini
        && let Some(deadline) = payment.payment_deadline
        && deadline < now
    {
        match sync_from_provider(pool, gateway, payment).await {
            Ok(_) => {}
            // The provider has no record of this payment
            // past its deadline it
            // can never report it paid, so the expiry below
            // proceeds instead of the sync error stranding
            // the payment in the sweep's open set forever.
            // Any other failure still aborts this payment's
            // pass — a transient provider error must not
            // expire a payment that may in fact be paid.
            Err(SyncError::Gateway(GatewayError::Api { status: 404, .. })) => {
                tracing::warn!(
                    payment_id = payment.id,
                    "payment sweep: no provider record for a past-deadline konbini payment; expiring it"
                );
            }
            Err(e) => return Err(e),
        }
        let fresh = payment_repository::find_by_id(pool, payment.id).await?;
        if let Some(fresh) = fresh
            && fresh.is_open()
        {
            expire_payment(pool, fresh.id).await?;
        }
        return Ok(());
    }
    // Card/PayPay: the abandonment TTL.
    if method != PaymentMethod::Konbini
        && payment.created_at + Duration::hours(ABANDONED_TTL_HOURS) < now
    {
        if let Some(provider_payment_id) = payment.provider_payment_id() {
            // Best effort: void the open payment so the customer
            // cannot pay a session we are about to abandon. The
            // answer applies through the one path; if it did not
            // close the payment, the expiry below still does.
            if let Ok(sighting) = gateway.cancel(provider_payment_id).await {
                apply_provider_payment(pool, &sighting).await?;
            }
        }
        let fresh = payment_repository::find_by_id(pool, payment.id).await?;
        if let Some(fresh) = fresh
            && fresh.is_open()
        {
            expire_payment(pool, fresh.id).await?;
        }
        return Ok(());
    }
    // Everything else: reconcile when stale.
    if payment.updated_at + Duration::minutes(SWEEP_STALE_MINUTES) < now {
        sync_from_provider(pool, gateway, payment).await?;
    }
    Ok(())
}

/// Starts the background sweep: one pass per configured
/// interval, until the shutdown watch fires. The task runs
/// even without a gateway: provider
/// reconciliation idles, but the refund-compensation retries
/// inside each pass need no gateway — a yen-0 or COD
/// compensation never talks to KOMOJU — and stranding them
/// on a missing gateway was the bug this fixes.
///
/// The caller keeps the returned handle and drains the task at
/// shutdown (P-3): the watch is consulted only between passes,
/// so a pass in flight always runs to completion and its
/// writes are never cut in half.
pub fn start_background(
    pool: PgPool,
    mut shutdown: tokio::sync::watch::Receiver<bool>,
) -> Option<tokio::task::JoinHandle<()>> {
    let interval_secs = CONFIG.payment_sweep_interval_secs.max(10);
    let handle = tokio::spawn(async move {
        let mut ticker = tokio::time::interval(std::time::Duration::from_secs(interval_secs));
        loop {
            tokio::select! {
                _ = ticker.tick() => {}
                _ = shutdown.changed() => break,
            }
            sweep_once(&pool).await;
        }
        tracing::info!("payment reconciliation sweep stopped");
    });
    tracing::info!(interval_secs, "payment reconciliation sweep started");
    Some(handle)
}
