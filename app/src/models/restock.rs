//! Back-in-stock notification models: one subscription per
//! (variant, email) pair, mailed once when the combination's
//! stock returns.

use chrono::{DateTime, Utc};

/// Subscription status: waiting for stock.
pub const STATUS_PENDING: &str = "pending";
/// Subscription status: claimed by a sweep pass that is
/// sending (or died trying — stale claims return to pending).
pub const STATUS_SENDING: &str = "sending";
/// Subscription status: the notification mail was sent; the
/// subscription is complete and one-shot by design.
pub const STATUS_SENT: &str = "sent";
/// Subscription status: cancelled before sending.
pub const STATUS_CANCELLED: &str = "cancelled";

/// One back-in-stock subscription row. The email is stored
/// PII-encrypted; repositories decrypt it into this struct,
/// so a value read from here is the plain address.
#[derive(Clone, sqlx::FromRow)]
pub struct RestockSubscription {
    /// Surrogate ID.
    pub id: i64,
    /// The sold-out combination the shopper is waiting for.
    pub variant_id: i64,
    /// The subscribing member, when signed in at registration.
    pub user_id: Option<i64>,
    /// The notification address (decrypted on read).
    pub email: String,
    /// One of the `STATUS_*` values.
    pub status: String,
    /// When the subscription was registered.
    pub created_at: DateTime<Utc>,
    /// When the notification mail was sent, once sent.
    pub notified_at: Option<DateTime<Utc>>,
}

/// One due subscription for the sweep: a pending row whose
/// combination is back in stock, with everything the mail
/// needs (the address still encrypted at this layer — the
/// service decrypts just before sending).
pub struct RestockDue {
    /// Subscription ID.
    pub id: i64,
    /// The encrypted notification address.
    pub email_encrypted: String,
    /// The product the combination belongs to.
    pub product_id: i64,
    /// The product's current name.
    pub product_name: String,
    /// The combination's display label; empty for the
    /// default variant.
    pub variant_label: String,
}
