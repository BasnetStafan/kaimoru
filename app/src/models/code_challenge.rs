//! Code challenge model: one in-progress verification-code flow
//! for an existing member (password reset or email change).

use chrono::{DateTime, Utc};

/// A verification-code flow in progress, as stored in
/// `password_resets` or `email_changes`.
///
/// Both tables carry the same code state as a pending registration
/// (hash only, attempts and resends counted in the database), but
/// the account already exists; the row is deleted on completion.
#[derive(Clone, sqlx::FromRow)]
pub struct CodeChallenge {
    /// Surrogate ID of the flow.
    pub id: i64,
    /// The member the flow belongs to.
    pub user_id: i64,
    /// The address the code was sent to: the account email for a
    /// password reset, the NEW address for an email change.
    pub email: String,
    /// HMAC-SHA-256 hash of the current verification code.
    pub code_hash: String,
    /// Wrong code entries so far, counted in the database on purpose.
    pub attempts: i32,
    /// How many times a code has been resent.
    pub resend_count: i32,
    /// When the current code was sent.
    pub code_sent_at: DateTime<Utc>,
    /// When the current code stops being valid.
    pub expires_at: DateTime<Utc>,
    /// Whether the current code has been entered correctly.
    pub verified: bool,
}
