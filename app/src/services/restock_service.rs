//! Back-in-stock notification service: registration from
//! the product page and the background sweep that mails
//! each subscriber once when their combination's stock
//! returns.
//!
//! Registration only records the subscription. No
//! stock-changing path (admin edits, cancellations, CSV
//! imports) carries notification logic: the sweep is the
//! single place that finds pending subscriptions whose
//! variant is back in stock, claims each row, sends, and
//! marks it sent — so a restock is never missed because it
//! arrived through an unusual door, and never mailed twice
//! because two doors fired.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

use sqlx::PgPool;
use tokio::sync::watch::Receiver;

use crate::config::CONFIG;
use crate::error::AppError;
use crate::mail;
use crate::messages::EMAIL_FORMAT_MESSAGE;
use crate::models::restock::{STATUS_CANCELLED, STATUS_SENT};
use crate::pii::{blind_index_email, decrypt_field_checked, encrypt_field};
use crate::repositories::{product_repository, restock_repository, variant_repository};
use crate::util::rate_limit_knobs::SWEEP_THRESHOLD;
use crate::validation::{FieldError, is_valid_email};

/// Subscriptions mailed per sweep pass.
const BATCH_SIZE: i64 = 50;
/// Minutes a claimed row may sit in `sending` before a later
/// pass treats its worker as crashed and reclaims it.
const STALE_CLAIM_MINUTES: i64 = 15;
/// Registrations allowed per IP inside the window.
const SUBMIT_LIMIT: usize = 10;
/// The registration window.
const SUBMIT_WINDOW: Duration = Duration::from_secs(3600);

/// Registration times per IP, for the rate limit. In-memory
/// like the inquiry limiter: disposable by design.
static SUBMIT_LOG: LazyLock<Mutex<HashMap<String, Vec<Instant>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// The one rate-limit message, shared wording with the
/// inquiry flow's.
pub const RATE_LIMIT_MESSAGE: &str =
    "短時間に多くの登録が送信されました。しばらく時間をおいてから再度お試しください。";

/// Whether one more registration from this IP is allowed,
/// recording it when it is.
fn ip_allowed(ip: &str) -> bool {
    let now = Instant::now();
    let mut log = SUBMIT_LOG
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if log.len() >= SWEEP_THRESHOLD && !log.contains_key(ip) {
        log.retain(|_, times| {
            times.retain(|t| now.duration_since(*t) < SUBMIT_WINDOW);
            !times.is_empty()
        });
    }
    let entry = log.entry(ip.to_string()).or_default();
    entry.retain(|t| now.duration_since(*t) < SUBMIT_WINDOW);
    if entry.len() >= SUBMIT_LIMIT {
        return false;
    }
    entry.push(now);
    true
}

/// How a registration ended.
#[derive(Debug, PartialEq, Eq)]
pub enum SubscribeOutcome {
    /// A new (or renewed) subscription is now pending.
    Subscribed,
    /// The pair was already waiting for this restock.
    AlreadyRegistered,
    /// The combination is not registrable: gone, disabled,
    /// back in stock already, backorderable, a preorder, or
    /// on an untracked / unlisted product.
    NotAvailable,
    /// The per-IP registration limit was hit.
    RateLimited,
}

/// Validates the registration's email field: the only
/// shopper-typed value, checked before anything is stored.
pub fn validate_email(email: &str) -> Vec<FieldError> {
    if is_valid_email(email.trim()) {
        Vec::new()
    } else {
        vec![FieldError::new("email", EMAIL_FORMAT_MESSAGE)]
    }
}

/// Registers one (variant, email) subscription.
///
/// # Errors
///
/// Returns [`AppError`] on database failure; every refusal
/// a shopper can cause is a [`SubscribeOutcome`], not an
/// error.
pub async fn subscribe(
    pool: &PgPool,
    ip: &str,
    variant_id: i64,
    email: &str,
    user_id: Option<i64>,
) -> Result<SubscribeOutcome, AppError> {
    if !ip_allowed(ip) {
        return Ok(SubscribeOutcome::RateLimited);
    }
    // Registrable means: the combination exists, is enabled
    // and is sold out, on a listed product that tracks stock
    // without backorders or preorders — the same conditions
    // under which the product page offered the form.
    let Some(variant) = variant_repository::find_variant(pool, variant_id).await? else {
        return Ok(SubscribeOutcome::NotAvailable);
    };
    if !variant.is_enabled || variant.stock > 0 {
        return Ok(SubscribeOutcome::NotAvailable);
    }
    // The repository's read is public-gated (listed and
    // inside its publication window), so a missing product
    // here means not registrable, exactly like a draft.
    let Some(product) = product_repository::find_by_id(pool, variant.product_id).await? else {
        return Ok(SubscribeOutcome::NotAvailable);
    };
    if !product.track_stock || product.allow_backorder || product.is_preorder {
        return Ok(SubscribeOutcome::NotAvailable);
    }
    let email = email.trim();
    let bidx = blind_index_email(email);
    let inserted =
        restock_repository::insert_ignore(pool, variant_id, user_id, &encrypt_field(email), &bidx)
            .await?;
    if inserted {
        tracing::info!(action = "restock.subscribe", variant_id);
        return Ok(SubscribeOutcome::Subscribed);
    }
    match restock_repository::find_status(pool, variant_id, &bidx).await? {
        Some(status) if status == STATUS_SENT || status == STATUS_CANCELLED => {
            // A finished subscription renews: the shopper is
            // waiting for the *next* sell-out's restock.
            restock_repository::reset_to_pending(pool, variant_id, &bidx).await?;
            Ok(SubscribeOutcome::Subscribed)
        }
        Some(_) => Ok(SubscribeOutcome::AlreadyRegistered),
        None => Ok(SubscribeOutcome::NotAvailable),
    }
}

