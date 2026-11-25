//! Admin login and logout.
//!
//! The login page is standalone (no sidebar). The rules, all
//! enforced here or in [`crate::services::admin::auth_service`]:
//! empty fields get per-field messages and never reach the
//! credential check; credential failures share one generic
//! message; a throttled attempt gets its own notice; success
//! cycles the admin session ID (fixation defence), stores the
//! admin ID (the CSRF middleware keeps the token and the
//! credential fingerprint in the same record), stamps
//! last-login, writes the audit row, and lands on the dashboard.

use std::net::SocketAddr;

use axum::{
    Form, Router,
    extract::{ConnectInfo, State},
    http::HeaderMap,
    response::{IntoResponse, Redirect, Response},
    routing::{get, post},
};
use serde::Deserialize;
use sqlx::PgPool;
use tower_sessions::Session;

use super::render;
use crate::messages::{LOGIN_FAILED_MESSAGE, PASSWORD_REQUIRED_MESSAGE};
use crate::services::admin::auth_service::{self, AdminLoginOutcome};
use crate::services::admin::totp_service;
use crate::session::{
    CurrentAdmin, SESSION_ADMIN_ID, SESSION_ADMIN_TOTP_ATTEMPTS, SESSION_ADMIN_TOTP_PENDING,
    SESSION_ADMIN_TOTP_PENDING_SINCE, client_ip,
};
use crate::views::admin::login::LoginView;

/// Routes: login page + submit, and logout.
pub fn router() -> Router<PgPool> {
    Router::new()
        .route("/admin/login", get(login_page).post(login))
        .route("/admin/logout", post(logout))
}

// The generic credential error: identical for unknown email,
// wrong password and lookup failure, so the response never
// reveals whether an admin account exists for the address.

/// The notice shown while the attempt budget is exhausted.
const THROTTLED_NOTICE: &str =
    "ログイン試行回数が上限に達しました。しばらくしてから再度お試しください";

/// Shows the login form. An already-signed-in admin goes
/// straight to the dashboard; everyone else — guests and
/// members alike — simply sees the form (a member session is
/// not an admin session, and the page does not say so).
pub async fn login_page(State(pool): State<PgPool>, session: Session) -> Response {
    if crate::session::current_admin(&pool, &session)
        .await
        .is_some()
    {
        return Redirect::to("/admin").into_response();
    }
    render(LoginView::fresh())
}

/// The login form body.
#[derive(Deserialize)]
pub struct LoginForm {
    /// Email as typed.
    email: String,
    /// Password as typed.
    password: String,
}

/// Handles the login submit.
pub async fn login(
    State(pool): State<PgPool>,
    session: Session,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Form(form): Form<LoginForm>,
) -> Response {
    let email = form.email.trim().to_string();
    // Empty fields: per-field messages, both shown together,
    // and the credential check does not run — an empty submit
    // must not consume the throttle budget either.
    let email_error = email
        .is_empty()
        .then(|| "メールアドレスを入力してください".to_string());
    let password_error = form
        .password
        .is_empty()
        .then(|| PASSWORD_REQUIRED_MESSAGE.to_string());
    if email_error.is_some() || password_error.is_some() {
        return render(LoginView {
            email,
            asset_fingerprint: crate::util::asset_fingerprint::asset_fingerprint(),
            error: None,
            throttled: None,
            email_error,
            password_error,
        });
    }

    let ip = client_ip(&addr, &headers);
    match auth_service::login(&pool, &email, &form.password, &ip).await {
        AdminLoginOutcome::Success(admin) => {
            // Two-factor gate: an
            // enrolled admin is NOT signed in yet — the
            // password only parks a pending marker, and the
            // challenge step (controllers::admin::totp)
            // completes the login. The gate fails CLOSED:
            // a state read error refuses the login rather
            // than reducing an enrolled account to
            // password-only.
            match totp_service::is_enabled_strict(&pool, admin.id).await {
                Ok(true) => {
                    let now = chrono::Utc::now().timestamp();
                    let _ = session.insert(SESSION_ADMIN_TOTP_PENDING, admin.id).await;
                    let _ = session.insert(SESSION_ADMIN_TOTP_PENDING_SINCE, now).await;
                    let _ = session.insert(SESSION_ADMIN_TOTP_ATTEMPTS, 0_i64).await;
                    return Redirect::to("/admin/login/totp").into_response();
                }
                Ok(false) => {}
                Err(e) => {
                    tracing::error!(admin_id = admin.id, "admin login totp gate failed: {e}");
                    return render(LoginView {
                        email,
                        asset_fingerprint: crate::util::asset_fingerprint::asset_fingerprint(),
                        error: Some(LOGIN_FAILED_MESSAGE.to_string()),
                        throttled: None,
                        email_error: None,
                        password_error: None,
                    });
                }
            }
            // Rotate the session ID before storing identity: a
            // session fixed before login must not become the
            // authenticated one (fixation defence).
            let _ = session.cycle_id().await;
            if session.insert(SESSION_ADMIN_ID, admin.id).await.is_err() {
                return render(LoginView {
                    email,
                    asset_fingerprint: crate::util::asset_fingerprint::asset_fingerprint(),
                    error: Some(LOGIN_FAILED_MESSAGE.to_string()),
                    throttled: None,
                    email_error: None,
                    password_error: None,
                });
            }
            // The last-login stamp and the audit row: if they
            // fail, the error is already logged; the login itself
            // stands (the identity write above is the security
            // boundary).
            let _ = auth_service::record_login(&pool, admin.id).await;
            Redirect::to("/admin").into_response()
        }
        AdminLoginOutcome::Throttled => render(LoginView {
            email,
            asset_fingerprint: crate::util::asset_fingerprint::asset_fingerprint(),
            error: None,
            throttled: Some(THROTTLED_NOTICE.to_string()),
            email_error: None,
            password_error: None,
        }),
        AdminLoginOutcome::BadCredentials => render(LoginView {
            email,
            asset_fingerprint: crate::util::asset_fingerprint::asset_fingerprint(),
            error: Some(LOGIN_FAILED_MESSAGE.to_string()),
            throttled: None,
            email_error: None,
            password_error: None,
        }),
    }
}

/// Logs the admin out: clears ONLY the admin session (the
/// member session in the other cookie is untouched) and lands
/// on the login page. POST-only, with the browser's native
/// confirmation wired in admin.js.
pub async fn logout(_admin: CurrentAdmin, session: Session) -> Response {
    let _ = session.delete().await;
    Redirect::to("/admin/login").into_response()
}
