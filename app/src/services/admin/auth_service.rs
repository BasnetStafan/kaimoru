//! Admin authentication.
//!
//! The rules mirror the member login's shape on purpose — the
//! same Argon2id verification, the same dummy-hash timing
//! equalisation for unknown emails, the same in-memory throttle
//! thresholds — over a separate identity and separate throttle
//! counters, so neither side's lock can affect the other. Two
//! deliberate differences: the login form shows a distinct
//! message while throttled (the operator must be told, not left
//! guessing), and success is audited.

use sqlx::PgPool;

use crate::config::CONFIG;
use crate::error::AppError;
use crate::messages::{
    CURRENT_PASSWORD_MISMATCH_MESSAGE, DISPLAY_NAME_REQUIRED_MESSAGE,
    PASSWORD_SAME_AS_CURRENT_MESSAGE,
};
use crate::middleware::auth_throttle;
use crate::models::Admin;
use crate::repositories::{admin::admin_repository, session_revocation_repository};
use crate::services::admin::totp_service;
use crate::services::password;
use crate::validation::FieldError;

/// Outcome of one admin login attempt.
pub enum AdminLoginOutcome {
    /// Credentials accepted; carries the signed-in admin.
    Success(Admin),
    /// Credentials refused: unknown email, wrong password, or a
    /// database failure presented as a refusal. The caller shows
    /// the single generic message.
    BadCredentials,
    /// The (email, IP) pair or the IP is throttled; credentials
    /// were not checked at all.
    Throttled,
}

/// Attempts one admin login. `email` is normalised (trimmed,
/// lowercased) the same way the member login normalises.
pub async fn login(pool: &PgPool, email: &str, password: &str, ip: &str) -> AdminLoginOutcome {
    let email = email.trim().to_lowercase();
    // Throttle check first: a locked pair or IP never reaches
    // the password check, and attempts during a lock do not
    // extend it (the throttle's own rule).
    if auth_throttle::admin_is_blocked(&email, ip) {
        return AdminLoginOutcome::Throttled;
    }
    let admin = match admin_repository::find_admin_by_email(pool, &email).await {
        Ok(admin) => admin,
        Err(e) => {
            // A database failure presents as a refusal to the
            // caller, but the real cause is logged: the operator
            // must never see a stack trace, and the failure must
            // not look like a wrong password in the logs either.
            tracing::error!("admin login lookup failed: {e}");
            return AdminLoginOutcome::BadCredentials;
        }
    };
    // Unknown email: verify against the dummy hash anyway, so the
    // response time does not reveal whether the account exists.
    let (hash, exists) = match &admin {
        Some(admin) => (admin.password_hash.clone(), true),
        None => (password::dummy_hash().to_string(), false),
    };
    let matched = password::verify_password_async(password.to_string(), hash).await;
    // A disabled account with the correct password still gets
    // the generic refusal (and counts as a failure): the
    // response must never reveal that the account exists but
    // is disabled.
    let active = admin.as_ref().map(|a| a.is_active).unwrap_or(false);
    if exists && matched && active {
        let admin = admin.expect("admin exists when matched");
        // The throttle pair is cleared only by a COMPLETE
        // login: when a TOTP
        // challenge still stands between this password
        // match and a session, clearing here would reset
        // the challenge's failure budget on every re-login,
        // so the challenge step records the success itself
        // when it completes. A TOTP state that cannot be
        // read counts as a pending challenge (fail closed);
        // the controller refuses such a login at its own
        // strict gate anyway.
        let challenge_pending = !matches!(
            totp_service::is_enabled_strict(pool, admin.id).await,
            Ok(false)
        );
        if !challenge_pending {
            auth_throttle::admin_record_success(&email, ip);
        }
        AdminLoginOutcome::Success(admin)
    } else {
        auth_throttle::admin_record_failure(&email, ip);
        AdminLoginOutcome::BadCredentials
    }
}

/// Minimum length of the environment bootstrap password.
pub(crate) const BOOTSTRAP_PASSWORD_MIN_CHARS: usize = 12;

