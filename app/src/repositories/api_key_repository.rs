//! API key repository.
//!
//! Owns every query against `api_keys`, the MCP clients'
//! bearer credentials. Only the secret's Argon2id hash is
//! stored; the prefix is the lookup key. Revocation is a
//! timestamp so a revoked key stays explainable.

use chrono::{DateTime, Utc};
use sqlx::PgPool;

/// One API key as stored. The hash is a credential: this row
/// never leaves the service layer, and it deliberately does
/// not derive `Debug`, so no log line can print the hash.
#[derive(sqlx::FromRow)]
pub struct ApiKeyRow {
    /// Surrogate ID.
    pub id: i64,
    /// Operator-chosen label.
    pub name: String,
    /// Public lookup prefix (not a secret).
    pub prefix: String,
    /// Argon2id hash of the secret portion.
    pub secret_hash: String,
    /// Granted scope: `catalog:read` or `cart:write`.
    pub scope: String,
    /// When the key was issued.
    pub created_at: DateTime<Utc>,
    /// When the key last authenticated successfully.
    pub last_used_at: Option<DateTime<Utc>>,
    /// When the key was revoked; `None` while active.
    pub revoked_at: Option<DateTime<Utc>>,
}

/// All columns of an API key, in table order of use.
const API_KEY_COLUMNS: &str = "id, name, prefix, secret_hash, scope, created_at, \
    last_used_at, revoked_at";

/// Inserts one key and returns its row (with the new ID).
pub async fn insert_key(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    name: &str,
    prefix: &str,
    secret_hash: &str,
    scope: &str,
) -> Result<ApiKeyRow, sqlx::Error> {
    sqlx::query_as::<_, ApiKeyRow>(&format!(
        "INSERT INTO api_keys (name, prefix, secret_hash, scope) \
         VALUES ($1, $2, $3, $4) RETURNING {API_KEY_COLUMNS}"
    ))
    .bind(name)
    .bind(prefix)
    .bind(secret_hash)
    .bind(scope)
    .fetch_one(&mut **tx)
    .await
}

/// Finds one key by its public prefix.
pub async fn find_by_prefix(pool: &PgPool, prefix: &str) -> Result<Option<ApiKeyRow>, sqlx::Error> {
    sqlx::query_as::<_, ApiKeyRow>(&format!(
        "SELECT {API_KEY_COLUMNS} FROM api_keys WHERE prefix = $1"
    ))
    .bind(prefix)
    .fetch_optional(pool)
    .await
}

/// Lists every key, newest first, for the admin page.
pub async fn list_keys(pool: &PgPool) -> Result<Vec<ApiKeyRow>, sqlx::Error> {
    sqlx::query_as::<_, ApiKeyRow>(&format!(
        "SELECT {API_KEY_COLUMNS} FROM api_keys ORDER BY id DESC"
    ))
    .fetch_all(pool)
    .await
}

/// Stamps a successful authentication. Best-effort by the
/// caller's contract: authentication has already succeeded
/// when this runs, so a failed stamp must not refuse it.
pub async fn touch_last_used(pool: &PgPool, id: i64) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE api_keys SET last_used_at = now() WHERE id = $1")
        .bind(id)
        .execute(pool)
        .await
        .map(|_| ())
}

/// Revokes one key inside the caller's transaction; returns
/// false when the key does not exist or is already revoked.
pub async fn revoke_key(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    id: i64,
) -> Result<bool, sqlx::Error> {
    sqlx::query("UPDATE api_keys SET revoked_at = now() WHERE id = $1 AND revoked_at IS NULL")
        .bind(id)
        .execute(&mut **tx)
        .await
        .map(|r| r.rows_affected() == 1)
}
