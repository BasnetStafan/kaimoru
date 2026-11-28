//! Persistence for the points ledger
//! (internal points design record).
//!
//! Every write takes the caller's connection so the caller
//! composes ledger movements into its own transaction —
//! placement, shipment and cancellation each move points
//! atomically with the order change that causes them.

use sqlx::{PgConnection, PgPool};

use crate::models::points::{GrantLot, PointLedgerRow};

/// The member's spendable balance right now: unexpired
/// grant remainders. Expired grants count for nothing
/// whether or not the sweep has reached them yet.
pub async fn balance(pool: &PgPool, user_id: i64) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar::<_, i64>(
        "SELECT COALESCE(SUM(remaining), 0)::bigint FROM point_ledger \
         WHERE user_id = $1 AND kind = 'earn' AND remaining > 0 \
         AND expires_at > now()",
    )
    .bind(user_id)
    .fetch_one(pool)
    .await
}

/// The soonest expiry among the member's spendable grants
/// (what the MyPage points screen shows as 次回失効).
pub async fn next_expiry(
    pool: &PgPool,
    user_id: i64,
) -> Result<Option<(i64, chrono::DateTime<chrono::Utc>)>, sqlx::Error> {
    sqlx::query_as::<_, (i64, chrono::DateTime<chrono::Utc>)>(
        "SELECT remaining, expires_at FROM point_ledger \
         WHERE user_id = $1 AND kind = 'earn' AND remaining > 0 \
         AND expires_at > now() ORDER BY expires_at ASC LIMIT 1",
    )
    .bind(user_id)
    .fetch_optional(pool)
    .await
}

/// Locks the member's spendable grants in expiry order
/// (FIFO) inside the caller's transaction and returns them
/// as lots for the spend planner. The row locks are the
/// double-spend guard: a second placement waits, then sees
/// the reduced remainders.
pub async fn lock_spendable_grants(
    conn: &mut PgConnection,
    user_id: i64,
) -> Result<Vec<GrantLot>, sqlx::Error> {
    let rows = sqlx::query_as::<_, (i64, i64)>(
        "SELECT id, remaining FROM point_ledger \
         WHERE user_id = $1 AND kind = 'earn' AND remaining > 0 \
         AND expires_at > now() ORDER BY expires_at ASC, id ASC \
         FOR UPDATE",
    )
    .bind(user_id)
    .fetch_all(&mut *conn)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(id, remaining)| GrantLot { id, remaining })
        .collect())
}

/// Records one ledger movement and returns its id. For an
/// `earn` row, `remaining` starts at the granted amount and
/// `expires_at` is the grant's expiry; movement rows pass
/// `None` for both.
#[allow(clippy::too_many_arguments)]
pub async fn insert_movement(
    conn: &mut PgConnection,
    user_id: i64,
    kind: &str,
    points: i64,
    remaining: Option<i64>,
    order_id: Option<i64>,
    expires_at: Option<chrono::DateTime<chrono::Utc>>,
) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar::<_, i64>(
        "INSERT INTO point_ledger \
         (user_id, kind, points, remaining, order_id, expires_at) \
         VALUES ($1, $2, $3, $4, $5, $6) RETURNING id",
    )
    .bind(user_id)
    .bind(kind)
    .bind(points)
    .bind(remaining)
    .bind(order_id)
    .bind(expires_at)
    .fetch_one(&mut *conn)
    .await
}

/// Reduces one grant lot's remainder by a spend (or a
/// revocation), never below zero.
pub async fn reduce_grant_remaining(
    conn: &mut PgConnection,
    grant_id: i64,
    by: i64,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE point_ledger SET remaining = GREATEST(remaining - $2, 0) \
         WHERE id = $1 AND kind = 'earn'",
    )
    .bind(grant_id)
    .bind(by)
    .execute(&mut *conn)
    .await
    .map(|_| ())
}

/// Restores points to a grant lot (a spend refund): the
/// remainder rises by the restored amount. This function
/// never decides what the restored points are worth in
/// time — the service restores into a live lot as-is, and
/// for a lot whose expiry has passed it settles the stale
/// remainder and moves the expiry with
/// [`set_grant_expiry`] first, so restored points never
/// land in a lot the balance rule already counts as dead.
pub async fn restore_grant_remaining(
    conn: &mut PgConnection,
    grant_id: i64,
    by: i64,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE point_ledger SET remaining = remaining + $2 \
         WHERE id = $1 AND kind = 'earn'",
    )
    .bind(grant_id)
    .bind(by)
    .execute(&mut *conn)
    .await
    .map(|_| ())
}

/// Locks ONE grant lot inside the caller's transaction,
/// whatever its expiry or remainder; `None` when the row
/// is not an earn grant. The spend reversal takes this
/// lock so its read of the lot's expiry and remainder
/// cannot race the expiry sweep's own locked read:
/// whichever transaction locks second sees the other's
/// outcome.
pub async fn lock_grant(
    conn: &mut PgConnection,
    grant_id: i64,
) -> Result<Option<PointLedgerRow>, sqlx::Error> {
    sqlx::query_as::<_, PointLedgerRow>(
        "SELECT id, user_id, kind, points, remaining, order_id, \
         expires_at, created_at FROM point_ledger \
         WHERE id = $1 AND kind = 'earn' \
         FOR UPDATE",
    )
    .bind(grant_id)
    .fetch_optional(&mut *conn)
    .await
}

