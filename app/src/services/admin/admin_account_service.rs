//! Admin account management.
//!
//! The rules behind the owner's account screens. Only owners
//! reach these functions (the permission guard owns that
//! refusal, and the controller repeats the check); the rails
//! enforced here are about the TARGET account:
//!
//! - an owner cannot edit their own account on this surface —
//!   the account settings page owns the signed-in admin's
//!   name and password, so the two surfaces never disagree;
//! - the last active owner cannot be deactivated or demoted,
//!   so the store can never lock itself out of its own back
//!   office;
//! - a disabled account is cut off immediately, because the
//!   session layer resolves the account fresh on every
//!   request (see [`crate::session::current_admin`]).

use sqlx::PgPool;

use crate::error::AppError;
use crate::messages::{
    DISPLAY_NAME_REQUIRED_MESSAGE, EMAIL_FORMAT_MESSAGE, EMAIL_REQUIRED_MESSAGE,
};
use crate::models::{ADMIN_AREAS, AdminSummary};
use crate::repositories::admin::admin_repository;
use crate::services::admin::audit_service;
use crate::services::admin::auth_service;
use crate::services::password;
use crate::validation::{FieldError, is_valid_email};

/// Longest login email the create form accepts, matching the
/// member side's cap for the same kind of value.
const EMAIL_MAX_CHARS: usize = 254;

/// Keeps only the known area keys from a submitted permission
/// list, in canonical sidebar order: the stored array is
/// always a subset of [`ADMIN_AREAS`], whatever was posted.
pub fn sanitize_permissions(submitted: &[String]) -> Vec<String> {
    ADMIN_AREAS
        .iter()
        .filter(|(key, _)| submitted.iter().any(|p| p == key))
        .map(|(key, _)| key.to_string())
        .collect()
}

/// Validates the create form's fields, all errors at once.
/// The duplicate-email check needs the database and lives in
/// [`create_admin`].
pub fn validate_create(
    email: &str,
    display_name: &str,
    role: &str,
    pw: &str,
    pw_confirm: &str,
) -> Vec<FieldError> {
    let mut errors = Vec::new();
    let err = |field: &'static str, message: String| FieldError { field, message };
    if email.is_empty() {
        errors.push(err("email", EMAIL_REQUIRED_MESSAGE.to_string()));
    } else if email.chars().count() > EMAIL_MAX_CHARS || !is_valid_email(email) {
        errors.push(err("email", EMAIL_FORMAT_MESSAGE.to_string()));
    }
    errors.extend(validate_name_and_role(display_name, role));
    errors.extend(validate_password_pair("password", pw, pw_confirm));
    errors
}

/// Validates the edit form's fields, all errors at once.
pub fn validate_update(display_name: &str, role: &str) -> Vec<FieldError> {
    validate_name_and_role(display_name, role)
}

/// Validates the password-reset form's fields.
pub fn validate_reset(new_pw: &str, new_pw_confirm: &str) -> Vec<FieldError> {
    validate_password_pair("new_password", new_pw, new_pw_confirm)
}

/// The display-name and role checks the create and edit forms
/// share: the name is required and capped (it renders in the
/// sidebar of every admin page), the role is one of the two
/// stored values.
fn validate_name_and_role(display_name: &str, role: &str) -> Vec<FieldError> {
    let mut errors = Vec::new();
    if display_name.is_empty() {
        errors.push(FieldError {
            field: "display_name",
            message: DISPLAY_NAME_REQUIRED_MESSAGE.to_string(),
        });
    } else if display_name.chars().count() > auth_service::DISPLAY_NAME_MAX_CHARS {
        errors.push(FieldError {
            field: "display_name",
            message: format!(
                "表示名は{}文字以内で入力してください。",
                auth_service::DISPLAY_NAME_MAX_CHARS
            ),
        });
    }
    if role != "owner" && role != "staff" {
        errors.push(FieldError {
            field: "role",
            message: "役割を選択してください。".to_string(),
        });
    }
    errors
}

/// The password-pair checks: present, at the admin floor
/// (12 characters, length only — the same floor the settings
/// page and the bootstrap enforce), and confirmed.
fn validate_password_pair(field: &'static str, pw: &str, pw_confirm: &str) -> Vec<FieldError> {
    let mut errors = Vec::new();
    if pw.is_empty() {
        errors.push(FieldError {
            field,
            message: "パスワードを入力してください。".to_string(),
        });
    } else if pw.chars().count() < auth_service::BOOTSTRAP_PASSWORD_MIN_CHARS {
        errors.push(FieldError {
            field,
            message: format!(
                "パスワードは{}文字以上で入力してください。",
                auth_service::BOOTSTRAP_PASSWORD_MIN_CHARS
            ),
        });
    }
    if pw != pw_confirm {
        errors.push(FieldError {
            field: if field == "password" {
                "password_confirm"
            } else {
                "new_password_confirm"
            },
            message: "パスワード（確認）が一致しません。".to_string(),
        });
    }
    errors
}

/// Lists every admin account for the management screen.
pub async fn list_admins(pool: &PgPool) -> Result<Vec<AdminSummary>, AppError> {
    admin_repository::list_admins(pool).await
}

/// Finds one admin account for the management screens.
pub async fn find_admin(pool: &PgPool, id: i64) -> Result<Option<AdminSummary>, AppError> {
    admin_repository::find_admin_summary(pool, id).await
}