/// Creates the very first admin from the `ADMIN_EMAIL` /
/// `ADMIN_PASSWORD` environment pair, exactly once: only while
/// the admins table is empty, and never again — an existing
/// admin's password is never reset by configuration. With the
/// pair unset, bootstrap is a no-op. The password is hashed with
/// Argon2id like every credential; nothing here logs the email
/// or the password.
pub async fn bootstrap_admin(pool: &PgPool) -> Result<(), AppError> {
    let (Some(email), Some(password)) = (CONFIG.admin_email(), CONFIG.admin_password()) else {
        return Ok(());
    };
    if admin_repository::count_admins(pool).await? > 0 {
        return Ok(());
    }
    // Strength floor: this credential becomes the first admin —
    // in a fresh deployment, briefly the only admin — from
    // nothing but an environment variable, so a one-character
    // value must never take effect; refusing to start beats
    // starting weak. The floor is length alone (12 characters):
    // length is what resists guessing for an operator-set secret,
    // and the member rule's letter+digit composition
    // ([`password::password_rule_error`]) is deliberately not
    // reused — it would reject the local development value for no
    // strength gain at this length. The check runs only on the
    // bootstrap path (empty table), so an existing admin is never
    // locked out by it.
    if password.chars().count() < BOOTSTRAP_PASSWORD_MIN_CHARS {
        return Err(AppError::Conflict(format!(
            "ADMIN_PASSWORD is too weak: the bootstrap admin password must be              at least {BOOTSTRAP_PASSWORD_MIN_CHARS} characters; refusing to start"
        )));
    }
    let email = email.trim().to_lowercase();
    let hash = password::hash_password_async(password).await?;
    let display_name = CONFIG.admin_display_name();
    let id = admin_repository::insert_admin(pool, &email, &hash, &display_name).await?;
    // The action and the new admin's ID only — never the email.
    tracing::info!(admin_id = id, "admin_bootstrapped");
    Ok(())
}

/// Longest display name the settings page accepts. The name
/// renders in the sidebar of every admin page; the cap keeps a
/// paste accident from breaking that layout, nothing more.
pub(crate) const DISPLAY_NAME_MAX_CHARS: usize = 50;

/// Validates the settings page's password form, all errors at
/// once (the codebase's form convention): the current password
/// must be present, the new one must meet the bootstrap floor —
/// 12 characters, length only (the member composition rule adds
/// no strength at this length) — and the confirmation must match.
/// The credential check itself is [`change_password`]'s.
pub fn validate_password_change(current: &str, new: &str, confirm: &str) -> Vec<FieldError> {
    let mut errors = Vec::new();
    if current.is_empty() {
        errors.push(FieldError {
            field: "current_password",
            message: "現在のパスワードを入力してください。".to_string(),
        });
    }
    if new.is_empty() {
        errors.push(FieldError {
            field: "new_password",
            message: "新しいパスワードを入力してください。".to_string(),
        });
    } else if new.chars().count() < BOOTSTRAP_PASSWORD_MIN_CHARS {
        errors.push(FieldError {
            field: "new_password",
            message: format!(
                "パスワードは{BOOTSTRAP_PASSWORD_MIN_CHARS}文字以上で入力してください。"
            ),
        });
    }
    if new != confirm {
        errors.push(FieldError {
            field: "new_password_confirm",
            message: "新しいパスワード（確認）が一致しません。".to_string(),
        });
    }
    errors
}