/// Returns one product's (waiting, sent) subscription
/// counts, for the admin product form's read-only line.
///
/// # Errors
///
/// Returns [`AppError`] on database failure.
pub async fn counts_for_product(pool: &PgPool, product_id: i64) -> Result<(i64, i64), AppError> {
    Ok(restock_repository::counts_for_product(pool, product_id).await?)
}

/// Starts the restock sweep under the server's supervision:
/// a pass on the mail worker's interval, draining on the
/// shared shutdown watch.
pub fn start_worker(pool: PgPool, mut shutdown: Receiver<bool>) -> tokio::task::JoinHandle<()> {
    let interval_secs = CONFIG.mail_worker_interval_secs.max(5);
    let handle = tokio::spawn(async move {
        let mut ticker = tokio::time::interval(std::time::Duration::from_secs(interval_secs));
        loop {
            tokio::select! {
            _ = ticker.tick() => {}
            _ = shutdown.changed() => break,
            }
            if let Err(e) = run_once(&pool).await {
                tracing::error!("restock sweep pass failed: {e}");
            }
        }
        tracing::info!("restock sweep stopped");
    });
    tracing::info!(interval_secs, "restock sweep started");
    handle
}

/// Runs one sweep pass: reclaims stale claims, then for each
/// due subscription claims the row, sends the mail and marks
/// it sent. A failed send returns the row to pending for the
/// next pass instead of losing the notification.
///
/// # Errors
///
/// Returns [`AppError`] when the due-batch read fails; a
/// single row's failure is logged and never fails the pass.
pub async fn run_once(pool: &PgPool) -> Result<(), AppError> {
    match restock_repository::reclaim_stale_claims(pool, STALE_CLAIM_MINUTES).await {
        Ok(reclaimed) if reclaimed > 0 => {
            tracing::warn!(reclaimed, "restock sweep reclaimed stale claims");
        }
        Ok(_) => {}
        Err(e) => {
            tracing::error!("restock stale-claim reclaim failed: {e}");
        }
    }
    let due = restock_repository::due_batch(pool, BATCH_SIZE).await?;
    for item in due {
        if !restock_repository::claim(pool, item.id).await? {
            continue;
        }
        // The address is decrypted only here, at send time,
        // and is never logged. The checked variant, never the
        // panicking one: one undecryptable row must not kill
        // the whole sweep. Legacy plaintext (written before
        // encryption) is used as-is, exactly as the panicking
        // variant's passthrough treated it.
        let email = if crate::pii::is_encrypted_value(&item.email_encrypted) {
            match decrypt_field_checked(&item.email_encrypted) {
                Some(email) => email,
                None => {
                    tracing::error!(
                        subscription_id = item.id,
                        "restock subscription email undecryptable; claim reverted"
                    );
                    if let Err(e) = restock_repository::revert_claim(pool, item.id).await {
                        tracing::error!(
                            subscription_id = item.id,
                            "restock claim revert failed: {e}"
                        );
                    }
                    continue;
                }
            }
        } else {
            item.email_encrypted.clone()
        };
        // The mail error is flattened to a string at once:
        // the boxed transport error is not `Send`, and the
        // pass runs on a spawned task.
        let sent = mail::send_restock_notification(
            pool,
            &email,
            item.product_id,
            &item.product_name,
            &item.variant_label,
        )
        .await
        .map_err(|e| e.to_string());
        match sent {
            Ok(()) => {
                restock_repository::mark_sent(pool, item.id).await?;
                tracing::info!(action = "restock.notify", subscription_id = item.id);
            }
            Err(e) => {
                tracing::error!(subscription_id = item.id, "restock mail failed: {e}");
                if let Err(revert_err) = restock_repository::revert_claim(pool, item.id).await {
                    tracing::error!(
                        subscription_id = item.id,
                        "restock claim revert failed: {revert_err}"
                    );
                }
            }
        }
    }
    Ok(())
}
