//! Admin two-factor controller.
//!
//! The settings posts live under the account settings page
//! (`/admin/password` renders the section); the login
//! challenge lives beside the login form. Both mirror the
//! member flows in `crate::controllers::storefront::totp`
//! and `crate::controllers::storefront::login`: the pending
//! enrolment secret is held PII-encrypted in the session
//! only, the challenge is windowed and attempt-capped, and
//! a wrong code never says which part failed.

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

use super::{db_error, render};
use crate::error::AppError;
use crate::messages::{
    LOGIN_FAILED_MESSAGE, TOTP_PROOF_ATTEMPTS_EXCEEDED_MESSAGE, VERIFICATION_CODE_MISMATCH_MESSAGE,
};
use crate::middleware::auth_throttle;
use crate::services::admin::auth_service;
use crate::services::admin::totp_service;
use crate::services::totp;
use crate::session::{
    CurrentAdmin, SESSION_ADMIN_ID, SESSION_ADMIN_TOTP_ATTEMPTS, SESSION_ADMIN_TOTP_PENDING,
    SESSION_ADMIN_TOTP_PENDING_SINCE, SESSION_ADMIN_TOTP_PROOF_ATTEMPTS,
    SESSION_TOTP_SETUP_ATTEMPTS, SESSION_TOTP_SETUP_SECRET, client_ip,
};
use crate::views::admin::totp::{AdminRecoveryCodesView, TotpChallengeView};

/// Enrolment confirmation attempts allowed per pending
/// secret; the fifth failure discards the secret.
const MAX_SETUP_ATTEMPTS: i64 = 5;

/// How long a password-verified challenge stays
/// completable (the member window).
const TOTP_PENDING_WINDOW_SECS: i64 = 600;

/// Challenge attempts allowed per pending login; at the
/// cap the pending login is discarded (the member cap).
const MAX_TOTP_ATTEMPTS: i64 = 5;

/// Settings-proof attempts allowed per session for the
/// recovery-code regeneration; the
/// (email, IP) throttle accumulates across sessions on
/// top of this cap.
const MAX_PROOF_ATTEMPTS: i64 = 5;

/// Routes: the settings posts and the login challenge.
pub fn router() -> Router<PgPool> {
    Router::new()
        .route("/admin/totp/setup", post(setup_start))
        .route("/admin/totp/enable", post(enable))
        .route("/admin/totp/recovery/regenerate", post(regenerate))
        .route("/admin/totp/disable", post(disable))
        .route(
            "/admin/login/totp",
            get(challenge_page).post(challenge_submit),
        )
}

/// Form posted by the code steps (enable, regenerate,
/// challenge).
#[derive(Deserialize)]
pub struct CodeForm {
    /// A 6-digit authenticator code, or a recovery code
    /// where the flow accepts one.
    code: String,
}

/// Form posted by the disable step.
#[derive(Deserialize)]
pub struct DisableForm {
    /// The admin's current password.
    password: String,
    /// A current authenticator or recovery code.
    code: String,
}

/// POST /admin/totp/setup: begins an enrolment. The fresh
/// secret is held in the session only — the admins row is
/// not written until a code proves the operator's app
/// holds it.
async fn setup_start(
    CurrentAdmin(admin): CurrentAdmin,
    session: Session,
    State(pool): State<PgPool>,
) -> Response {
    if totp_service::is_enabled(&pool, admin.id).await {
        return Redirect::to("/admin/password").into_response();
    }
    let secret = totp::generate_secret();
    // The session record lives in the database: the
    // pending secret is stored PII-encrypted, so the
    // sessions table never holds a usable plaintext TOTP
    // secret at rest.
    let _ = session
        .insert(
            SESSION_TOTP_SETUP_SECRET,
            crate::pii::encrypt_field(&secret),
        )
        .await;
    let _ = session.insert(SESSION_TOTP_SETUP_ATTEMPTS, 0_i64).await;
    render_settings(&pool, &admin, &session, None).await
}

