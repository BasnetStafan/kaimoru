//! Persistence for administrator two-factor authentication
//!.
//!
//! The exact mirror of [`crate::repositories::totp_repository`]
//! over the `admins` columns and the `admin_recovery_codes`
//! table (migration 41): the secret never rides on the
//! `Admin` struct, so it cannot leak into a template by
//! accident — the same discipline as the password hash.

use sqlx::PgPool;

use crate::repositories::totp_repository::TotpState;

/// Loads an admin's two-factor state; `None` when the
/// admin does not exist.
pub async fn state(pool: &PgPool, admin_id: i64) -> Result<Option<TotpState>, sqlx::Error> {
    sqlx::query_as::<_, TotpState>(
        "SELECT totp_secret AS secret, totp_enabled_at AS enabled_at, totp_last_step AS last_step \
         FROM admins WHERE id = $1",
    )
    .bind(admin_id)
    .fetch_optional(pool)
    .await
}

/// Persists a confirmed enrolment: secret and enablement
/// moment together, so a half-written state can never read
/// as enabled.
pub async fn set_enabled(pool: &PgPool, admin_id: i64, secret: &str) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE admins SET totp_secret = $2, totp_enabled_at = now(), totp_last_step = NULL \
         WHERE id = $1",
    )
    .bind(admin_id)
    .bind(secret)
    .execute(pool)
    .await
    .map(|_| ())
}

/// Clears the enrolment (disable): secret, timestamp and
/// all recovery codes together, in one transaction.
pub async fn clear(pool: &PgPool, admin_id: i64) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;
    sqlx::query(
        "UPDATE admins SET totp_secret = NULL, totp_enabled_at = NULL, totp_last_step = NULL \
         WHERE id = $1",
    )
    .bind(admin_id)
    .execute(&mut *tx)
    .await?;
    sqlx::query("DELETE FROM admin_recovery_codes WHERE admin_id = $1")
        .bind(admin_id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await
}

/// Replaces the admin's recovery codes with a fresh set of
/// hashes: every earlier code stops working at this moment.
pub async fn replace_recovery_codes(
    pool: &PgPool,
    admin_id: i64,
    code_hashes: &[String],
) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;
    sqlx::query("DELETE FROM admin_recovery_codes WHERE admin_id = $1")
        .bind(admin_id)
        .execute(&mut *tx)
        .await?;
    for hash in code_hashes {
        sqlx::query("INSERT INTO admin_recovery_codes (admin_id, code_hash) VALUES ($1, $2)")
            .bind(admin_id)
            .bind(hash)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await
}

/// The admin's unused recovery codes as (id, hash) pairs.
pub async fn unused_recovery_codes(
    pool: &PgPool,
    admin_id: i64,
) -> Result<Vec<(i64, String)>, sqlx::Error> {
    sqlx::query_as::<_, (i64, String)>(
        "SELECT id, code_hash FROM admin_recovery_codes \
         WHERE admin_id = $1 AND used_at IS NULL ORDER BY id",
    )
    .bind(admin_id)
    .fetch_all(pool)
    .await
}

/// Marks one recovery code used. The `used_at IS NULL`
/// guard makes the spend atomic: two concurrent
/// submissions of the same code cannot both succeed.
pub async fn mark_recovery_used(pool: &PgPool, id: i64) -> Result<bool, sqlx::Error> {
    let rows = sqlx::query(
        "UPDATE admin_recovery_codes SET used_at = now() WHERE id = $1 AND used_at IS NULL",
    )
    .bind(id)
    .execute(pool)
    .await?;
    Ok(rows.rows_affected() > 0)
}

/// Records an accepted authenticator step (the replay
/// guard): succeeds only for a step strictly newer than
/// the stored one.
pub async fn record_step(pool: &PgPool, admin_id: i64, step: i64) -> Result<bool, sqlx::Error> {
    let rows = sqlx::query(
        "UPDATE admins SET totp_last_step = $2 \
         WHERE id = $1 AND (totp_last_step IS NULL OR totp_last_step < $2)",
    )
    .bind(admin_id)
    .bind(step)
    .execute(pool)
    .await?;
    Ok(rows.rows_affected() > 0)
}
