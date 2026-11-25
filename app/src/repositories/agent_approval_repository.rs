//! Persistence for agent checkout approvals.
//!
//! Ownership follows the cart's rule exactly: a member's
//! `user_id` XOR a guest's session token, passed as the pair
//! `(user_id, session_token)` with exactly one side set.
//! Status moves that must hold exactly once (consume) are
//! conditional updates — the WHERE clause carries the
//! expected current status, so a raced second attempt
//! changes nothing and reports `false`.

use chrono::{DateTime, Utc};
use sqlx::PgPool;

use crate::models::agent_approval::{AgentApproval, STATUS_CONSUMED, STATUS_PENDING};

/// Every column of an `agent_approvals` row, named - never
/// `SELECT *`, so a schema change cannot silently change
/// what a query loads (the same rule as the other repositories).
const APPROVAL_COLUMNS: &str = "id, user_id, session_id, fingerprint, snapshot, status, consumed_order_id, created_at, expires_at, updated_at";

/// Inserts one approval and returns its ID. The caller voids
/// the owner's older pending approvals first when the new one
/// supersedes them.
pub async fn insert(
    pool: &PgPool,
    user_id: Option<i64>,
    session_token: Option<&str>,
    fingerprint: &str,
    snapshot: &serde_json::Value,
    expires_at: DateTime<Utc>,
) -> Result<i64, sqlx::Error> {
    let row: (i64,) = sqlx::query_as(
        "INSERT INTO agent_approvals
             (user_id, session_id, fingerprint, snapshot, expires_at)
         VALUES ($1, $2, $3, $4, $5)
         RETURNING id",
    )
    .bind(user_id)
    .bind(session_token)
    .bind(fingerprint)
    .bind(snapshot)
    .bind(expires_at)
    .fetch_one(pool)
    .await?;
    Ok(row.0)
}

/// The owner's most recent approval, whatever its status:
/// the approval surfaces reason over the latest record.
pub async fn find_latest_for_owner(
    pool: &PgPool,
    user_id: Option<i64>,
    session_token: Option<&str>,
) -> Result<Option<AgentApproval>, sqlx::Error> {
    sqlx::query_as::<_, AgentApproval>(&format!(
        "SELECT {APPROVAL_COLUMNS} FROM agent_approvals
         WHERE user_id IS NOT DISTINCT FROM $1
           AND session_id IS NOT DISTINCT FROM $2
         ORDER BY id DESC
         LIMIT 1"
    ))
    .bind(user_id)
    .bind(session_token)
    .fetch_optional(pool)
    .await
}

/// One approval by ID. Ownership is the caller's check: the
/// placement path compares the row's owner to the session's.
pub async fn find_by_id(pool: &PgPool, id: i64) -> Result<Option<AgentApproval>, sqlx::Error> {
    sqlx::query_as::<_, AgentApproval>(&format!(
        "SELECT {APPROVAL_COLUMNS} FROM agent_approvals WHERE id = $1"
    ))
    .bind(id)
    .fetch_optional(pool)
    .await
}

/// Moves one approval from `from` to `to`, only if it still
/// rests in `from`. Returns whether the move happened.
pub async fn transition(pool: &PgPool, id: i64, from: &str, to: &str) -> Result<bool, sqlx::Error> {
    let result = sqlx::query(
        "UPDATE agent_approvals
         SET status = $3, updated_at = now()
         WHERE id = $1 AND status = $2",
    )
    .bind(id)
    .bind(from)
    .bind(to)
    .execute(pool)
    .await?;
    Ok(result.rows_affected() == 1)
}

/// Consumes one pending approval for the order just placed,
/// linking the two in the same write. Conditional on the
/// approval still being pending: a second consume (a raced
/// double submit) changes nothing and reports `false`.
pub async fn consume(pool: &PgPool, id: i64, order_id: i64) -> Result<bool, sqlx::Error> {
    let result = sqlx::query(
        "UPDATE agent_approvals
         SET status = $3, consumed_order_id = $2, updated_at = now()
         WHERE id = $1 AND status = $4",
    )
    .bind(id)
    .bind(order_id)
    .bind(STATUS_CONSUMED)
    .bind(STATUS_PENDING)
    .execute(pool)
    .await?;
    Ok(result.rows_affected() == 1)
}

/// Re-binds one pending approval to its final state at the
/// pay click: the headline fingerprint becomes the hash of
/// the final totals, method and address, and the snapshot
/// carries the whole agreed state. Conditional on the
/// approval still being pending — a record that was spent
/// or killed first changes nothing and reports `false`.
pub async fn update_binding(
    pool: &PgPool,
    id: i64,
    fingerprint: &str,
    snapshot: &serde_json::Value,
) -> Result<bool, sqlx::Error> {
    let result = sqlx::query(
        "UPDATE agent_approvals
         SET fingerprint = $2, snapshot = $3, updated_at = now()
         WHERE id = $1 AND status = $4",
    )
    .bind(id)
    .bind(fingerprint)
    .bind(snapshot)
    .bind(STATUS_PENDING)
    .execute(pool)
    .await?;
    Ok(result.rows_affected() == 1)
}

/// Voids every pending approval the owner holds. Creating a
/// fresh approval supersedes the old ones, and the owner's
/// revoke action kills the live one; both are this write.
pub async fn void_pending_for_owner(
    pool: &PgPool,
    user_id: Option<i64>,
    session_token: Option<&str>,
) -> Result<u64, sqlx::Error> {
    let result = sqlx::query(
        "UPDATE agent_approvals
         SET status = 'voided', updated_at = now()
         WHERE user_id IS NOT DISTINCT FROM $1
           AND session_id IS NOT DISTINCT FROM $2
           AND status = 'pending'",
    )
    .bind(user_id)
    .bind(session_token)
    .execute(pool)
    .await?;
    Ok(result.rows_affected())
}
