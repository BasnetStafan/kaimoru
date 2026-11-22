//! Persistence for two-factor authentication.
//!
//! Owns the three `users` columns (secret, enablement moment,
//! last accepted step)
//! and the `user_recovery_codes` table. The secret is loaded
//! through this module's state read and the credential-
//! fingerprint read in `user_repository`, the same discipline
//! as the password hash: it never rides on the `User` struct,
//! so it cannot leak into a template by accident.

use chrono::{DateTime, Utc};
use sqlx::PgPool;

/// A member's two-factor storage state.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct TotpState {
    /// The base32 enrolment secret; `None` when not enrolled.
    pub secret: Option<String>,
    /// When enrolment was confirmed; `None` when not enrolled.
    /// Secret and timestamp are always set and cleared together.
    pub enabled_at: Option<DateTime<Utc>>,
    /// The last accepted authenticator step (SEC-5 replay
    /// guard); `None` until the first accepted code.
    pub last_step: Option<i64>,
}

impl TotpState {
    /// Whether two-factor authentication is live for the member.
    pub fn is_enabled(&self) -> bool {
        self.secret.is_some() && self.enabled_at.is_some()
    }
}

/// Loads a member's two-factor state; `None` when the member
/// does not exist.
pub async fn state(pool: &PgPool, user_id: i64) -> Result<Option<TotpState>, sqlx::Error> {
    sqlx::query_as::<_, TotpState>(
        "SELECT totp_secret AS secret, totp_enabled_at AS enabled_at, totp_last_step AS last_step \
         FROM users WHERE id = $1",
    )
    .bind(user_id)
    .fetch_optional(pool)
    .await
}

/// Persists a confirmed enrolment: secret and enablement
/// moment together, so a half-written state can never read
/// as enabled.
pub async fn set_enabled(pool: &PgPool, user_id: i64, secret: &str) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE users SET totp_secret = $2, totp_enabled_at = now() WHERE id = $1")
        .bind(user_id)
        .bind(secret)
        .execute(pool)
        .await
        .map(|_| ())
}

/// Clears the enrolment (disable): secret, timestamp and all
/// recovery codes together, in one transaction.
pub async fn clear(pool: &PgPool, user_id: i64) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;
    sqlx::query("UPDATE users SET totp_secret = NULL, totp_enabled_at = NULL WHERE id = $1")
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM user_recovery_codes WHERE user_id = $1")
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await
}

/// Replaces the member's recovery codes with a fresh set of
/// hashes: every earlier code stops working at this moment.
pub async fn replace_recovery_codes(
    pool: &PgPool,
    user_id: i64,
    code_hashes: &[String],
) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;
    sqlx::query("DELETE FROM user_recovery_codes WHERE user_id = $1")
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
    for hash in code_hashes {
        sqlx::query("INSERT INTO user_recovery_codes (user_id, code_hash) VALUES ($1, $2)")
            .bind(user_id)
            .bind(hash)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await
}

/// The member's unused recovery codes as (id, hash) pairs.
pub async fn unused_recovery_codes(
    pool: &PgPool,
    user_id: i64,
) -> Result<Vec<(i64, String)>, sqlx::Error> {
    sqlx::query_as::<_, (i64, String)>(
        "SELECT id, code_hash FROM user_recovery_codes \
         WHERE user_id = $1 AND used_at IS NULL ORDER BY id",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await
}

/// Marks one recovery code used. The `used_at IS NULL` guard
/// makes the spend atomic: two concurrent submissions of the
/// same code cannot both succeed.
pub async fn mark_recovery_used(pool: &PgPool, id: i64) -> Result<bool, sqlx::Error> {
    let rows = sqlx::query(
        "UPDATE user_recovery_codes SET used_at = now() WHERE id = $1 AND used_at IS NULL",
    )
    .bind(id)
    .execute(pool)
    .await?;
    Ok(rows.rows_affected() > 0)
}

/// Deletes a member's two-factor data inside the withdrawal
/// transaction: the scrubbed tombstone must hold no live
/// secret and no usable recovery code.
pub async fn delete_all_for_user(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    user_id: i64,
) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE users SET totp_secret = NULL, totp_enabled_at = NULL WHERE id = $1")
        .bind(user_id)
        .execute(&mut **tx)
        .await?;
    sqlx::query("DELETE FROM user_recovery_codes WHERE user_id = $1")
        .bind(user_id)
        .execute(&mut **tx)
        .await
        .map(|_| ())
}

/// Records an accepted authenticator step, refusing any step
/// at or below the stored one (SEC-5): the conditional update
/// is the replay guard, so two concurrent submissions of the
/// same code cannot both be recorded. Returns whether the
/// step was accepted as new.
pub async fn record_step(pool: &PgPool, user_id: i64, step: i64) -> Result<bool, sqlx::Error> {
    let rows = sqlx::query(
        "UPDATE users SET totp_last_step = $2 \
         WHERE id = $1 AND (totp_last_step IS NULL OR totp_last_step < $2)",
    )
    .bind(user_id)
    .bind(step)
    .execute(pool)
    .await?;
    Ok(rows.rows_affected() > 0)
}
