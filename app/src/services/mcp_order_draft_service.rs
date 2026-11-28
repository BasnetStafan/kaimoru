//! MCP order draft service: the frozen cart an
//! `orders:write` key prepares and a person approves in
//! the browser.
//!
//! A draft is the MCP analogue of the agent settlement's
//! approval record, on the same machinery: the cart's
//! lines and totals are bound by
//! [`agent_approval_service::fingerprint_for`] and listed
//! by [`agent_approval_service::snapshot_for`], so a cart
//! change breaks the binding here exactly as it does
//! there. The differences are the caller (an API key, not
//! the cart owner's session) and the delivery address,
//! which the client supplies at creation and the approval
//! page merely shows. The draft never places an order:
//! the pay click on the approval page does, through the
//! storefront's ordinary placement, after re-verifying
//! the binding.
//!
//! The approval token follows the Phase 2 claim
//! discipline: 256 bits of randomness, only its
//! HMAC-SHA-256 digest stored (the shared hash in
//! [`super::mcp_cart_service`]).

use chrono::{DateTime, Utc};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;

use crate::config::CONFIG;
use crate::error::AppError;
use crate::messages::order_failed_message;
use crate::models::agent_approval::ApprovalSnapshot;
use crate::models::{CartLine, PaymentMethod};
use crate::repositories::mcp_order_draft_repository::{self, McpOrderDraftRow};
use crate::services::agent_approval_service::{self, PlacementRefusal};
use crate::services::cart_service::{CartOwner, CartTotals};
use crate::services::mcp_cart_service;
use crate::services::order_service::CheckoutFlow;
use crate::util::hmac_util::hex_encode;

/// Random bytes behind an approval token (64 hex chars).
const APPROVAL_TOKEN_BYTES: usize = 32;

/// The delivery address a draft orders to: field for field
/// the guest checkout's delivery step, because the draft
/// is placed as a guest order with exactly these values.
/// Personal data — no `Debug`, so it can never reach a log
/// line or a panic payload.
#[derive(Clone, Serialize, Deserialize)]
pub struct DraftAddress {
    /// Contact email the order confirms to.
    pub email: String,
    /// Recipient family name.
    pub family_name: String,
    /// Recipient given name.
    pub given_name: String,
    /// Postal code as entered.
    pub postal_code: String,
    /// Prefecture.
    pub prefecture: String,
    /// City / ward.
    pub city: String,
    /// Street address.
    pub address_line: String,
    /// Building, when given.
    pub building: String,
    /// Phone number as entered.
    pub phone: String,
}

impl DraftAddress {
    /// Builds the guest checkout flow this address places
    /// with: the address fields, the payment method the
    /// person picked on the approval page (marked chosen,
    /// so its COD fee applies), and every other flow field
    /// at its no-preference default — a draft carries no
    /// coupon, no points and no gift options, exactly like
    /// the agent settlement's flow.
    pub fn to_checkout_flow(&self, method: PaymentMethod) -> CheckoutFlow {
        CheckoutFlow {
            email: self.email.clone(),
            family_name: self.family_name.clone(),
            given_name: self.given_name.clone(),
            postal_code: self.postal_code.clone(),
            prefecture: self.prefecture.clone(),
            city: self.city.clone(),
            address_line: self.address_line.clone(),
            building: self.building.clone(),
            phone: self.phone.clone(),
            delivery_slot: "none".to_string(),
            payment_method: method,
            payment_method_chosen: true,
            ..CheckoutFlow::default()
        }
    }
}

/// Why creating a draft did not happen, in the tool's
/// terms. The handler maps each to its protocol answer.
#[derive(Debug)]
pub enum DraftError {
    /// The cart does not exist or belongs to another key:
    /// one variant for both, so the answer is no oracle.
    CartNotOwned,
    /// The cart holds no purchasable line.
    EmptyCart,
    /// A store failure; the detail stays in the server log.
    Internal(AppError),
}

impl From<AppError> for DraftError {
    fn from(error: AppError) -> Self {
        Self::Internal(error)
    }
}

impl From<sqlx::Error> for DraftError {
    fn from(error: sqlx::Error) -> Self {
        Self::Internal(AppError::Db(error))
    }
}

