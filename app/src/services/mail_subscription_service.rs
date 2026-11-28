//! Marketing-mail consent.
//!
//! The 特定電子メール法 rule lives here: marketing mail
//! requires a prior opt-in, the grant is stored with its
//! time and source, and every state change appends a
//! consent-history row. The surfaces (sign-up, MyPage, the
//! unsubscribe link) call into this service; the campaign
//! worker reads the same records before every send. A
//! member with no subscription row is unsubscribed.

use rand::RngCore;
use sqlx::PgPool;

use crate::error::AppError;
use crate::models::mail::EmailSubscription;
use crate::repositories::mail_repository;
use crate::repositories::user_repository;

/// Consent surface name recorded for the sign-up checkbox.
pub const SOURCE_SIGNUP: &str = "signup";
/// Consent surface name recorded for the MyPage toggle.
pub const SOURCE_MYPAGE: &str = "mypage";
/// Consent surface name recorded for the unsubscribe link.
pub const SOURCE_UNSUBSCRIBE_LINK: &str = "unsubscribe_link";

/// Generates one unsubscribe token: 32 random bytes as hex.
/// The token is the member's only no-login credential for
/// the unsubscribe link; it is printed solely in marketing
/// mail addressed to that member and never logged.
fn new_unsubscribe_token() -> String {
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Records the sign-up checkbox: an opted-in member starts
/// subscribed with their consent stamped; an unchecked
/// member gets the row in its unsubscribed state, so the
/// MyPage toggle and any future unsubscribe link always
/// have a record to work on. Called after the account
/// exists; the caller logs a failure rather than failing
/// the registration — consent bookkeeping must never block
/// a sign-up, and the failure direction is safe (no row
/// reads as unsubscribed).
pub async fn record_signup(pool: &PgPool, user_id: i64, opted_in: bool) -> Result<(), AppError> {
    apply_preference(pool, user_id, opted_in, SOURCE_SIGNUP).await
}

/// Syncs the consent record inside the caller's transaction:
/// the profile edit writes the `users` row in the same unit,
/// so a preference change and its consent record commit or
/// fail together — a silently lost opt-out would keep mail
/// flowing to a member who asked it to stop.
pub async fn sync_preference_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    user_id: i64,
    wants: bool,
) -> Result<(), AppError> {
    apply_preference_tx(tx, user_id, wants, SOURCE_MYPAGE).await
}

/// Applies one consent state: creates the row on first
/// contact, flips it on a real change, and appends the
/// consent-history row for actual transitions only — a
/// form saved without touching the checkbox writes nothing.
async fn apply_preference(
    pool: &PgPool,
    user_id: i64,
    wants: bool,
    source: &str,
) -> Result<(), AppError> {
    let mut tx = pool.begin().await.map_err(|e| AppError::db("begin", e))?;
    apply_preference_tx(&mut tx, user_id, wants, source).await?;
    tx.commit().await.map_err(|e| AppError::db("commit", e))?;
    Ok(())
}

/// The body of [`apply_preference`] on the caller's
/// transaction: reads the current row under its lock, flips
/// it on a real change, and appends the history row.
async fn apply_preference_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    user_id: i64,
    wants: bool,
    source: &str,
) -> Result<(), AppError> {
    let desired = if wants { "subscribed" } else { "unsubscribed" };
    let current = mail_repository::find_subscription_tx(tx, user_id).await?;
    let changed = current_status_changed(&current, desired);
    match current {
        None => {
            let token = new_unsubscribe_token();
            let consented_at = wants.then(chrono::Utc::now);
            mail_repository::insert_subscription(
                tx,
                user_id,
                desired,
                consented_at,
                source,
                &token,
            )
            .await?;
            if wants {
                mail_repository::insert_consent_log(tx, user_id, "opt_in", source).await?;
            }
        }
        Some(row) if row.status != desired => {
            let consented_at = wants.then(chrono::Utc::now);
            mail_repository::update_subscription(tx, user_id, desired, consented_at, source)
                .await?;
            let action = if wants { "opt_in" } else { "opt_out" };
            mail_repository::insert_consent_log(tx, user_id, action, source).await?;
        }
        Some(_) => {}
    }
    if changed {
        tracing::info!(
            user_id,
            action = if wants { "mail_opt_in" } else { "mail_opt_out" },
            source,
            "mail consent changed"
        );
    }
    Ok(())
}

/// Whether a transition actually happened (for the log
/// line): a fresh opt-in, or a stored state that differs.
fn current_status_changed(current: &Option<EmailSubscription>, desired: &str) -> bool {
    match current {
        None => desired == "subscribed",
        Some(row) => row.status != desired,
    }
}

/// Returns the subscription row behind one unsubscribe
/// token, if the token exists.
pub async fn find_by_token(
    pool: &PgPool,
    token: &str,
) -> Result<Option<EmailSubscription>, AppError> {
    mail_repository::find_subscription_by_token(pool, token).await
}

/// Applies an unsubscribe from the no-login link: flips the
/// subscription, appends the consent-history row, and
/// clears the mirrored `users.mail_magazine` flag so the
/// MyPage checkbox cannot show a stale "subscribed". The
/// row is read inside the transaction under its lock, so
/// the state flipped is the state that commits.
/// Idempotent: an already-unsubscribed token writes no
/// second history row. Returns whether the opt-out
/// transitioned (`Some(true)`), was already in effect
/// (`Some(false)`), or the token is unknown (`None`).
pub async fn unsubscribe_by_token(pool: &PgPool, token: &str) -> Result<Option<bool>, AppError> {
    let mut tx = pool.begin().await.map_err(|e| AppError::db("begin", e))?;
    let Some(subscription) = mail_repository::find_subscription_by_token_tx(&mut tx, token).await?
    else {
        return Ok(None);
    };
    let user_id = subscription.user_id;
    let transitioned = subscription.is_subscribed();
    if transitioned {
        mail_repository::update_subscription(
            &mut tx,
            user_id,
            "unsubscribed",
            None,
            SOURCE_UNSUBSCRIBE_LINK,
        )
        .await?;
        mail_repository::insert_consent_log(&mut tx, user_id, "opt_out", SOURCE_UNSUBSCRIBE_LINK)
            .await?;
        tracing::info!(
            user_id,
            action = "mail_opt_out",
            source = SOURCE_UNSUBSCRIBE_LINK,
            "mail consent changed"
        );
    }
    user_repository::set_mail_magazine(&mut tx, user_id, false).await?;
    tx.commit().await.map_err(|e| AppError::db("commit", e))?;
    Ok(Some(transitioned))
}

/// Whether marketing mail may be sent to this member right
/// now: the worker's send-time re-check. A missing row, an
/// unsubscribed row, a withdrawn account or a missing
/// account all read as "do not send".
pub async fn may_send(pool: &PgPool, user_id: i64) -> Result<bool, AppError> {
    let Some(subscription) = mail_repository::find_subscription(pool, user_id).await? else {
        return Ok(false);
    };
    if !subscription.is_subscribed() {
        return Ok(false);
    }
    let user = user_repository::find_by_id(pool, user_id)
        .await
        .map_err(|e| AppError::db("load user for mail check", e))?;
    Ok(user.is_some_and(|u| u.withdrawn_at.is_none()))
}
