//! The audit viewer's read side.
//!
//! Kept apart from [`crate::services::admin::audit_service`], the
//! write side every admin mutation records through: that module
//! appends rows inside the mutation's own transaction, while this
//! one only reads the finished trail. The trail is append-only —
//! neither side exposes an update or a delete.

use chrono::NaiveDate;
use sqlx::PgPool;

use crate::error::AppError;
use crate::models::AuditLogRow;
use crate::repositories::admin::admin_repository;

/// Returns one page of the audit viewer, newest first — target
/// type, action, and an inclusive JST date range filters — plus
/// the total row count. The 管理者 (actor) filter is deliberately
/// absent: with a single admin it can only ever select that admin,
/// so it waits until a second admin exists.
pub async fn list_audit_logs(
    pool: &PgPool,
    target_type: Option<&str>,
    action: Option<&str>,
    date_from: Option<NaiveDate>,
    date_to: Option<NaiveDate>,
    limit: i64,
    offset: i64,
) -> Result<(Vec<AuditLogRow>, i64), AppError> {
    admin_repository::list_audit_logs(pool, target_type, action, date_from, date_to, limit, offset)
        .await
}

/// Most rows one audit CSV export may carry: the trail is the
/// shop's own record, so the cap is generous, but an export is
/// still a single response, not a table dump.
pub const AUDIT_EXPORT_MAX_ROWS: i64 = 10_000;

/// Returns the audit rows a CSV export writes: the viewer's
/// own filters, unpaginated up to [`AUDIT_EXPORT_MAX_ROWS`].
pub async fn export_audit_logs(
    pool: &PgPool,
    target_type: Option<&str>,
    action: Option<&str>,
    date_from: Option<NaiveDate>,
    date_to: Option<NaiveDate>,
) -> Result<Vec<AuditLogRow>, AppError> {
    let (rows, _) = list_audit_logs(
        pool,
        target_type,
        action,
        date_from,
        date_to,
        AUDIT_EXPORT_MAX_ROWS,
        0,
    )
    .await?;
    Ok(rows)
}

/// Returns every audit row recorded against one order, newest
/// first — the order detail's 変更履歴. An order can only be
/// advanced a handful of times, so no pagination is needed.
pub async fn order_history(pool: &PgPool, order_id: i64) -> Result<Vec<AuditLogRow>, AppError> {
    admin_repository::list_audit_logs_for_target(pool, "order", order_id).await
}