/// Creates a draft binding the caller's cart as it stands,
/// or returns the draft an earlier call with the same
/// idempotency key already created — a retried create is
/// the same draft, whatever state it has since reached
/// (pending, void, expired or consumed), so a client can
/// always recover the outcome of a create it is unsure
/// about instead of drafting twice.
///
/// The raw approval token comes back only with a fresh
/// creation: it is never stored (only its digest is), so
/// a replayed create cannot show the approval URL again.
///
/// # Errors
/// Returns [`DraftError::CartNotOwned`] when the cart is
/// not the caller's, [`DraftError::EmptyCart`] when it
/// holds nothing purchasable, and
/// [`DraftError::Internal`] on a store failure.
pub async fn create_draft(
    pool: &PgPool,
    api_key_id: i64,
    cart_id: &str,
    idempotency_key: &str,
    address: &DraftAddress,
) -> Result<(McpOrderDraftRow, Option<String>), DraftError> {
    if let Some(existing) =
        mcp_order_draft_repository::find_by_idempotency(pool, api_key_id, idempotency_key).await?
    {
        let existing = resolve_expiry(pool, existing).await?;
        return Ok((existing, None));
    }
    let owner_key = mcp_cart_service::owner_key_id(pool, cart_id).await?;
    if owner_key != Some(api_key_id) {
        return Err(DraftError::CartNotOwned);
    }
    let owner = CartOwner::Session(cart_id.to_string());
    let (lines, totals) = agent_approval_service::cart_state(pool, &owner).await?;
    if !lines.iter().any(|line| line.purchasable()) {
        return Err(DraftError::EmptyCart);
    }
    let fingerprint = agent_approval_service::fingerprint_for(&lines, &totals);
    let snapshot = serde_json::to_value(agent_approval_service::snapshot_for(&lines, &totals))
        .expect("the approval snapshot always serializes");
    let address_json = serde_json::to_value(address).expect("the draft address always serializes");
    let mut token_bytes = [0u8; APPROVAL_TOKEN_BYTES];
    rand::thread_rng().fill_bytes(&mut token_bytes);
    let token = hex_encode(&token_bytes);
    let token_hash = mcp_cart_service::hash_claim_token(&token);
    // The draft's window is the agent approvals' window:
    // one TTL for "a frozen cart awaits a person".
    let expires_at = Utc::now() + chrono::Duration::minutes(CONFIG.agent_approval_ttl_minutes);
    let id = match mcp_order_draft_repository::insert(
        pool,
        api_key_id,
        cart_id,
        idempotency_key,
        &address_json,
        &fingerprint,
        &snapshot,
        &token_hash,
        expires_at,
    )
    .await
    {
        Ok(id) => id,
        Err(e) => {
            // A racing create with the same idempotency key
            // won the insert: its draft is this call's
            // answer too. Anything else is a store failure.
            if is_unique_violation(&e)
                && let Some(existing) = mcp_order_draft_repository::find_by_idempotency(
                    pool,
                    api_key_id,
                    idempotency_key,
                )
                .await?
            {
                let existing = resolve_expiry(pool, existing).await?;
                return Ok((existing, None));
            }
            return Err(AppError::Db(e).into());
        }
    };
    let row = mcp_order_draft_repository::find_by_id_and_key(pool, id, api_key_id)
        .await?
        .expect("the draft was just inserted");
    Ok((row, Some(token)))
}

/// Whether a database error is a unique violation — the
/// idempotency race's signature.
fn is_unique_violation(error: &sqlx::Error) -> bool {
    matches!(error, sqlx::Error::Database(db) if db.is_unique_violation())
}

/// Reads one of the caller's drafts, resolving a pending
/// draft past its window into `expired` on the way (the
/// approvals' lazy expiry: no background job is needed).
/// Another key's draft answers exactly like a missing
/// one.
///
/// # Errors
/// Returns a store error when a read or the lazy
/// transition fails.
pub async fn get_draft(
    pool: &PgPool,
    api_key_id: i64,
    draft_id: i64,
) -> Result<Option<McpOrderDraftRow>, AppError> {
    let Some(row) = mcp_order_draft_repository::find_by_id_and_key(pool, draft_id, api_key_id)
        .await
        .map_err(|e| AppError::db("load mcp order draft", e))?
    else {
        return Ok(None);
    };
    Ok(Some(resolve_expiry(pool, row).await?))
}

