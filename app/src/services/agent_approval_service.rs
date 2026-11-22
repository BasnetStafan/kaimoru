//! Agent checkout approvals and the in-chat settlement
//!.
//!
//! The human half of the agent checkout: the shopping agent
//! may fill a cart, but the order is settled by the cart
//! owner, in the chat thread itself, as a server-driven
//! step machine — agreement to the policy, the delivery
//! address, the payment method, a final confirmation, and
//! only then the pay click. Free-text chat never advances
//! the settlement; only the step buttons' form posts do,
//! and the state lives server-side (the approval record in
//! the database, the step in the session), never in page
//! state.
//!
//! The approval record is created when the settlement
//! starts and binds the cart's purchasable lines. Any later
//! change to the cart — an edit on the cart page, a quantity
//! stepper, a fresh agent run replacing its own lines —
//! breaks the binding, and the settlement dies instead of
//! ordering something unapproved. At the pay click the
//! record is re-bound to the final state — lines, final
//! totals (method-dependent: cash on delivery adds its
//! fee), the chosen payment method, the delivery address —
//! and consumed by the order that state places, through
//! the unchanged [`super::order_service::place_order`] and
//! the storefront's ordinary KOMOJU machinery. No KOMOJU
//! session is ever created without a consumed approval
//! behind it.
//!
//! Expiry and voiding are lazy: reads resolve a stale
//! pending record into its terminal state as they pass, so
//! no background job is needed for the approval TTL.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

use chrono::Utc;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::PgPool;

use crate::config::CONFIG;
use crate::error::AppError;
use crate::messages::order_failed_message;
use crate::models::agent_approval::{
    AgentApproval, ApprovalDelivery, ApprovalSnapshot, ApprovalSnapshotLine, STATUS_EXPIRED,
    STATUS_PENDING, STATUS_VOIDED,
};
use crate::models::{CartLine, PaymentMethod};
use crate::repositories::agent_approval_repository;
use crate::services::cart_service::{self, CartOwner, CartTotals};
use crate::services::order_service::CheckoutFlow;
use crate::services::tax_service;

/// Why a pay-time re-check refused: the Japanese explanation
/// the surfaces show the shopper. The refusal always leaves
/// the shopper able to start over — approve again from the
/// thread or check out by hand — so nobody is stranded.
pub struct PlacementRefusal {
    /// The shopper-facing explanation.
    pub message: String,
}

/// The settlement's current step. The machine moves one
/// step per button post; the order is policy, address,
/// payment method, confirmation, and the pay click is only
/// honoured from the confirmation step.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SettlementStage {
    /// The agreement policy is on screen, not yet agreed.
    Policy,
    /// The delivery address is being chosen or typed.
    Address,
    /// The payment method is being chosen.
    Payment,
    /// The final confirmation is on screen; pay is armed.
    Confirm,
}

/// The settlement in progress, held in the visitor's
/// session: which approval it walks, where it stands, and
/// the delivery state chosen so far (address first, then
/// the payment method it carries). The approval record in
/// the database is the authority — this is only the walk's
/// place in it, and any read that finds the record dead
/// discards the walk.
#[derive(Clone, Serialize, Deserialize)]
pub struct AgentSettlement {
    /// The approval record this settlement walks.
    pub approval_id: i64,
    /// The current step.
    pub stage: SettlementStage,
    /// The delivery state: set when the address step
    /// completes (the address fields plus, later, the
    /// chosen payment method in its method fields).
    pub delivery: Option<CheckoutFlow>,
    /// The address step's validation failures from its last
    /// submit, as (field, message) pairs, shown on the
    /// address step until the next address submit replaces
    /// or clears them.
    #[serde(default)]
    pub address_errors: Vec<(String, String)>,
}

/// The outcome of a settlement-approval creation attempt.
pub enum CreateSettlement {
    /// A fresh approval now binds the cart; the settlement
    /// can start on it.
    Created(AgentApproval),
    /// The per-owner hourly cap refused the attempt.
    Capped,
    /// The cart holds no purchasable agent-added line, so
    /// there is nothing for the agent path to approve.
    NoAgentLines,
}

/// The owner's latest approval resolved against the cart
/// the surfaces are about to show: the live record when one
/// still binds this cart, and the explanation when the
/// latest record died on this read (or an earlier one).
pub struct ResolvedLatest {
    /// The live pending approval binding the current cart.
    pub live: Option<AgentApproval>,
    /// Why the previous approval died, when it did.
    pub notice: Option<String>,
}

