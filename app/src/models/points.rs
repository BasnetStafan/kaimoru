//! Points programme model (EC V2 core 1;
//! internal points design record).
//!
//! Points live in a ledger, never as a bare balance column:
//! an `earn` row is a grant lot carrying its own spendable
//! remainder and expiry, and every other row is a signed
//! movement. The member's balance is the sum of unexpired
//! grant remainders — the same discipline as the stock
//! ledger.

use chrono::{DateTime, Utc};
use sqlx::FromRow;

/// Movement kind: a grant of new points (order shipped,
/// or an admin adjustment recorded with this kind plus a
/// note in the admin log).
pub const KIND_EARN: &str = "earn";
/// Movement kind: points spent at order placement.
pub const KIND_SPEND: &str = "spend";
/// Movement kind: a spend returned by an order
/// cancellation; restores the original grant lots.
pub const KIND_SPEND_REFUND: &str = "spend_refund";
/// Movement kind: a grant revoked by a post-shipment
/// cancellation or return.
pub const KIND_EARN_REVOKE: &str = "earn_revoke";
/// Movement kind: a grant remainder that reached its
/// expiry and was swept.
pub const KIND_EXPIRE: &str = "expire";
/// Movement kind: a manual operator adjustment.
pub const KIND_ADMIN_ADJUST: &str = "admin_adjust";

/// One row of the points ledger.
#[derive(Clone, Debug, FromRow)]
pub struct PointLedgerRow {
    /// Ledger row id.
    pub id: i64,
    /// The member the movement belongs to.
    pub user_id: i64,
    /// Movement kind (the `KIND_*` constants).
    pub kind: String,
    /// Signed movement amount in points (1 pt = ¥1).
    pub points: i64,
    /// For `earn` rows: points of this grant still
    /// spendable. `None` for pure movement rows.
    pub remaining: Option<i64>,
    /// The order the movement belongs to, when it has one.
    pub order_id: Option<i64>,
    /// For `earn` rows: when the grant stops being
    /// spendable. `None` for pure movement rows.
    pub expires_at: Option<DateTime<Utc>>,
    /// When the movement was recorded.
    pub created_at: DateTime<Utc>,
}

/// A spendable grant lot, as the spend planner sees it.
#[derive(Clone, Debug)]
pub struct GrantLot {
    /// The earn row's ledger id.
    pub id: i64,
    /// Points still spendable from this grant.
    pub remaining: i64,
}
