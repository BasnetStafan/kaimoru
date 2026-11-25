//! Two-factor settings controller.
//! The /mypage/totp pages: enrolment (pending secret held
//! PII-encrypted in the session only), recovery code
//! regeneration and disable.
//! Member-only throughout, behind the [`CurrentUser`]
//! extractor.

use std::net::SocketAddr;

use axum::{
    Form, Router,
    extract::{ConnectInfo, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Redirect, Response},
    routing::{get, post},
};
use serde::Deserialize;
use sqlx::PgPool;
use tower_sessions::Session;

use super::{header_for, render};
use crate::error::AppError;
use crate::messages::{
    LOGIN_FAILED_MESSAGE, TOTP_PROOF_ATTEMPTS_EXCEEDED_MESSAGE, VERIFICATION_CODE_MISMATCH_MESSAGE,
};
use crate::middleware::auth_throttle;
use crate::services::{member_account_service, totp, totp_service};
use crate::session::{
    CurrentUser, SESSION_FLASH, SESSION_TOTP_PROOF_ATTEMPTS, SESSION_TOTP_SETUP_ATTEMPTS,
    SESSION_TOTP_SETUP_SECRET, client_ip,
};
use crate::views::storefront::totp::{RecoveryCodesView, TotpSettingsView, TotpSetup};

/// Enrolment confirmation attempts allowed per pending secret;
/// the fifth failure discards the secret, so an attacker with
/// a stolen session cannot grind through the code space.
const MAX_SETUP_ATTEMPTS: i64 = 5;

/// Settings-proof attempts allowed per session for the
/// recovery-code regeneration (the admin cap, mirrored);
/// the (email, IP) throttle accumulates across sessions
/// on top of this cap.
const MAX_PROOF_ATTEMPTS: i64 = 5;

/// Routes owned by the two-factor settings controller.
pub fn router() -> Router<PgPool> {
    Router::new()
        .route("/mypage/totp", get(settings))
        .route("/mypage/totp/setup", post(setup_start))
        .route("/mypage/totp/enable", post(enable))
        .route("/mypage/totp/recovery/regenerate", post(regenerate))
        .route("/mypage/totp/disable", post(disable))
}

/// Form posted by the code steps (enable, regenerate).
#[derive(Deserialize)]
pub struct CodeForm {
    /// The 6-digit authenticator code.
    code: String,
}

/// Form posted by the disable step.
#[derive(Deserialize)]
pub struct DisableForm {
    /// The member's current password.
    password: String,
    /// A current authenticator or recovery code.
    code: String,
}

/// GET /mypage/totp: the settings page in its current state.
/// The flash is taken here so the disable completion message
/// (stashed before its redirect) shows exactly once.
async fn settings(
    CurrentUser(user): CurrentUser,
    session: Session,
    State(pool): State<PgPool>,
) -> Response {
    render_settings(&pool, &session, &user, None, None).await
}

/// POST /mypage/totp/setup: begins an enrolment. The fresh
/// secret is held in the session only — the users row is not
/// written until a code proves the member's app holds it.
async fn setup_start(
    CurrentUser(user): CurrentUser,
    session: Session,
    State(pool): State<PgPool>,
) -> Response {
    if totp_service::is_enabled(&pool, user.id).await {
        return Redirect::to("/mypage/totp").into_response();
    }
    let secret = totp::generate_secret();
    // The session record lives in the database: the pending
    // secret is stored PII-encrypted, so the sessions table
    // never holds a usable plaintext TOTP secret at rest.
    let _ = session
        .insert(
            SESSION_TOTP_SETUP_SECRET,
            crate::pii::encrypt_field(&secret),
        )
        .await;
    let _ = session.insert(SESSION_TOTP_SETUP_ATTEMPTS, 0_i64).await;
    render_settings(&pool, &session, &user, None, None).await
}

