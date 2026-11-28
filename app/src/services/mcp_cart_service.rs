//! MCP cart service: the cart handles an MCP client works
//! with, and the single-use claim that hands a cart to a
//! person's browser.
//!
//! A cart handle is an unguessable id that is also the cart's
//! `cart_items` session_id, so an MCP cart is a guest cart in
//! every mechanical respect — same lines table, same totals,
//! same abandoned-cart sweep. The `mcp_carts` binding adds
//! the one thing a guest token does not have: an owner (the
//! API key), checked on every tool call.
//!
//! The claim token follows the verification-code discipline
//! of `password_resets` / `email_changes`: the raw token is
//! never stored, only its HMAC-SHA-256 digest, keyed by the
//! server secret through the shared `hmac_util`. The token
//! itself is 256 bits of randomness, so unlike a 6-digit
//! code it needs no per-account binding — the digest alone
//! is the lookup key.

use chrono::{DateTime, Duration, Utc};
use rand::RngCore;
use sqlx::PgPool;
use tower_sessions::Session;

use crate::error::AppError;
use crate::repositories::{cart_repository, mcp_cart_repository};
use crate::services::cart_service::{self, CartOwner};
use crate::util::hmac_util::{hex_encode, hmac_sha256_hex};

/// The cart handle's fixed leading marker.
pub const CART_ID_PREFIX: &str = "mcp_";
/// Random bytes behind a cart handle (32 hex chars).
const CART_ID_BYTES: usize = 16;
/// Random bytes behind a claim token (64 hex chars).
const CLAIM_TOKEN_BYTES: usize = 32;
/// How long a minted checkout link stays usable.
pub const CLAIM_TTL_MINUTES: i64 = 30;

/// Hashes a claim token for storage or lookup, exactly as
/// the code challenges hash their codes: HMAC-SHA-256 keyed
/// by the server's verification secret, through the one
/// shared helper, so a leaked table alone cannot be brute
/// forced offline without the secret.
fn hash_claim_token(token: &str) -> String {
    // Fail fast, as with the code challenges: without this
    // secret a leaked claim-hash table could be attacked
    // offline, so the app must not run without it. Startup
    // validation in the server binary checks the variable
    // before any flow can reach this.
    static SECRET: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| {
        std::env::var("VERIFICATION_CODE_SECRET")
            .expect("VERIFICATION_CODE_SECRET must be set (see README / run.sh)")
    });
    let secret: &str = &SECRET;
    hmac_sha256_hex(secret.as_bytes(), &[token.as_bytes()])
}

/// Creates a cart bound to the calling key and returns its
/// handle. No lines exist yet; the handle alone is the cart.
pub async fn create_cart(pool: &PgPool, api_key_id: i64) -> Result<String, AppError> {
    let mut bytes = [0u8; CART_ID_BYTES];
    rand::thread_rng().fill_bytes(&mut bytes);
    let cart_id = format!("{CART_ID_PREFIX}{}", hex_encode(&bytes));
    mcp_cart_repository::create_cart(pool, &cart_id, api_key_id).await?;
    Ok(cart_id)
}

/// Returns the owning key's ID for a cart handle, or `None`
/// when no MCP cart has it.
pub async fn owner_key_id(pool: &PgPool, cart_id: &str) -> Result<Option<i64>, AppError> {
    Ok(mcp_cart_repository::owner_key_id(pool, cart_id).await?)
}

/// Mints a fresh claim for a cart, replacing any previous
/// one, and returns the raw token (never stored) with its
/// expiry. The checkout URL is built by the caller, which
/// knows the request's public base.
pub async fn mint_claim(pool: &PgPool, cart_id: &str) -> Result<(String, DateTime<Utc>), AppError> {
    let mut bytes = [0u8; CLAIM_TOKEN_BYTES];
    rand::thread_rng().fill_bytes(&mut bytes);
    let token = hex_encode(&bytes);
    let expires_at = Utc::now() + Duration::minutes(CLAIM_TTL_MINUTES);
    mcp_cart_repository::set_claim(pool, cart_id, &hash_claim_token(&token), expires_at).await?;
    Ok((token, expires_at))
}

/// Redeems a claim token for the current visitor: the MCP
/// cart's lines move onto the visitor's own cart (the
/// member's, or their guest session's) in one transaction
/// with the claim being marked used, so the handoff happens
/// exactly once or not at all.
///
/// Returns `false` for every refusal — unknown, replaced,
/// already used, or expired — with no distinction between
/// them: the link is a bearer credential and its failure
/// modes are not the bearer's to enumerate.
pub async fn redeem_claim(pool: &PgPool, session: &Session, token: &str) -> Result<bool, AppError> {
    let token_hash = hash_claim_token(token);
    let Some(row) = mcp_cart_repository::find_by_claim_hash(pool, &token_hash).await? else {
        return Ok(false);
    };
    if row.claim_used_at.is_some() {
        return Ok(false);
    }
    match row.claim_expires_at {
        Some(expires_at) if expires_at > Utc::now() => {}
        _ => return Ok(false),
    }
    // Identity resolution only — the one Session use a
    // service may make (the cart service's standing rule).
    let owner = cart_service::owner_for(pool, session).await;
    let mut tx = pool.begin().await.map_err(|e| AppError::db("begin", e))?;
    match &owner {
        CartOwner::User(user_id) => {
            cart_repository::rekey_session_to_user(&mut tx, &row.cart_id, *user_id).await?;
        }
        CartOwner::Session(guest_token) => {
            if guest_token != &row.cart_id {
                cart_repository::rekey_session_to_session(&mut tx, &row.cart_id, guest_token)
                    .await?;
            }
        }
    }
    // The conditional UPDATE is the single-use guard riding
    // the mutation itself: if a racing redemption (or a
    // re-mint) changed the claim since the read above, this
    // matches nothing and the whole handoff rolls back.
    if !mcp_cart_repository::mark_claim_used(&mut tx, &row.cart_id, &token_hash).await? {
        return Ok(false);
    }
    tx.commit().await.map_err(|e| AppError::db("commit", e))?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cart_handles_carry_the_prefix() {
        // The shape contract the tool boundary relies on:
        // handles are distinguishable from guest tokens.
        assert_eq!(CART_ID_PREFIX, "mcp_");
        assert_eq!(CART_ID_BYTES * 2, 32);
        assert_eq!(CLAIM_TOKEN_BYTES * 2, 64);
    }
}
