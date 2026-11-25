//! Member withdrawal （退会） (Account Deletion spec).
//!
//! Anonymise in place: the `users` row survives as a
//! tombstone keying the retained orders, scrubbed of every
//! identity field. Eligibility blocks on in-flight
//! business. The execution is one transaction —
//! identity scrub, cart, saved addresses, favourites, TOTP,
//! SNS identities, pending verification records, coupon
//! voiding, order-header and inquiry scrubbing — followed,
//! after the commit, by the
//! session revocation (the password-change precedent: the
//! machinery is pool-based and logs its own failures) and
//! the completion mail (D8: never inside the transaction).
//!
//! Artefacts the spec's disposition table predates are
//! handled by its own principle (personal data the shop
//! no longer has a reason to hold goes): saved addresses
//! are deleted, and inquiries keep their support record
//! with name/email scrubbed and the member link removed.

use sqlx::PgPool;

use crate::error::AppError;
use crate::models::{CodeChallenge, Order};
use crate::repositories::{
    cart_repository, challenge_repository, coupon_repository, favorite_repository,
    inquiry_repository, member_address_repository, order_repository, session_revocation_repository,
    totp_repository, user_identity_repository, user_repository,
};
use crate::services::password;

/// The result of one withdrawal attempt.
pub enum WithdrawOutcome {
    /// The withdrawal committed (sessions and mail follow
    /// outside the transaction).
    Done,
    /// The re-entered password did not match.
    WrongPassword,
    /// The email code proof was missing, unverified, or
    /// expired (passwordless accounts only).
    ProofInvalid,
    /// In-flight business blocks the withdrawal (spec
    /// D3): carries the blocking orders.
    Blocked(Vec<Order>),
    /// The account was already withdrawn: the idempotent
    /// landing of spec section 5.
    AlreadyWithdrawn,
    /// No such member (defensive; the session normally
    /// proves the account).
    NotFound,
}

/// HMAC-SHA-256 of a normalised email, keyed by the
/// verification secret — the once-per-email-ever
/// WELCOME500 check of D4 stores and compares only this,
/// never the address itself.
pub fn email_hash(email: &str) -> String {
    static SECRET: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| {
        std::env::var("VERIFICATION_CODE_SECRET")
            .expect("VERIFICATION_CODE_SECRET must be set (see README / run.sh)")
    });
    let normalized = email.trim().to_lowercase();
    crate::util::hmac_util::hmac_sha256_hex(
        SECRET.as_bytes(),
        &[b"welcome-email:", normalized.as_bytes()],
    )
}

/// Whether a withdrawn account under this email ever
/// redeemed WELCOME500: a returner whose predecessor
/// redeemed it is not granted it again; one who never
/// redeemed it is.
///
/// # Errors
///
/// Returns [`AppError`] on database failure.
pub async fn welcome_previously_redeemed(pool: &PgPool, email: &str) -> Result<bool, AppError> {
    user_repository::welcome_previously_redeemed(pool, &email_hash(email))
        .await
        .map_err(|e| AppError::db("welcome previously redeemed", e))
}

/// The orders currently blocking a member's withdrawal
///: unshipped orders, orders whose payment is
/// still pending, and orders with a refund in flight.
///
/// # Errors
///
/// Returns [`AppError`] on database failure.
pub async fn blockers(pool: &PgPool, user_id: i64) -> Result<Vec<Order>, AppError> {
    order_repository::list_blocking_for_user(pool, user_id)
        .await
        .map_err(|e| AppError::db("withdrawal blockers", e))
}

/// Withdraws a member. The password is verified before
/// anything is read for execution; eligibility is then
/// re-checked inside the transaction under the user
/// row's lock, so a state change between the page render
/// and the POST cannot slip a blocker past.
///
/// # Errors
///
/// Returns [`AppError`] on database failure; the
/// transaction rolls back with it, leaving the member
/// fully in (no half-withdrawn state).
pub async fn withdraw(
    pool: &PgPool,
    user_id: i64,
    password_input: &str,
) -> Result<WithdrawOutcome, AppError> {
    let Some(stored_hash) = user_repository::password_hash_for_id(pool, user_id)
        .await
        .map_err(|e| AppError::db("withdrawal password load", e))?
    else {
        return Ok(WithdrawOutcome::NotFound);
    };
    let verified = password::verify_password_async(password_input.to_string(), stored_hash).await;
    if !verified {
        // Logged with the action and member id only —
        // never the attempted password.
        tracing::warn!(
            user_id,
            action = "member.withdraw_failed",
            "withdrawal password mismatch"
        );
        return Ok(WithdrawOutcome::WrongPassword);
    }

    execute_withdrawal(pool, user_id).await
}

/// Withdraws a passwordless (SNS-registered) member on the
/// email code proof instead of a password: the caller ran the
/// challenge verification, and this entry point re-checks the
/// proof (owner-matched, verified, unexpired) before running
/// the same ceremony as the password path.
///
/// # Errors
///
/// Returns [`AppError`] on database failure.
pub async fn withdraw_with_code_proof(
    pool: &PgPool,
    user_id: i64,
    challenge: &CodeChallenge,
) -> Result<WithdrawOutcome, AppError> {
    if challenge.user_id != user_id || !challenge.verified {
        return Ok(WithdrawOutcome::ProofInvalid);
    }
    if challenge.expires_at <= chrono::Utc::now() {
        return Ok(WithdrawOutcome::ProofInvalid);
    }
    execute_withdrawal(pool, user_id).await
}

