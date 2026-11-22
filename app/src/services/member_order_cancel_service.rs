//! Member order cancellation (Order Cancellation spec).
//!
//! One compensation, two doors: the member path
//! runs the same `order_service::cancel_order_in_tx` the
//! admin path uses, inside one locked transaction, and only
//! then — after the commit — talks to the payment gateway
//! about money already captured (C4: never hold a database
//! transaction across a network call). The coupon rule is
//! shared with the admin door: released if and
//! only if no money was ever settled on the order. The
//! member is the audit actor through `actor_user_id`.

use sqlx::PgPool;

use crate::error::AppError;
use crate::models::Order;
use crate::models::payment::{Payment, PaymentMethod};
use crate::repositories::{order_repository, payment_repository};
use crate::services::{admin::audit_service, order_service, payment_service};

/// What a cancellation means for the member's money. One
/// derivation drives the confirmation page's contract
/// wording, the order detail's completion block and the
/// cancellation mail, so the three can never disagree
///.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MoneyOutcome {
    /// Nothing was taken (COD, unsettled or failed hosted
    /// payment): nothing comes back.
    NoneTaken,
    /// Captured card / PayPay money was refunded.
    Refunded,
    /// A refund is owed but has not landed (the gateway
    /// call failed or payments are unavailable): the
    /// payment row is flagged for the shop to retry.
    Processing,
    /// Captured konbini money: refundable only by hand
    /// through the provider's refund-request flow.
    KonbiniManual,
}

/// The result of one member cancel attempt.
pub enum MemberCancelOutcome {
    /// The cancel committed; carries the money outcome.
    Cancelled(MoneyOutcome),
    /// The order was already cancelled (by anyone): the
    /// idempotent success of spec section 5.5 — no second
    /// compensation, no second refund attempt.
    AlreadyCancelled,
    /// The order has moved past `received`; nothing changed.
    Refused,
    /// Missing or another member's order.
    NotFound,
}

/// The flag left on a captured konbini payment whose
/// member cancelled: the admin order detail surfaces
/// `flag_note`, and this text is the refund-to-arrange
/// marker. The dashboard named
/// is the payment's own provider.
fn konbini_refund_flag(provider: &str) -> String {
    format!(
        "会員キャンセル：返金対応が必要（コンビニ決済は{}ダッシュボードの返金リクエストで対応してください）",
        payment_service::provider_display_name(provider)
    )
}

/// The flag left when an automatic refund could not be
/// completed after the cancel committed.
const REFUND_FAILED_FLAG: &str =
    "会員キャンセル：自動返金に失敗しました。管理画面から返金を再実行してください。";

/// The money state an order is in, read off its payment
/// row: the confirmation page's consequence derives from
/// this, before anything is cancelled.
enum MoneyState {
    /// COD, the agent dummy, or a legacy order without a
    /// payment row.
    Cod,
    /// Hosted money that never settled (pending or failed).
    Unsettled,
    /// Captured card / PayPay (fully or partly refunded
    /// already: the remainder is what a cancel refunds).
    CardCaptured,
    /// Konbini authorised, not yet paid at the counter.
    KonbiniPending,
    /// Konbini paid at the counter.
    KonbiniCaptured,
}

/// Classifies an order's payment row for the wording and
/// behaviour decisions.
fn money_state(payment: Option<&Payment>) -> MoneyState {
    let Some(payment) = payment else {
        return MoneyState::Cod;
    };
    if !payment.is_hosted() {
        return MoneyState::Cod;
    }
    if payment.method_kind() == PaymentMethod::Konbini {
        if payment.is_paid() {
            MoneyState::KonbiniCaptured
        } else {
            MoneyState::KonbiniPending
        }
    } else if payment.money_settled() {
        MoneyState::CardCaptured
    } else {
        MoneyState::Unsettled
    }
}

/// The confirmation page's texts: the
/// money consequence, the coupon consequence when a
/// coupon was used, and the shop contact line for the
/// captured-konbini case when the shop publishes one.
pub struct Consequence {
    /// The money paragraph, exactly one of the spec's
    /// wordings.
    pub main: String,
    /// The coupon paragraph, when the order used a coupon.
    pub coupon_line: Option<String>,
    /// The shop contact line (captured konbini only, and
    /// only when the shop settings carry a contact email).
    pub contact_line: Option<String>,
}

