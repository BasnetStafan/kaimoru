//! MCP order draft repository.
//!
//! Owns every query against `mcp_order_drafts`, the frozen
//! cart a person approves in the browser: the creating
//! principal, the cart it binds, the idempotency key that
//! makes creation safe to retry, the delivery address, the
//! creation fingerprint and snapshot, and the single-use
//! approval token's digest. Status moves exactly like the
//! agent approvals' — every transition is a conditional
//! UPDATE on the status it expects, so a race resolves to
//! "already moved" instead of a double effect.
//!
//! Ownership is the `principal_key` (Phase 4): one value
//! for both credential kinds, `key:<id>` or
//! `oauth:<subject>`; idempotency is unique per principal.
//! The `api_key_id` / `oauth_subject` columns are
//! provenance — exactly one of them is set.

use chrono::{DateTime, Utc};
use sqlx::PgPool;

/// Draft status: awaiting the person's approval.
pub const STATUS_PENDING: &str = "pending";
/// Draft status: the pay click placed the order.
pub const STATUS_CONSUMED: &str = "consumed";
/// Draft status: the approval window closed unused.
pub const STATUS_EXPIRED: &str = "expired";
/// Draft status: the cart changed under the draft.
pub const STATUS_VOID: &str = "void";

/// One order draft as stored. The address is the buyer's
/// personal data and the token hash a credential digest:
/// the row never leaves the service layer and does not
/// derive `Debug`.
#[derive(sqlx::FromRow)]
pub struct McpOrderDraftRow {
    /// The draft's ID.
    pub id: i64,
    /// The API key that created the draft, when a key did.
    pub api_key_id: Option<i64>,
    /// The one ownership value every check compares.
    pub principal_key: String,
    /// The OAuth subject that created the draft, when a
    /// token did.
    pub oauth_subject: Option<String>,
    /// The MCP cart handle the draft binds.
    pub cart_id: String,
    /// The client's idempotency key for this creation.
    pub idempotency_key: String,
    /// The delivery address the draft orders to, as JSON.
    pub address: serde_json::Value,
    /// The creation fingerprint (lines + cart totals).
    pub fingerprint: String,
    /// The creation display snapshot, as JSON.
    pub snapshot: serde_json::Value,
    /// HMAC-SHA-256 hash of the approval token.
    pub approval_token_hash: String,
    /// One of the `STATUS_*` values.
    pub status: String,
    /// The order the pay click placed, once consumed.
    pub order_id: Option<i64>,
    /// When the draft was created.
    pub created_at: DateTime<Utc>,
    /// When the approval window closes.
    pub expires_at: DateTime<Utc>,
}

/// All columns of an order draft, in table order of use.
const DRAFT_COLUMNS: &str = "id, api_key_id, principal_key, oauth_subject, cart_id, \
    idempotency_key, address, fingerprint, snapshot, approval_token_hash, status, \
    order_id, created_at, expires_at";

/// Inserts a draft and returns its ID. The
/// `(principal_key, idempotency_key)` uniqueness is the
/// idempotency guard: a racing duplicate insert fails
/// with a unique violation and the caller re-reads.
///
/// # Errors
/// Returns the database error, including the unique
/// violation a duplicate idempotency key raises.
#[expect(
    clippy::too_many_arguments,
    reason = "one argument per stored column of the creation write; a parameter struct would only rename the same list"
)]
pub async fn insert(
    pool: &PgPool,
    api_key_id: Option<i64>,
    oauth_subject: Option<&str>,
    principal_key: &str,
    cart_id: &str,
    idempotency_key: &str,
    address: &serde_json::Value,
    fingerprint: &str,
    snapshot: &serde_json::Value,
    approval_token_hash: &str,
    expires_at: DateTime<Utc>,
) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar(
        "INSERT INTO mcp_order_drafts (api_key_id, oauth_subject, principal_key, cart_id, \
         idempotency_key, address, fingerprint, snapshot, approval_token_hash, expires_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10) RETURNING id",
    )
    .bind(api_key_id)
    .bind(oauth_subject)
    .bind(principal_key)
    .bind(cart_id)
    .bind(idempotency_key)
    .bind(address)
    .bind(fingerprint)
    .bind(snapshot)
    .bind(approval_token_hash)
    .bind(expires_at)
    .fetch_one(pool)
    .await
}

/// Finds the draft a principal created under one
/// idempotency key — the retried create's answer.
pub async fn find_by_idempotency(
    pool: &PgPool,
    principal_key: &str,
    idempotency_key: &str,
) -> Result<Option<McpOrderDraftRow>, sqlx::Error> {
    sqlx::query_as::<_, McpOrderDraftRow>(&format!(
        "SELECT {DRAFT_COLUMNS} FROM mcp_order_drafts \
         WHERE principal_key = $1 AND idempotency_key = $2"
    ))
    .bind(principal_key)
    .bind(idempotency_key)
    .fetch_optional(pool)
    .await
}