/// The withdrawal ceremony shared by both proofs: everything
/// from the random-secret scrub to the completion mail. The
/// entry points differ only in how ownership was proven.
async fn execute_withdrawal(pool: &PgPool, user_id: i64) -> Result<WithdrawOutcome, AppError> {
    // The scrub replaces the stored credential with a hash
    // of a random, unguessable secret nobody knows, so the
    // old password dies even against a future database leak.
    // The hash depends on nothing in the database, so it is
    // computed here — before the transaction opens — via the
    // async wrapper (Argon2 on the blocking pool) rather
    // than stalling inside the transaction.
    let random_secret = {
        use rand::Rng;
        let mut rng = rand::thread_rng();
        format!(
            "{:016x}{:016x}",
            rng.gen_range(0..=u64::MAX),
            rng.gen_range(0..=u64::MAX)
        )
    };
    let random_hash = password::hash_password_async(random_secret).await?;

    let mut tx = pool
        .begin()
        .await
        .map_err(|e| AppError::db("withdrawal begin", e))?;
    let Some((email, withdrawn_at)) = user_repository::lock_withdrawal_state(&mut tx, user_id)
        .await
        .map_err(|e| AppError::db("withdrawal user lock", e))?
    else {
        return Ok(WithdrawOutcome::NotFound);
    };
    if withdrawn_at.is_some() {
        tx.rollback()
            .await
            .map_err(|e| AppError::db("withdrawal rollback", e))?;
        return Ok(WithdrawOutcome::AlreadyWithdrawn);
    }
    let blocking = order_repository::list_blocking_for_user(&mut *tx, user_id)
        .await
        .map_err(|e| AppError::db("withdrawal blockers in tx", e))?;
    if !blocking.is_empty() {
        tx.rollback()
            .await
            .map_err(|e| AppError::db("withdrawal rollback", e))?;
        return Ok(WithdrawOutcome::Blocked(blocking));
    }

    // The pre-scrub address is captured for the completion
    // mail; after the scrub it exists nowhere.
    let marker_email = format!("deleted-{user_id}@invalid.example");
    let hash = email_hash(&email);
    user_repository::scrub_for_withdrawal(&mut tx, user_id, &marker_email, &random_hash, &hash)
        .await
        .map_err(|e| AppError::db("withdrawal scrub", e))?;
    cart_repository::delete_all_for_user(&mut tx, user_id)
        .await
        .map_err(|e| AppError::db("withdrawal cart cleanup", e))?;
    member_address_repository::delete_all_for_user(&mut tx, user_id)
        .await
        .map_err(|e| AppError::db("withdrawal address cleanup", e))?;
    favorite_repository::delete_all_for_user(&mut tx, user_id)
        .await
        .map_err(|e| AppError::db("withdrawal favourite cleanup", e))?;
    totp_repository::delete_all_for_user(&mut tx, user_id)
        .await
        .map_err(|e| AppError::db("withdrawal totp cleanup", e))?;
    user_identity_repository::delete_all_for_user(&mut tx, user_id)
        .await
        .map_err(|e| AppError::db("withdrawal identity cleanup", e))?;
    challenge_repository::delete_all_for_user(&mut tx, user_id)
        .await
        .map_err(|e| AppError::db("withdrawal pending cleanup", e))?;
    coupon_repository::void_unused_for_user(&mut tx, user_id)
        .await
        .map_err(|e| AppError::db("withdrawal coupon voiding", e))?;
    order_repository::scrub_delivery_for_user(&mut tx, user_id, &marker_email)
        .await
        .map_err(|e| AppError::db("withdrawal order scrub", e))?;
    inquiry_repository::scrub_for_user(&mut tx, user_id, &marker_email)
        .await
        .map_err(|e| AppError::db("withdrawal inquiry scrub", e))?;
    tx.commit()
        .await
        .map_err(|e| AppError::db("withdrawal commit", e))?;
    tracing::info!(user_id, action = "member.withdraw", "member withdrew");

    // Access ends everywhere, immediately. The
    // revocation machinery is pool-based and logs its own
    // failures, like the password-change path before it.
    match session_revocation_repository::delete_sessions_for_user(pool, user_id, None).await {
        Ok(count) => {
            tracing::info!(
                user_id,
                revoked = count,
                action = "sessions_revoked",
                "auth action"
            );
        }
        Err(e) => {
            tracing::error!(user_id, "withdrawal session revocation failed: {e}");
        }
    }
    spawn_completion_mail(pool.clone(), email);
    Ok(WithdrawOutcome::Done)
}

/// Spawns the 退会完了 mail off the request path:
/// addressed to the pre-scrub address captured before
/// anonymisation; a failure is logged, never surfaced —
/// the account is already gone and there is no one to
/// retry for.
fn spawn_completion_mail(pool: PgPool, to_email: String) {
    tokio::spawn(async move {
        if let Err(e) = crate::mail::send_withdrawal_completed(&pool, &to_email, "").await {
            tracing::error!("withdrawal completion mail failed: {e}");
        }
    });
}
