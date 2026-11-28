//! Back-in-stock subscription repository: registration
//! upserts, the sweep's due/claim/send bookkeeping, and the
//! admin counts. The email column is PII ciphertext; the
//! blind index column beside it carries lookups and the
//! uniqueness rule, exactly as on the users table.

use sqlx::PgPool;

use crate::models::restock::{RestockDue, RestockSubscription};
use crate::repositories::variant_repository::LABEL_SQL;

/// Inserts one subscription, ignoring a duplicate (variant,
/// email) pair. Returns whether a row was inserted.
pub async fn insert_ignore(
    pool: &PgPool,
    variant_id: i64,
    user_id: Option<i64>,
    email_encrypted: &str,
    email_bidx: &str,
) -> Result<bool, sqlx::Error> {
    let inserted = sqlx::query(
        "INSERT INTO restock_subscriptions (variant_id, user_id, email, email_bidx) \
         VALUES ($1, $2, $3, $4) ON CONFLICT (variant_id, email_bidx) DO NOTHING",
    )
    .bind(variant_id)
    .bind(user_id)
    .bind(email_encrypted)
    .bind(email_bidx)
    .execute(pool)
    .await?;
    Ok(inserted.rows_affected() > 0)
}

/// Returns the status of one (variant, email) subscription,
/// when a row exists.
pub async fn find_status(
    pool: &PgPool,
    variant_id: i64,
    email_bidx: &str,
) -> Result<Option<String>, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT status FROM restock_subscriptions WHERE variant_id = $1 AND email_bidx = $2",
    )
    .bind(variant_id)
    .bind(email_bidx)
    .fetch_optional(pool)
    .await
}

/// Returns a finished subscription (sent or cancelled) to
/// pending: re-registering after a notification starts a
/// fresh one-shot subscription for the next sell-out.
pub async fn reset_to_pending(
    pool: &PgPool,
    variant_id: i64,
    email_bidx: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE restock_subscriptions \
         SET status = 'pending', claimed_at = NULL, notified_at = NULL \
         WHERE variant_id = $1 AND email_bidx = $2",
    )
    .bind(variant_id)
    .bind(email_bidx)
    .execute(pool)
    .await?;
    Ok(())
}

/// Loads one subscription by ID, decrypted by the caller.
pub async fn find_by_id(
    pool: &PgPool,
    id: i64,
) -> Result<Option<RestockSubscription>, sqlx::Error> {
    let row = sqlx::query_as::<_, RestockSubscription>(
        "SELECT id, variant_id, user_id, email, status, created_at, notified_at \
         FROM restock_subscriptions WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|mut sub| {
        crate::pii::decrypt_restock(&mut sub);
        sub
    }))
}

/// Returns the pending subscriptions whose combination is
/// back in stock on a still-public product, oldest first,
/// bounded to one sweep batch.
pub async fn due_batch(pool: &PgPool, limit: i64) -> Result<Vec<RestockDue>, sqlx::Error> {
    let sql = format!(
        "SELECT s.id, s.email AS email_encrypted, v.product_id, p.name AS product_name, \
         {LABEL_SQL} AS variant_label \
         FROM restock_subscriptions s \
         JOIN product_variants v ON v.id = s.variant_id \
         JOIN products p ON p.id = v.product_id \
         WHERE s.status = 'pending' AND v.stock > 0 AND v.is_enabled AND p.status = 'public' \
         ORDER BY s.created_at, s.id LIMIT $1"
    );
    sqlx::query_as::<_, RestockDueRow>(&sql)
        .bind(limit)
        .fetch_all(pool)
        .await
        .map(|rows| rows.into_iter().map(RestockDue::from).collect())
}

