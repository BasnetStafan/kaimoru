//! Pending registration model.
//! A sign-up in progress, before the account exists.

use chrono::{DateTime, NaiveDate, Utc};

/// A registration in progress, as stored in pending_registrations.
/// Holds the email and code state first, then the member details once
/// the code has verified. Deleted when the account is created.
#[derive(Clone, sqlx::FromRow)]
pub struct PendingRegistration {
    /// Surrogate ID of the pending registration.
    pub id: i64,
    /// Email address being registered.
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
    /// Argon2id password hash, set at the details step.
    pub password_hash: Option<String>,
    /// Family name in kanji, set at the details step.
    pub family_name: Option<String>,
    /// Given name in kanji, set at the details step.
    pub given_name: Option<String>,
    /// Family name in katakana, set at the details step.
    pub family_name_kana: Option<String>,
    /// Given name in katakana, set at the details step.
    pub given_name_kana: Option<String>,
    /// Birth date, optional at sign-up.
    pub birth_date: Option<NaiveDate>,
    /// Gender, optional at sign-up.
    pub gender: Option<String>,
    /// Postal code, set at the details step.
    pub postal_code: Option<String>,
    /// Prefecture name, set at the details step.
    pub prefecture: Option<String>,
    /// City and ward, set at the details step.
    pub city: Option<String>,
    /// Street address, set at the details step.
    pub address_line: Option<String>,
    /// Building name, optional.
    pub building: Option<String>,
    /// Phone number, set at the details step.
    pub phone: Option<String>,
    /// Whether the member accepts the mail magazine.
    pub mail_magazine: bool,
}