/// Composes the confirmation page's contract texts from
/// the order and its payment row.
pub fn consequence_text(
    order: &Order,
    payment: Option<&Payment>,
    contact_email: Option<&str>,
) -> Consequence {
    let main = match money_state(payment) {
        MoneyState::Cod => "お支払いは発生していないため、返金はありません。\
代金引換のお支払いも発生しません。"
            .to_string(),
        MoneyState::Unsettled => "お支払いはまだ完了していないため、返金はありません。\
この注文の決済は無効になります。"
            .to_string(),
        MoneyState::CardCaptured => {
            let remaining = payment
                .map(|p| p.amount_yen - p.amount_refunded_yen)
                .unwrap_or(order.total_yen);
            format!(
                "お支払い済みの {} を、ご利用のお支払い方法へ返金します。\
返金の反映には、カード会社・決済事業者の処理により数日かかる場合があります。",
                crate::pricing::display_price(remaining)
            )
        }
        MoneyState::KonbiniPending => {
            "コンビニでのお支払いは不要になります。すでにお支払い済みの場合は、\
下記の「お支払い済みのコンビニ決済」をご確認ください。"
                .to_string()
        }
        MoneyState::KonbiniCaptured => {
            "コンビニ決済はオンラインで自動返金できません。キャンセル後、店舗から\
返金方法（ご返金先口座の確認）についてご連絡します。"
                .to_string()
        }
    };
    let coupon_line = order.coupon_code.as_ref().map(|code| {
        if payment.is_some_and(|p| p.money_settled()) {
            "ご利用のクーポンは、返金後も再利用できません。".to_string()
        } else {
            format!("ご利用のクーポン「{code}」は、再度ご利用いただけるようになります。")
        }
    });
    let contact_line = match money_state(payment) {
        MoneyState::KonbiniCaptured => contact_email
            .filter(|email| !email.is_empty())
            .map(|email| format!("お問い合わせ：{email}")),
        _ => None,
    };
    Consequence {
        main,
        coupon_line,
        contact_line,
    }
}

/// The refund line on a cancelled order's detail page,
/// derived from the payment row's current state (spec
/// section 5.5). `None` when nothing about money needs
/// saying (unsettled or COD cancellations).
pub fn cancelled_refund_line(payment: Option<&Payment>) -> Option<String> {
    let payment = payment?;
    if payment.status == crate::models::payment::STATUS_REFUNDED {
        return Some("お支払い済みの金額は返金済みです。".to_string());
    }
    if payment.is_paid() && payment.is_hosted() {
        if payment.method_kind() == PaymentMethod::Konbini {
            return Some("コンビニ決済の返金については、店舗からご連絡します。".to_string());
        }
        if payment.flag_note.is_some() {
            return Some("返金処理中です。完了までしばらくお待ちください。".to_string());
        }
    }
    None
}

/// Cancels one of the member's own orders. Eligibility is
/// re-checked under the order row's lock: only `received` cancels; a second cancel is
/// the idempotent success.
///
/// # Errors
///
/// Returns [`AppError`] on database failure; the cancel
/// transaction rolls back with it.
pub async fn cancel_order(
    pool: &PgPool,
    user_id: i64,
    order_id: i64,
) -> Result<MemberCancelOutcome, AppError> {
    let Some(order) = order_service::find_order_for_user(pool, order_id, user_id).await? else {
        return Ok(MemberCancelOutcome::NotFound);
    };
    match order.status.as_str() {
        "cancelled" => return Ok(MemberCancelOutcome::AlreadyCancelled),
        "received" => {}
        _ => return Ok(MemberCancelOutcome::Refused),
    }

    // The points expiry window for the reversal below,
    // read before the transaction opens (the A6 settings
    // pattern).
    let points_expiry_days = crate::services::shop_settings_service::load(pool)
        .await
        .points_settings()
        .expiry_days;
    let mut tx = pool
        .begin()
        .await
        .map_err(|e| AppError::db("member cancel begin", e))?;
    let locked = order_repository::lock_by_id(&mut tx, order_id)
        .await
        .map_err(|e| AppError::db("member cancel order lock", e))?;
    let Some(locked) = locked else {
        return Ok(MemberCancelOutcome::NotFound);
    };
    if locked.user_id != Some(user_id) {
        return Ok(MemberCancelOutcome::NotFound);
    }
    match locked.status.as_str() {
        "cancelled" => {
            tx.rollback()
                .await
                .map_err(|e| AppError::db("member cancel rollback", e))?;
            return Ok(MemberCancelOutcome::AlreadyCancelled);
        }
        "received" => {}
        _ => {
            tx.rollback()
                .await
                .map_err(|e| AppError::db("member cancel rollback", e))?;
            return Ok(MemberCancelOutcome::Refused);
        }
    }
    let payment = payment_repository::find_by_order_id(&mut *tx, order_id)
        .await
        .map_err(|e| AppError::db("member cancel payment load", e))?;
    let state = money_state(payment.as_ref());
    // The shared coupon rule: release if
    // and only if no money was ever settled.
    let release_coupon = !payment.as_ref().is_some_and(|p| p.money_settled());
    let cancelled =
        order_service::cancel_order_in_tx(&mut tx, &locked, release_coupon, points_expiry_days)
            .await?;
    if !cancelled {
        tx.rollback()
            .await
            .map_err(|e| AppError::db("member cancel rollback", e))?;
        return Ok(MemberCancelOutcome::Refused);
    }
    if matches!(state, MoneyState::KonbiniCaptured)
        && let Some(payment) = &payment
    {
        payment_repository::set_flag(
            &mut *tx,
            payment.id,
            &konbini_refund_flag(&payment.provider),
        )
        .await
        .map_err(|e| AppError::db("member cancel konbini flag", e))?;
    }
    audit_service::record_member(
        &mut tx,
        user_id,
        "order.member_cancel",
        "order",
        Some(order_id),
    )
    .await?;
    tx.commit()
        .await
        .map_err(|e| AppError::db("member cancel commit", e))?;
    tracing::info!(
        order_id,
        action = "order.member_cancel",
        "member cancelled order"
    );

    // Money already captured moves after the commit.
    let outcome = match state {
        MoneyState::KonbiniCaptured => MoneyOutcome::KonbiniManual,
        MoneyState::CardCaptured => refund_captured(pool, user_id, order_id).await,
        _ => MoneyOutcome::NoneTaken,
    };
    spawn_cancellation_mail(pool.clone(), order_id, outcome);
    Ok(MemberCancelOutcome::Cancelled(outcome))
}