/// POST /mypage/totp/enable: confirms the pending enrolment
/// with a code from the member's app. Success persists the
/// secret and issues the first recovery codes, shown once on
/// the answer page. Failure keeps the pending secret for a
/// retry, up to the attempt cap.
async fn enable(
    CurrentUser(user): CurrentUser,
    session: Session,
    State(pool): State<PgPool>,
    Form(form): Form<CodeForm>,
) -> Response {
    let Some(secret) = pending_secret(&session).await else {
        return Redirect::to("/mypage/totp").into_response();
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
                &session,
                &user,
                Some(
                    "認証コードの試行回数が上限に達しました。設定をはじめからやり直してください。"
                        .to_string(),
                ),
                None,
            )
            .await;
        }
        let _ = session.insert(SESSION_TOTP_SETUP_ATTEMPTS, attempts).await;
        return render_settings(
            &pool,
            &session,
            &user,
            Some(VERIFICATION_CODE_MISMATCH_MESSAGE.to_string()),
            None,
        )
        .await;
    }
    clear_pending(&session).await;
    match totp_service::enable(&pool, user.id, &secret).await {
        Ok(codes) => render(RecoveryCodesView {
            header: header_for(&pool, &session, false).await,
            codes,
        }),
        Err(e) => {
            tracing::error!(user_id = user.id, "totp enable failed: {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, "Internal Server Error").into_response()
        }
    }
}

