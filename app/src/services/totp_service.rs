//! Two-factor flows: enrolment, recovery-code regeneration,
//! disable, and the login challenge's verification.
//!
//! The settings flows run behind the member session; the login
//! verification runs before one exists, keyed by the pending
//! marker the login controller holds in the session.

use chrono::Utc;
use sqlx::PgPool;

use crate::error::AppError;
use crate::repositories::totp_repository::{self, TotpState};
use crate::services::totp;

/// Whether two-factor authentication is live for the member.
/// A failed read answers "not enabled" only after logging:
/// the login controller treats that as a plain login, which
/// is the fail-open direction for availability — but the
/// settings page reads the state again and shows the truth.
pub async fn state(pool: &PgPool, user_id: i64) -> Option<TotpState> {
    match totp_repository::state(pool, user_id).await {
        Ok(mut state) => {
            if let Some(s) = state.as_mut() {
                s.secret = s
                    .secret
                    .take()
                    .map(|stored| decrypt_secret(user_id, &stored));
            }
            state
        }
        Err(e) => {
            tracing::error!(user_id, "totp state failed: {e}");
            None
        }
    }
}

/// Reads a stored enrolment secret (SEC-4): values written
/// since the encryption change are `v1:` ciphertext, decrypted
/// through the PII cipher; an older plaintext value is used
/// as-is (no such rows exist today - the fallback only keeps
/// the read total). An undecryptable value becomes an invalid
/// base32 sentinel, so the member still reads as enabled -
/// the challenge is issued - but no code can ever verify
/// against it: the failure direction is closed, never open.
fn decrypt_secret(user_id: i64, stored: &str) -> String {
    if stored.starts_with("v1:") {
        match crate::pii::decrypt_field_checked(stored) {
            Some(plain) => plain,
            None => {
                tracing::error!(user_id, "totp secret could not be decrypted");
                "!".to_string()
            }
        }
    } else {
        stored.to_string()
    }
}

/// Whether the member must pass the login challenge.
pub async fn is_enabled(pool: &PgPool, user_id: i64) -> bool {
    state(pool, user_id).await.is_some_and(|s| s.is_enabled())
}

/// The login gate's fallible form (SEC-2): a state read that
/// fails is an error the caller must surface, never a silent
/// "not enabled" - failing open here would reduce an enrolled
/// account to password-only login on a transient DB error.
pub async fn is_enabled_strict(pool: &PgPool, user_id: i64) -> Result<bool, AppError> {
    match totp_repository::state(pool, user_id).await {
        Ok(state) => Ok(state.is_some_and(|s| s.is_enabled())),
        Err(e) => Err(AppError::db("totp state read", e)),
    }
}

/// Confirms an enrolment: persists the secret and issues the
/// first recovery code set. Returns the plaintext codes —
/// shown to the member once, never stored.
pub async fn enable(
    pool: &PgPool,
    user_id: i64,
    secret_base32: &str,
) -> Result<Vec<String>, AppError> {
    // SEC-4: the secret is stored as PII ciphertext, never
    // plaintext; state() decrypts it back on every read.
    let stored = crate::pii::encrypt_field(secret_base32);
    totp_repository::set_enabled(pool, user_id, &stored).await?;
    issue_recovery_codes(pool, user_id).await
}

/// Issues a fresh recovery code set, invalidating every
/// earlier code. Returns the plaintext codes, shown once.
pub async fn issue_recovery_codes(pool: &PgPool, user_id: i64) -> Result<Vec<String>, AppError> {
    let codes = totp::generate_recovery_codes();
    let hashes: Vec<String> = codes
        .iter()
        .map(|code| hash_recovery_code(user_id, code))
        .collect();
    totp_repository::replace_recovery_codes(pool, user_id, &hashes).await?;
    tracing::info!(user_id, action = "totp.recovery_issued", "auth action");
    Ok(codes)
}

/// Disables two-factor authentication: secret, timestamp and
/// recovery codes cleared together. Session revocation is the
/// caller's step, matching the password change's shape.
pub async fn disable(pool: &PgPool, user_id: i64) -> Result<(), AppError> {
    totp_repository::clear(pool, user_id).await?;
    tracing::info!(user_id, action = "totp.disabled", "auth action");
    Ok(())
}