/// Moves one grant lot's expiry to a new moment (the
/// expired-lot spend refund: the restored points get a
/// fresh validity counted from the refund).
pub async fn set_grant_expiry(
    conn: &mut PgConnection,
    grant_id: i64,
    expires_at: chrono::DateTime<chrono::Utc>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE point_ledger SET expires_at = $2 \
         WHERE id = $1 AND kind = 'earn'",
    )
    .bind(grant_id)
    .bind(expires_at)
    .execute(&mut *conn)
    .await
    .map(|_| ())
}

/// Records which grant lots one spend consumed.
pub async fn insert_allocation(
    conn: &mut PgConnection,
    spend_ledger_id: i64,
    grant_ledger_id: i64,
    points: i64,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO point_spend_allocations \
         (spend_ledger_id, grant_ledger_id, points) VALUES ($1, $2, $3)",
    )
    .bind(spend_ledger_id)
    .bind(grant_ledger_id)
    .bind(points)
    .execute(&mut *conn)
    .await
    .map(|_| ())
}

/// The spend movements of one order (placement writes at
/// most one, but the reversal does not assume so).
pub async fn spends_for_order(
    conn: &mut PgConnection,
    order_id: i64,
) -> Result<Vec<PointLedgerRow>, sqlx::Error> {
    sqlx::query_as::<_, PointLedgerRow>(
        "SELECT id, user_id, kind, points, remaining, order_id, \
         expires_at, created_at FROM point_ledger \
         WHERE order_id = $1 AND kind = 'spend' ORDER BY id",
    )
    .bind(order_id)
    .fetch_all(&mut *conn)
    .await
}

/// The unreversed allocations of one spend movement.
pub async fn allocations_for_spend(
    conn: &mut PgConnection,
    spend_ledger_id: i64,
) -> Result<Vec<(i64, i64)>, sqlx::Error> {
    sqlx::query_as::<_, (i64, i64)>(
        "SELECT grant_ledger_id, points FROM point_spend_allocations \
         WHERE spend_ledger_id = $1 AND reversed_at IS NULL ORDER BY id",
    )
    .bind(spend_ledger_id)
    .fetch_all(&mut *conn)
    .await
}

/// Marks every allocation of one spend as reversed (the
/// refund has restored those lots).
pub async fn mark_spend_reversed(
    conn: &mut PgConnection,
    spend_ledger_id: i64,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE point_spend_allocations SET reversed_at = now() \
         WHERE spend_ledger_id = $1 AND reversed_at IS NULL",
    )
    .bind(spend_ledger_id)
    .execute(&mut *conn)
    .await
    .map(|_| ())
}

/// The earn grant of one order, when it was granted.
pub async fn earn_for_order(
    conn: &mut PgConnection,
    order_id: i64,
) -> Result<Option<PointLedgerRow>, sqlx::Error> {
    sqlx::query_as::<_, PointLedgerRow>(
        "SELECT id, user_id, kind, points, remaining, order_id, \
         expires_at, created_at FROM point_ledger \
         WHERE order_id = $1 AND kind = 'earn'",
    )
    .bind(order_id)
    .fetch_optional(&mut *conn)
    .await
}

/// Whether a movement kind already exists for an order
/// (the idempotence check reversals run before writing).
pub async fn movement_exists_for_order(
    conn: &mut PgConnection,
    order_id: i64,
    kind: &str,
) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(SELECT 1 FROM point_ledger \
         WHERE order_id = $1 AND kind = $2)",
    )
    .bind(order_id)
    .bind(kind)
    .fetch_one(&mut *conn)
    .await
}

/// The sweep's candidate grants (expired, unspent),
/// read WITHOUT locks: selection only — each candidate is
/// re-locked inside its own transaction by
/// [`lock_expired_grant`] before anything moves.
pub async fn expired_grant_candidates(
    pool: &PgPool,
    limit: i64,
) -> Result<Vec<PointLedgerRow>, sqlx::Error> {
    sqlx::query_as::<_, PointLedgerRow>(
        "SELECT id, user_id, kind, points, remaining, order_id, \
         expires_at, created_at FROM point_ledger \
         WHERE kind = 'earn' AND remaining > 0 AND expires_at <= now() \
         ORDER BY expires_at ASC LIMIT $1",
    )
    .bind(limit)
    .fetch_all(pool)
    .await
}

/// Locks ONE expired grant inside the caller's
/// transaction; `None` when it stopped qualifying since
/// selection (spent, moved or already expired).
pub async fn lock_expired_grant(
    conn: &mut PgConnection,
    grant_id: i64,
) -> Result<Option<PointLedgerRow>, sqlx::Error> {
    sqlx::query_as::<_, PointLedgerRow>(
        "SELECT id, user_id, kind, points, remaining, order_id, \
         expires_at, created_at FROM point_ledger \
         WHERE id = $1 AND kind = 'earn' AND remaining > 0 AND expires_at <= now() \
         FOR UPDATE",
    )
    .bind(grant_id)
    .fetch_optional(&mut *conn)
    .await
}

/// The member's recent ledger history, newest first.
pub async fn history(
    pool: &PgPool,
    user_id: i64,
    limit: i64,
) -> Result<Vec<PointLedgerRow>, sqlx::Error> {
    sqlx::query_as::<_, PointLedgerRow>(
        "SELECT id, user_id, kind, points, remaining, order_id, \
         expires_at, created_at FROM point_ledger \
         WHERE user_id = $1 ORDER BY id DESC LIMIT $2",
    )
    .bind(user_id)
    .bind(limit)
    .fetch_all(pool)
    .await
}
