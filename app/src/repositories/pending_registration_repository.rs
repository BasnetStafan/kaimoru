//! Pending registration repository.
//! Owns every query against pending_registrations, the table a sign-up
//! lives in until the account is created.

use chrono::{DateTime, Utc};
use sqlx::PgPool;

use crate::models::PendingRegistration;
use crate::pii::{blind_index_email, decrypt_pending, encrypt_field, encrypt_field_opt};

/// All columns of the pending registration, in table order of use.
const PENDING_COLUMNS: &str = "id, email, code_hash, attempts, resend_count, code_sent_at, \
    expires_at, verified, password_hash, family_name, given_name, family_name_kana, \
    given_name_kana, birth_date, gender, postal_code, prefecture, city, address_line, \
    building, phone, mail_magazine";

/// Starts a new pending registration for an email.
/// Any earlier pending row for the same email is removed first, so one
/// email has at most one sign-up in progress.
pub async fn create(
    pool: &PgPool,
    email: &str,
    code_hash: &str,
    expires_at: DateTime<Utc>,
) -> Result<i64, sqlx::Error> {
    // One transaction: two concurrent starts for the same email
    // must not interleave delete and insert.
    let mut tx = pool.begin().await?;
    sqlx::query("DELETE FROM pending_registrations WHERE email_bidx = $1")
        .bind(blind_index_email(email))
        .execute(&mut *tx)
        .await?;

    let id = sqlx::query_scalar(
        "INSERT INTO pending_registrations (email, email_bidx, code_hash, expires_at) \
         VALUES ($1, $2, $3, $4) RETURNING id",
    )
    .bind(encrypt_field(email))
    .bind(blind_index_email(email))
    .bind(code_hash)
    .bind(expires_at)
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(id)
}

/// Deletes abandoned sign-ups a day after their code expired.
///
/// Until this runs, an abandoned row holds a password hash and
/// personal details; a day of grace covers slow completions.
pub async fn delete_expired(pool: &PgPool) -> Result<(), sqlx::Error> {
    sqlx::query(
        "DELETE FROM pending_registrations \
         WHERE expires_at < now() - interval '1 day'",
    )
    .execute(pool)
    .await?;
    Ok(())
}

/// Finds a pending registration by ID.
pub async fn find_by_id(
    pool: &PgPool,
    id: i64,
) -> Result<Option<PendingRegistration>, sqlx::Error> {
    let mut pending = sqlx::query_as::<_, PendingRegistration>(&format!(
        "SELECT {PENDING_COLUMNS} FROM pending_registrations WHERE id = $1"
    ))
    .bind(id)
    .fetch_optional(pool)
    .await?;
    if let Some(pending) = &mut pending {
        decrypt_pending(pending);
    }
    Ok(pending)
}

/// Records one wrong code entry, persisted on purpose: a restart
/// must never hand out fresh guesses.
///
/// The increment is atomic with the limit in the WHERE clause, so
/// parallel verify requests cannot overshoot the budget. Returns
/// whether the attempt was counted (false = limit already hit).
pub async fn record_failed_attempt_capped(
    pool: &PgPool,
    id: i64,
    max_attempts: i32,
) -> Result<bool, sqlx::Error> {
    let updated: Option<i64> = sqlx::query_scalar(
        "UPDATE pending_registrations SET attempts = attempts + 1 \
         WHERE id = $1 AND attempts < $2 RETURNING id",
    )
    .bind(id)
    .bind(max_attempts)
    .fetch_optional(pool)
    .await?;
    Ok(updated.is_some())
}

/// Marks the pending registration as code verified.
pub async fn mark_verified(pool: &PgPool, id: i64) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE pending_registrations SET verified = TRUE WHERE id = $1")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Replaces the code after a resend: new hash, fresh attempt budget,
/// new expiry, and the resend counted.
pub async fn replace_code(
    pool: &PgPool,
    id: i64,
    code_hash: &str,
    expires_at: DateTime<Utc>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE pending_registrations SET code_hash = $2, attempts = 0, \
         resend_count = resend_count + 1, code_sent_at = now(), expires_at = $3, \
         verified = FALSE WHERE id = $1",
    )
    .bind(id)
    .bind(code_hash)
    .bind(expires_at)
    .execute(pool)
    .await?;
    Ok(())
}

/// The member details collected at the details step.
/// The password arrives already hashed with Argon2id.
#[allow(clippy::too_many_arguments)]
pub async fn save_details(
    pool: &PgPool,
    id: i64,
    password_hash: Option<&str>,
    family_name: &str,
    given_name: &str,
    family_name_kana: &str,
    given_name_kana: &str,
    birth_date: Option<chrono::NaiveDate>,
    gender: Option<&str>,
    postal_code: &str,
    prefecture: &str,
    city: &str,
    address_line: &str,
    building: Option<&str>,
    phone: &str,
    mail_magazine: bool,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE pending_registrations SET password_hash = $2, family_name = $3, \
         given_name = $4, family_name_kana = $5, given_name_kana = $6, \
         birth_date = $7, gender = $8, postal_code = $9, prefecture = $10, \
         city = $11, address_line = $12, building = $13, phone = $14, \
         mail_magazine = $15 WHERE id = $1",
    )
    .bind(id)
    .bind(password_hash)
    .bind(encrypt_field(family_name))
    .bind(encrypt_field(given_name))
    .bind(encrypt_field(family_name_kana))
    .bind(encrypt_field(given_name_kana))
    .bind(birth_date)
    .bind(gender)
    .bind(encrypt_field(postal_code))
    .bind(encrypt_field(prefecture))
    .bind(encrypt_field(city))
    .bind(encrypt_field(address_line))
    .bind(encrypt_field_opt(building))
    .bind(encrypt_field(phone))
    .bind(mail_magazine)
    .execute(pool)
    .await?;
    Ok(())
}

/// Starts a pending registration whose email is already
/// verified by an external provider (SNS sign-up): the row is
/// born verified, its code hash an unusable placeholder (no
/// code is ever sent for it), and it exists so the details and
/// confirm steps key off the same pending state as the email
/// pipeline.
pub async fn create_verified(pool: &PgPool, email: &str) -> Result<i64, sqlx::Error> {
    let mut tx = pool.begin().await?;
    sqlx::query("DELETE FROM pending_registrations WHERE email_bidx = $1")
        .bind(blind_index_email(email))
        .execute(&mut *tx)
        .await?;
    let id = sqlx::query_scalar(
        "INSERT INTO pending_registrations (email, email_bidx, code_hash, expires_at, verified) \
         VALUES ($1, $2, '', now() + interval '1 day', TRUE) RETURNING id",
    )
    .bind(encrypt_field(email))
    .bind(blind_index_email(email))
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(id)
}

/// Deletes one pending registration by ID, when a flow is
/// rerouted (an SNS email that turns out to belong to an
/// existing account becomes a link request instead).
pub async fn delete_by_id(pool: &PgPool, id: i64) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM pending_registrations WHERE id = $1")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}
