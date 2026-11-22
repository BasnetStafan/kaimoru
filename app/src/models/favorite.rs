//! A member's favourite product mark.
//!
//! One row per (member, product) pair — the pair is the table's
//! primary key, so the mark either exists or does not. Favourites
//! are member-only personal data: the withdrawal ceremony deletes
//! them, and the foreign keys cascade if the member or product
//! row itself is ever deleted.

use chrono::{DateTime, Utc};

/// One favourite mark.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Favorite {
    /// Owning member.
    pub user_id: i64,
    /// The favourited product.
    pub product_id: i64,
    /// When the mark was made.
    pub created_at: DateTime<Utc>,
}
