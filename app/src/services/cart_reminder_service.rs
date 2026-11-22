//! Cart-abandonment reminders: one mail per member
//! cart state, from a supervised background worker.
//!
//! A cart qualifies when it is a MEMBER cart — or a guest
//! cart whose checkout email belongs to a member account
//! whose subscription is `subscribed` (see `remind_guest`
//! for the consent rule) — untouched for the
//! configured delay, every line still purchasable and in
//! stock, and no order was placed after the cart's last
//! touch. Opt-outs are absolute: a member whose subscription
//! row says `unsubscribed` is never reminded. The cart's
//! fingerprint (its variant/quantity multiset) is recorded
//! on send, so the same cart state is never reminded twice;
//! changing the cart makes a new state that can be reminded
//! once more after the delay.
//!
//! Multi-instance safety follows the house rule: the whole
//! pass runs under a pg advisory lock, so two `worker`
//! replicas never double-send.

use sha2::Digest;
use sqlx::PgPool;
use tokio::sync::watch::Receiver;

use crate::config::CONFIG;
use crate::error::AppError;
use crate::mail;
use crate::repositories::{cart_reminder_repository, user_repository};

/// Advisory lock key for the reminder pass (741_003; the
/// payment sweep owns 741_001, low-stock alerts 741_002).
const REMINDER_LOCK_KEY: i64 = 741_003;

/// The cart fingerprint: a hash of its (variant, quantity)
/// lines in stable order.
fn fingerprint(lines: &[cart_reminder_repository::ReminderLine]) -> String {
    let mut parts: Vec<String> = lines
        .iter()
        .map(|l| format!("{}:{}", l.variant_id, l.quantity))
        .collect();
    parts.sort();
    let mut hasher = sha2::Sha256::new();
    hasher.update(parts.join(",").as_bytes());
    format!("{:x}", hasher.finalize())
}

/// Whether every line can still be bought as-is.
fn all_available(lines: &[cart_reminder_repository::ReminderLine]) -> bool {
    !lines.is_empty()
        && lines.iter().all(|l| {
            l.variant_enabled
                && l.product_status == "public"
                && (!l.track_stock || l.variant_stock >= l.quantity)
        })
}

/// Runs one reminder pass; see the module doc for the rules.
pub async fn run_once(pool: &PgPool) -> Result<(), AppError> {
    // One pass at a time across replicas: hold the advisory
    // lock on a dedicated connection for the pass's life.
    let mut lock_conn = pool
        .acquire()
        .await
        .map_err(|e| AppError::db("reminder lock acquire", e))?;
    let locked: bool = sqlx::query_scalar("SELECT pg_try_advisory_lock($1)")
        .bind(REMINDER_LOCK_KEY)
        .fetch_one(&mut *lock_conn)
        .await
        .map_err(|e| AppError::db("reminder try lock", e))?;
    if !locked {
        return Ok(());
    }
    let result = pass(pool).await;
    // A failed unlock leaves the session-scoped lock on the
    // connection: close it rather than returning it to the
    // pool, where it would stall later passes (the pattern
    // `agent_run_service::apply` established for its lock).
    if let Err(e) = sqlx::query("SELECT pg_advisory_unlock($1)")
        .bind(REMINDER_LOCK_KEY)
        .execute(&mut *lock_conn)
        .await
    {
        tracing::error!("reminder unlock failed; closing the lock connection: {e}");
        let _ = lock_conn.close().await;
    }
    result
}

/// The pass body, under the caller's advisory lock.
async fn pass(pool: &PgPool) -> Result<(), AppError> {
    let delay = CONFIG.cart_reminder_delay_hours.max(1);
    // Retention first: contacts whose cart is gone can
    // never qualify again, so the address is dropped
    // before any sending is considered.
    cart_reminder_repository::delete_orphan_guest_contacts(pool).await?;
    for cart in cart_reminder_repository::idle_member_carts(pool, delay).await? {
        // One member's failure never fails the pass.
        if let Err(e) = remind_one(pool, cart.user_id, cart.last_touch).await {
            tracing::warn!(user_id = cart.user_id, "cart reminder failed: {e}");
        }
    }
    for cart in cart_reminder_repository::idle_guest_carts(pool, delay).await? {
        // One guest's failure never fails the pass.
        if let Err(e) = remind_guest(pool, cart).await {
            tracing::warn!("guest cart reminder failed: {e}");
        }
    }
    Ok(())
}

/// Captures the email a guest typed at checkout against
/// their cart's session owner (C3 completion), PII-
/// encrypted exactly like an order's snapshot. Best
/// effort by design: the caller ignores failures, so a
/// capture problem can never block a checkout.
pub async fn capture_guest_contact(pool: &PgPool, session_id: &str, email: &str) {
    let email = email.trim().to_lowercase();
    if email.is_empty() {
        return;
    }
    let stored = crate::pii::encrypt_field(&email);
    let bidx = crate::pii::blind_index_email(&email);
    if let Err(e) =
        cart_reminder_repository::upsert_guest_contact(pool, session_id, &stored, &bidx).await
    {
        tracing::warn!("guest contact capture failed: {e}");
    }
}

