//! One admin audit-log row as the viewer lists it.

use chrono::{DateTime, Utc};
use sqlx::FromRow;

/// One row of the `admin_audit_logs` table joined with the
/// acting admin's display name: the audit viewer's row.
#[derive(Debug, Clone, FromRow)]
pub struct AuditLogRow {
    /// Primary key.
    pub id: i64,
    /// The admin who performed the action, when the
    /// actor is an admin; member-actor rows leave this
    /// empty and carry `actor_user_id` instead.
    pub admin_id: Option<i64>,
    /// That admin's display name, for the 管理者 column.
    pub admin_name: String,
    /// Action name (e.g. `product.create`, `order.status`).
    pub action: String,
    /// What kind of record was touched (`product`, `order`, …).
    pub target_type: String,
    /// The touched record's ID, when the action has one.
    pub target_id: Option<i64>,
    /// When the action happened.
    pub created_at: DateTime<Utc>,
}