/// The explanation shown when an approval died of old age.
/// The TTL is configuration, so the minutes are read from it
/// rather than baked into the wording.
pub fn expired_notice() -> String {
    format!(
        "承認の有効期限（{}分）が切れたため、前の承認は無効になりました。内容を確認して、もう一度はじめからお試しください。",
        CONFIG.agent_approval_ttl_minutes
    )
}

/// The explanation shown when an approval died because the
/// cart moved under it.
pub const CHANGED_NOTICE: &str = "承認のあとにカートの内容が変わったため、前の承認は無効になりました。内容を確認して、もう一度はじめからお試しください。";

/// The cap refusal the approval surfaces show.
pub const CAP_MESSAGE: &str =
    "承認の回数が上限に達しました。しばらく時間をおいてからお試しください。";

/// Computes the fingerprint of a cart state: every
/// purchasable line's variant, quantity and unit price, in
/// variant order, plus the subtotal, shipping and grand
/// total of the given totals. Pure — the same inputs always
/// give the same fingerprint, which is what makes the
/// re-check a simple equality. With no-method totals this
/// is the approval's lines binding; the pay click's final
/// fingerprint builds on the same line section.
pub fn fingerprint_for(lines: &[CartLine], totals: &CartTotals) -> String {
    let mut canonical = lines_canonical(lines);
    canonical.push_str(&format!(
        "|{}|{}|{}",
        totals.subtotal_yen, totals.shipping_yen, totals.total_yen
    ));
    sha256_hex(&canonical)
}

/// The canonical line section shared by every fingerprint:
/// the purchasable lines in variant order, each as
/// variant, quantity and unit price.
fn lines_canonical(lines: &[CartLine]) -> String {
    let mut canonical = String::new();
    for line in purchasable_sorted(lines) {
        canonical.push_str(&format!(
            "v{}:{}:{},",
            line.variant_id, line.quantity, line.unit_price_yen
        ));
    }
    canonical
}

