//! User identity model.
//! One external SNS identity linked to a member account.

/// An external identity (LINE / Google / Yahoo) linked to a
/// member account. The provider's stable user id (the `sub`
/// claim of its ID token) is the durable sign-in key; the email
/// is display context from link time, never an identity key.
#[derive(Clone, sqlx::FromRow)]
pub struct UserIdentity {
    /// Surrogate ID of the link.
    pub id: i64,
    /// The member this identity belongs to.
    pub user_id: i64,
    /// Provider slug: `line`, `google`, or `yahoo`.
    pub provider: String,
    /// The provider's stable user id for this member.
    pub provider_user_id: String,
    /// Email the provider reported at link time, when it
    /// reported one.
    pub provider_email: Option<String>,
    /// When the link was created.
    pub created_at: chrono::DateTime<chrono::Utc>,
    /// When the link last changed.
    pub updated_at: chrono::DateTime<chrono::Utc>,
}
