//! Admin account settings.
//!
//! The signed-in admin's own page: display name and password.
//! Until now both were set once by the environment bootstrap
//! and could never change — a leaked bootstrap password would
//! have been permanent. The rules live in
//! `crate::services::admin::auth_service`: the current password
//! is verified (Argon2id), the new one meets the 12-character
//! floor, the admin's other sessions are revoked, and both
//! changes are audited. The login throttle is not involved —
//! this is an authenticated flow, not anonymous guessing.
//!
//! Both forms answer success with a redirect back to this page
//! (the admin surface's Post/Redirect/Get pattern). The password
//! POST also rotates the CSRF token: the middleware re-derives
//! the admin's credential fingerprint after the response and
//! rotates when it moved — no per-path rule is involved.

use axum::{
    Form, Router,
    extract::State,
    response::{IntoResponse, Redirect, Response},
    routing::{get, post},
};
use serde::Deserialize;
use sqlx::PgPool;
use tower_sessions::Session;

use super::{db_error, layout_for, render, set_flash};
use crate::error::AppError;
use crate::models::AdminIdentity;
use crate::services::admin::auth_service;
use crate::session::CurrentAdmin;
use crate::validation::field_error;
use crate::views::admin::account_settings::SettingsView;

/// Routes for the settings surface.
pub fn router() -> Router<PgPool> {
    Router::new()
        .route("/admin/password", get(page).post(change_password))
        .route("/admin/profile", post(update_profile))
}

/// The settings page, freshly rendered.
pub async fn page(
    State(pool): State<PgPool>,
    CurrentAdmin(admin): CurrentAdmin,
    session: Session,
) -> Response {
    render(blank(&pool, &admin, &session).await)
}

/// The password form body. Passwords are never trimmed,
/// logged or echoed back into a page (the login form's rule).
#[derive(Deserialize)]
pub struct PasswordForm {
    /// The current password, as typed.
    current_password: String,
    /// The new password, as typed.
    new_password: String,
    /// The new password again, as typed.
    new_password_confirm: String,
}

/// The profile form body.
#[derive(Deserialize)]
pub struct ProfileForm {
    /// The display name, as typed.
    display_name: String,
}

/// Handles the password change.
pub async fn change_password(
    State(pool): State<PgPool>,
    CurrentAdmin(admin): CurrentAdmin,
    session: Session,
    Form(form): Form<PasswordForm>,
) -> Response {
    // Field errors first, all at once: a refused submit shows
    // every problem together, never one per round trip.
    let errors = auth_service::validate_password_change(
        &form.current_password,
        &form.new_password,
        &form.new_password_confirm,
    );
    if !errors.is_empty() {
        let mut view = blank(&pool, &admin, &session).await;
        view.current_error = field_error(&errors, "current_password");
        view.new_error = field_error(&errors, "new_password");
        view.confirm_error = field_error(&errors, "new_password_confirm");
        return render(view);
    }
    // The current session is passed so the change revokes every
    // other admin session and keeps this one.
    let keep_session_id = session.id().map(|id| id.to_string());
    match auth_service::change_password(
        &pool,
        admin.id,
        &form.current_password,
        &form.new_password,
        keep_session_id,
    )
    .await
    {
        Ok(()) => {
            set_flash(&session, "パスワードを変更しました").await;
            Redirect::to("/admin/password").into_response()
        }
        Err(e @ AppError::Db(_)) => db_error("settings", e),
        Err(e) => {
            let mut view = blank(&pool, &admin, &session).await;
            view.pw_error = Some(e.user_message());
            render(view)
        }
    }
}

/// Handles the display name update.
pub async fn update_profile(
    State(pool): State<PgPool>,
    CurrentAdmin(admin): CurrentAdmin,
    session: Session,
    Form(form): Form<ProfileForm>,
) -> Response {
    match auth_service::update_display_name(&pool, admin.id, &form.display_name).await {
        Ok(_) => {
            set_flash(&session, "表示名を更新しました").await;
            Redirect::to("/admin/password").into_response()
        }
        Err(AppError::Validation(errors)) => {
            let mut view = blank(&pool, &admin, &session).await;
            // The submitted value stays in the input so the
            // operator fixes it instead of retyping.
            view.display_name = form.display_name.clone();
            view.name_error = field_error(&errors, "display_name");
            render(view)
        }
        Err(e @ AppError::Db(_)) => db_error("settings", e),
        Err(e) => {
            let mut view = blank(&pool, &admin, &session).await;
            view.name_error = Some(e.user_message());
            render(view)
        }
    }
}

/// The page with no form state: stored values, no errors.
async fn blank(pool: &PgPool, admin: &AdminIdentity, session: &Session) -> SettingsView {
    settings_view(pool, admin, session, None).await
}

/// The settings page's full state, including the
/// two-factor section: the
/// enrolment state, and the pending setup block while an
/// enrolment is in progress. Shared with the two-factor
/// controller, whose posts answer on this page.
pub(crate) async fn settings_view(
    pool: &PgPool,
    admin: &AdminIdentity,
    session: &Session,
    totp_error: Option<String>,
) -> SettingsView {
    let state = crate::services::admin::totp_service::state(pool, admin.id).await;
    let totp_enabled = state.as_ref().is_some_and(|s| s.is_enabled());
    let totp_enabled_date = state.as_ref().and_then(|s| s.enabled_at).map(|at| {
        at.with_timezone(&crate::models::jst::jst())
            .format("%Y年%m月%d日")
            .to_string()
    });
    let totp_setup = if totp_enabled {
        None
    } else {
        match super::totp::pending_secret(session).await {
            Some(secret) => {
                let uri = crate::services::totp::otpauth_uri(&secret, &admin.email);
                Some(crate::views::admin::totp::AdminTotpSetup {
                    qr_svg: crate::services::totp::qr_svg(&uri).unwrap_or_default(),
                    grouped_secret: crate::services::totp::grouped_secret(&secret),
                })
            }
            None => None,
        }
    };
    SettingsView {
        layout: layout_for(pool, admin, session).await,
        nav: "account_settings",
        email: admin.email.clone(),
        display_name: admin.display_name.clone(),
        name_error: None,
        pw_error: None,
        current_error: None,
        new_error: None,
        confirm_error: None,
        totp_enabled,
        totp_enabled_date,
        totp_setup,
        totp_error,
    }
}