/// Evaluates and possibly reminds one idle guest cart.
///
/// The consent rule (the strict reading of the shop's
/// member-keyed consent model): a guest address receives
/// a reminder ONLY when it belongs to a member account
/// whose `email_subscriptions` status is `subscribed`.
/// The shop's consent artefact is the subscription row;
/// a checkout email alone is order communication, not
/// marketing consent, so an address with no subscribed
/// account behind it is never mailed — and its stored
/// copy is dropped, since it can never qualify. The
/// reminder carries that account's tokenised unsubscribe
/// link, and the stored email is cleared at claim time.
async fn remind_guest(
    pool: &PgPool,
    cart: cart_reminder_repository::GuestIdleCart,
) -> Result<(), AppError> {
    let email = crate::pii::decrypt_field(&cart.email);
    let bidx = crate::pii::blind_index_email(&email);
    let Some(member_id) = cart_reminder_repository::member_id_by_bidx(pool, &bidx).await? else {
        // No account behind the address: it can never
        // present consent, so the contact is dropped.
        cart_reminder_repository::delete_guest_contact(pool, &cart.session_id).await?;
        return Ok(());
    };
    if cart_reminder_repository::subscription_status(pool, member_id)
        .await?
        .as_deref()
        != Some("subscribed")
    {
        // Unsubscribed (or never consented): never mailed.
        cart_reminder_repository::delete_guest_contact(pool, &cart.session_id).await?;
        return Ok(());
    }
    let Some(user) = user_repository::find_by_id(pool, member_id)
        .await
        .map_err(AppError::from)?
    else {
        return Ok(());
    };
    if user.withdrawn_at.is_some() {
        cart_reminder_repository::delete_guest_contact(pool, &cart.session_id).await?;
        return Ok(());
    }
    // An order after the cart's last touch means this
    // cart is not the abandoned one.
    if let Some(ordered_at) = cart_reminder_repository::last_order_at(pool, member_id).await?
        && ordered_at > cart.last_touch
    {
        return Ok(());
    }
    let lines = cart_reminder_repository::guest_cart_lines(pool, &cart.session_id).await?;
    if !all_available(&lines) {
        return Ok(());
    }
    let fp = fingerprint(&lines);
    if cart.cart_fingerprint.as_deref() == Some(fp.as_str()) {
        return Ok(());
    }
    let print_lines: Vec<(String, i32)> = lines
        .iter()
        .map(|l| (l.product_name.clone(), l.quantity))
        .collect();
    let token = cart_reminder_repository::subscription_token(pool, member_id).await?;
    // Claim BEFORE the send (same rule as members): the
    // fingerprint is recorded and the stored email is
    // cleared in the same write, so the address is held
    // for exactly one use.
    cart_reminder_repository::claim_guest_reminder(pool, &cart.session_id, &fp).await?;
    match mail::send_cart_reminder(pool, &email, &print_lines, token.as_deref()).await {
        Ok(()) => tracing::info!(member_id, "guest cart reminder sent"),
        Err(e) => tracing::warn!(member_id, "guest cart reminder send failed: {e}"),
    }
    Ok(())
}

/// Evaluates and possibly reminds one member's cart.
async fn remind_one(
    pool: &PgPool,
    user_id: i64,
    last_touch: chrono::DateTime<chrono::Utc>,
) -> Result<(), AppError> {
    // Opt-out is absolute (see the module doc).
    if cart_reminder_repository::subscription_status(pool, user_id)
        .await?
        .as_deref()
        == Some("unsubscribed")
    {
        return Ok(());
    }
    // An order after the cart's last touch means this cart
    // is not the abandoned one (bought elsewhere meanwhile).
    if let Some(ordered_at) = cart_reminder_repository::last_order_at(pool, user_id).await?
        && ordered_at > last_touch
    {
        return Ok(());
    }
    let lines = cart_reminder_repository::cart_lines(pool, user_id).await?;
    if !all_available(&lines) {
        return Ok(());
    }
    let print_lines: Vec<(String, i32)> = lines
        .iter()
        .map(|l| (l.product_name.clone(), l.quantity))
        .collect();
    let fp = fingerprint(&lines);
    if cart_reminder_repository::reminded_fingerprint(pool, user_id)
        .await?
        .as_deref()
        == Some(fp.as_str())
    {
        return Ok(());
    }
    let Some(user) = user_repository::find_by_id(pool, user_id)
        .await
        .map_err(AppError::from)?
    else {
        return Ok(());
    };
    if user.withdrawn_at.is_some() {
        return Ok(());
    }
    // Record BEFORE the send attempt completes the claim:
    // a crashed send may lose one reminder, but the member
    // can never receive two for the same cart state (the
    // fingerprint row is the claim).
    cart_reminder_repository::record_reminder(pool, user_id, &fp).await?;
    let token = cart_reminder_repository::subscription_token(pool, user_id).await?;
    match mail::send_cart_reminder(pool, &user.email, &print_lines, token.as_deref()).await {
        Ok(()) => tracing::info!(user_id, "cart reminder sent"),
        // The fingerprint stays recorded (see above): a lost
        // reminder is acceptable, a duplicate is not.
        Err(e) => tracing::warn!(user_id, "cart reminder send failed: {e}"),
    }
    Ok(())
}

/// Starts the reminder worker under the server's
/// supervision, on the typed-config interval.
pub fn start_worker(pool: PgPool, mut shutdown: Receiver<bool>) -> tokio::task::JoinHandle<()> {
    let interval_secs = CONFIG.cart_reminder_interval_secs.max(30);
    let handle = tokio::spawn(async move {
        let mut ticker = tokio::time::interval(std::time::Duration::from_secs(interval_secs));
        loop {
            tokio::select! {
            _ = ticker.tick() => {}
            _ = shutdown.changed() => break,
            }
            if let Err(e) = run_once(&pool).await {
                tracing::error!("cart reminder pass failed: {e}");
            }
        }
        tracing::info!("cart reminder worker stopped");
    });
    tracing::info!(interval_secs, "cart reminder worker started");
    handle
}
