//! Password reset controller.
//! The forgotten-password flow: email entry, code
//! verification, new password. Rules live in the auth service;
//! this layer moves the browser between the steps.
//!
//! Anti-enumeration is the design centre: every response to the
//! request and code steps is identical whether or not the
//! submitted address has an account, so the flow can never be
//! used to test which addresses are registered. A code
//! submission with no challenge behind it (the unknown-address
//! case) is answered with the same wrong-code error a real
//! wrong code gets, and the pages never show the address.

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

use super::{header_for, render};
use crate::messages::{
    PASSWORD_CONFIRM_MISMATCH_MESSAGE, TOO_MANY_REQUESTS_MESSAGE,
    VERIFICATION_CODE_MISMATCH_MESSAGE, VERIFICATION_CODE_RESENT_NOTICE,
};
use crate::middleware::auth_throttle;
use crate::models::CodeChallenge;
use crate::repositories::challenge_repository::ChallengeTable;
use crate::services::member_account_service;
use crate::session::{SESSION_FLASH, SESSION_RESET_ID, reset_id};
use crate::views::storefront::password_reset::{
    ResetPasswordView, ResetRequestView, ResetVerifyView,
};

// The wrong-code message for a code submitted with no challenge
// behind it. A real wrong code is answered by the service with
// its attempts-remaining message; this one names no count,
// because there is nothing to count — and its first sentence
// matches, so the two cases read the same at a glance.

/// Routes owned by the password reset controller.
pub fn router() -> Router<PgPool> {
    Router::new()
        .route("/password-reset", get(request_page).post(request_submit))
        .route(
            "/password-reset/verify",
            get(verify_page).post(verify_submit),
        )
        .route("/password-reset/resend", post(resend_submit))
        .route(
            "/password-reset/password",
            get(password_page).post(password_submit),
        )
}

/// Form posted by the email entry step.
#[derive(Deserialize)]
pub struct ResetRequestForm {
    email: String,
}

/// Form posted by the code entry step.
#[derive(Deserialize)]
pub struct ResetCodeForm {
    code: String,
}

/// Form posted by the new password step.
#[derive(Deserialize)]
pub struct ResetPasswordForm {
    new_password: String,
    new_password_confirm: String,
}

/// GET /password-reset: the email entry page.
async fn request_page(session: Session, State(pool): State<PgPool>) -> Response {
    render(ResetRequestView {
        header: header_for(&pool, &session, false).await,
        error: None,
        email: String::new(),
    })
}

/// POST /password-reset: starts the reset and moves to the code
/// step. The redirect is unconditional (see the module docs):
/// the service decides silently whether a code was really sent.
async fn request_submit(
    session: Session,
    State(pool): State<PgPool>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Form(form): Form<ResetRequestForm>,
) -> Response {
    // Starting a reset sends a real email: throttle by IP and by
    // address before any mail is sent, exactly as at sign-up
    // start. The counter is fed for every submitted address,
    // member or not, so the refusal itself reveals nothing.
    let ip = crate::session::client_ip(&addr, &headers);
    if !auth_throttle::reset_start_allowed(&form.email, &ip) {
        return render(ResetRequestView {
            header: header_for(&pool, &session, false).await,
            error: Some(TOO_MANY_REQUESTS_MESSAGE.to_string()),
            email: form.email.clone(),
        });
    }
    match member_account_service::start_password_reset(&pool, &form.email).await {
        Ok(maybe_id) => {
            if let Some(id) = maybe_id {
                let _ = session.insert(SESSION_RESET_ID, id).await;
            }
            Redirect::to("/password-reset/verify").into_response()
        }
        Err(e) => render(ResetRequestView {
            header: header_for(&pool, &session, false).await,
            error: Some(e.user_message()),
            email: form.email.clone(),
        }),
    }
}

/// GET /password-reset/verify: the code entry page. Rendered
/// whether or not this session holds a challenge — sending a
/// visitor who never started a reset back to the start page
/// would make the unknown-address case (which holds no
/// challenge) look different from the real one.
async fn verify_page(session: Session, State(pool): State<PgPool>) -> Response {
    render(ResetVerifyView {
        header: header_for(&pool, &session, false).await,
        error: None,
        notice: None,
    })
}