/// Creates one admin account and audits it in the same
/// transaction. `email` arrives normalised (trimmed,
/// lowercased) and validated by [`validate_create`]; the
/// duplicate check happens here, against the stored accounts.
///
/// # Errors
///
/// [`AppError::Validation`] with the email error when the
/// address is already registered; [`AppError::Db`] on a
/// database failure.
pub async fn create_admin(
    pool: &PgPool,
    actor_id: i64,
    email: &str,
    display_name: &str,
    role: &str,
    permissions: &[String],
    pw: &str,
) -> Result<i64, AppError> {
    if admin_repository::find_admin_by_email(pool, email)
        .await?
        .is_some()
    {
        return Err(AppError::Validation(vec![FieldError {
            field: "email",
            message: "このメールアドレスは既に登録されています。".to_string(),
        }]));
    }
    // Hash before opening the transaction: Argon2id is slow by
    // design, and no transaction should be held across it.
    let hash = password::hash_password_async(pw.to_string()).await?;
    let mut tx = pool.begin().await.map_err(|e| AppError::db("begin", e))?;
    let id = admin_repository::insert_admin_account(
        &mut tx,
        email,
        &hash,
        display_name,
        role,
        &sanitize_permissions(permissions),
    )
    .await?;
    audit_service::record(&mut tx, actor_id, "admin.create", "admin", Some(id)).await?;
    tx.commit().await.map_err(|e| AppError::db("commit", e))?;
    Ok(id)
}

/// Replaces one account's name, role, permissions and active
/// flag, audited in the same transaction.
///
/// # Errors
///
/// [`AppError::Conflict`] when the target is the actor
/// themselves or the last active owner would be lost;
/// [`AppError::NotFound`] when the account does not exist;
/// [`AppError::Db`] on a database failure.
#[allow(clippy::too_many_arguments)]
pub async fn update_admin(
    pool: &PgPool,
    actor_id: i64,
    target: &AdminSummary,
    display_name: &str,
    role: &str,
    permissions: &[String],
    is_active: bool,
    expected: i64,
) -> Result<(), AppError> {
    if target.id == actor_id {
        return Err(AppError::Conflict(
            "自分自身のアカウントは、アカウント設定のページで変更してください。".to_string(),
        ));
    }
    let mut tx = pool.begin().await.map_err(|e| AppError::db("begin", e))?;
    // The form-ticket staleness guard (see `form_ticket_service`).
    let current = admin_repository::updated_at_for_update(&mut tx, target.id).await?;
    crate::services::form_ticket_service::ensure_fresh(current, expected)?;
    // The last-owner guard runs inside this transaction,
    // after the target row is locked: its owner count reads
    // rows locked for update, so it cannot be raced.
    refuse_last_owner_loss(&mut tx, target, role, is_active).await?;
    admin_repository::update_admin_account(
        &mut tx,
        target.id,
        display_name,
        role,
        &sanitize_permissions(permissions),
        is_active,
    )
    .await?;
    audit_service::record(&mut tx, actor_id, "admin.update", "admin", Some(target.id)).await?;
    tx.commit().await.map_err(|e| AppError::db("commit", e))?;
    Ok(())
}

/// Sets a new password for another admin's account (the
/// owner-triggered reset): replaces the hash, audits the
/// reset as a credential event, and revokes every session the
/// target holds — a reset exists to cut off whoever might be
/// using the old password, so no session survives it.
///
/// # Errors
///
/// [`AppError::Conflict`] when the target is the actor
/// themselves; [`AppError::NotFound`] when the account does
/// not exist; [`AppError::Db`] on a database failure.
pub async fn reset_password(
    pool: &PgPool,
    actor_id: i64,
    target_id: i64,
    new_pw: &str,
) -> Result<(), AppError> {
    if target_id == actor_id {
        return Err(AppError::Conflict(
            "自分自身のパスワードは、アカウント設定のページで変更してください。".to_string(),
        ));
    }
    if admin_repository::find_admin_auth_by_id(pool, target_id)
        .await?
        .is_none()
    {
        return Err(AppError::NotFound);
    }
    let hash = password::hash_password_async(new_pw.to_string()).await?;
    admin_repository::update_admin_password(pool, target_id, &hash).await?;
    // Credential events audit standalone, like the settings
    // page's own change: a failed insert is logged, never
    // raised — the password has already changed at this point.
    if let Err(e) = admin_repository::insert_audit_standalone(
        pool,
        actor_id,
        "admin.password_reset",
        "admin",
        Some(target_id),
    )
    .await
    {
        tracing::error!(
            admin_id = actor_id,
            "admin password reset audit insert failed: {e}"
        );
    }
    auth_service::revoke_admin_sessions(pool, target_id, None).await;
    Ok(())
}

/// Refuses a change that would leave the store with no active
/// owner: the target is an active owner, the change deactivates
/// or demotes them, and no other active owner exists.
async fn refuse_last_owner_loss(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    target: &AdminSummary,
    new_role: &str,
    new_is_active: bool,
) -> Result<(), AppError> {
    if !target.is_owner() || !target.is_active {
        return Ok(());
    }
    if new_is_active && new_role == "owner" {
        return Ok(());
    }
    if admin_repository::active_owner_ids_for_update(tx, target.id)
        .await?
        .is_empty()
    {
        return Err(AppError::Conflict(
            "最後のオーナーは無効化・降格できません。先に別の管理者をオーナーにしてください。"
                .to_string(),
        ));
    }
    Ok(())
}
