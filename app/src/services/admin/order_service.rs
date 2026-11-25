//! Admin order rules.
//!
//! The fulfilment flow is fixed and forward-only: received →
//! preparing → shipped → delivered, with cancellation allowed
//! only from the first two states. The service re-checks every
//! posted transition against the order's current stored state —
//! a hand-crafted POST naming an illegal jump is rejected, not
//! applied. Cancellation restores stock in the same transaction
//! as the status change and its audit row.

use sqlx::PgPool;

use crate::error::AppError;
use crate::messages::{
    ORDER_NOT_FOUND_MESSAGE, STATUS_ALREADY_CHANGED_MESSAGE, STATUS_CHANGE_NOT_ALLOWED_MESSAGE,
};
use crate::models::order::{OrderLineDetail, legal_next_statuses};
use crate::models::{AdminOrderListRow, Order};
use crate::repositories::{
    admin::order_repository as admin_order_repository, order_repository, payment_repository,
    user_repository,
};
use crate::services::admin::audit_service;
use crate::services::order_service;

/// Returns one page of the admin order list plus the total count.
/// `status` is one of the list's fixed filter values (`unshipped`
/// = received + preparing) or `None` for all; `number` is a
/// partial match against the displayed order number.
#[allow(clippy::too_many_arguments)]
pub async fn list_orders(
    pool: &PgPool,
    status: Option<&str>,
    number: Option<String>,
    payment: Option<&str>,
    placed_from: Option<chrono::DateTime<chrono::Utc>>,
    placed_to: Option<chrono::DateTime<chrono::Utc>>,
    customer: Option<String>,
    limit: i64,
    offset: i64,
) -> Result<(Vec<AdminOrderListRow>, i64), AppError> {
    admin_order_repository::list_orders(
        pool,
        status,
        number,
        payment,
        placed_from,
        placed_to,
        customer,
        limit,
        offset,
    )
    .await
}

/// How many KOMOJU payments are still awaiting payment — the
/// order list's 入金待ち chip count.
pub async fn pending_payment_count(pool: &PgPool) -> Result<i64, AppError> {
    payment_repository::count_pending_hosted(pool)
        .await
        .map_err(|e| AppError::db("pending payment count", e))
}

/// Returns how many orders sit in each fulfilment status, over
/// all orders — the raw rows behind the list's filter chips.
pub async fn status_counts(pool: &PgPool) -> Result<Vec<(String, i64)>, AppError> {
    admin_order_repository::status_counts(pool).await
}

/// Loads one order by ID, for the detail page. Reads through the
/// storefront order repository — orders are one shared table.
pub async fn find_order(pool: &PgPool, order_id: i64) -> Result<Option<Order>, AppError> {
    order_repository::find_by_id(pool, order_id)
        .await
        .map_err(|e| AppError::db("admin order load", e))
}

/// Returns the order's lines with each product's current
/// thumbnail joined in, for the detail page.
pub async fn order_lines(pool: &PgPool, order_id: i64) -> Result<Vec<OrderLineDetail>, AppError> {
    order_repository::lines_with_images(pool, order_id)
        .await
        .map_err(|e| AppError::db("admin order lines", e))
}

/// Returns the buyer display name for the detail page: the
/// member's name, or 「会員」 for a member record that no longer
/// resolves, or 「ゲスト」 for a guest order.
///
/// The name is decoration — the order and its lines are the
/// page's substance — so a failed lookup degrades to the generic
/// label, but it is logged, never silent.
pub async fn buyer_name(pool: &PgPool, order: &Order) -> String {
    let Some(user_id) = order.user_id else {
        return "ゲスト".to_string();
    };
    match user_repository::find_by_id(pool, user_id).await {
        Ok(Some(user)) => format!("{} {}", user.family_name, user.given_name),
        Ok(None) => "会員".to_string(),
        Err(e) => {
            tracing::error!("admin order buyer lookup failed (order {}): {e}", order.id);
            "会員".to_string()
        }
    }
}