/// The disable ceremony: the current password plus a current
/// authenticator or recovery code must both prove out before
/// anything is cleared. One generic refusal covers both
/// proofs, so the answer never says which one failed.
pub async fn disable_with_proof(
    pool: &PgPool,
    user_id: i64,
    password_input: &str,
    code: &str,
) -> Result<(), AppError> {
    let refused =
        || AppError::Conflict("パスワードまたは認証コードが正しくありません。".to_string());
    let stored = crate::repositories::user_repository::password_hash_for_id(pool, user_id)
        .await?
        .ok_or_else(refused)?;
    if !crate::services::password::verify_password_async(password_input.to_string(), stored).await {
        return Err(refused());
    }
    if !verify_proof_code(pool, user_id, code).await {
        return Err(refused());
    }
    disable(pool, user_id).await
}

/// Verifies a login-challenge submission for the member: a
/// 6-digit authenticator code, or a recovery code (spent
/// atomically on success, so it can never be used twice).
pub async fn verify_login_code(pool: &PgPool, user_id: i64, submitted: &str) -> bool {
    verify_code(pool, user_id, submitted, true).await
}

/// Verifies a code as a settings proof (the disable ceremony):
/// same checks as the login challenge, but the step is NOT
/// recorded as consumed. The replay guard belongs to the
/// login gate (SEC-5's scenario is a parallel pending login);
/// a settings proof already stands behind an authenticated
/// session plus the account password, and recording here
/// would refuse a member who disables two-factor in the same
/// step window as the login that brought them in.
pub async fn verify_proof_code(pool: &PgPool, user_id: i64, submitted: &str) -> bool {
    verify_code(pool, user_id, submitted, false).await
}

/// Shared verification behind the login challenge (step
/// recorded) and settings proofs (step not recorded).
/// Recovery codes are single-use in both: the spend is
/// atomic in the repository.
async fn verify_code(pool: &PgPool, user_id: i64, submitted: &str, record_step: bool) -> bool {
    let Some(state) = state(pool, user_id).await else {
        return false;
    };
    if !state.is_enabled() {
        return false;
    }
    let secret = state.secret.as_deref().unwrap_or_default();
    let trimmed = submitted.trim();
    if trimmed.len() == totp::DIGITS as usize && trimmed.bytes().all(|b| b.is_ascii_digit()) {
        if secret.is_empty() {
            return false;
        }
        let now = Utc::now().timestamp().max(0) as u64;
        let Some(step) = totp::verify_step(secret, trimmed, now) else {
            return false;
        };
        if !record_step {
            return true;
        }
        // SEC-5: at the login gate the matched step is
        // recorded as consumed; a step at or below the stored
        // one is a replay.
        return match totp_repository::record_step(pool, user_id, step as i64).await {
            Ok(true) => true,
            Ok(false) => {
                tracing::warn!(user_id, "totp code replay refused");
                false
            }
            Err(e) => {
                tracing::error!(user_id, "totp step record failed: {e}");
                false
            }
        };
    }
    let Some(normalised) = totp::normalize_recovery_code(trimmed) else {
        return false;
    };
    let candidate = hash_recovery_code(user_id, &normalised);
    let unused = match totp_repository::unused_recovery_codes(pool, user_id).await {
        Ok(rows) => rows,
        Err(e) => {
            tracing::error!(user_id, "recovery code lookup failed: {e}");
            return false;
        }
    };
    for (id, stored_hash) in unused {
        // Hash-to-hash comparison in constant time, the code
        // challenge system's discipline: the stored value is a
        // hash, so a table leak alone yields nothing usable.
        if crate::session::constant_time_eq(&stored_hash, &candidate) {
            return match totp_repository::mark_recovery_used(pool, id).await {
                Ok(true) => {
                    tracing::info!(user_id, action = "totp.recovery_used", "auth action");
                    true
                }
                Ok(false) => false,
                Err(e) => {
                    tracing::error!(user_id, "recovery code spend failed: {e}");
                    false
                }
            };
        }
    }
    false
}

/// Hashes a recovery code for storage: HMAC-SHA-256 keyed by
/// the verification secret and bound to the member, exactly
/// the code-challenge system's construction, so a leaked
/// table cannot be brute forced offline without the secret
/// and a code never works for another member.
fn hash_recovery_code(user_id: i64, code: &str) -> String {
    static SECRET: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| {
        std::env::var("VERIFICATION_CODE_SECRET")
            .expect("VERIFICATION_CODE_SECRET must be set (see README / run.sh)")
    });
    let secret: &str = &SECRET;
    let normalised = totp::normalize_recovery_code(code).unwrap_or_else(|| code.to_string());
    crate::util::hmac_util::hmac_sha256_hex(
        secret.as_bytes(),
        &[user_id.to_string().as_bytes(), b":", normalised.as_bytes()],
    )
}
