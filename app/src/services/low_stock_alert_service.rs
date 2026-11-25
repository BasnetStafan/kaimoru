//! Low-stock alert worker: mails the operator once a day
//! about public products at or below their stock threshold.
//!
//! The dashboard already counts these products; this pass is
//! the push version, so a sell-out does not wait for someone
//! to open the dashboard. One advisory lock per pass keeps
//! two processes from evaluating the same morning twice, the
//! `low_stock_alerts` primary key is the per-day claim that
//! makes a repeat pass a no-op, and rows are recorded only
//! when a mail actually went out — a day with no recipient
//! configured never suppresses a later day's alert.

use sqlx::PgPool;
use tokio::sync::watch::Receiver;

use crate::config::CONFIG;
use crate::error::AppError;
use crate::repositories::low_stock_alert_repository;
use crate::services::admin::dashboard_service;
use crate::services::shop_settings_service;

/// The advisory-lock key of one low-stock alert pass: a fixed
/// constant, so two app processes can never run the pass at
/// the same time (the payment sweep's pattern, its own key).
const LOW_STOCK_ALERT_LOCK_KEY: i64 = 741_002;

/// Starts the low-stock alert loop, paced by the typed
/// config interval and stopped by the same watch channel as
/// the other supervised tasks.
pub fn start_worker(pool: PgPool, mut shutdown: Receiver<bool>) -> tokio::task::JoinHandle<()> {
    let interval_secs = CONFIG.low_stock_alert_interval_secs.max(60);
    let handle = tokio::spawn(async move {
        let mut ticker = tokio::time::interval(std::time::Duration::from_secs(interval_secs));
        loop {
            tokio::select! {
            _ = ticker.tick() => {}
            _ = shutdown.changed() => break,
            }
            if let Err(e) = run_once(&pool).await {
                tracing::error!("low stock alert pass failed: {e}");
            }
        }
        tracing::info!("low stock alert worker stopped");
    });
    tracing::info!(interval_secs, "low stock alert worker started");
    handle
}

/// Runs one alert pass: finds the products under their
/// threshold, drops the ones already mailed about today, and
/// sends one summary mail to the shop's contact address.
///
/// # Errors
///
/// Returns [`AppError`] when the product read fails; a failed
/// send is logged and recorded nowhere, so the next pass
/// retries the same products.
pub async fn run_once(pool: &PgPool) -> Result<(), AppError> {
    // One pass at a time across processes: the lock lives on
    // a dedicated pooled connection for the pass's duration.
    let mut conn = pool
        .acquire()
        .await
        .map_err(|e| AppError::db("low stock alert lock connection", e))?;
    let locked = sqlx::query_scalar::<_, bool>("SELECT pg_try_advisory_lock($1)")
        .bind(LOW_STOCK_ALERT_LOCK_KEY)
        .fetch_one(&mut *conn)
        .await
        .map_err(|e| AppError::db("low stock alert try lock", e))?;
    if !locked {
        tracing::info!("low stock alert pass skipped: another process holds the lock");
        return Ok(());
    }
    let result = run_pass(pool).await;
    // Same rule as the reminder pass: a failed unlock closes
    // the lock connection instead of pooling a locked one.
    if let Err(e) = sqlx::query("SELECT pg_advisory_unlock($1)")
        .bind(LOW_STOCK_ALERT_LOCK_KEY)
        .execute(&mut *conn)
        .await
    {
        tracing::error!("low stock alert unlock failed; closing the lock connection: {e}");
        let _ = conn.close().await;
    }
    result
}

/// The pass itself, under the caller's advisory lock.
async fn run_pass(pool: &PgPool) -> Result<(), AppError> {
    let products = dashboard_service::low_stock_products(pool, CONFIG.low_stock_default).await?;
    if products.is_empty() {
        return Ok(());
    }
    let today = crate::models::jst::jst_today();
    let ids: Vec<i64> = products.iter().map(|p| p.id).collect();
    let alerted = low_stock_alert_repository::alerted_on(pool, &ids, today).await?;
    let fresh: Vec<_> = products
        .iter()
        .filter(|p| !alerted.contains(&p.id))
        .collect();
    if fresh.is_empty() {
        return Ok(());
    }
    let settings = shop_settings_service::load(pool).await;
    if settings.contact_email.is_empty() {
        // Nobody to mail: the dashboard still shows these
        // products, and nothing is recorded, so the day the
        // operator sets a contact address the alerts begin.
        tracing::warn!(
            count = fresh.len(),
            "low stock alert skipped: no contact email in shop settings"
        );
        return Ok(());
    }
    let items: Vec<(i64, String, i64)> = fresh
        .iter()
        .map(|p| (p.id, p.name.clone(), i64::from(p.stock)))
        .collect();
    // A failed send records nothing: the next pass retries
    // the same products instead of losing the alert to a
    // bad mail day.
    if let Err(e) = crate::mail::send_low_stock_alert(pool, &settings.contact_email, &items).await {
        tracing::error!("low stock alert send failed: {e}");
        return Ok(());
    }
    for (product_id, _, _) in &items {
        low_stock_alert_repository::record(pool, *product_id, today).await?;
    }
    tracing::info!(
        count = items.len(),
        action = "product.low_stock_alert",
        "low stock alert mailed"
    );
    Ok(())
}