/// POST /admin/totp/enable: confirms the pending enrolment
/// with a code from the operator's app. Success persists
/// the secret and issues the first recovery codes, shown
/// once. Failure keeps the pending secret for a retry, up
/// to the attempt cap.
async fn enable(
    CurrentAdmin(admin): CurrentAdmin,
    session: Session,
    State(pool): State<PgPool>,
    Form(form): Form<CodeForm>,
) -> Response {
    let Some(secret) = pending_secret(&session).await else {
        return Redirect::to("/admin/password").into_response();
    };
    let now = chrono::Utc::now().timestamp().max(0) as u64;
    if !totp::verify(&secret, &form.code, now) {
        let attempts = session
            .get::<i64>(SESSION_TOTP_SETUP_ATTEMPTS)
            .await
            .ok()
            .flatten()
            .unwrap_or(0)
            + 1;
        if attempts >= MAX_SETUP_ATTEMPTS {
            clear_pending(&session).await;
            return render_settings(
                &pool,
                &admin,
                &session,
                Some(
                    "認証コードの試行回数が上限に達しました。設定をはじめからやり直してください。"
                        .to_string(),
                ),
            )
            .await;
        }
        let _ = session.insert(SESSION_TOTP_SETUP_ATTEMPTS, attempts).await;
        return render_settings(
            &pool,
            &admin,
            &session,
            Some(VERIFICATION_CODE_MISMATCH_MESSAGE.to_string()),
        )
        .await;
    }
    clear_pending(&session).await;
    match totp_service::enable(&pool, admin.id, &secret).await {
        Ok(codes) => render(AdminRecoveryCodesView {
            layout: super::layout_for(&pool, &admin, &session).await,
            nav: "account_settings",
            codes,
        }),
        Err(e) => db_error("admin totp enable", e),
    }
}

/// POST /admin/totp/recovery/regenerate: issues a fresh
/// recovery code set (every earlier code dies) after a
/// current authenticator code proves the operator still
/// holds the app. The proof carries the login challenge's
/// defences: the (email, IP)
/// throttle and a per-session attempt cap — minting a
/// recovery set is a full second-factor bypass, so the
/// uncapped check that stood here before was a
/// brute-force oracle for the secret.
async fn regenerate(
    CurrentAdmin(admin): CurrentAdmin,
    session: Session,
    State(pool): State<PgPool>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Form(form): Form<CodeForm>,
) -> Response {
    let Some(state) = totp_service::state(&pool, admin.id).await else {
        return Redirect::to("/admin/password").into_response();
    };
    if !state.is_enabled() {
        return Redirect::to("/admin/password").into_response();
    }
    let ip = client_ip(&addr, &headers);
    if auth_throttle::admin_is_blocked(&admin.email, &ip) {
        return render_settings(
            &pool,
            &admin,
            &session,
            Some(LOGIN_FAILED_MESSAGE.to_string()),
        )
        .await;
    }
    if !totp_service::verify_proof_code(&pool, admin.id, &form.code).await {
        auth_throttle::admin_record_failure(&admin.email, &ip);
        let attempts = session
            .get::<i64>(SESSION_ADMIN_TOTP_PROOF_ATTEMPTS)
            .await
            .ok()
            .flatten()
            .unwrap_or(0)
            + 1;
        if attempts >= MAX_PROOF_ATTEMPTS {
            let _ = session
                .remove::<i64>(SESSION_ADMIN_TOTP_PROOF_ATTEMPTS)
                .await;
            return render_settings(
                &pool,
                &admin,
                &session,
                Some(TOTP_PROOF_ATTEMPTS_EXCEEDED_MESSAGE.to_string()),
            )
            .await;
        }
        let _ = session
            .insert(SESSION_ADMIN_TOTP_PROOF_ATTEMPTS, attempts)
            .await;
        return render_settings(
            &pool,
            &admin,
            &session,
            Some(VERIFICATION_CODE_MISMATCH_MESSAGE.to_string()),
        )
        .await;
    }
    let _ = session
        .remove::<i64>(SESSION_ADMIN_TOTP_PROOF_ATTEMPTS)
        .await;
    auth_throttle::admin_record_success(&admin.email, &ip);
    match totp_service::issue_recovery_codes(&pool, admin.id).await {
        Ok(codes) => render(AdminRecoveryCodesView {
            layout: super::layout_for(&pool, &admin, &session).await,
            nav: "account_settings",
            codes,
        }),
        Err(e) => db_error("admin totp recovery", e),
    }
}