/// Hex SHA-256 of a canonical string.
fn sha256_hex(canonical: &str) -> String {
    let digest = Sha256::digest(canonical.as_bytes());
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// The fingerprint of a settlement's final state, computed
/// at the pay click: the lines, the FINAL totals (with the
/// chosen method's cash-on-delivery fee when it applies),
/// the chosen method, and the delivery address. This is
/// the hash the consumed approval carries — the proof of
/// exactly what was agreed, none of it re-derivable from a
/// cart that has since been emptied by placement.
pub fn final_fingerprint(
    lines: &[CartLine],
    totals: &CartTotals,
    method: PaymentMethod,
    delivery: &CheckoutFlow,
) -> String {
    let mut canonical = lines_canonical(lines);
    canonical.push_str(&format!(
        "|{}|{}|{}|{}|m:{}|a:{}|{}|{}|{}|{}|{}|{}|{}|{}",
        totals.subtotal_yen,
        totals.shipping_yen,
        totals.cod_fee_yen,
        totals.total_yen,
        method.slug(),
        delivery.email.trim(),
        delivery.family_name.trim(),
        delivery.given_name.trim(),
        delivery.postal_code.trim(),
        delivery.prefecture.trim(),
        delivery.city.trim(),
        delivery.address_line.trim(),
        delivery.building.trim(),
        delivery.phone.trim(),
    ));
    sha256_hex(&canonical)
}

/// Builds the display snapshot of a cart state: the lines
/// and totals an approval surface lists, in the same
/// canonical order the fingerprint uses. The snapshot
/// records its own lines fingerprint — the binding the
/// cart-change check compares against for the record's
/// whole life.
pub fn snapshot_for(lines: &[CartLine], totals: &CartTotals) -> ApprovalSnapshot {
    ApprovalSnapshot {
        lines: purchasable_sorted(lines)
            .iter()
            .map(|line| ApprovalSnapshotLine {
                variant_id: line.variant_id,
                product_id: line.product_id,
                name: line.name.clone(),
                variant_label: line.variant_label.clone(),
                unit_price_yen: line.unit_price_yen,
                quantity: i64::from(line.quantity),
                line_total_yen: line.line_total_yen(),
            })
            .collect(),
        subtotal_yen: totals.subtotal_yen,
        shipping_yen: totals.shipping_yen,
        total_yen: totals.total_yen,
        lines_fingerprint: fingerprint_for(lines, totals),
        payment_method: None,
        cod_fee_yen: totals.cod_fee_yen,
        delivery: None,
    }
}

/// The final snapshot the pay click binds: the creation
/// snapshot with the final totals, the chosen method, its
/// cash-on-delivery fee, and the delivery address. The
/// lines fingerprint is preserved from the creation
/// snapshot, so the cart-change check keeps its meaning
/// across the re-bind.
pub fn final_snapshot(
    lines: &[CartLine],
    totals: &CartTotals,
    method: PaymentMethod,
    delivery: &CheckoutFlow,
    lines_fingerprint: &str,
) -> ApprovalSnapshot {
    let mut snapshot = snapshot_for(lines, totals);
    snapshot.lines_fingerprint = lines_fingerprint.to_string();
    snapshot.payment_method = Some(method.slug().to_string());
    snapshot.delivery = Some(ApprovalDelivery {
        email: delivery.email.trim().to_string(),
        family_name: delivery.family_name.trim().to_string(),
        given_name: delivery.given_name.trim().to_string(),
        postal_code: delivery.postal_code.trim().to_string(),
        prefecture: delivery.prefecture.trim().to_string(),
        city: delivery.city.trim().to_string(),
        address_line: delivery.address_line.trim().to_string(),
        building: delivery.building.trim().to_string(),
        phone: delivery.phone.trim().to_string(),
    });
    snapshot
}

/// The purchasable lines in canonical (variant) order: the
/// one order the fingerprint and the snapshot both use, so
/// storage order in the cart can never split them.
fn purchasable_sorted(lines: &[CartLine]) -> Vec<&CartLine> {
    let mut purchasable: Vec<&CartLine> = lines.iter().filter(|line| line.purchasable()).collect();
    purchasable.sort_by_key(|line| line.variant_id);
    purchasable
}

/// Whether the purchasable lines include one the agent
/// added: the agent path exists only for carts the agent
/// actually stocked.
pub fn has_agent_line(lines: &[CartLine]) -> bool {
    lines
        .iter()
        .filter(|line| line.purchasable())
        .any(|line| line.added_by_agent)
}

/// Loads the owner's cart with the totals the agent path
/// binds before a method exists: no coupon, no payment
/// method (so no COD fee), the same computation at
/// settlement start and at every re-check.
pub async fn cart_state(
    pool: &PgPool,
    owner: &CartOwner,
) -> Result<(Vec<CartLine>, CartTotals), AppError> {
    let lines = cart_service::lines(pool, owner).await?;
    let tax = tax_service::params_today_or_fallback(pool).await;
    let fees = crate::services::shop_settings_service::load(pool)
        .await
        .fee_settings();
    let totals = cart_service::totals(&lines, 0, 0, None, tax, fees);
    Ok((lines, totals))
}

/// The lines binding a stored approval carries: its
/// snapshot's lines fingerprint, or — for a record written
/// before the snapshot learned to carry one — its headline
/// fingerprint, which was the lines binding then.
fn lines_binding(approval: &AgentApproval) -> String {
    approval
        .snapshot()
        .map(|snapshot| snapshot.lines_fingerprint)
        .filter(|fingerprint| !fingerprint.is_empty())
        .unwrap_or_else(|| approval.fingerprint.clone())
}

/// Resolves the owner's latest approval against the cart's
/// current lines fingerprint: past its TTL a pending record
/// becomes `expired`, off-binding it becomes `voided`, and
/// only an exact match stays live. Returns the live record
/// when there is one, and the notice that explains the
/// latest record's death when there is not.
pub async fn resolve_latest(
    pool: &PgPool,
    owner: &CartOwner,
    lines_fingerprint: &str,
) -> Result<ResolvedLatest, AppError> {
    let latest = agent_approval_repository::find_latest_for_owner(
        pool,
        owner.user_id(),
        owner.session_key(),
    )
    .await?;
    let Some(latest) = latest else {
        return Ok(ResolvedLatest {
            live: None,
            notice: None,
        });
    };
    match latest.status.as_str() {
        STATUS_PENDING => {
            if latest.is_expired(Utc::now()) {
                agent_approval_repository::transition(
                    pool,
                    latest.id,
                    STATUS_PENDING,
                    STATUS_EXPIRED,
                )
                .await?;
                return Ok(ResolvedLatest {
                    live: None,
                    notice: Some(expired_notice()),
                });
            }
            if lines_binding(&latest) != lines_fingerprint {
                agent_approval_repository::transition(
                    pool,
                    latest.id,
                    STATUS_PENDING,
                    STATUS_VOIDED,
                )
                .await?;
                return Ok(ResolvedLatest {
                    live: None,
                    notice: Some(CHANGED_NOTICE.to_string()),
                });
            }
            Ok(ResolvedLatest {
                live: Some(latest),
                notice: None,
            })
        }
        // A record voided by a cart change is exactly the
        // case the shopper needs explained; one the owner
        // revoked themselves (its binding still matches)
        // explains nothing.
        STATUS_VOIDED if lines_binding(&latest) != lines_fingerprint => Ok(ResolvedLatest {
            live: None,
            notice: Some(CHANGED_NOTICE.to_string()),
        }),
        STATUS_EXPIRED => Ok(ResolvedLatest {
            live: None,
            notice: Some(expired_notice()),
        }),
        _ => Ok(ResolvedLatest {
            live: None,
            notice: None,
        }),
    }
}

/// Creates the approval a new settlement walks on, bound to
/// the owner's current cart and superseding any pending
/// one. The caller supplies the rate-limit key (the same
/// session fingerprint the run limiter uses), so the cap
/// scopes exactly like the runs'.
pub async fn create_settlement_approval(
    pool: &PgPool,
    owner: &CartOwner,
    limit_key: &str,
) -> Result<CreateSettlement, AppError> {
    let (lines, totals) = cart_state(pool, owner).await?;
    if !has_agent_line(&lines) {
        return Ok(CreateSettlement::NoAgentLines);
    }
    if !try_begin_approval(limit_key) {
        return Ok(CreateSettlement::Capped);
    }
    let fingerprint = fingerprint_for(&lines, &totals);
    let snapshot = serde_json::to_value(snapshot_for(&lines, &totals))
        .expect("the approval snapshot always serializes");
    let expires_at = Utc::now() + chrono::Duration::minutes(CONFIG.agent_approval_ttl_minutes);
    let id = void_and_insert_approval(pool, owner, &fingerprint, &snapshot, expires_at).await?;
    let approval = agent_approval_repository::find_by_id(pool, id)
        .await?
        .expect("the approval was just inserted");
    Ok(CreateSettlement::Created(approval))
}

/// Voids the owner's pending approvals and inserts the new
/// one under the per-owner advisory lock: two creates racing
/// for the same owner would otherwise interleave void and
/// insert and leave TWO live pendings, where the settlement
/// flow assumes exactly one. The lock follows the agent
/// apply's pattern — a session lock on a dedicated
/// connection, closed rather than pooled when the unlock
/// fails.
async fn void_and_insert_approval(
    pool: &PgPool,
    owner: &CartOwner,
    fingerprint: &str,
    snapshot: &serde_json::Value,
    expires_at: chrono::DateTime<Utc>,
) -> Result<i64, AppError> {
    let lock_key = approval_lock_key(owner);
    let mut conn = pool
        .acquire()
        .await
        .map_err(|e| AppError::db("agent approval lock connection", e))?;
    sqlx::query("SELECT pg_advisory_lock(hashtextextended($1, 0))")
        .bind(&lock_key)
        .execute(&mut *conn)
        .await
        .map_err(|e| AppError::db("agent approval lock", e))?;
    let result = void_and_insert(pool, owner, fingerprint, snapshot, expires_at).await;
    // The unlock must not fail silently: the lock is
    // session-scoped, so a connection returned to the pool
    // still holding it would serialise this owner's creates
    // for that connection's whole life. On an unlock
    // failure the connection is closed instead of pooled.
    let unlocked = sqlx::query("SELECT pg_advisory_unlock(hashtextextended($1, 0))")
        .bind(&lock_key)
        .execute(&mut *conn)
        .await;
    if unlocked.is_err() {
        let _ = conn.close().await;
    }
    result
}

/// The locked unit itself: void the owner's pendings, then
/// insert the superseding approval.
async fn void_and_insert(
    pool: &PgPool,
    owner: &CartOwner,
    fingerprint: &str,
    snapshot: &serde_json::Value,
    expires_at: chrono::DateTime<Utc>,
) -> Result<i64, AppError> {
    agent_approval_repository::void_pending_for_owner(pool, owner.user_id(), owner.session_key())
        .await?;
    let id = agent_approval_repository::insert(
        pool,
        owner.user_id(),
        owner.session_key(),
        fingerprint,
        snapshot,
        expires_at,
    )
    .await?;
    Ok(id)
}

/// The advisory-lock key naming one cart owner's approval
/// creation: the two owner kinds get distinct namespaces,
/// mirroring the apply lock's key shape.
fn approval_lock_key(owner: &CartOwner) -> String {
    match owner {
        CartOwner::User(id) => format!("agent-approval:user:{id}"),
        CartOwner::Session(token) => format!("agent-approval:session:{token}"),
    }
}

/// Revokes the owner's pending approval (the settlement's
/// cancel): the escape hatch back to the ordinary state,
/// where the normal checkout — coupons and all — lives.
pub async fn void_all_own(pool: &PgPool, owner: &CartOwner) -> Result<(), AppError> {
    agent_approval_repository::void_pending_for_owner(pool, owner.user_id(), owner.session_key())
        .await?;
    Ok(())
}

/// The pay gate: re-checks the approval a settlement walks
/// against the live cart, at the last moment before the
/// final binding and [`super::order_service::place_order`].
/// The record must exist, belong to this owner, still be
/// pending, be inside its TTL, and still bind the live
/// cart's lines. A stale record is resolved (expired /
/// voided) on the way out, so the surfaces can explain what
/// happened.
pub async fn verify_for_pay(
    pool: &PgPool,
    owner: &CartOwner,
    approval_id: i64,
) -> Result<AgentApproval, PlacementRefusal> {
    let refuse = |message: &str| PlacementRefusal {
        message: message.to_string(),
    };
    let approval = agent_approval_repository::find_by_id(pool, approval_id)
        .await
        .map_err(|_| refuse("承認の確認に失敗しました。しばらくしてから再度お試しください。"))?
        .filter(|row| {
            row.user_id == owner.user_id() && row.session_id.as_deref() == owner.session_key()
        });
    let Some(approval) = approval else {
        return Err(refuse(
            "承認が確認できませんでした。内容を確認して、もう一度はじめからお試しください。",
        ));
    };
    match approval.status.as_str() {
        STATUS_PENDING => {}
        crate::models::agent_approval::STATUS_CONSUMED => {
            return Err(refuse(
                "この承認はすでに使用されています。内容を確認して、もう一度はじめからお試しください。",
            ));
        }
        STATUS_EXPIRED => {
            return Err(PlacementRefusal {
                message: order_failed_message(&expired_notice()),
            });
        }
        _ => {
            return Err(refuse(
                "この承認は無効になっています。内容を確認して、もう一度はじめからお試しください。",
            ));
        }
    }
    if approval.is_expired(Utc::now()) {
        let _ = agent_approval_repository::transition(
            pool,
            approval.id,
            STATUS_PENDING,
            STATUS_EXPIRED,
        )
        .await;
        return Err(PlacementRefusal {
            message: order_failed_message(&expired_notice()),
        });
    }
    let (lines, totals) = cart_state(pool, owner)
        .await
        .map_err(|_| refuse("カートの確認に失敗しました。しばらくしてから再度お試しください。"))?;
    if lines_binding(&approval) != fingerprint_for(&lines, &totals) {
        let _ =
            agent_approval_repository::transition(pool, approval.id, STATUS_PENDING, STATUS_VOIDED)
                .await;
        return Err(PlacementRefusal {
            message: order_failed_message(CHANGED_NOTICE),
        });
    }
    Ok(approval)
}

/// Binds an approval to the settlement's final state at
/// the pay click: the headline fingerprint becomes the
/// final hash and the snapshot the final snapshot.
/// `false` means the record stopped being pending first —
/// a race the caller treats as a refusal, because nothing
/// may be placed on an approval that is no longer live.
pub async fn bind_final(
    pool: &PgPool,
    approval: &AgentApproval,
    lines: &[CartLine],
    totals: &CartTotals,
    method: PaymentMethod,
    delivery: &CheckoutFlow,
) -> Result<bool, AppError> {
    let fingerprint = final_fingerprint(lines, totals, method, delivery);
    let snapshot = serde_json::to_value(final_snapshot(
        lines,
        totals,
        method,
        delivery,
        &lines_binding(approval),
    ))
    .expect("the approval snapshot always serializes");
    Ok(
        agent_approval_repository::update_binding(pool, approval.id, &fingerprint, &snapshot)
            .await?,
    )
}

/// Consumes the approval for the order just placed. The
/// conditional update makes it exactly-once; `false` means
/// a race already spent it, which the caller logs loudly —
/// the order exists either way.
pub async fn consume(pool: &PgPool, approval_id: i64, order_id: i64) -> Result<bool, AppError> {
    Ok(agent_approval_repository::consume(pool, approval_id, order_id).await?)
}

// ----------------------------------------------------------------
// The approval cap: the run limiter's pattern, keyed the same
// way, so one visitor's settlements per hour are bounded like
// their agent runs are.

/// Approval attempts are counted in a sliding window of
/// this length.
const APPROVAL_WINDOW: Duration = Duration::from_secs(3600);

/// Recent approval-attempt instants per limiter key.
static APPROVAL_ATTEMPTS: LazyLock<Mutex<HashMap<String, Vec<Instant>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Records an approval attempt for `key` and returns
/// whether it is within the configured hourly cap.
fn try_begin_approval(key: &str) -> bool {
    let cap = CONFIG.agent_checkout_max_per_hour.max(1) as usize;
    let now = Instant::now();
    let mut attempts = APPROVAL_ATTEMPTS
        .lock()
        .expect("approval limiter mutex poisoned");
    if attempts.len() > crate::util::rate_limit_knobs::SWEEP_THRESHOLD {
        attempts.retain(|_, times| {
            times.retain(|t| now.duration_since(*t) < APPROVAL_WINDOW);
            !times.is_empty()
        });
    }
    let times = attempts.entry(key.to_string()).or_default();
    times.retain(|t| now.duration_since(*t) < APPROVAL_WINDOW);
    if times.len() >= cap {
        return false;
    }
    times.push(now);
    true
}

/// The limiter key for one cart owner: the session
/// fingerprint of the owner's own token — the same truncated
/// hash the request log carries, never a raw session ID.
/// Defined here so every approval surface scopes the cap
/// identically.
pub fn limit_key(owner: &CartOwner) -> String {
    match owner {
        CartOwner::User(id) => crate::util::logging::session_fingerprint(&format!("user:{id}")),
        CartOwner::Session(token) => crate::util::logging::session_fingerprint(token),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A purchasable cart line with the given identity,
    /// quantity and unit price; every other field sits at
    /// the on-sale defaults the cart tests use.
    fn line(variant_id: i64, quantity: i32, unit_price_yen: i64) -> CartLine {
        CartLine {
            variant_id,
            product_id: variant_id,
            name: "テスト商品".to_string(),
            variant_label: String::new(),
            unit_price_yen,
            stock: 10,
            status: "public".to_string(),
            image_path: String::new(),
            quantity,
            added_by_agent: false,
            variant_enabled: true,
            publish_open: true,
            track_stock: true,
            allow_backorder: false,
            free_shipping: false,
            tax_class_code: "standard".to_string(),
            tax_class_name: "標準税率".to_string(),
            tax_rate_percent: 10,
            undeliverable_regions: String::new(),
        }
    }

    /// Totals over the given money fields; the tax
    /// decomposition is informational and stays empty.
    fn money(subtotal_yen: i64, shipping_yen: i64, cod_fee_yen: i64, total_yen: i64) -> CartTotals {
        CartTotals {
            subtotal_yen,
            discount_yen: 0,
            shipping_yen,
            shipping_discount_yen: 0,
            cod_fee_yen,
            total_yen,
            free_shipping_remaining_yen: 0,
            tax_breakdown: Vec::new(),
            tax_total_yen: 0,
        }
    }

    /// A delivery address for the final fingerprint.
    fn delivery() -> CheckoutFlow {
        CheckoutFlow {
            email: "test@example.com".to_string(),
            family_name: "山田".to_string(),
            given_name: "太郎".to_string(),
            postal_code: "1000001".to_string(),
            prefecture: "東京都".to_string(),
            city: "千代田区".to_string(),
            address_line: "千代田1-1-1".to_string(),
            building: String::new(),
            phone: "09012345678".to_string(),
            delivery_date: String::new(),
            delivery_slot: String::new(),
            coupon_codes: Vec::new(),
            payment_method: PaymentMethod::Cod,
            payment_method_chosen: true,
            save_address: false,
            noshi: String::new(),
            noshi_name: String::new(),
            gift_message: String::new(),
            points_to_use: 0,
        }
    }

    #[test]
    fn fingerprint_is_deterministic_hex() {
        let lines = [line(1, 1, 2_980), line(2, 2, 550)];
        let totals = money(4_080, 550, 0, 4_630);
        let first = fingerprint_for(&lines, &totals);
        assert_eq!(first, fingerprint_for(&lines, &totals));
        assert_eq!(first.len(), 64);
        assert!(
            first
                .chars()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
        );
    }

    #[test]
    fn fingerprint_ignores_line_storage_order() {
        let totals = money(4_080, 550, 0, 4_630);
        let forward = [line(1, 1, 2_980), line(2, 2, 550)];
        let reverse = [line(2, 2, 550), line(1, 1, 2_980)];
        assert_eq!(
            fingerprint_for(&forward, &totals),
            fingerprint_for(&reverse, &totals)
        );
    }

    #[test]
    fn fingerprint_changes_with_quantity_price_and_variant() {
        let totals = money(4_080, 550, 0, 4_630);
        let base = fingerprint_for(&[line(1, 1, 2_980)], &totals);
        assert_ne!(base, fingerprint_for(&[line(1, 2, 2_980)], &totals));
        assert_ne!(base, fingerprint_for(&[line(1, 1, 2_981)], &totals));
        assert_ne!(base, fingerprint_for(&[line(9, 1, 2_980)], &totals));
    }

    #[test]
    fn fingerprint_changes_when_a_line_is_added_or_removed() {
        let totals = money(4_080, 550, 0, 4_630);
        let one = fingerprint_for(&[line(1, 1, 2_980)], &totals);
        let two = fingerprint_for(&[line(1, 1, 2_980), line(2, 1, 550)], &totals);
        assert_ne!(one, two);
        assert_ne!(fingerprint_for(&[], &totals), one);
    }

    #[test]
    fn fingerprint_ignores_non_purchasable_lines() {
        let totals = money(4_080, 550, 0, 4_630);
        let base = fingerprint_for(&[line(1, 1, 2_980)], &totals);
        // A line taken off sale after carting (drafted
        // product, sold-out tracked variant) is not part of
        // what was approved, so it cannot void the approval.
        let mut drafted = line(2, 1, 550);
        drafted.status = "draft".to_string();
        assert_eq!(
            base,
            fingerprint_for(&[line(1, 1, 2_980), drafted], &totals)
        );
        let mut sold_out = line(3, 1, 550);
        sold_out.stock = 0;
        assert_eq!(
            base,
            fingerprint_for(&[line(1, 1, 2_980), sold_out], &totals)
        );
    }

    #[test]
    fn fingerprint_changes_with_each_bound_total() {
        let lines = [line(1, 1, 2_980)];
        let base = fingerprint_for(&lines, &money(2_980, 550, 0, 3_530));
        assert_ne!(base, fingerprint_for(&lines, &money(2_981, 550, 0, 3_530)));
        assert_ne!(base, fingerprint_for(&lines, &money(2_980, 0, 0, 3_530)));
        assert_ne!(base, fingerprint_for(&lines, &money(2_980, 550, 0, 3_531)));
    }

    #[test]
    fn final_fingerprint_binds_method_fee_and_address() {
        let lines = [line(1, 1, 2_980)];
        let totals = money(2_980, 550, 330, 3_860);
        let address = delivery();
        let base = final_fingerprint(&lines, &totals, PaymentMethod::Cod, &address);
        // The final fingerprint is a different binding from
        // the lines-only approval fingerprint.
        assert_ne!(base, fingerprint_for(&lines, &totals));
        // The chosen method is bound.
        assert_ne!(
            base,
            final_fingerprint(&lines, &totals, PaymentMethod::CreditCard, &address)
        );
        // The method's fee is bound through the totals.
        assert_ne!(
            base,
            final_fingerprint(
                &lines,
                &money(2_980, 550, 0, 3_530),
                PaymentMethod::Cod,
                &address
            )
        );
        // Every address field is bound.
        let mutations: Vec<CheckoutFlow> = vec![
            CheckoutFlow {
                email: "other@example.com".to_string(),
                ..delivery()
            },
            CheckoutFlow {
                family_name: "佐藤".to_string(),
                ..delivery()
            },
            CheckoutFlow {
                postal_code: "0600001".to_string(),
                ..delivery()
            },
            CheckoutFlow {
                address_line: "千代田2-2-2".to_string(),
                ..delivery()
            },
            CheckoutFlow {
                phone: "08011112222".to_string(),
                ..delivery()
            },
        ];
        for mutated in mutations {
            assert_ne!(
                base,
                final_fingerprint(&lines, &totals, PaymentMethod::Cod, &mutated)
            );
        }
    }

    #[test]
    fn final_fingerprint_trims_address_fields() {
        let lines = [line(1, 1, 2_980)];
        let totals = money(2_980, 550, 330, 3_860);
        let mut padded = delivery();
        padded.family_name = " 山田 ".to_string();
        padded.postal_code = " 1000001 ".to_string();
        assert_eq!(
            final_fingerprint(&lines, &totals, PaymentMethod::Cod, &delivery()),
            final_fingerprint(&lines, &totals, PaymentMethod::Cod, &padded)
        );
    }

    #[test]
    fn snapshot_lines_follow_canonical_order() {
        let lines = [line(2, 2, 550), line(1, 1, 2_980)];
        let totals = money(4_080, 550, 0, 4_630);
        let snapshot = snapshot_for(&lines, &totals);
        let ids: Vec<i64> = snapshot.lines.iter().map(|l| l.variant_id).collect();
        assert_eq!(ids, vec![1, 2]);
        assert_eq!(snapshot.lines[0].line_total_yen, 2_980);
        assert_eq!(snapshot.lines[1].line_total_yen, 1_100);
        assert_eq!(snapshot.lines_fingerprint, fingerprint_for(&lines, &totals));
        assert!(snapshot.payment_method.is_none());
        assert!(snapshot.delivery.is_none());
    }

    #[test]
    fn final_snapshot_preserves_the_lines_binding() {
        let lines = [line(1, 1, 2_980)];
        let totals = money(2_980, 550, 330, 3_860);
        let snapshot = final_snapshot(
            &lines,
            &totals,
            PaymentMethod::Cod,
            &delivery(),
            "bound-at-creation",
        );
        // The lines fingerprint is the creation binding, not
        // a recomputation: the cart-change check must keep
        // its meaning across the pay-click re-bind.
        assert_eq!(snapshot.lines_fingerprint, "bound-at-creation");
        assert_eq!(snapshot.payment_method.as_deref(), Some("cod"));
        let delivery = snapshot.delivery.expect("delivery bound");
        assert_eq!(delivery.postal_code, "1000001");
        assert_eq!(snapshot.cod_fee_yen, 330);
    }

    #[test]
    fn has_agent_line_requires_a_purchasable_agent_line() {
        let mut agent_line = line(1, 1, 2_980);
        agent_line.added_by_agent = true;
        assert!(has_agent_line(&[agent_line.clone()]));
        assert!(!has_agent_line(&[line(1, 1, 2_980)]));
        // An agent line taken off sale no longer counts:
        // the agent path exists only for carts it can sell.
        agent_line.status = "draft".to_string();
        assert!(!has_agent_line(&[agent_line]));
    }

    #[test]
    fn approval_expires_at_its_ttl_boundary() {
        let now = Utc::now();
        let approval = |expires_at: chrono::DateTime<Utc>| AgentApproval {
            id: 1,
            user_id: Some(8),
            session_id: None,
            fingerprint: "x".to_string(),
            snapshot: serde_json::json!(null),
            status: "pending".to_string(),
            consumed_order_id: None,
            created_at: now,
            expires_at,
            updated_at: now,
        };
        // Expiring exactly now is expired; one second of
        // life left is not.
        assert!(approval(now).is_expired(now));
        assert!(!approval(now + chrono::Duration::seconds(1)).is_expired(now));
        assert!(approval(now - chrono::Duration::seconds(1)).is_expired(now));
    }
}
