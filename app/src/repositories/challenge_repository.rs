//! Verification repository.
//!
//! Owns every query against `password_resets`, `email_changes`
//! and `withdrawal_codes`, the three tables carrying a
//! verification-code flow. All three tables have the same shape,
//! so the queries live here once, parameterised by a fixed table
//! choice: the table name comes from the [`ChallengeTable`] enum
//! in code, never from request input, and every value is a bound
//! parameter.

use chrono::{DateTime, Utc};
use sqlx::PgPool;

use crate::models::CodeChallenge;

/// Which code-flow table a query targets.
#[derive(Clone, Copy)]
pub enum ChallengeTable {
    /// Password resets.
    PasswordResets,
    /// Email changes.
    EmailChanges,
    /// Withdrawal proofs for passwordless accounts.
    WithdrawalCodes,
}

impl ChallengeTable {
    /// The table name, a fixed literal per variant — the one place
    /// a table name enters the SQL text; it can never carry
    /// request data.
    fn name(self) -> &'static str {
        match self {
            Self::PasswordResets => "password_resets",
            Self::EmailChanges => "email_changes",
            Self::WithdrawalCodes => "withdrawal_codes",
        }
    }
}

/// All columns of a code challenge, in table order of use.
const CHALLENGE_COLUMNS: &str = "id, user_id, email, code_hash, attempts, resend_count, \
    code_sent_at, expires_at, verified";

/// Starts a new flow for a member.
/// Any earlier flow of the same kind for the same member is
/// removed first, so a member has at most one reset and one
/// email change in progress.
pub async fn create(
    pool: &PgPool,
    table: ChallengeTable,
    user_id: i64,
    email: &str,
    code_hash: &str,
    expires_at: DateTime<Utc>,
) -> Result<i64, sqlx::Error> {
    // One transaction, as with pending registrations: two
    // concurrent starts must not interleave delete and insert.
    let mut tx = pool.begin().await?;
    sqlx::query(&format!("DELETE FROM {} WHERE user_id = $1", table.name()))
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
    let id = sqlx::query_scalar(&format!(
        "INSERT INTO {} (user_id, email, code_hash, expires_at) \
         VALUES ($1, $2, $3, $4) RETURNING id",
        table.name()
    ))
    .bind(user_id)
    .bind(email)
    .bind(code_hash)
    .bind(expires_at)
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(id)
}

/// Finds a flow by ID.
pub async fn find_by_id(
    pool: &PgPool,
    table: ChallengeTable,
    id: i64,
) -> Result<Option<CodeChallenge>, sqlx::Error> {
    sqlx::query_as::<_, CodeChallenge>(&format!(
        "SELECT {CHALLENGE_COLUMNS} FROM {} WHERE id = $1",
        table.name()
    ))
    .bind(id)
    .fetch_optional(pool)
    .await
}

/// Deletes a flow, when it completes or is replaced by a fresh one.
pub async fn delete_by_id(
    pool: &PgPool,
    table: ChallengeTable,
    id: i64,
) -> Result<(), sqlx::Error> {
    sqlx::query(&format!("DELETE FROM {} WHERE id = $1", table.name()))
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Deletes abandoned flows a day after their code expired.
///
/// Until this runs, an abandoned row holds a code hash for a flow
/// that will never complete; a day of grace covers slow completions.
pub async fn delete_expired(pool: &PgPool, table: ChallengeTable) -> Result<(), sqlx::Error> {
    sqlx::query(&format!(
        "DELETE FROM {} WHERE expires_at < now() - interval '1 day'",
        table.name()
    ))
    .execute(pool)
    .await?;
    Ok(())
}

/// Records one wrong code entry, persisted on purpose: a restart
/// must never hand out fresh guesses.
///
/// The increment is atomic with the limit in the WHERE clause, so
/// parallel verify requests cannot overshoot the budget. Returns
/// whether the attempt was counted (false = limit already hit).
pub async fn record_failed_attempt_capped(
    pool: &PgPool,
    table: ChallengeTable,
    id: i64,
    max_attempts: i32,
) -> Result<bool, sqlx::Error> {
    let updated: Option<i64> = sqlx::query_scalar(&format!(
        "UPDATE {} SET attempts = attempts + 1 \
         WHERE id = $1 AND attempts < $2 RETURNING id",
        table.name()
    ))
    .bind(id)
    .bind(max_attempts)
    .fetch_optional(pool)
    .await?;
    Ok(updated.is_some())
}

/// Marks the flow as code verified.
pub async fn mark_verified(
    pool: &PgPool,
    table: ChallengeTable,
    id: i64,
) -> Result<(), sqlx::Error> {
    sqlx::query(&format!(
        "UPDATE {} SET verified = TRUE WHERE id = $1",
        table.name()
    ))
    .bind(id)
    .execute(pool)
    .await?;
    Ok(())
}

/// Replaces the code after a resend: new hash, fresh attempt
/// budget, new expiry, and the resend counted.
pub async fn replace_code(
    pool: &PgPool,
    table: ChallengeTable,
    id: i64,
    code_hash: &str,
    expires_at: DateTime<Utc>,
) -> Result<(), sqlx::Error> {
    sqlx::query(&format!(
        "UPDATE {} SET code_hash = $2, attempts = 0, \
         resend_count = resend_count + 1, code_sent_at = now(), expires_at = $3, \
         verified = FALSE WHERE id = $1",
        table.name()
    ))
    .bind(id)
    .bind(code_hash)
    .bind(expires_at)
    .execute(pool)
    .await?;
    Ok(())
}

/// Deletes a member's pending verification records
/// (password resets and email changes) inside the
/// caller's transaction: a live record is a
/// credential-equivalent and must not resurrect a
/// withdrawn account (Account Deletion spec 3.4).
pub async fn delete_all_for_user(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    user_id: i64,
) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM password_resets WHERE user_id = $1")
        .bind(user_id)
        .execute(&mut **tx)
        .await?;
    sqlx::query("DELETE FROM email_changes WHERE user_id = $1")
        .bind(user_id)
        .execute(&mut **tx)
        .await?;
    sqlx::query("DELETE FROM withdrawal_codes WHERE user_id = $1")
        .bind(user_id)
        .execute(&mut **tx)
        .await
        .map(|_| ())
}