/// POST /password-reset/verify: checks the entered code. With no
/// challenge behind the session, the answer is the generic
/// wrong-code error (see the module docs).
async fn verify_submit(
    session: Session,
    State(pool): State<PgPool>,
    Form(form): Form<ResetCodeForm>,
) -> Response {
    let header = header_for(&pool, &session, false).await;
    let Some(challenge) = load_flow_challenge(&session, &pool).await else {
        return render(ResetVerifyView {
            header,
            error: Some(VERIFICATION_CODE_MISMATCH_MESSAGE.to_string()),
            notice: None,
        });
    };
    match member_account_service::verify_challenge(
        &pool,
        ChallengeTable::PasswordResets,
        &challenge,
        &form.code,
    )
    .await
    {
        Ok(()) => Redirect::to("/password-reset/password").into_response(),
        Err(e) => render(ResetVerifyView {
            header,
            error: Some(e.user_message()),
            notice: None,
        }),
    }
}

/// POST /password-reset/resend: sends a fresh code under the
/// cooldown rules. With no challenge behind the session, the
/// page answers with the same notice a real resend gets — the
/// alternative (an error, or a redirect away) would again mark
/// the unknown-address case as different.
async fn resend_submit(session: Session, State(pool): State<PgPool>) -> Response {
    let header = header_for(&pool, &session, false).await;
    let Some(challenge) = load_flow_challenge(&session, &pool).await else {
        return render(ResetVerifyView {
            header,
            error: None,
            notice: Some(VERIFICATION_CODE_RESENT_NOTICE.to_string()),
        });
    };
    let (error, notice) = match member_account_service::resend_challenge(
        &pool,
        ChallengeTable::PasswordResets,
        &challenge,
    )
    .await
    {
        Ok(()) => (None, Some(VERIFICATION_CODE_RESENT_NOTICE.to_string())),
        Err(e) => (Some(e.user_message()), None),
    };
    render(ResetVerifyView {
        header,
        error,
        notice,
    })
}

/// GET /password-reset/password: the new password form, only
/// after the code has verified. Earlier steps bounce the
/// visitor back to where the flow actually stands.
async fn password_page(session: Session, State(pool): State<PgPool>) -> Response {
    let Some(challenge) = load_flow_challenge(&session, &pool).await else {
        return Redirect::to("/password-reset").into_response();
    };
    if !challenge.verified {
        return Redirect::to("/password-reset/verify").into_response();
    }
    render(ResetPasswordView {
        header: header_for(&pool, &session, false).await,
        error: None,
    })
}

/// POST /password-reset/password: replaces the password,
/// consumes the challenge and revokes every session of the
/// member (both inside the service), then sends the member to
/// the login page with a one-time message. This session is a
/// guest throughout — it holds no principal, so there is no
/// credential fingerprint for the CSRF middleware to move;
/// the member's next sign-in rotates the token as the
/// principal changes.
async fn password_submit(
    session: Session,
    State(pool): State<PgPool>,
    Form(form): Form<ResetPasswordForm>,
) -> Response {
    let Some(challenge) = load_flow_challenge(&session, &pool).await else {
        return Redirect::to("/password-reset").into_response();
    };
    if !challenge.verified {
        return Redirect::to("/password-reset/verify").into_response();
    }
    if form.new_password != form.new_password_confirm {
        return render(ResetPasswordView {
            header: header_for(&pool, &session, false).await,
            error: Some(PASSWORD_CONFIRM_MISMATCH_MESSAGE.to_string()),
        });
    }
    match member_account_service::complete_password_reset(&pool, &challenge, &form.new_password)
        .await
    {
        Ok(()) => {
            let _ = session.remove::<i64>(SESSION_RESET_ID).await;
            let _ = session
                .insert(
                    SESSION_FLASH,
                    "パスワードを再設定しました。新しいパスワードでログインしてください。",
                )
                .await;
            Redirect::to("/login").into_response()
        }
        Err(e) => render(ResetPasswordView {
            header: header_for(&pool, &session, false).await,
            error: Some(e.user_message()),
        }),
    }
}

/// Loads the password reset this session is working through.
async fn load_flow_challenge(session: &Session, pool: &PgPool) -> Option<CodeChallenge> {
    let id = reset_id(session).await?;
    member_account_service::load_challenge(pool, ChallengeTable::PasswordResets, id)
        .await
        .ok()
}
