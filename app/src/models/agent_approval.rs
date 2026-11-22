//! Agent checkout approval (settlement flow).
//!
//! The record of one human approval: a cart owner walked
//! the in-chat settlement for the cart the agent built and
//! agreed to its final state. The record is created when
//! the settlement starts, bound to the cart's purchasable
//! lines (variant, quantity, unit price) — anything that
//! changes the cart afterwards, an edit, a quantity change,
//! a later agent run replacing its lines, breaks that
//! binding and the settlement dies instead of ordering
//! something unapproved. At the pay click the record is
//! re-bound to the final state: the fingerprint becomes a
//! hash of the lines plus the final totals, the chosen
//! payment method and the delivery address, and the
//! snapshot carries all of it, so the consumed record
//! proves exactly what was agreed.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Approval status: created, waiting for its checkout.
pub const STATUS_PENDING: &str = "pending";
/// Approval status: spent by the order it approved.
pub const STATUS_CONSUMED: &str = "consumed";
/// Approval status: killed before use — the cart changed, a
/// newer approval superseded it, or the owner revoked it.
pub const STATUS_VOIDED: &str = "voided";
/// Approval status: the TTL ran out unused.
pub const STATUS_EXPIRED: &str = "expired";

/// One stored approval row.
#[derive(Clone, sqlx::FromRow)]
pub struct AgentApproval {
    /// Surrogate ID.
    pub id: i64,
    /// Owning member, for a member's cart (XOR with
    /// `session_id`, the same ownership rule as cart items).
    pub user_id: Option<i64>,
    /// Owning guest session token, for a guest's cart.
    pub session_id: Option<String>,
    /// Fingerprint of the approved cart (see
    /// [`crate::services::agent_approval_service`]).
    pub fingerprint: String,
    /// What the owner was shown at approval time.
    pub snapshot: serde_json::Value,
    /// One of the `STATUS_*` values.
    pub status: String,
    /// The order this approval paid for, once consumed.
    pub consumed_order_id: Option<i64>,
    /// When the approval was created.
    pub created_at: DateTime<Utc>,
    /// When the approval stops being usable.
    pub expires_at: DateTime<Utc>,
    /// When the row last changed.
    pub updated_at: DateTime<Utc>,
}

impl AgentApproval {
    /// Whether the TTL has run out at `now`.
    pub fn is_expired(&self, now: DateTime<Utc>) -> bool {
        self.expires_at <= now
    }

    /// The stored snapshot, parsed. A snapshot that does not
    /// parse reads as `None`: the row was written by this
    /// codebase, so a failure means corruption, and callers
    /// treat the approval as unusable rather than guessing.
    pub fn snapshot(&self) -> Option<ApprovalSnapshot> {
        serde_json::from_value(self.snapshot.clone()).ok()
    }
}

/// One line of the approval snapshot: what the approval card
/// listed, in the cart's canonical (variant) order.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ApprovalSnapshotLine {
    /// The line's variant (the cart line identity).
    pub variant_id: i64,
    /// The line's product.
    pub product_id: i64,
    /// Product name at approval time.
    pub name: String,
    /// Variant label at approval time (empty when plain).
    pub variant_label: String,
    /// Unit price in yen at approval time.
    pub unit_price_yen: i64,
    /// Quantity at approval time.
    pub quantity: i64,
    /// Line total in yen at approval time.
    pub line_total_yen: i64,
}

/// The delivery address an approval's final state binds:
/// the address snapshot the pay click agreed to, field for
/// field the delivery the order is placed with.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ApprovalDelivery {
    /// Contact email the order confirms to.
    pub email: String,
    /// Recipient family name.
    pub family_name: String,
    /// Recipient given name.
    pub given_name: String,
    /// Postal code as entered.
    pub postal_code: String,
    /// Prefecture.
    pub prefecture: String,
    /// City / ward.
    pub city: String,
    /// Street address.
    pub address_line: String,
    /// Building / room, empty when none.
    pub building: String,
    /// Phone number as entered.
    pub phone: String,
}

/// The display half of an approval: the lines and totals the
/// owner approved, exactly as placement will re-derive them.
/// At creation the totals carry no payment method (no COD
/// fee yet); the pay click's re-bind replaces them with the
/// final totals and fills the method, fee and delivery
/// fields below.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ApprovalSnapshot {
    /// Purchasable lines, sorted by variant.
    pub lines: Vec<ApprovalSnapshotLine>,
    /// Product subtotal in yen (no coupon on this path).
    pub subtotal_yen: i64,
    /// Shipping fee in yen.
    pub shipping_yen: i64,
    /// Grand total in yen: what the payment settles.
    pub total_yen: i64,
    /// The lines-only fingerprint this approval binds: the
    /// cart-change check compares the live cart against it
    /// for the approval's whole life, across the pay click's
    /// re-bind of the headline fingerprint. Empty on rows
    /// written by the earlier settlement flow (their headline
    /// fingerprint was the lines binding).
    #[serde(default)]
    pub lines_fingerprint: String,
    /// Chosen payment method slug, once the pay click has
    /// bound the final state.
    #[serde(default)]
    pub payment_method: Option<String>,
    /// Cash-on-delivery fee in yen inside `total_yen`, once
    /// bound (0 for every other method).
    #[serde(default)]
    pub cod_fee_yen: i64,
    /// Delivery address the final state binds, once bound.
    #[serde(default)]
    pub delivery: Option<ApprovalDelivery>,
}
