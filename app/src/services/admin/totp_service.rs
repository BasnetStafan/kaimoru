//! Administrator two-factor flows:
//! enrolment, recovery-code regeneration, disable, and
//! the login challenge's verification.
//!
//! The deliberate mirror of the member flows in
//! [`crate::services::totp_service`] over the admin
//! identity — the member side had TOTP while the far more
//! powerful admin surface was password-only. Every rule is
//! the member rule restated: the secret is PII ciphertext
//! at rest, recovery codes are HMAC hashes bound to the
//! admin (under an `admin:` domain separator, so a member
//! code and an admin code can never cross over), an
//! accepted login step is recorded so it cannot replay,
//! and the login gate reads state strictly — a failed read
//! refuses the login instead of failing open.

use chrono::Utc;
use sqlx::PgPool;

use crate::error::AppError;
use crate::repositories::admin::{admin_repository, totp_repository};
use crate::repositories::totp_repository::TotpState;
use crate::services::totp;

/// Whether two-factor authentication is live for the
/// admin. A failed read answers "not enabled" only after
/// logging; the settings page reads the state again and
/// shows the truth. The login gate uses
/// [`is_enabled_strict`] instead.
pub async fn state(pool: &PgPool, admin_id: i64) -> Option<TotpState> {
    match totp_repository::state(pool, admin_id).await {
        Ok(mut state) => {
            if let Some(s) = state.as_mut() {
                s.secret = s
                    .secret
                    .take()
                    .map(|stored| decrypt_secret(admin_id, &stored));
            }
            state
        }
        Err(e) => {
            tracing::error!(admin_id, "admin totp state failed: {e}");
            None
        }
    }
}

/// Reads a stored enrolment secret: `v1:` ciphertext is
/// decrypted through the PII cipher; an undecryptable
/// value becomes an invalid base32 sentinel, so the admin
/// still reads as enabled — the challenge is issued — but
/// no code can ever verify against it (closed, never
/// open). Same construction as the member side.
fn decrypt_secret(admin_id: i64, stored: &str) -> String {
    if stored.starts_with("v1:") {
        match crate::pii::decrypt_field_checked(stored) {
            Some(plain) => plain,
            None => {
                tracing::error!(admin_id, "admin totp secret could not be decrypted");
                "!".to_string()
            }
        }
    } else {
        stored.to_string()
    }
}

/// Whether the admin must pass the login challenge.
pub async fn is_enabled(pool: &PgPool, admin_id: i64) -> bool {
    state(pool, admin_id).await.is_some_and(|s| s.is_enabled())
}

/// The login gate's fallible form: a state read that
/// fails is an error the caller must surface, never a
/// silent "not enabled".
pub async fn is_enabled_strict(pool: &PgPool, admin_id: i64) -> Result<bool, AppError> {
    match totp_repository::state(pool, admin_id).await {
        Ok(state) => Ok(state.is_some_and(|s| s.is_enabled())),
        Err(e) => Err(AppError::db("admin totp state read", e)),
    }
}

/// Confirms an enrolment: persists the secret (as PII
/// ciphertext) and issues the first recovery code set.
/// Returns the plaintext codes — shown once, never
/// stored.
pub async fn enable(
    pool: &PgPool,
    admin_id: i64,
    secret_base32: &str,
) -> Result<Vec<String>, AppError> {
    let stored = crate::pii::encrypt_field(secret_base32);
    totp_repository::set_enabled(pool, admin_id, &stored).await?;
    issue_recovery_codes(pool, admin_id).await
}

/// Issues a fresh recovery code set, invalidating every
/// earlier code. Returns the plaintext codes, shown once.
pub async fn issue_recovery_codes(pool: &PgPool, admin_id: i64) -> Result<Vec<String>, AppError> {
    let codes = totp::generate_recovery_codes();
    let hashes: Vec<String> = codes
        .iter()
        .map(|code| hash_recovery_code(admin_id, code))
        .collect();
    totp_repository::replace_recovery_codes(pool, admin_id, &hashes).await?;
    tracing::info!(
        admin_id,
        action = "admin_totp.recovery_issued",
        "auth action"
    );
    Ok(codes)
}

/// Disables two-factor authentication: secret, timestamp
/// and recovery codes cleared together.
pub async fn disable(pool: &PgPool, admin_id: i64) -> Result<(), AppError> {
    totp_repository::clear(pool, admin_id).await?;
    tracing::info!(admin_id, action = "admin_totp.disabled", "auth action");
    Ok(())
}

