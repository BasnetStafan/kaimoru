//! The admin audit trail's write side.
//!
//! A mutation audited through here records one row in the same
//! database transaction as the change itself: an action and its
//! record commit together or roll back together, so the log can
//! never claim an action that did not happen, nor miss one that
//! did. The admin auth surface is the deliberate exception: its
//! credential events use standalone inserts (a failure is
//! logged, never raised), because a login has no business
//! transaction to join. Rows carry IDs and action names only —
//! never an email, a review body, or a before/after payload.

use crate::error::AppError;
use crate::repositories::admin::admin_repository;

/// Appends the audit row for one mutation inside the caller's
/// transaction. Audit failures fail the whole action on
/// purpose: on this path there is no silent, unrecorded
/// mutation.
pub async fn record(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    admin_id: i64,
    action: &str,
    target_type: &str,
    target_id: Option<i64>,
) -> Result<(), AppError> {
    admin_repository::insert_audit(tx, admin_id, action, target_type, target_id).await
}

/// Appends the audit row for a MEMBER actor's mutation
/// (today: order cancellation, a refund request) inside the
/// caller's
/// transaction, under the same all-or-nothing rule as
/// [`record`]: the row names the member through
/// `actor_user_id` and leaves the admin actor empty.
pub async fn record_member(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    user_id: i64,
    action: &str,
    target_type: &str,
    target_id: Option<i64>,
) -> Result<(), AppError> {
    admin_repository::insert_audit_member(tx, user_id, action, target_type, target_id).await
}

/// Appends one audit row per target in one statement inside the
/// caller's transaction — the bulk path of [`record`], under the
/// same all-or-nothing rule: if the audit write fails, the
/// mutation rolls back with it.
pub async fn record_many(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    admin_id: i64,
    action: &str,
    target_type: &str,
    target_ids: &[i64],
) -> Result<(), AppError> {
    admin_repository::insert_audit_many(tx, admin_id, action, target_type, target_ids).await
}