/// Finds the draft an approval token belongs to — the
/// approval page's read — with the same lazy expiry as
/// [`get_draft`].
///
/// # Errors
/// Returns a store error when a read or the lazy
/// transition fails.
pub async fn find_for_page(
    pool: &PgPool,
    token: &str,
) -> Result<Option<McpOrderDraftRow>, AppError> {
    let token_hash = mcp_cart_service::hash_claim_token(token);
    let Some(row) = mcp_order_draft_repository::find_by_token_hash(pool, &token_hash)
        .await
        .map_err(|e| AppError::db("load mcp order draft by token", e))?
    else {
        return Ok(None);
    };
    Ok(Some(resolve_expiry(pool, row).await?))
}

/// Resolves a pending draft whose window has closed into
/// `expired`, in place. The conditional transition is the
/// guard: a draft consumed in the meantime stays consumed.
async fn resolve_expiry(
    pool: &PgPool,
    mut row: McpOrderDraftRow,
) -> Result<McpOrderDraftRow, AppError> {
    if row.status == mcp_order_draft_repository::STATUS_PENDING && row.expires_at <= Utc::now() {
        let moved = mcp_order_draft_repository::transition(
            pool,
            row.id,
            mcp_order_draft_repository::STATUS_PENDING,
            mcp_order_draft_repository::STATUS_EXPIRED,
        )
        .await
        .map_err(|e| AppError::db("expire mcp order draft", e))?;
        if moved {
            row.status = mcp_order_draft_repository::STATUS_EXPIRED.to_string();
        }
    }
    Ok(row)
}

/// The draft's stored display snapshot, when it parses —
/// a corrupt snapshot degrades the reads that use it,
/// never panics them.
pub fn snapshot_of(row: &McpOrderDraftRow) -> Option<ApprovalSnapshot> {
    serde_json::from_value(row.snapshot.clone()).ok()
}

/// The draft's delivery address, when it parses.
pub fn address_of(row: &McpOrderDraftRow) -> Option<DraftAddress> {
    serde_json::from_value(row.address.clone()).ok()
}

/// The lines binding a draft carries: its snapshot's
/// lines fingerprint — the same definition the agent
/// approvals re-check against, preserved across the pay
/// click's re-bind of the headline fingerprint.
fn lines_binding(row: &McpOrderDraftRow) -> String {
    snapshot_of(row)
        .map(|snapshot| snapshot.lines_fingerprint)
        .filter(|fingerprint| !fingerprint.is_empty())
        .unwrap_or_else(|| row.fingerprint.clone())
}

/// The pay gate, mirroring
/// [`agent_approval_service::verify_for_pay`]: the draft
/// must still be pending, inside its window, and still
/// bind the live cart's lines. A stale draft is resolved
/// (expired / voided) on the way out, so the approval
/// page can explain what happened. On success the live
/// lines and their no-method totals come back — the pay
/// click binds its final figures from them.
///
/// # Errors
/// Returns a [`PlacementRefusal`] carrying the
/// shopper-facing explanation for every refusal.
pub async fn verify_for_pay(
    pool: &PgPool,
    draft: &McpOrderDraftRow,
) -> Result<(Vec<CartLine>, CartTotals), PlacementRefusal> {
    let refuse = |message: &str| PlacementRefusal {
        message: message.to_string(),
    };
    match draft.status.as_str() {
        mcp_order_draft_repository::STATUS_PENDING => {}
        mcp_order_draft_repository::STATUS_CONSUMED => {
            return Err(refuse(
                "この注文はすでに完了しています。内容を確認して、もう一度はじめからお試しください。",
            ));
        }
        mcp_order_draft_repository::STATUS_EXPIRED => {
            return Err(PlacementRefusal {
                message: order_failed_message(&agent_approval_service::expired_notice()),
            });
        }
        _ => {
            return Err(PlacementRefusal {
                message: order_failed_message(agent_approval_service::CHANGED_NOTICE),
            });
        }
    }
    if draft.expires_at <= Utc::now() {
        let _ = mcp_order_draft_repository::transition(
            pool,
            draft.id,
            mcp_order_draft_repository::STATUS_PENDING,
            mcp_order_draft_repository::STATUS_EXPIRED,
        )
        .await;
        return Err(PlacementRefusal {
            message: order_failed_message(&agent_approval_service::expired_notice()),
        });
    }
    let owner = CartOwner::Session(draft.cart_id.clone());
    let (lines, totals) = agent_approval_service::cart_state(pool, &owner)
        .await
        .map_err(|_| refuse("カートの確認に失敗しました。しばらくしてから再度お試しください。"))?;
    if lines_binding(draft) != agent_approval_service::fingerprint_for(&lines, &totals) {
        let _ = mcp_order_draft_repository::transition(
            pool,
            draft.id,
            mcp_order_draft_repository::STATUS_PENDING,
            mcp_order_draft_repository::STATUS_VOID,
        )
        .await;
        return Err(PlacementRefusal {
            message: order_failed_message(agent_approval_service::CHANGED_NOTICE),
        });
    }
    Ok((lines, totals))
}