/// POST /mypage/totp/recovery/regenerate: issues a fresh
/// recovery code set (every earlier code dies) after a current
/// authenticator code proves the member still holds the app.
/// The proof carries the admin ceremony's defences: the
/// (email, IP) throttle and a per-session attempt cap —
/// minting a recovery set is a full second-factor bypass,
/// so an uncapped check would be a brute-force oracle for
/// the secret.
async fn regenerate(
    CurrentUser(user): CurrentUser,
    session: Session,
    State(pool): State<PgPool>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Form(form): Form<CodeForm>,
) -> Response {
    let Some(state) = totp_service::state(&pool, user.id).await else {
        return Redirect::to("/mypage/totp").into_response();
    };
    if !state.is_enabled() {
        return Redirect::to("/mypage/totp").into_response();
    }
    let ip = client_ip(&addr, &headers);
    if auth_throttle::is_blocked(&user.email, &ip) {
        return render_settings(
            &pool,
            &session,
            &user,
            Some(LOGIN_FAILED_MESSAGE.to_string()),
            None,
        )
        .await;
    }
    let now = chrono::Utc::now().timestamp().max(0) as u64;
    let verified = state
        .secret
        .as_deref()
        .is_some_and(|secret| totp::verify(secret, &form.code, now));
    if !verified {
        auth_throttle::record_failure(&user.email, &ip);
        let attempts = session
            .get::<i64>(SESSION_TOTP_PROOF_ATTEMPTS)
            .await
            .ok()
            .flatten()
            .unwrap_or(0)
            + 1;
        if attempts >= MAX_PROOF_ATTEMPTS {
            let _ = session.remove::<i64>(SESSION_TOTP_PROOF_ATTEMPTS).await;
            return render_settings(
                &pool,
                &session,
                &user,
                Some(TOTP_PROOF_ATTEMPTS_EXCEEDED_MESSAGE.to_string()),
                None,
            )
            .await;
        }
        let _ = session.insert(SESSION_TOTP_PROOF_ATTEMPTS, attempts).await;
        return render_settings(
            &pool,
            &session,
            &user,
            Some(VERIFICATION_CODE_MISMATCH_MESSAGE.to_string()),
            None,
        )
        .await;
    }
    let _ = session.remove::<i64>(SESSION_TOTP_PROOF_ATTEMPTS).await;
    auth_throttle::record_success(&user.email, &ip);
    match totp_service::issue_recovery_codes(&pool, user.id).await {
        Ok(codes) => render(RecoveryCodesView {
            header: header_for(&pool, &session, false).await,
            codes,
        }),
        Err(e) => {
            tracing::error!(user_id = user.id, "recovery regenerate failed: {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, "Internal Server Error").into_response()
        }
    }
}

/// POST /mypage/totp/disable: the current password plus a
/// current code prove the member, then the enrolment is
/// cleared and every other session is revoked — the password
/// change's rule for credential changes. Failed ceremonies
/// feed the (email, IP) throttle like every other member
/// credential check, and a blocked pair never reaches
/// verification.
async fn disable(
    CurrentUser(user): CurrentUser,
    session: Session,
    State(pool): State<PgPool>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Form(form): Form<DisableForm>,
) -> Response {
    let ip = client_ip(&addr, &headers);
    if auth_throttle::is_blocked(&user.email, &ip) {
        return render_settings(
            &pool,
            &session,
            &user,
            Some(LOGIN_FAILED_MESSAGE.to_string()),
            None,
        )
        .await;
    }
    match totp_service::disable_with_proof(&pool, user.id, &form.password, &form.code).await {
        Ok(()) => {
            auth_throttle::record_success(&user.email, &ip);
            let keep_session_id = session.id().map(|id| id.to_string());
            member_account_service::revoke_other_sessions(&pool, user.id, keep_session_id).await;
            let _ = session
                .insert(SESSION_FLASH, "2段階認証を無効にしました。")
                .await;
            Redirect::to("/mypage/totp").into_response()
        }
        Err(e) => {
            // A database failure is not a failed proof: only
            // refused ceremonies feed the throttle.
            if !matches!(e, AppError::Db(_)) {
                auth_throttle::record_failure(&user.email, &ip);
            }
            render_settings(&pool, &session, &user, Some(e.user_message()), None).await
        }
    }
}

/// The pending enrolment secret held by this session, if any.
async fn pending_secret(session: &Session) -> Option<String> {
    let stored = session
        .get::<String>(SESSION_TOTP_SETUP_SECRET)
        .await
        .ok()
        .flatten()?;
    // Stored encrypted (see `setup_start`); the checked
    // decrypt fails closed — an undecryptable or legacy
    // plaintext value reads as no pending enrolment, and
    // the member simply starts the setup again.
    crate::pii::decrypt_field_checked(&stored)
}

/// Drops the pending enrolment state from the session.
async fn clear_pending(session: &Session) {
    let _ = session.remove::<String>(SESSION_TOTP_SETUP_SECRET).await;
    let _ = session.remove::<i64>(SESSION_TOTP_SETUP_ATTEMPTS).await;
}

/// Renders the settings page for the member's current state,
/// with the pending setup block when an enrolment is in
/// progress (a page reload mid-setup keeps the same QR).
async fn render_settings(
    pool: &PgPool,
    session: &Session,
    user: &crate::models::User,
    error: Option<String>,
    notice: Option<String>,
) -> Response {
    let state = totp_service::state(pool, user.id).await;
    let enabled = state.as_ref().is_some_and(|s| s.is_enabled());
    let enabled_date = state.as_ref().and_then(|s| s.enabled_at).map(|at| {
        at.with_timezone(&crate::models::jst::jst())
            .format("%Y年%m月%d日")
            .to_string()
    });
    let setup = if enabled {
        None
    } else {
        match pending_secret(session).await {
            Some(secret) => {
                let uri = totp::otpauth_uri(&secret, &user.email);
                Some(TotpSetup {
                    qr_svg: totp::qr_svg(&uri).unwrap_or_default(),
                    grouped_secret: totp::grouped_secret(&secret),
                })
            }
            None => None,
        }
    };
    // take_flash=true: the disable completion message is
    // stashed for exactly this page.
    let header = header_for(pool, session, true).await;
    render(TotpSettingsView {
        header,
        enabled,
        enabled_date,
        setup,
        error,
        notice,
    })
}