/// POST /admin/totp/disable: the current password plus a
/// current code prove the operator, then the enrolment is
/// cleared. Unlike the member flow, the admin's other
/// sessions are NOT revoked here: the password did not
/// change, and the operator performing the disable keeps
/// working — the next login simply skips the challenge.
/// Failed ceremonies feed the (email, IP) throttle like
/// every other admin credential check, and a blocked pair never reaches verification.
async fn disable(
    CurrentAdmin(admin): CurrentAdmin,
    session: Session,
    State(pool): State<PgPool>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Form(form): Form<DisableForm>,
) -> Response {
    let ip = client_ip(&addr, &headers);
    if auth_throttle::admin_is_blocked(&admin.email, &ip) {
        return render_settings(
            &pool,
            &admin,
            &session,
            Some(LOGIN_FAILED_MESSAGE.to_string()),
        )
        .await;
    }
    match totp_service::disable_with_proof(&pool, admin.id, &form.password, &form.code).await {
        Ok(()) => {
            auth_throttle::admin_record_success(&admin.email, &ip);
            super::set_flash(&session, "2段階認証を無効にしました").await;
            Redirect::to("/admin/password").into_response()
        }
        Err(e @ AppError::Db(_)) => db_error("admin totp disable", e),
        Err(e) => {
            auth_throttle::admin_record_failure(&admin.email, &ip);
            render_settings(&pool, &admin, &session, Some(e.user_message())).await
        }
    }
}

/// GET /admin/login/totp: the challenge page. Only a
/// session holding a live pending marker may see it;
/// everyone else goes to the login page (a signed-in
/// admin to the dashboard).
async fn challenge_page(State(pool): State<PgPool>, session: Session) -> Response {
    if crate::session::current_admin(&pool, &session)
        .await
        .is_some()
    {
        return Redirect::to("/admin").into_response();
    }
    if pending_admin(&session).await.is_none() {
        return Redirect::to("/admin/login").into_response();
    }
    render(TotpChallengeView {
        error: None,
        asset_fingerprint: crate::util::asset_fingerprint::asset_fingerprint(),
    })
}