/// Row shape for [`due_batch`]; the public struct is built
/// from it so the query's column aliases stay private.
#[derive(sqlx::FromRow)]
struct RestockDueRow {
    /// Subscription ID.
    id: i64,
    /// The encrypted notification address.
    email_encrypted: String,
    /// The product the combination belongs to.
    product_id: i64,
    /// The product's current name.
    product_name: String,
    /// The combination's display label.
    variant_label: String,
}

impl From<RestockDueRow> for RestockDue {
    fn from(row: RestockDueRow) -> Self {
        RestockDue {
            id: row.id,
            email_encrypted: row.email_encrypted,
            product_id: row.product_id,
            product_name: row.product_name,
            variant_label: row.variant_label,
        }
    }
}

/// Claims one pending subscription for sending. Returns
/// whether the claim was won; a lost claim means another
/// pass (or process) is already sending it.
pub async fn claim(pool: &PgPool, id: i64) -> Result<bool, sqlx::Error> {
    let claimed = sqlx::query(
        "UPDATE restock_subscriptions SET status = 'sending', claimed_at = now() \
         WHERE id = $1 AND status = 'pending'",
    )
    .bind(id)
    .execute(pool)
    .await?;
    Ok(claimed.rows_affected() > 0)
}

/// Marks a claimed subscription sent, stamping the time.
pub async fn mark_sent(pool: &PgPool, id: i64) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE restock_subscriptions SET status = 'sent', notified_at = now() \
         WHERE id = $1 AND status = 'sending'",
    )
    .bind(id)
    .execute(pool)
    .await?;
    Ok(())
}

/// Returns a claimed subscription to pending after a failed
/// send, so the next pass retries it.
pub async fn revert_claim(pool: &PgPool, id: i64) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE restock_subscriptions \
         SET status = 'pending', claimed_at = NULL \
         WHERE id = $1 AND status = 'sending'",
    )
    .bind(id)
    .execute(pool)
    .await?;
    Ok(())
}

/// Returns stale claims to pending: a pass that died between
/// claiming and sending must not strand its subscriptions.
/// Returns how many rows were reclaimed.
pub async fn reclaim_stale_claims(pool: &PgPool, minutes: i64) -> Result<u64, sqlx::Error> {
    let reclaimed = sqlx::query(
        "UPDATE restock_subscriptions SET status = 'pending', claimed_at = NULL \
         WHERE status = 'sending' AND claimed_at < now() - $1 * interval '1 minute'",
    )
    .bind(minutes)
    .execute(pool)
    .await?;
    Ok(reclaimed.rows_affected())
}

/// Returns one product's subscription counts as (waiting,
/// sent), for the admin product form's read-only line.
pub async fn counts_for_product(pool: &PgPool, product_id: i64) -> Result<(i64, i64), sqlx::Error> {
    let row: (i64, i64) = sqlx::query_as(
        "SELECT \
           COUNT(*) FILTER (WHERE s.status IN ('pending', 'sending')), \
           COUNT(*) FILTER (WHERE s.status = 'sent') \
         FROM restock_subscriptions s \
         JOIN product_variants v ON v.id = s.variant_id \
         WHERE v.product_id = $1",
    )
    .bind(product_id)
    .fetch_one(pool)
    .await?;
    Ok(row)
}

/// All subscriptions of one product, newest first, for the
/// admin list: waiting and sent alike.
pub async fn list_for_product(
    pool: &PgPool,
    product_id: i64,
) -> Result<Vec<RestockSubscription>, sqlx::Error> {
    let mut rows = sqlx::query_as::<_, RestockSubscription>(
        "SELECT s.id, s.variant_id, s.user_id, s.email, s.status, s.created_at, s.notified_at \
         FROM restock_subscriptions s \
         JOIN product_variants v ON v.id = s.variant_id \
         WHERE v.product_id = $1 ORDER BY s.created_at DESC, s.id DESC",
    )
    .bind(product_id)
    .fetch_all(pool)
    .await?;
    for sub in &mut rows {
        crate::pii::decrypt_restock(sub);
    }
    Ok(rows)
}
