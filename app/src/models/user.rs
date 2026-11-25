//! User model.
//! One registered member of the store.

/// A registered member, as stored in the users table.
/// The password hash is deliberately not part of this struct: it is
/// loaded only inside the repository during login checks, so it can
/// never leak into a template by accident.
#[derive(Clone, sqlx::FromRow)]
pub struct User {
    /// Surrogate ID of the member.
    pub id: i64,
    /// Login email address.
    pub email: String,
    /// Family name in kanji, shown in the header greeting.
    pub family_name: String,
    /// Given name in kanji.
    pub given_name: String,
    /// Family name in katakana.
    pub family_name_kana: String,
    /// Given name in katakana.
    pub given_name_kana: String,
    /// Phone number, digits only.
    pub phone: String,
    /// Postal code, 7 digits.
    pub postal_code: String,
    /// Prefecture name.
    pub prefecture: String,
    /// City and ward.
    pub city: String,
    /// Street address.
    pub address_line: String,
    /// Building name, when given.
    pub building: Option<String>,
    /// Whether the member accepts the mail magazine.
    pub mail_magazine: bool,
    /// Whether the account has a usable password. Accounts
    /// registered through SNS start without one: they sign in
    /// through their identity until a password is set.
    pub has_password: bool,
    /// When the member withdrew, when they have: the row
    /// is then an anonymised tombstone that must never
    /// authenticate (Account Deletion spec 3.4).
    pub withdrawn_at: Option<chrono::DateTime<chrono::Utc>>,
}
