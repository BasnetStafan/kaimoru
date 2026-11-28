//! API key service: issuing, authentication and revocation
//! of the bearer keys external MCP clients present.
//!
//! A token is `kmcp_<prefix>_<secret>`. The prefix is a
//! public lookup key; the secret is stored only as an
//! Argon2id hash (the password treatment), so the full token
//! exists exactly once — in the return value of
//! [`create_key`], shown to the operator at creation.

use rand::RngCore;
use sqlx::PgPool;

use crate::error::AppError;
use crate::repositories::api_key_repository::{self, ApiKeyRow};
use crate::services::admin::audit_service;
use crate::services::password;
use crate::util::hmac_util::hex_encode;

/// The read-only scope: catalog tools only.
pub const SCOPE_CATALOG_READ: &str = "catalog:read";
/// The cart scope: catalog plus cart tools.
pub const SCOPE_CART_WRITE: &str = "cart:write";
/// Every scope a key may be issued with, in display order.
pub const SCOPES: [&str; 2] = [SCOPE_CATALOG_READ, SCOPE_CART_WRITE];

/// The token's fixed leading marker.
const TOKEN_MARKER: &str = "kmcp_";
/// Random bytes behind the public prefix (12 hex chars).
const PREFIX_BYTES: usize = 6;
/// Random bytes behind the secret (64 hex chars).
const SECRET_BYTES: usize = 32;

/// Who a presented token authenticated as. Carries no secret
/// material, so it is safe to pass around and log by ID.
#[derive(Clone, Debug)]
pub struct ApiKeyIdentity {
    /// The key's ID.
    pub id: i64,
    /// The key's granted scope.
    pub scope: String,
}

/// Splits a presented token into its prefix and secret.
/// Pure shape validation: exact marker, exact lengths, hex
/// only — anything else is not one of our tokens at all.
fn parse_token(token: &str) -> Option<(&str, &str)> {
    let rest = token.strip_prefix(TOKEN_MARKER)?;
    let (prefix, secret) = rest.split_once('_')?;
    let is_hex = |s: &str| s.bytes().all(|b| b.is_ascii_hexdigit());
    if prefix.len() == PREFIX_BYTES * 2
        && secret.len() == SECRET_BYTES * 2
        && is_hex(prefix)
        && is_hex(secret)
    {
        Some((prefix, secret))
    } else {
        None
    }
}

/// Mints a fresh token's parts: the stored prefix, the
/// secret to hash, and the full token to show once.
fn mint_token() -> (String, String, String) {
    let mut prefix_bytes = [0u8; PREFIX_BYTES];
    rand::thread_rng().fill_bytes(&mut prefix_bytes);
    let mut secret_bytes = [0u8; SECRET_BYTES];
    rand::thread_rng().fill_bytes(&mut secret_bytes);
    let prefix = hex_encode(&prefix_bytes);
    let secret = hex_encode(&secret_bytes);
    let token = format!("{TOKEN_MARKER}{prefix}_{secret}");
    (prefix, secret, token)
}

/// Issues a key for the admin surface, audited in the same
/// transaction. Returns the stored row and the full token —
/// the token is never stored and can never be shown again.
///
/// # Errors
/// Returns [`AppError::Conflict`] with an operator-facing
/// message when the name is blank or the scope is not one of
/// [`SCOPES`], and a database error when the write fails.
pub async fn create_key(
    pool: &PgPool,
    admin_id: i64,
    name: &str,
    scope: &str,
) -> Result<(ApiKeyRow, String), AppError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(AppError::Conflict("名前を入力してください。".to_string()));
    }
    if !SCOPES.contains(&scope) {
        return Err(AppError::Conflict("スコープが不正です。".to_string()));
    }
    let (prefix, secret, token) = mint_token();
    // Argon2id is CPU work: it runs on the blocking pool via
    // the password service, never on a Tokio worker.
    let secret_hash = password::hash_password_async(secret).await?;
    let mut tx = pool.begin().await.map_err(|e| AppError::db("begin", e))?;
    let row = api_key_repository::insert_key(&mut tx, name, &prefix, &secret_hash, scope).await?;
    audit_service::record(&mut tx, admin_id, "api_key.create", "api_key", Some(row.id)).await?;
    tx.commit().await.map_err(|e| AppError::db("commit", e))?;
    Ok((row, token))
}

/// Authenticates a presented bearer token. Every failure —
/// malformed, unknown prefix, revoked, wrong secret — is the
/// same `Err(())`: the answer must never say which step
/// failed, or the endpoint becomes an oracle for probing
/// prefixes and revocation state.
#[expect(
    clippy::result_unit_err,
    reason = "the unit error is the no-oracle contract: there is deliberately no error value to return"
)]
pub async fn authenticate(pool: &PgPool, presented: &str) -> Result<ApiKeyIdentity, ()> {
    let (prefix, secret) = parse_token(presented).ok_or(())?;
    let row = api_key_repository::find_by_prefix(pool, prefix)
        .await
        .map_err(|e| {
            tracing::error!("api key lookup failed: {e}");
        })?
        .ok_or(())?;
    if row.revoked_at.is_some() {
        return Err(());
    }
    if !password::verify_password_async(secret.to_string(), row.secret_hash.clone()).await {
        return Err(());
    }
    // The stamp is bookkeeping, not part of the decision:
    // authentication already succeeded, so a failed stamp is
    // logged, never raised over the success.
    if let Err(e) = api_key_repository::touch_last_used(pool, row.id).await {
        tracing::warn!(key_id = row.id, "api key last-used stamp failed: {e}");
    }
    Ok(ApiKeyIdentity {
        id: row.id,
        scope: row.scope,
    })
}

/// Lists every key for the admin page, newest first.
pub async fn list_keys(pool: &PgPool) -> Result<Vec<ApiKeyRow>, AppError> {
    Ok(api_key_repository::list_keys(pool).await?)
}

/// Revokes one key, audited in the same transaction.
/// Returns false when the key is unknown or already revoked.
pub async fn revoke_key(pool: &PgPool, admin_id: i64, id: i64) -> Result<bool, AppError> {
    let mut tx = pool.begin().await.map_err(|e| AppError::db("begin", e))?;
    if !api_key_repository::revoke_key(&mut tx, id).await? {
        return Ok(false);
    }
    audit_service::record(&mut tx, admin_id, "api_key.revoke", "api_key", Some(id)).await?;
    tx.commit().await.map_err(|e| AppError::db("commit", e))?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn minted_token_parses_back() {
        let (prefix, secret, token) = mint_token();
        let (parsed_prefix, parsed_secret) = parse_token(&token).expect("own token parses");
        assert_eq!(parsed_prefix, prefix);
        assert_eq!(parsed_secret, secret);
        assert_eq!(prefix.len(), 12);
        assert_eq!(secret.len(), 64);
    }

    #[test]
    fn malformed_tokens_are_refused() {
        let (_, _, token) = mint_token();
        // No marker.
        assert!(parse_token(&token[TOKEN_MARKER.len()..]).is_none());
        // No secret half.
        assert!(parse_token("kmcp_abcdef012345").is_none());
        // Short secret.
        assert!(parse_token("kmcp_abcdef012345_abcd").is_none());
        // Non-hex secret of the right length.
        let bad = format!("kmcp_abcdef012345_{}", "z".repeat(64));
        assert!(parse_token(&bad).is_none());
        // Empty and unrelated input.
        assert!(parse_token("").is_none());
        assert!(parse_token("Bearer abc").is_none());
    }
}
