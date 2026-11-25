//! Shipping-mail worker: drains the outbox the bulk ship
//! paths fill.
//!
//! The single-order ship transition mails inline after its
//! commit (the long-standing behaviour). A bulk action must
//! not hold a request open across dozens of SMTP sends, so
//! bulk transitions enqueue instead; this worker claims rows
//! atomically and sends through
//! [`crate::services::admin::order_service::send_shipping_mail`],
//! whose stamp claim is the send-time guard: an order already
//! stamped (by any path) sends nothing, so the outbox can
//! never produce a duplicate notification. Transactional
//! order mail is never subscription-suppressed — it goes to
//! the order's own address by design; the stamp is its only
//! suppression rule, re-checked at send time inside the send.

use sqlx::PgPool;
use tokio::sync::watch::Receiver;

use crate::error::AppError;
use crate::repositories::shipping_mail_repository;
use crate::services::admin::order_service::{ShippingMailOutcome, send_shipping_mail};

/// Rows claimed per worker pass.
const BATCH_SIZE: i64 = 20;
/// Minutes a claimed row may sit in `sending` before a later
/// pass treats its worker as crashed and reclaims it.
const STALE_CLAIM_MINUTES: i64 = 15;
/// Send attempts before a row is parked as `failed`; the
/// order's unsent stamp remains visible for a manual resend.
const MAX_ATTEMPTS: i32 = 3;
/// Seconds between worker passes.
const WORKER_INTERVAL_SECS: u64 = 30;

/// Starts the shipping-mail worker loop, paced like the other
/// supervised background tasks and stopped by the same watch
/// channel.
pub fn start_worker(pool: PgPool, mut shutdown: Receiver<bool>) -> tokio::task::JoinHandle<()> {
    let handle = tokio::spawn(async move {
        let mut ticker =
            tokio::time::interval(std::time::Duration::from_secs(WORKER_INTERVAL_SECS));
        loop {
            tokio::select! {
            _ = ticker.tick() => {}
            _ = shutdown.changed() => break,
            }
            if let Err(e) = run_once(&pool).await {
                tracing::error!("shipping mail worker pass failed: {e}");
            }
        }
        tracing::info!("shipping mail worker stopped");
    });
    tracing::info!(
        interval_secs = WORKER_INTERVAL_SECS,
        "shipping mail worker started"
    );
    handle
}

/// Runs one worker pass: reclaims stale claims, then sends
/// each claimed row's mail. A single row's failure is recorded
/// on the row and never fails the pass.
///
/// # Errors
///
/// Returns [`AppError`] when the claim read fails.
pub async fn run_once(pool: &PgPool) -> Result<(), AppError> {
    match shipping_mail_repository::reclaim_stale_claims(pool, STALE_CLAIM_MINUTES).await {
        Ok(reclaimed) if reclaimed > 0 => {
            tracing::warn!(reclaimed, "shipping mail worker reclaimed stale claims");
        }
        Ok(_) => {}
        Err(e) => {
            tracing::error!("shipping mail stale-claim reclaim failed: {e}");
        }
    }
    let rows = shipping_mail_repository::claim_batch(pool, BATCH_SIZE).await?;
    for row in rows {
        // The shipped date the mail names is the JST day the
        // ship transition enqueued the row.
        let shipped_date = row
            .created_at
            .with_timezone(&crate::models::jst::jst())
            .date_naive();
        // A row naming a shipment mails that parcel
        // through the parcel send; the rest are order-level.
        let outcome = match row.shipment_id {
            Some(shipment_id) => {
                crate::services::admin::shipment_service::send_shipment_mail(
                    pool,
                    row.order_id,
                    shipment_id,
                    Some(shipped_date),
                )
                .await
            }
            None => send_shipping_mail(pool, row.order_id, Some(shipped_date)).await,
        };
        match outcome {
            ShippingMailOutcome::Sent | ShippingMailOutcome::AlreadySent => {
                shipping_mail_repository::mark_sent(pool, row.id).await?;
            }
            ShippingMailOutcome::Failed => {
                tracing::warn!(
                    order_id = row.order_id,
                    action = "order.shipping_mail",
                    "queued shipping mail send failed"
                );
                shipping_mail_repository::mark_attempt_failed(pool, row.id, MAX_ATTEMPTS).await?;
            }
        }
    }
    Ok(())
}