/// Changes one admin's own password: verifies the current
/// password against the stored Argon2id hash, replaces the hash,
/// audits the change and revokes the admin's other sessions (the
/// session making the change, `keep_session_id`, survives). This
/// is an authenticated flow: the login throttle is deliberately
/// not involved, because its budget guards anonymous guessing,
/// not a signed-in operator proving they still know the current
/// password.
///
/// # Errors
///
/// [`AppError::Conflict`] with the generic current-password
/// message when verification fails (the page never learns more
/// than "wrong"), or when the new password equals the current
/// one; [`AppError::Db`] on a database failure.
pub async fn change_password(
    pool: &PgPool,
    admin_id: i64,
    current: &str,
    new: &str,
    keep_session_id: Option<String>,
) -> Result<(), AppError> {
    let admin = admin_repository::find_admin_auth_by_id(pool, admin_id)
        .await?
        .ok_or(AppError::NotFound)?;
    if !password::verify_password_async(current.to_string(), admin.password_hash).await {
        return Err(AppError::Conflict(
            CURRENT_PASSWORD_MISMATCH_MESSAGE.to_string(),
        ));
    }
    if current == new {
        return Err(AppError::Conflict(
            PASSWORD_SAME_AS_CURRENT_MESSAGE.to_string(),
        ));
    }
    let hash = password::hash_password_async(new.to_string()).await?;
    admin_repository::update_admin_password(pool, admin_id, &hash).await?;
    // The change is audited like every credential event on this
    // surface (login is too); a failed audit insert is logged,
    // never raised — the password has changed at that point, and
    // reporting failure would tell the operator something untrue.
    if let Err(e) = admin_repository::insert_audit_standalone(
        pool,
        admin_id,
        "admin.password",
        "admin",
        Some(admin_id),
    )
    .await
    {
        tracing::error!(admin_id, "admin password audit insert failed: {e}");
    }
    revoke_admin_sessions(pool, admin_id, keep_session_id.as_deref()).await;
    Ok(())
}

/// Deletes an admin's other sessions after a password change.
///
/// A revocation failure is logged, never raised, for the same
/// reason as the member side's: the password change it follows
/// has already committed.
pub(crate) async fn revoke_admin_sessions(
    pool: &PgPool,
    admin_id: i64,
    keep_session_id: Option<&str>,
) {
    match session_revocation_repository::delete_sessions_for_admin(pool, admin_id, keep_session_id)
        .await
    {
        Ok(count) => {
            tracing::info!(
                admin_id,
                revoked = count,
                action = "sessions_revoked",
                "auth action"
            );
        }
        Err(e) => {
            tracing::error!(admin_id, "admin session revocation failed: {e}");
        }
    }
}

/// Replaces one admin's own display name and returns the stored
/// value. Audited as `admin.profile`.
///
/// # Errors
///
/// [`AppError::Validation`] when the trimmed name is empty or
/// over the length cap; [`AppError::Db`] on a database failure.
pub async fn update_display_name(
    pool: &PgPool,
    admin_id: i64,
    display_name: &str,
) -> Result<String, AppError> {
    let name = display_name.trim();
    if name.is_empty() {
        return Err(AppError::Validation(vec![FieldError {
            field: "display_name",
            message: DISPLAY_NAME_REQUIRED_MESSAGE.to_string(),
        }]));
    }
    if name.chars().count() > DISPLAY_NAME_MAX_CHARS {
        return Err(AppError::Validation(vec![FieldError {
            field: "display_name",
            message: format!("表示名は{DISPLAY_NAME_MAX_CHARS}文字以内で入力してください。"),
        }]));
    }
    admin_repository::update_admin_display_name(pool, admin_id, name).await?;
    if let Err(e) = admin_repository::insert_audit_standalone(
        pool,
        admin_id,
        "admin.profile",
        "admin",
        Some(admin_id),
    )
    .await
    {
        tracing::error!(admin_id, "admin profile audit insert failed: {e}");
    }
    Ok(name.to_string())
}

/// Records a successful admin login: the last-login stamp on
/// the account and the audit row. Login mutates no business
/// record, so its audit row is a standalone insert.
pub async fn record_login(pool: &PgPool, admin_id: i64) -> Result<(), AppError> {
    admin_repository::touch_last_login(pool, admin_id).await?;
    admin_repository::insert_audit_standalone(
        pool,
        admin_id,
        "admin.login",
        "admin",
        Some(admin_id),
    )
    .await
}
