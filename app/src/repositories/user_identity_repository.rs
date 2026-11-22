//! User identity repository.
//! Owns every query against the user_identities table (SNS
//! login links between member accounts and external providers).

use sqlx::{PgPool, Postgres, Transaction};

use crate::models::UserIdentity;

/// All columns of a user identity, in table order of use.
const IDENTITY_COLUMNS: &str = "id, user_id, provider, provider_user_id, provider_email, \
    created_at, updated_at";

/// Finds the identity for one provider account, when it has
/// been linked: this is the SNS sign-in lookup.
pub async fn find_by_provider_sub(
    pool: &PgPool,
    provider: &str,
    provider_user_id: &str,
) -> Result<Option<UserIdentity>, sqlx::Error> {
    sqlx::query_as::<_, UserIdentity>(&format!(
        "SELECT {IDENTITY_COLUMNS} FROM user_identities \
         WHERE provider = $1 AND provider_user_id = $2"
    ))
    .bind(provider)
    .bind(provider_user_id)
    .fetch_optional(pool)
    .await
}

/// Lists every identity linked to one member, oldest first.
pub async fn list_for_user(pool: &PgPool, user_id: i64) -> Result<Vec<UserIdentity>, sqlx::Error> {
    sqlx::query_as::<_, UserIdentity>(&format!(
        "SELECT {IDENTITY_COLUMNS} FROM user_identities \
         WHERE user_id = $1 ORDER BY id"
    ))
    .bind(user_id)
    .fetch_all(pool)
    .await
}

/// Counts the identities linked to one member: the unlink
/// guard needs it to protect a passwordless account's only
/// remaining sign-in method.
pub async fn count_for_user(pool: &PgPool, user_id: i64) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar("SELECT COUNT(*) FROM user_identities WHERE user_id = $1")
        .bind(user_id)
        .fetch_one(pool)
        .await
}

/// Inserts one identity link inside a caller-owned transaction
/// (account creation links the identity atomically with the
/// account itself).
pub async fn insert_in_tx(
    tx: &mut Transaction<'_, Postgres>,
    user_id: i64,
    provider: &str,
    provider_user_id: &str,
    provider_email: Option<&str>,
) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar(
        "INSERT INTO user_identities (user_id, provider, provider_user_id, provider_email) \
         VALUES ($1, $2, $3, $4) RETURNING id",
    )
    .bind(user_id)
    .bind(provider)
    .bind(provider_user_id)
    .bind(provider_email)
    .fetch_one(&mut **tx)
    .await
}

/// Links one identity to an existing member (My Page linking
/// and the post-login link confirmation).
pub async fn insert(
    pool: &PgPool,
    user_id: i64,
    provider: &str,
    provider_user_id: &str,
    provider_email: Option<&str>,
) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar(
        "INSERT INTO user_identities (user_id, provider, provider_user_id, provider_email) \
         VALUES ($1, $2, $3, $4) RETURNING id",
    )
    .bind(user_id)
    .bind(provider)
    .bind(provider_user_id)
    .bind(provider_email)
    .fetch_one(pool)
    .await
}

/// Removes one provider link from a member. Returns whether a
/// row was deleted.
pub async fn delete(pool: &PgPool, user_id: i64, provider: &str) -> Result<bool, sqlx::Error> {
    let result = sqlx::query("DELETE FROM user_identities WHERE user_id = $1 AND provider = $2")
        .bind(user_id)
        .bind(provider)
        .execute(pool)
        .await?;
    Ok(result.rows_affected() > 0)
}

/// Deletes every identity of a member inside a transaction:
/// part of the withdrawal scrub, so a withdrawn account leaves
/// no sign-in path behind.
pub async fn delete_all_for_user(
    tx: &mut Transaction<'_, Postgres>,
    user_id: i64,
) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM user_identities WHERE user_id = $1")
        .bind(user_id)
        .execute(&mut **tx)
        .await?;
    Ok(())
}
