//! MCP cart repository.
//!
//! Owns every query against `mcp_carts`, the binding between
//! an MCP cart handle and the API key that created it. The
//! cart's lines live in `cart_items` under the cart_id as
//! their session_id (the guest-cart shape); this table only
//! answers "whose cart is this" and carries the single-use
//! browser-handoff claim.

use chrono::{DateTime, Utc};
use sqlx::PgPool;

/// One MCP cart binding as stored. The claim hash is a
/// credential digest: the row never leaves the service
/// layer and does not derive `Debug`.
#[derive(sqlx::FromRow)]
pub struct McpCartRow {
    /// The cart handle; also the cart's `cart_items` session_id.
    pub cart_id: String,
    /// The API key that created the cart.
    pub api_key_id: i64,
    /// HMAC-SHA-256 hash of the live claim token, if one was minted.
    pub claim_token_hash: Option<String>,
    /// When the live claim stops being valid.
    pub claim_expires_at: Option<DateTime<Utc>>,
    /// When the claim was used; set exactly once.
    pub claim_used_at: Option<DateTime<Utc>>,
    /// When the cart was created.
    pub created_at: DateTime<Utc>,
}

/// All columns of an MCP cart binding, in table order of use.
const MCP_CART_COLUMNS: &str = "cart_id, api_key_id, claim_token_hash, claim_expires_at, \
    claim_used_at, created_at";

/// Binds a fresh cart handle to its creating key.
pub async fn create_cart(pool: &PgPool, cart_id: &str, api_key_id: i64) -> Result<(), sqlx::Error> {
    sqlx::query("INSERT INTO mcp_carts (cart_id, api_key_id) VALUES ($1, $2)")
        .bind(cart_id)
        .bind(api_key_id)
        .execute(pool)
        .await
        .map(|_| ())
}

/// Returns the owning key's ID for a cart, or `None` when no
/// MCP cart has this handle.
pub async fn owner_key_id(pool: &PgPool, cart_id: &str) -> Result<Option<i64>, sqlx::Error> {
    sqlx::query_scalar("SELECT api_key_id FROM mcp_carts WHERE cart_id = $1")
        .bind(cart_id)
        .fetch_optional(pool)
        .await
}

/// Replaces the cart's claim with a fresh one: minting a new
/// checkout link invalidates the previous link, and a claim
/// that was already used is cleared back to unused.
pub async fn set_claim(
    pool: &PgPool,
    cart_id: &str,
    token_hash: &str,
    expires_at: DateTime<Utc>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE mcp_carts SET claim_token_hash = $2, claim_expires_at = $3, \
         claim_used_at = NULL WHERE cart_id = $1",
    )
    .bind(cart_id)
    .bind(token_hash)
    .bind(expires_at)
    .execute(pool)
    .await
    .map(|_| ())
}

/// Finds the cart a claim token belongs to, by its hash.
pub async fn find_by_claim_hash(
    pool: &PgPool,
    token_hash: &str,
) -> Result<Option<McpCartRow>, sqlx::Error> {
    sqlx::query_as::<_, McpCartRow>(&format!(
        "SELECT {MCP_CART_COLUMNS} FROM mcp_carts WHERE claim_token_hash = $1"
    ))
    .bind(token_hash)
    .fetch_optional(pool)
    .await
}

/// Marks the claim used inside the caller's transaction —
/// the same transaction that moves the cart's lines, so a
/// claim can never be spent without its cart moving, nor a
/// cart move without the claim being spent. The conditional
/// UPDATE is the guard: a claim already used (or replaced)
/// matches nothing and the caller rolls back.
pub async fn mark_claim_used(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    cart_id: &str,
    token_hash: &str,
) -> Result<bool, sqlx::Error> {
    sqlx::query(
        "UPDATE mcp_carts SET claim_used_at = now() \
         WHERE cart_id = $1 AND claim_token_hash = $2 AND claim_used_at IS NULL",
    )
    .bind(cart_id)
    .bind(token_hash)
    .execute(&mut **tx)
    .await
    .map(|r| r.rows_affected() == 1)
}