/// POST /admin/login/totp: verifies the challenge code
/// and completes the login (session cycle, identity,
/// last-login stamp and audit — the password step's own
/// completion, deferred to here). Failures count against
/// the attempt cap and the admin login throttle; at the
/// cap the pending login is discarded.
async fn challenge_submit(
    State(pool): State<PgPool>,
    session: Session,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Form(form): Form<CodeForm>,
) -> Response {
    let ip = client_ip(&addr, &headers);
    let Some(admin_id) = pending_admin(&session).await else {
        return Redirect::to("/admin/login").into_response();
    };
    // The pending admin, re-read: a deactivated or deleted
    // account ends the challenge here, never completes it.
    let admin =
        match crate::repositories::admin::admin_repository::find_admin_auth_by_id(&pool, admin_id)
            .await
        {
            Ok(Some(admin)) if admin.is_active => admin,
            _ => {
                clear_pending_login(&session).await;
                return Redirect::to("/admin/login").into_response();
            }
        };
    if auth_throttle::admin_is_blocked(&admin.email, &ip) {
        return render(TotpChallengeView {
            error: Some(LOGIN_FAILED_MESSAGE.to_string()),
            asset_fingerprint: crate::util::asset_fingerprint::asset_fingerprint(),
        });
    }
    if totp_service::verify_login_code(&pool, admin_id, &form.code).await {
        clear_pending_login(&session).await;
        auth_throttle::admin_record_success(&admin.email, &ip);
        // The password step's completion, deferred: rotate
        // the session ID before storing identity (fixation
        // defence), then the stamp and audit.
        let _ = session.cycle_id().await;
        if session.insert(SESSION_ADMIN_ID, admin.id).await.is_err() {
            return render(TotpChallengeView {
                error: Some(LOGIN_FAILED_MESSAGE.to_string()),
                asset_fingerprint: crate::util::asset_fingerprint::asset_fingerprint(),
            });
        }
        let _ = auth_service::record_login(&pool, admin.id).await;
        return Redirect::to("/admin").into_response();
    }
    auth_throttle::admin_record_failure(&admin.email, &ip);
    let attempts = session
        .get::<i64>(SESSION_ADMIN_TOTP_ATTEMPTS)
        .await
        .ok()
        .flatten()
        .unwrap_or(0)
        + 1;
    if attempts >= MAX_TOTP_ATTEMPTS {
        clear_pending_login(&session).await;
        return Redirect::to("/admin/login").into_response();
    }
    let _ = session.insert(SESSION_ADMIN_TOTP_ATTEMPTS, attempts).await;
    render(TotpChallengeView {
        error: Some(VERIFICATION_CODE_MISMATCH_MESSAGE.to_string()),
        asset_fingerprint: crate::util::asset_fingerprint::asset_fingerprint(),
    })
}

/// The pending enrolment secret held by this session, if
/// any (stored PII-encrypted; an undecryptable value
/// reads as no pending enrolment).
pub(crate) async fn pending_secret(session: &Session) -> Option<String> {
    let stored = session
        .get::<String>(SESSION_TOTP_SETUP_SECRET)
        .await
        .ok()
        .flatten()?;
    crate::pii::decrypt_field_checked(&stored)
}

/// Drops the pending enrolment state from the session.
async fn clear_pending(session: &Session) {
    let _ = session.remove::<String>(SESSION_TOTP_SETUP_SECRET).await;
    let _ = session.remove::<i64>(SESSION_TOTP_SETUP_ATTEMPTS).await;
}

/// The pending two-factor login held by this session,
/// when it exists and is still inside its window. An
/// expired marker is cleared and reported as absent.
async fn pending_admin(session: &Session) -> Option<i64> {
    let admin_id = session
        .get::<i64>(SESSION_ADMIN_TOTP_PENDING)
        .await
        .ok()
        .flatten()?;
    let since = session
        .get::<i64>(SESSION_ADMIN_TOTP_PENDING_SINCE)
        .await
        .ok()
        .flatten()?;
    if chrono::Utc::now().timestamp() - since > TOTP_PENDING_WINDOW_SECS {
        clear_pending_login(session).await;
        return None;
    }
    Some(admin_id)
}

/// Drops every pending-challenge key from the session.
async fn clear_pending_login(session: &Session) {
    let _ = session.remove::<i64>(SESSION_ADMIN_TOTP_PENDING).await;
    let _ = session
        .remove::<i64>(SESSION_ADMIN_TOTP_PENDING_SINCE)
        .await;
    let _ = session.remove::<i64>(SESSION_ADMIN_TOTP_ATTEMPTS).await;
}

/// Renders the account settings page with a two-factor
/// error (the settings posts answer in place, like the
/// member settings do).
async fn render_settings(
    pool: &PgPool,
    admin: &crate::models::AdminIdentity,
    session: &Session,
    totp_error: Option<String>,
) -> Response {
    render(super::account::settings_view(pool, admin, session, totp_error).await)
}