/// Binds a draft to the pay click's final state: the
/// headline fingerprint becomes the final hash (lines,
/// method-dependent totals, method, address) and the
/// snapshot the final snapshot, through the approval
/// machinery's own builders. `false` means the draft
/// stopped being pending first — a race the caller
/// treats as a refusal, because nothing may be placed on
/// a draft that is no longer live.
///
/// # Errors
/// Returns a store error when the write fails.
pub async fn bind_final(
    pool: &PgPool,
    draft: &McpOrderDraftRow,
    lines: &[CartLine],
    totals: &CartTotals,
    method: PaymentMethod,
    delivery: &CheckoutFlow,
) -> Result<bool, AppError> {
    let fingerprint = agent_approval_service::final_fingerprint(lines, totals, method, delivery);
    let snapshot = serde_json::to_value(agent_approval_service::final_snapshot(
        lines,
        totals,
        method,
        delivery,
        &lines_binding(draft),
    ))
    .expect("the approval snapshot always serializes");
    mcp_order_draft_repository::update_binding(pool, draft.id, &fingerprint, &snapshot)
        .await
        .map_err(|e| AppError::db("bind mcp order draft", e))
}

/// Consumes the draft for the order just placed. The
/// conditional update makes it exactly-once; `false`
/// means a race already spent it, which the caller logs
/// loudly — the order exists either way.
///
/// # Errors
/// Returns a store error when the write fails.
pub async fn consume(pool: &PgPool, draft_id: i64, order_id: i64) -> Result<bool, AppError> {
    mcp_order_draft_repository::consume(pool, draft_id, order_id)
        .await
        .map_err(|e| AppError::db("consume mcp order draft", e))
}

/// Reads one order's status for the key whose draft
/// placed it. Orders placed any other way — another
/// key's drafts, the storefront, the agent settlement —
/// are invisible here.
///
/// # Errors
/// Returns a store error when the read fails.
pub async fn order_status(
    pool: &PgPool,
    api_key_id: i64,
    order_id: i64,
) -> Result<Option<mcp_order_draft_repository::McpOrderStatusRow>, AppError> {
    mcp_order_draft_repository::find_order_status(pool, order_id, api_key_id)
        .await
        .map_err(|e| AppError::db("read mcp order status", e))
}

/// The draft's expiry as a timestamp value for callers
/// that render it.
pub fn expires_at(row: &McpOrderDraftRow) -> DateTime<Utc> {
    row.expires_at
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A complete, valid address in the checkout's shape.
    fn address() -> DraftAddress {
        DraftAddress {
            email: "buyer@example.com".to_string(),
            family_name: "検証".to_string(),
            given_name: "太郎".to_string(),
            postal_code: "184-0004".to_string(),
            prefecture: "東京都".to_string(),
            city: "小金井市".to_string(),
            address_line: "本町1-1-1".to_string(),
            building: String::new(),
            phone: "09012345678".to_string(),
        }
    }

    #[test]
    fn address_builds_a_guest_flow_with_the_method_chosen() {
        let flow = address().to_checkout_flow(PaymentMethod::Cod);
        assert_eq!(flow.email, "buyer@example.com");
        assert_eq!(flow.family_name, "検証");
        assert_eq!(flow.postal_code, "184-0004");
        assert_eq!(flow.payment_method, PaymentMethod::Cod);
        assert!(flow.payment_method_chosen);
        // A draft carries no coupon, points or gift state.
        assert!(flow.coupon_codes.is_empty());
        assert_eq!(flow.points_to_use, 0);
        assert!(flow.delivery_date.is_empty());
    }

    #[test]
    fn address_json_round_trips() {
        let json = serde_json::to_value(address()).expect("serializes");
        let back: DraftAddress = serde_json::from_value(json).expect("parses back");
        assert_eq!(back.email, "buyer@example.com");
        assert_eq!(back.phone, "09012345678");
    }
}
