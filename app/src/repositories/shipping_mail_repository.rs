//! Shipping-mail outbox repository: the queue rows the bulk
//! ship paths write and the shipping-mail worker drains.

use sqlx::PgPool;

use crate::error::AppError;

/// One claimed outbox row: the order to mail and when the row
/// was enqueued (the JST date of that instant is the shipped
/// date the mail names).
#[derive(sqlx::FromRow)]
pub struct OutboxRow {
    /// Outbox row ID.
    pub id: i64,
    /// The shipped order to notify about.
    pub order_id: i64,
    /// The parcel to notify about; `None` is the
    /// order-level single-shipment mail.
    pub shipment_id: Option<i64>,
    /// When the ship transition enqueued the row.
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// Enqueues one order's shipping mail inside the caller's
/// transaction. Idempotent per order: a second enqueue (a
/// re-ship cannot happen, but a racing bulk action might try)
/// keeps the first row, so the worker can never mail twice
/// for one shipment.
pub async fn enqueue_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    order_id: i64,
) -> Result<(), AppError> {
    sqlx::query(
        "INSERT INTO shipping_mail_outbox (order_id) VALUES ($1) \
         ON CONFLICT (order_id) DO NOTHING",
    )
    .bind(order_id)
    .execute(&mut **tx)
    .await
    .map_err(|e| AppError::db("enqueue shipping mail", e))?;
    Ok(())
}

/// Claims one batch of queued rows: the rows flip to `sending`
/// and stamp `claimed_at` in the same statement that selects
/// them, `SKIP LOCKED` keeping two worker processes from ever
/// claiming the same row. The caller sends exactly the rows it
/// claimed (the mail foundation's claim pattern).
pub async fn claim_batch(pool: &PgPool, limit: i64) -> Result<Vec<OutboxRow>, AppError> {
    sqlx::query_as::<_, OutboxRow>(
        "WITH claimed AS ( \
             SELECT id FROM shipping_mail_outbox \
             WHERE status = 'queued' \
             ORDER BY id LIMIT $1 FOR UPDATE SKIP LOCKED \
         ) \
         UPDATE shipping_mail_outbox o SET status = 'sending', claimed_at = now() \
         FROM claimed WHERE o.id = claimed.id \
         RETURNING o.id, o.order_id, o.shipment_id, o.created_at",
    )
    .bind(limit)
    .fetch_all(pool)
    .await
    .map_err(|e| AppError::db("claim shipping mail batch", e))
}

/// Enqueues one parcel's notification inside the caller's
/// transaction: one row per shipment, kept idempotent
/// by the scoped unique index.
pub async fn enqueue_shipment_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    order_id: i64,
    shipment_id: i64,
) -> Result<(), AppError> {
    sqlx::query(
        "INSERT INTO shipping_mail_outbox (order_id, shipment_id) \
         VALUES ($1, $2) \
         ON CONFLICT (shipment_id) WHERE shipment_id IS NOT NULL \
         DO NOTHING",
    )
    .bind(order_id)
    .bind(shipment_id)
    .execute(&mut **tx)
    .await
    .map_err(|e| AppError::db("enqueue shipment mail", e))?;
    Ok(())
}

/// Returns stale claims to the queue: a worker that died
/// mid-send leaves its rows in `sending`, and once the claim
/// is older than the staleness window another pass may pick
/// them up. The send itself is stamp-claimed, so a reclaimed
/// row whose mail actually went out sends nothing twice.
/// Returns the rows reclaimed.
pub async fn reclaim_stale_claims(pool: &PgPool, stale_minutes: i64) -> Result<u64, AppError> {
    let result = sqlx::query(
        "UPDATE shipping_mail_outbox SET status = 'queued', claimed_at = NULL \
         WHERE status = 'sending' AND claimed_at < now() - ($1 * interval '1 minute')",
    )
    .bind(stale_minutes)
    .execute(pool)
    .await
    .map_err(|e| AppError::db("reclaim stale shipping mail claims", e))?;
    Ok(result.rows_affected())
}

/// Marks one claimed row sent.
pub async fn mark_sent(pool: &PgPool, id: i64) -> Result<(), AppError> {
    sqlx::query("UPDATE shipping_mail_outbox SET status = 'sent', sent_at = now() WHERE id = $1")
        .bind(id)
        .execute(pool)
        .await
        .map_err(|e| AppError::db("mark shipping mail sent", e))?;
    Ok(())
}

/// Records one failed send attempt on a claimed row: back to
/// `queued` for a later pass while attempts remain, terminal
/// `failed` once they run out — the order detail's unsent-mail
/// state stays the operator's recovery signal either way.
pub async fn mark_attempt_failed(
    pool: &PgPool,
    id: i64,
    max_attempts: i32,
) -> Result<(), AppError> {
    sqlx::query(
        "UPDATE shipping_mail_outbox \
         SET attempts = attempts + 1, \
             status = CASE WHEN attempts + 1 >= $2 THEN 'failed' ELSE 'queued' END, \
             claimed_at = NULL \
         WHERE id = $1",
    )
    .bind(id)
    .bind(max_attempts)
    .execute(pool)
    .await
    .map_err(|e| AppError::db("mark shipping mail attempt failed", e))?;
    Ok(())
}

/// Counts rows still waiting (queued or sending): the ops
/// status panel's shipping-mail backlog figure.
pub async fn count_waiting(pool: &PgPool) -> Result<i64, AppError> {
    sqlx::query_scalar::<_, i64>(
        "SELECT count(*) FROM shipping_mail_outbox WHERE status IN ('queued', 'sending')",
    )
    .fetch_one(pool)
    .await
    .map_err(|e| AppError::db("count waiting shipping mail", e))
}

/// How many queued shipping mails exhausted their attempts:
/// the ops status panel's failure figure.
pub async fn count_failed(pool: &PgPool) -> Result<i64, AppError> {
    sqlx::query_scalar::<_, i64>(
        "SELECT count(*) FROM shipping_mail_outbox WHERE status = 'failed'",
    )
    .fetch_one(pool)
    .await
    .map_err(|e| AppError::db("shipping mail failed count", e))
}