/// Attempts the post-commit refund of a captured card /
/// PayPay payment through the shared refund path, and
/// flags the payment row when the money could not move
///. Never fails the cancellation: the
/// cancel is the member's settled intent.
async fn refund_captured(pool: &PgPool, user_id: i64, order_id: i64) -> MoneyOutcome {
    // The staleness guard the admin path has always had
    // the refund is computed from
    // THIS payment's refunded total, so a refund that
    // landed between the cancel and here refuses instead
    // of double-refunding against a stale remainder. The
    // same load picks the gateway: dispatch is by the
    // payment ROW's provider, so a payment taken on either
    // rail refunds through its own gateway.
    let loaded = payment_repository::find_by_order_id(pool, order_id).await;
    let expected_refunded = match &loaded {
        Ok(payment) => payment.as_ref().map(|p| p.amount_refunded_yen),
        Err(e) => {
            tracing::error!(order_id, "member cancel refund guard load failed: {e}");
            None
        }
    };
    let Some(gateway) = loaded
        .ok()
        .flatten()
        .and_then(|payment| payment_service::gateway_for(&payment.provider))
    else {
        flag_refund_failed(pool, order_id).await;
        return MoneyOutcome::Processing;
    };
    match payment_service::refund(pool, &gateway, order_id, None, expected_refunded).await {
        Ok(_) => {
            // The refund joins the audit trail with the
            // member as its actor.
            if let Ok(mut tx) = pool.begin().await {
                let recorded = audit_service::record_member(
                    &mut tx,
                    user_id,
                    "payment.refund_requested",
                    "order",
                    Some(order_id),
                )
                .await;
                match recorded {
                    Ok(()) => {
                        let _ = tx.commit().await;
                    }
                    Err(e) => {
                        tracing::error!(order_id, "member refund audit failed: {e}");
                    }
                }
            }
            MoneyOutcome::Refunded
        }
        Err(e) => {
            tracing::error!(order_id, "member cancel refund failed: {e}");
            flag_refund_failed(pool, order_id).await;
            MoneyOutcome::Processing
        }
    }
}

/// Marks a payment whose automatic refund failed, so the
/// admin order detail surfaces it for a manual retry.
async fn flag_refund_failed(pool: &PgPool, order_id: i64) {
    if let Ok(Some(payment)) = payment_repository::find_by_order_id(pool, order_id).await
        && let Err(e) = payment_repository::set_flag(pool, payment.id, REFUND_FAILED_FLAG).await
    {
        tracing::error!(order_id, "member cancel refund flag failed: {e}");
    }
}

/// Spawns the cancellation mail off the request path
/// a slow or failing mail server
/// never blocks or rolls back the cancel.
fn spawn_cancellation_mail(pool: PgPool, order_id: i64, outcome: MoneyOutcome) {
    tokio::spawn(async move {
        let sent: Result<(), String> = async {
            let order = order_service::find_order(&pool, order_id)
                .await
                .map_err(|e| e.to_string())?
                .ok_or("order vanished before cancellation mail".to_string())?;
            let lines = order_service::order_lines(&pool, order_id)
                .await
                .map_err(|e| e.to_string())?;
            let breakdown = crate::repositories::order_repository::tax_breakdowns(&pool, order_id)
                .await
                .unwrap_or_default();
            let money = match outcome {
                MoneyOutcome::NoneTaken => crate::mail::CancellationMoney::NoneTaken,
                MoneyOutcome::Refunded => crate::mail::CancellationMoney::Refunded,
                MoneyOutcome::Processing => crate::mail::CancellationMoney::Processing,
                MoneyOutcome::KonbiniManual => crate::mail::CancellationMoney::KonbiniManual,
            };
            crate::mail::send_order_cancellation(&pool, &order, &lines, &breakdown, money)
                .await
                .map_err(|e| e.to_string())
        }
        .await;
        if let Err(message) = sent {
            tracing::error!(order_id, "cancellation mail failed: {message}");
        }
    });
}