/// Why a status change was refused.
pub enum StatusChangeError {
    /// The order does not exist.
    NotFound,
    /// The requested transition is not legal from the current
    /// state (or the target is not a status at all).
    IllegalTransition,
    /// The order has already moved since the form was rendered
    /// (another tab, a double submit): the form's expected status
    /// no longer matches the stored one (the optimistic guard).
    /// Nothing was changed and no audit row was written.
    Stale,
    /// Shipping was refused because a KOMOJU payment has not
    /// settled (TC-17): an unpaid order never leaves the shelf.
    PaymentNotSettled,
    /// The order has split shipments: it ships parcel
    /// by parcel, never as a whole.
    SplitShipment,
    /// The shipping details posted with a ship transition
    /// failed validation (an unknown carrier code, a malformed
    /// tracking number); nothing was changed.
    InvalidShipping(String),
    /// A database failure, already logged at its source.
    Db(AppError),
}

impl From<AppError> for StatusChangeError {
    fn from(e: AppError) -> Self {
        Self::Db(e)
    }
}

/// How a ship transition's notification reaches the
/// customer: mailed inline after the commit (the single-order
/// path's long-standing behaviour), or enqueued into the
/// shipping-mail outbox inside the transition's transaction
/// (the bulk paths, where a request must never block on SMTP).
#[derive(Clone, Copy, PartialEq, Eq)]
enum ShipMailMode {
    /// Send after the commit, from this call.
    Inline,
    /// Enqueue an outbox row in the transaction; the
    /// shipping-mail worker sends it.
    Queued,
}

/// Moves one order to `target`, re-checking legality against the
/// stored state inside the transaction. On cancellation, every
/// tracked line's quantity returns to stock in the same commit
/// (the restock is exactly-once). Audited
/// as `order.status`; the audit schema stores IDs only (no
/// from/to payload).
pub async fn change_status(
    pool: &PgPool,
    admin_id: i64,
    order_id: i64,
    target: &str,
    expected: Option<&str>,
    carrier: &str,
    tracking_number: &str,
) -> Result<(), StatusChangeError> {
    change_status_with_mail_mode(
        pool,
        admin_id,
        order_id,
        target,
        expected,
        carrier,
        tracking_number,
        ShipMailMode::Inline,
    )
    .await
}