/// Finds one of a principal's own drafts by ID. Another
/// principal's draft answers exactly like a missing one —
/// the caller cannot probe across principals.
pub async fn find_by_id_and_principal(
    pool: &PgPool,
    id: i64,
    principal_key: &str,
) -> Result<Option<McpOrderDraftRow>, sqlx::Error> {
    sqlx::query_as::<_, McpOrderDraftRow>(&format!(
        "SELECT {DRAFT_COLUMNS} FROM mcp_order_drafts WHERE id = $1 AND principal_key = $2"
    ))
    .bind(id)
    .bind(principal_key)
    .fetch_optional(pool)
    .await
}

/// Finds the draft an approval token belongs to, by its
/// hash — the browser approval page's lookup.
pub async fn find_by_token_hash(
    pool: &PgPool,
    token_hash: &str,
) -> Result<Option<McpOrderDraftRow>, sqlx::Error> {
    sqlx::query_as::<_, McpOrderDraftRow>(&format!(
        "SELECT {DRAFT_COLUMNS} FROM mcp_order_drafts WHERE approval_token_hash = $1"
    ))
    .bind(token_hash)
    .fetch_optional(pool)
    .await
}

/// Moves a draft from one status to another, only if it
/// still holds the expected status. `false` means a race
/// already moved it — the agent approvals' discipline.
pub async fn transition(pool: &PgPool, id: i64, from: &str, to: &str) -> Result<bool, sqlx::Error> {
    let result =
        sqlx::query("UPDATE mcp_order_drafts SET status = $3 WHERE id = $1 AND status = $2")
            .bind(id)
            .bind(from)
            .bind(to)
            .execute(pool)
            .await?;
    Ok(result.rows_affected() == 1)
}

/// Voids every pending draft bound to one cart: a cart
/// mutation makes each outstanding draft's binding stale,
/// eagerly, so the draft reads `void` from then on (the
/// pay-time fingerprint re-check remains the hard
/// guarantee). Returns how many drafts were voided.
pub async fn void_pending_for_cart(pool: &PgPool, cart_id: &str) -> Result<u64, sqlx::Error> {
    let result =
        sqlx::query("UPDATE mcp_order_drafts SET status = $2 WHERE cart_id = $1 AND status = $3")
            .bind(cart_id)
            .bind(STATUS_VOID)
            .bind(STATUS_PENDING)
            .execute(pool)
            .await?;
    Ok(result.rows_affected())
}

/// Re-binds one pending draft to its final state at the
/// pay click: the headline fingerprint becomes the final
/// hash and the snapshot the final snapshot. Conditional
/// on the draft still being pending — `false` means it
/// stopped being pending first, which the caller treats
/// as a refusal.
pub async fn update_binding(
    pool: &PgPool,
    id: i64,
    fingerprint: &str,
    snapshot: &serde_json::Value,
) -> Result<bool, sqlx::Error> {
    let result = sqlx::query(
        "UPDATE mcp_order_drafts SET fingerprint = $2, snapshot = $3 \
         WHERE id = $1 AND status = $4",
    )
    .bind(id)
    .bind(fingerprint)
    .bind(snapshot)
    .bind(STATUS_PENDING)
    .execute(pool)
    .await?;
    Ok(result.rows_affected() == 1)
}

/// Consumes one pending draft for the order just placed,
/// linking the two in the same write. Conditional on the
/// draft still being pending: a second consume (a raced
/// double pay) matches nothing instead of overwriting.
pub async fn consume(pool: &PgPool, id: i64, order_id: i64) -> Result<bool, sqlx::Error> {
    let result = sqlx::query(
        "UPDATE mcp_order_drafts SET status = $3, order_id = $2 WHERE id = $1 AND status = $4",
    )
    .bind(id)
    .bind(order_id)
    .bind(STATUS_CONSUMED)
    .bind(STATUS_PENDING)
    .execute(pool)
    .await?;
    Ok(result.rows_affected() == 1)
}

/// One placed order's status as its drafting key may see
/// it: fulfilment status and the shipment facts, nothing
/// about the buyer.
#[derive(sqlx::FromRow)]
pub struct McpOrderStatusRow {
    /// The order's ID.
    pub order_id: i64,
    /// Fulfilment status key (`received`, `preparing`, …).
    pub status: String,
    /// Grand total in yen.
    pub total_yen: i64,
    /// When the order was placed.
    pub created_at: DateTime<Utc>,
    /// When the order was delivered, when it has been.
    pub delivered_at: Option<DateTime<Utc>>,
    /// Delivery carrier code, once shipped with one named.
    pub carrier: Option<String>,
    /// Carrier tracking number, once shipped with one.
    pub tracking_number: Option<String>,
}

/// Reads one order's status through the draft that
/// placed it: the join is the authorisation — an order no
/// draft of the caller's principal placed answers like a
/// missing one.
pub async fn find_order_status(
    pool: &PgPool,
    order_id: i64,
    principal_key: &str,
) -> Result<Option<McpOrderStatusRow>, sqlx::Error> {
    sqlx::query_as::<_, McpOrderStatusRow>(
        "SELECT o.id AS order_id, o.status, o.total_yen, o.created_at, o.delivered_at, \
         o.carrier, o.tracking_number \
         FROM mcp_order_drafts d JOIN orders o ON o.id = d.order_id \
         WHERE d.order_id = $1 AND d.principal_key = $2",
    )
    .bind(order_id)
    .bind(principal_key)
    .fetch_optional(pool)
    .await
}