/// The disable ceremony: the current password plus a
/// current authenticator or recovery code must both prove
/// out before anything is cleared. One generic refusal
/// covers both proofs.
pub async fn disable_with_proof(
    pool: &PgPool,
    admin_id: i64,
    password_input: &str,
    code: &str,
) -> Result<(), AppError> {
    let refused =
        || AppError::Conflict("パスワードまたは認証コードが正しくありません。".to_string());
    let admin = admin_repository::find_admin_auth_by_id(pool, admin_id)
        .await?
        .ok_or_else(refused)?;
    if !crate::services::password::verify_password_async(
        password_input.to_string(),
        admin.password_hash.clone(),
    )
    .await
    {
        return Err(refused());
    }
    if !verify_proof_code(pool, admin_id, code).await {
        return Err(refused());
    }
    disable(pool, admin_id).await
}

/// Verifies a login-challenge submission for the admin:
/// a 6-digit authenticator code, or a recovery code
/// (spent atomically on success).
pub async fn verify_login_code(pool: &PgPool, admin_id: i64, submitted: &str) -> bool {
    verify_code(pool, admin_id, submitted, true).await
}

/// Verifies a code as a settings proof (the disable
/// ceremony, recovery-code regeneration): same checks as
/// the login challenge, but the step is NOT recorded as
/// consumed — the operator has just completed the
/// challenge with this very step, and must be able to
/// prove possession again inside the same window. The
/// brute-force surface this leaves is closed at the
/// controller instead: the
/// (email, IP) throttle plus a per-session attempt cap
/// bound every proof endpoint, and the session is
/// already fully authenticated. Step consumption stays
/// exclusive to the login gate, where it matters.
pub async fn verify_proof_code(pool: &PgPool, admin_id: i64, submitted: &str) -> bool {
    verify_code(pool, admin_id, submitted, false).await
}

/// Shared verification behind the login challenge and
/// settings proofs; recovery codes are single-use in
/// both.
async fn verify_code(pool: &PgPool, admin_id: i64, submitted: &str, record_step: bool) -> bool {
    let Some(state) = state(pool, admin_id).await else {
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
        return match totp_repository::record_step(pool, admin_id, step as i64).await {
            Ok(true) => true,
            Ok(false) => {
                tracing::warn!(admin_id, "admin totp code replay refused");
                false
            }
            Err(e) => {
                tracing::error!(admin_id, "admin totp step record failed: {e}");
                false
            }
        };
    }
    let Some(normalised) = totp::normalize_recovery_code(trimmed) else {
        return false;
    };
    let candidate = hash_recovery_code(admin_id, &normalised);
    let unused = match totp_repository::unused_recovery_codes(pool, admin_id).await {
        Ok(rows) => rows,
        Err(e) => {
            tracing::error!(admin_id, "admin recovery code lookup failed: {e}");
            return false;
        }
    };
    for (id, stored_hash) in unused {
        if crate::session::constant_time_eq(&stored_hash, &candidate) {
            return match totp_repository::mark_recovery_used(pool, id).await {
                Ok(true) => {
                    tracing::info!(admin_id, action = "admin_totp.recovery_used", "auth action");
                    true
                }
                Ok(false) => false,
                Err(e) => {
                    tracing::error!(admin_id, "admin recovery code spend failed: {e}");
                    false
                }
            };
        }
    }
    false
}

/// Hashes a recovery code for storage: HMAC-SHA-256 keyed
/// by the verification secret and bound to the admin
/// under an `admin:` domain separator — the member
/// construction, with the separator keeping the two
/// populations' codes disjoint even for equal ids.
fn hash_recovery_code(admin_id: i64, code: &str) -> String {
    static SECRET: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| {
        std::env::var("VERIFICATION_CODE_SECRET")
            .expect("VERIFICATION_CODE_SECRET must be set (see README / run.sh)")
    });
    let secret: &str = &SECRET;
    let normalised = totp::normalize_recovery_code(code).unwrap_or_else(|| code.to_string());
    crate::util::hmac_util::hmac_sha256_hex(
        secret.as_bytes(),
        &[
            format!("admin:{admin_id}").as_bytes(),
            b":",
            normalised.as_bytes(),
        ],
    )
}