/// [`change_status`] with the ship notification's delivery
/// mode chosen by the caller; the transition itself — guards,
/// compensation, audit — is identical on both paths.
#[allow(clippy::too_many_arguments)]
async fn change_status_with_mail_mode(
    pool: &PgPool,
    admin_id: i64,
    order_id: i64,
    target: &str,
    expected: Option<&str>,
    carrier: &str,
    tracking_number: &str,
    mail_mode: ShipMailMode,
) -> Result<(), StatusChangeError> {
    // The shipping details are validated before anything is
    // written: they are inputs to a ship transition only, and
    // a malformed pair refuses the whole change before it
    // starts, exactly like an illegal transition.
    let shipping = if target == "shipped" {
        Some(
            crate::models::delivery::validate_shipping_details(carrier, tracking_number)
                .map_err(|e| StatusChangeError::InvalidShipping(e.message().to_string()))?,
        )
    } else {
        None
    };
    // The points settings are read BEFORE the transaction
    // opens: a settings save landing
    // mid-transition must not change the earn rules under a
    // shipment whose order state was read at its start, nor
    // the expiry window a cancellation's points reversal
    // restores with.
    let points_settings = if target == "shipped" || target == "cancelled" {
        Some(
            crate::services::shop_settings_service::load(pool)
                .await
                .points_settings(),
        )
    } else {
        None
    };
    let mut tx = pool
        .begin()
        .await
        .map_err(|e| StatusChangeError::Db(AppError::db("begin status change", e)))?;
    // Read the current status inside the transaction, row
    // locked; a missing order is the caller's styled 404.
    let current = admin_order_repository::current_status(&mut tx, order_id).await?;
    let Some(current) = current else {
        return Err(StatusChangeError::NotFound);
    };
    // The optimistic guard: when the form carried the status
    // it was rendered against, a mismatch means the order has
    // already moved — refuse without touching anything.
    if let Some(expected) = expected
        && expected != current
    {
        return Err(StatusChangeError::Stale);
    }
    if !legal_next_statuses(&current).contains(&target) {
        return Err(StatusChangeError::IllegalTransition);
    }
    if target == "shipped"
        && crate::repositories::admin::shipment_repository::order_has_shipments(&mut *tx, order_id)
            .await
            .map_err(|e| StatusChangeError::Db(AppError::db("split check", e)))?
    {
        // C5: an order with parcels ships parcel by parcel;
        // the whole-order ship would skip parcels and double
        // the notification.
        return Err(StatusChangeError::SplitShipment);
    }
    // The payment row, read inside the transaction: the
    // shipping guard and the cancellation's coupon decision
    // both depend on it.
    let payment = payment_repository::find_by_order_id(&mut *tx, order_id)
        .await
        .map_err(|e| StatusChangeError::Db(AppError::db("status change payment load", e)))?;
    let hosted_unsettled = payment
        .as_ref()
        .is_some_and(|p| p.is_hosted() && !p.is_paid());
    if target == "shipped" && hosted_unsettled {
        // TC-17: an order whose hosted money has not settled
        // never ships. COD and settled orders pass as always.
        return Err(StatusChangeError::PaymentNotSettled);
    }
    if target == "cancelled" {
        // The shared cancellation compensation (status, stock,
        // the payment flip): the same function the payment
        // state machine uses, so the two paths can never
        // compensate differently. The once-only coupon
        // follows the shared rule (Order Cancellation spec
        // 5.4, D3): released if and only if no money was
        // ever settled on the order — the same rule the
        // member cancel path applies, so who clicked makes
        // no difference to the coupon outcome.
        let release_coupon = !payment.as_ref().is_some_and(|p| p.money_settled());
        let order = order_repository::lock_by_id(&mut tx, order_id)
            .await
            .map_err(|e| StatusChangeError::Db(AppError::db("status change order lock", e)))?
            .ok_or(StatusChangeError::NotFound)?;
        let points = points_settings.expect("loaded for a cancel transition above");
        order_service::cancel_order_in_tx(&mut tx, &order, release_coupon, points.expiry_days)
            .await?;
    } else {
        admin_order_repository::set_order_status(&mut tx, order_id, target).await?;
        if target == "shipped" {
            // Points earn grants at shipment (EC V2 core 1),
            // in the transition's own transaction.
            let order = order_repository::lock_by_id(&mut tx, order_id)
                .await
                .map_err(|e| StatusChangeError::Db(AppError::db("status change earn lock", e)))?
                .ok_or(StatusChangeError::NotFound)?;
            let points = points_settings.expect("loaded for a ship transition above");
            crate::services::point_service::grant_earn_for_order(&mut tx, &order, points)
                .await
                .map_err(StatusChangeError::Db)?;
        }
        if let Some((carrier_code, tracking)) = &shipping {
            // The details ride the transition's own
            // transaction: they describe this shipment, and a
            // rolled-back transition must not leave them
            // behind on an unshipped order.
            order_repository::set_shipping_details(
                &mut tx,
                order_id,
                *carrier_code,
                tracking.as_deref(),
            )
            .await
            .map_err(|e| {
                StatusChangeError::Db(AppError::db("status change shipping details", e))
            })?;
        }
        if target == "delivered"
            && let Some(payment) = &payment
            && payment.provider == "cod"
            && payment.status == crate::models::payment::STATUS_PENDING
        {
            // Collection at the door is the COD settlement: the
            // payment row follows the delivery, so every screen
            // reads one truth.
            payment_repository::set_status(
                &mut *tx,
                payment.id,
                crate::models::payment::STATUS_PAID,
            )
            .await
            .map_err(|e| StatusChangeError::Db(AppError::db("status change payment flip", e)))?;
        }
    }
    audit_service::record(&mut tx, admin_id, "order.status", "order", Some(order_id)).await?;
    if target == "shipped" && mail_mode == ShipMailMode::Queued {
        // The outbox row is part of the transition: a committed
        // bulk shipment always has its notification queued, and
        // a rolled-back one never leaves a mail behind.
        crate::repositories::shipping_mail_repository::enqueue_tx(&mut tx, order_id).await?;
    }
    tx.commit()
        .await
        .map_err(|e| StatusChangeError::Db(AppError::db("commit status change", e)))?;
    // The shipping notification follows the committed ship
    // transition: sent after the commit, never inside the
    // transaction, and a failed send never fails the shipment
    // — the order's unset mail stamp keeps the failure visible
    // on the order detail until a resend succeeds. Queued mode
    // leaves the send to the worker instead.
    if target == "shipped" && mail_mode == ShipMailMode::Inline {
        let _ = send_shipping_mail(pool, order_id, Some(crate::models::jst::jst_today())).await;
    }
    Ok(())
}

/// One order's outcome in a bulk status change.
pub struct BulkOutcome {
    /// The order the outcome describes.
    pub order_id: i64,
    /// Whether the transition was applied.
    pub applied: bool,
    /// Why the order was skipped, for the operator's summary;
    /// empty when applied.
    pub reason: String,
}

/// Moves many orders to one `target`, one transaction per
/// order: an order that refuses (already moved, illegal jump,
/// unsettled payment) is skipped with its reason and never
/// blocks or half-applies its neighbours. Ship transitions
/// take a carrier for the whole batch and no tracking numbers
/// — per-order numbers arrive through the tracking CSV import
/// or the detail form. Audited per order, exactly like the
/// single-order path. A database failure aborts the batch at
/// that order; orders already applied stay applied.
///
/// # Errors
///
/// Returns [`StatusChangeError::InvalidShipping`] when the
/// batch's carrier is not in the catalogue (checked before
/// anything is written), or [`StatusChangeError::Db`] on a
/// database failure.
pub async fn change_status_bulk(
    pool: &PgPool,
    admin_id: i64,
    items: &[(i64, String)],
    target: &str,
    carrier: &str,
) -> Result<Vec<BulkOutcome>, StatusChangeError> {
    if target == "shipped" {
        // The batch's one shipping input is validated once, up
        // front: a bad carrier refuses the whole batch before
        // a single order moves.
        crate::models::delivery::validate_shipping_details(carrier, "")
            .map_err(|e| StatusChangeError::InvalidShipping(e.message().to_string()))?;
    }
    let mut outcomes = Vec::with_capacity(items.len());
    for (order_id, expected) in items {
        let result = change_status_with_mail_mode(
            pool,
            admin_id,
            *order_id,
            target,
            Some(expected.as_str()),
            carrier,
            "",
            ShipMailMode::Queued,
        )
        .await;
        let (applied, reason) = match result {
            Ok(()) => (true, String::new()),
            Err(StatusChangeError::NotFound) => (false, ORDER_NOT_FOUND_MESSAGE.to_string()),
            Err(StatusChangeError::Stale) => (false, STATUS_ALREADY_CHANGED_MESSAGE.to_string()),
            Err(StatusChangeError::IllegalTransition) => {
                (false, STATUS_CHANGE_NOT_ALLOWED_MESSAGE.to_string())
            }
            Err(StatusChangeError::PaymentNotSettled) => {
                (false, "入金が確認できていません".to_string())
            }
            Err(StatusChangeError::SplitShipment) => (
                false,
                "分割出荷中のため、出荷単位で発送してください".to_string(),
            ),
            Err(StatusChangeError::InvalidShipping(message)) => (false, message),
            Err(StatusChangeError::Db(e)) => return Err(StatusChangeError::Db(e)),
        };
        outcomes.push(BulkOutcome {
            order_id: *order_id,
            applied,
            reason,
        });
    }
    Ok(outcomes)
}

/// Moves one order with its ship notification queued in the
/// outbox instead of mailed from the call: the tracking CSV
/// import's path, where one request may ship many orders and
/// must never block on SMTP. All guards are [`change_status`]'s.
pub async fn change_status_queued_mail(
    pool: &PgPool,
    admin_id: i64,
    order_id: i64,
    target: &str,
    expected: Option<&str>,
    carrier: &str,
    tracking_number: &str,
) -> Result<(), StatusChangeError> {
    change_status_with_mail_mode(
        pool,
        admin_id,
        order_id,
        target,
        expected,
        carrier,
        tracking_number,
        ShipMailMode::Queued,
    )
    .await
}

/// The outcome of one shipping-mail send attempt.
#[derive(Debug, PartialEq, Eq)]
pub enum ShippingMailOutcome {
    /// The mail was sent and the order's stamp was set.
    Sent,
    /// A previous send already stamped the order; nothing was
    /// sent.
    AlreadySent,
    /// The send failed (or the order could not be loaded); the
    /// stamp stays unset, so the failure remains visible.
    Failed,
}

/// Sends the shipping notification for one shipped order and
/// stamps the order on success (Shipping Mail spec sections
/// 3.1/5.2). The stamp check runs under the order row lock;
/// the lock is released before the SMTP call — a transaction
/// is never held across a network call — and the stamp's
/// conditional update is the atomic claim that decides which
/// of two racing sends counts. Logs carry the action name and
/// the order ID only, never the address or the body.
pub async fn send_shipping_mail(
    pool: &PgPool,
    order_id: i64,
    shipped_date: Option<chrono::NaiveDate>,
) -> ShippingMailOutcome {
    let mut tx = match pool.begin().await {
        Ok(tx) => tx,
        Err(_) => return ShippingMailOutcome::Failed,
    };
    let order = match order_repository::lock_by_id(&mut tx, order_id).await {
        Ok(Some(order)) => order,
        _ => return ShippingMailOutcome::Failed,
    };
    if order.shipped_mail_sent_at.is_some() {
        return ShippingMailOutcome::AlreadySent;
    }
    // The row lock did its work (the stamp check); end the
    // transaction before the network call.
    let _ = tx.rollback().await;
    let lines = match order_service::order_lines(pool, order_id).await {
        Ok(lines) => lines,
        Err(_) => return ShippingMailOutcome::Failed,
    };
    // The mail error type is not Send, so the send outcome is
    // reduced to a flag before the stamping await; holding the
    // Result across it would make this future non-Send and the
    // admin handlers that await it unusable.
    let breakdown = crate::repositories::order_repository::tax_breakdowns(pool, order_id)
        .await
        .unwrap_or_default();
    let sent =
        crate::mail::send_shipping_notification(pool, &order, &lines, &breakdown, shipped_date)
            .await
            .is_ok();
    if !sent {
        tracing::warn!(
            order_id,
            action = "order.shipping_mail",
            "shipping mail send failed"
        );
        return ShippingMailOutcome::Failed;
    }
    match order_repository::stamp_shipped_mail_sent(pool, order_id).await {
        Ok(true) => {
            tracing::info!(
                order_id,
                action = "order.shipping_mail",
                "shipping mail sent"
            );
            ShippingMailOutcome::Sent
        }
        // A racing send stamped first; this send is the
        // duplicate the stamp exists to prevent, and the
        // order is correctly marked sent either way.
        Ok(false) => ShippingMailOutcome::AlreadySent,
        Err(_) => ShippingMailOutcome::Failed,
    }
}

/// Why a shipping-mail resend was refused before any send.
#[derive(Debug)]
pub enum ResendMailError {
    /// The order does not exist.
    NotFound,
    /// The order has not shipped; there is nothing to notify
    /// about.
    NotShipped,
    /// A database failure, already logged at its source.
    Db(AppError),
}

impl From<AppError> for ResendMailError {
    fn from(e: AppError) -> Self {
        Self::Db(e)
    }
}

/// Resends the shipping notification for a shipped order
/// whose automatic send failed — or which was shipped before
/// this feature existed and honestly shows as unsent. The
/// resend carries no shipped date: the store does not record
/// when the parcel actually left beyond the transition day,
/// and inventing one would falsify the mail, so the date
/// block is omitted (the spec's stated fallback). A
/// successful resend is audited; an order already stamped is
/// refused by the send itself and sends nothing.
pub async fn resend_shipping_mail(
    pool: &PgPool,
    admin_id: i64,
    order_id: i64,
) -> Result<ShippingMailOutcome, ResendMailError> {
    let order = find_order(pool, order_id)
        .await?
        .ok_or(ResendMailError::NotFound)?;
    if order.status != "shipped" && order.status != "delivered" {
        return Err(ResendMailError::NotShipped);
    }
    let outcome = send_shipping_mail(pool, order_id, None).await;
    if outcome == ShippingMailOutcome::Sent {
        let mut tx = pool.begin().await.map_err(|e| {
            ResendMailError::Db(AppError::db("begin shipping mail resend audit", e))
        })?;
        audit_service::record(
            &mut tx,
            admin_id,
            "order.shipping_mail_resend",
            "order",
            Some(order_id),
        )
        .await?;
        tx.commit().await.map_err(|e| {
            ResendMailError::Db(AppError::db("commit shipping mail resend audit", e))
        })?;
    }
    Ok(outcome)
}

#[allow(dead_code)]
fn send_probe(pool: &PgPool) {
    fn assert_send<T: Send>(_: T) {}
    assert_send(send_shipping_mail(pool, 0, None));
    assert_send(change_status(pool, 0, 0, "shipped", None, "", ""));
}
