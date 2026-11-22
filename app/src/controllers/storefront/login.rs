//! Login controller.
//! Login and logout only. Sign-up lives in the signup controller.
//! Throttled attempts, the validated `next` return path, and the
//! generic failure message that reveals nothing about which part
//! was wrong.

use std::net::SocketAddr;

use axum::{
    Form, Router,
    extract::{ConnectInfo, Query, State},
    response::{IntoResponse, Redirect, Response},
    routing::{get, post},
};
use serde::Deserialize;
use sqlx::PgPool;
use tower_sessions::Session;

use super::{header_for, render};
use crate::messages::{LOGIN_FAILED_MESSAGE, VERIFICATION_CODE_MISMATCH_MESSAGE};
use crate::middleware::auth_throttle;
use crate::services::cart_service;
use crate::services::member_account_service;
use crate::services::totp_service;
use crate::session::{
    SESSION_FLASH, SESSION_TOTP_PENDING_NEXT, SESSION_TOTP_PENDING_SINCE,
    SESSION_TOTP_PENDING_USER, SESSION_USER_ID, current_user,
};
use crate::views::storefront::login::{LoginTotpView, LoginView};
use crate::views::storefront::sns::sns_block;

/// Session key for the two-factor challenge's failed attempts.
/// At the cap the pending login is discarded, so a stolen
/// password alone can never grind through the code space.
const SESSION_TOTP_ATTEMPTS: &str = "totp_attempts";

/// How long a password-verified login may wait for its
/// two-factor code, in seconds.
const TOTP_PENDING_WINDOW_SECS: i64 = 600;

/// Failed challenge codes allowed before the pending login is
/// discarded.
const MAX_TOTP_ATTEMPTS: i64 = 5;

// The one failure message for every login failure.
// Identical to the auth service message, and also used when an
// attempt is refused by the throttle, so throttling is invisible
// to the person trying.

/// Routes owned by the login controller.
pub fn router() -> Router<PgPool> {
    Router::new()
        .route("/login", get(login_page).post(login_submit))
        .route("/login/totp", get(login_totp_page).post(login_totp_submit))
        .route("/logout", post(logout_submit))
}

/// Query parameters of the login page.
#[derive(Deserialize)]
pub struct LoginQuery {
    /// Return path requested by a member-only page, if any.
    next: Option<String>,
}

/// Form posted by the login page.
#[derive(Deserialize)]
pub struct LoginForm {
    email: String,
    password: String,
    /// Return path carried from the login page, empty for none.
    #[serde(default)]
    next: String,
}

/// GET /login: the login page. Members already logged in go home.
/// A valid `next` is carried into the form so a successful login can
/// return the member to the page they were bounced from.
async fn login_page(
    session: Session,
    State(pool): State<PgPool>,
    Query(query): Query<LoginQuery>,
) -> Response {
    if current_user(&pool, &session).await.is_some() {
        return Redirect::to("/").into_response();
    }
    let next = safe_next(query.next.as_deref()).unwrap_or_default();
    render(LoginView {
        // The flash is taken here (not only on the top page):
        // the password reset flow lands on this page with its
        // completion message.
        header: header_for(&pool, &session, true).await,
        error: None,
        email: String::new(),
        sns: sns_block(&next, false),
        next,
    })
}

/// POST /login: checks the throttle, then the credentials, and starts
/// the member session on success.
async fn login_submit(
    session: Session,
    State(pool): State<PgPool>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: axum::http::HeaderMap,
    Form(form): Form<LoginForm>,
) -> Response {
    let ip = crate::session::client_ip(&addr, &headers);
    let next = safe_next(Some(form.next.as_str())).unwrap_or_default();
    let back = async |message: String| LoginView {
        header: header_for(&pool, &session, false).await,
        error: Some(message),
        email: form.email.clone(),
        sns: sns_block(&next, false),
        next: next.clone(),
    };

    // Spec: locked attempts are refused before any credential check,
    // with the same message as any other failure.
    if auth_throttle::is_blocked(&form.email, &ip) {
        tracing::info!(action = "login_blocked");
        return render(back(LOGIN_FAILED_MESSAGE.to_string()).await);
    }

    match member_account_service::login(&pool, &form.email, &form.password).await {
        Ok(user) => {
            auth_throttle::record_success(&form.email, &ip);
            complete_login(&pool, &session, &user, &next).await
        }
        Err(e) => {
            auth_throttle::record_failure(&form.email, &ip);
            render(back(e.user_message()).await)
        }
    }
}

/// Completes a login whose first factor has verified, from
/// either the password form or an SNS provider: two-factor
/// members are not logged in yet - the first factor only parks
/// a pending marker (never SESSION_USER_ID, never the cart
/// merge) and the challenge step completes the login through
/// [`finish_login`], with the return path preserved. Sharing
/// this helper is what lets an SNS sign-in into a two-factor
/// account behave exactly like a password sign-in.
pub(crate) async fn complete_login(
    pool: &PgPool,
    session: &Session,
    user: &crate::models::User,
    next: &str,
) -> Response {
    // SEC-2: the two-factor gate fails CLOSED - a state read
    // error refuses the login (the error was logged when the
    // AppError was created) instead of silently reducing an
    // enrolled account to password-only.
    let two_factor = match totp_service::is_enabled_strict(pool, user.id).await {
        Ok(enabled) => enabled,
        Err(_) => return Redirect::to("/login").into_response(),
    };
    if two_factor {
        let now = chrono::Utc::now().timestamp();
        let _ = session.insert(SESSION_TOTP_PENDING_USER, user.id).await;
        let _ = session.insert(SESSION_TOTP_PENDING_SINCE, now).await;
        let _ = session.insert(SESSION_TOTP_PENDING_NEXT, next).await;
        let _ = session.insert(SESSION_TOTP_ATTEMPTS, 0_i64).await;
        return Redirect::to("/login/totp").into_response();
    }
    finish_login(pool, session, user, next).await
}

/// Completes a login whose factors have all verified: a fresh
/// session ID (session fixation defence), the user marker, and
/// the guest cart merge, then the validated return path. Used
/// by the plain password path and by the two-factor challenge
/// alike, so the two can never drift.
async fn finish_login(
    pool: &PgPool,
    session: &Session,
    user: &crate::models::User,
    next: &str,
) -> Response {
    // A fresh session ID at login prevents session fixation.
    let _ = session.cycle_id().await;
    let _ = session.insert(SESSION_USER_ID, user.id).await;
    // The guest cart follows the member: session rows are
    // re-keyed to the user inside one transaction. A merge
    // failure must never block a login, so it is logged
    // and the login continues.
    if let Err(e) = cart_service::merge_guest_cart_into_user(pool, session, user.id).await {
        tracing::error!("Cart merge on login failed: {e}");
    }
    let target = if next.is_empty() { "/" } else { next };
    Redirect::to(target).into_response()
}

/// The pending two-factor login held by this session, when it
/// exists and is still inside its window. An expired marker is
/// cleared and reported as absent.
async fn pending_totp_user(session: &Session) -> Option<i64> {
    let user_id = session
        .get::<i64>(SESSION_TOTP_PENDING_USER)
        .await
        .ok()
        .flatten()?;
    let since = session
        .get::<i64>(SESSION_TOTP_PENDING_SINCE)
        .await
        .ok()
        .flatten()?;
    if chrono::Utc::now().timestamp() - since > TOTP_PENDING_WINDOW_SECS {
        clear_pending_totp(session).await;
        return None;
    }
    Some(user_id)
}

/// Drops every pending-challenge key from the session.
async fn clear_pending_totp(session: &Session) {
    let _ = session.remove::<i64>(SESSION_TOTP_PENDING_USER).await;
    let _ = session.remove::<i64>(SESSION_TOTP_PENDING_SINCE).await;
    let _ = session.remove::<String>(SESSION_TOTP_PENDING_NEXT).await;
    let _ = session.remove::<i64>(SESSION_TOTP_ATTEMPTS).await;
}

/// GET /login/totp: the challenge page. Only a session holding
/// a live pending marker may see it; everyone else goes to
/// the login page (a signed-in member to their mypage).
async fn login_totp_page(session: Session, State(pool): State<PgPool>) -> Response {
    if current_user(&pool, &session).await.is_some() {
        return Redirect::to("/mypage").into_response();
    }
    if pending_totp_user(&session).await.is_none() {
        return Redirect::to("/login").into_response();
    }
    render(LoginTotpView {
        header: header_for(&pool, &session, false).await,
        error: None,
    })
}

/// Form posted by the two-factor challenge page.
#[derive(Deserialize)]
pub struct TotpForm {
    /// A 6-digit authenticator code, or a recovery code.
    code: String,
}

/// POST /login/totp: verifies the challenge code and completes
/// the login. Failures count against the attempt cap and the
/// login throttle; at the cap the pending login is discarded
/// and the member starts over from the password step.
async fn login_totp_submit(
    session: Session,
    State(pool): State<PgPool>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: axum::http::HeaderMap,
    Form(form): Form<TotpForm>,
) -> Response {
    let ip = crate::session::client_ip(&addr, &headers);
    let Some(user_id) = pending_totp_user(&session).await else {
        let _ = session
            .insert(
                SESSION_FLASH,
                "2段階認証の有効期限が切れました。もう一度ログインしてください。",
            )
            .await;
        return Redirect::to("/login").into_response();
    };
    // The pending member, re-read: a withdrawn or deleted
    // account ends the challenge here, never completes it.
    let Some(user) = member_account_service::load_user(&pool, user_id).await else {
        clear_pending_totp(&session).await;
        return Redirect::to("/login").into_response();
    };
    if auth_throttle::is_blocked(&user.email, &ip) {
        return render(LoginTotpView {
            header: header_for(&pool, &session, false).await,
            error: Some(LOGIN_FAILED_MESSAGE.to_string()),
        });
    }
    if totp_service::verify_login_code(&pool, user_id, &form.code).await {
        let next = session
            .get::<String>(SESSION_TOTP_PENDING_NEXT)
            .await
            .ok()
            .flatten()
            .unwrap_or_default();
        clear_pending_totp(&session).await;
        auth_throttle::record_success(&user.email, &ip);
        return finish_login(&pool, &session, &user, &next).await;
    }
    auth_throttle::record_failure(&user.email, &ip);
    let attempts = session
        .get::<i64>(SESSION_TOTP_ATTEMPTS)
        .await
        .ok()
        .flatten()
        .unwrap_or(0)
        + 1;
    if attempts >= MAX_TOTP_ATTEMPTS {
        clear_pending_totp(&session).await;
        let _ = session
            .insert(
                SESSION_FLASH,
                "認証コードの入力回数が上限に達しました。もう一度ログインしてください。",
            )
            .await;
        return Redirect::to("/login").into_response();
    }
    let _ = session.insert(SESSION_TOTP_ATTEMPTS, attempts).await;
    render(LoginTotpView {
        header: header_for(&pool, &session, false).await,
        error: Some(VERIFICATION_CODE_MISMATCH_MESSAGE.to_string()),
    })
}

/// POST /logout: ends the member session and returns to the top page.
async fn logout_submit(session: Session, State(_pool): State<PgPool>) -> Response {
    let _ = session.remove::<i64>(SESSION_USER_ID).await;
    let _ = session.cycle_id().await;
    Redirect::to("/").into_response()
}

/// Validates a requested return path.
///
/// Only allowlisted local paths pass. `//evil.example`,
/// `https://evil.example`, backslash tricks and non-allowlisted
/// paths all return `None`, and the caller falls back to `/`.
/// This is the open-redirect defence: the parameter is never
/// trusted, only matched.
pub(crate) fn safe_next(raw: Option<&str>) -> Option<String> {
    /// Pages a member may be returned to after login.
    const ALLOWED: [&str; 4] = ["/mypage", "/checkout", "/mypage/sns", "/mypage/orders"];
    let path = raw?;
    if path.starts_with("//") || path.contains('\\') || path.contains("://") {
        return None;
    }
    if ALLOWED.contains(&path) {
        return Some(path.to_string());
    }
    // The pattern paths on the list: an order detail page and a
    // product page (the favourites toggle bounces guests to
    // login from a product), digits only, nothing after them.
    for prefix in ["/mypage/orders/", "/products/"] {
        if let Some(rest) = path.strip_prefix(prefix) {
            if !rest.is_empty() && rest.bytes().all(|b| b.is_ascii_digit()) {
                return Some(path.to_string());
            }
            return None;
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::safe_next;

    #[test]
    fn allowlisted_pages_pass() {
        assert_eq!(safe_next(Some("/mypage")).as_deref(), Some("/mypage"));
        assert_eq!(safe_next(Some("/checkout")).as_deref(), Some("/checkout"));
        assert_eq!(
            safe_next(Some("/mypage/orders")).as_deref(),
            Some("/mypage/orders")
        );
    }

    #[test]
    fn open_redirect_attempts_fall_back() {
        assert_eq!(safe_next(Some("//evil.example")), None);
        assert_eq!(safe_next(Some("https://evil.example")), None);
        assert_eq!(safe_next(Some("/mypage\\@evil.example")), None);
        assert_eq!(safe_next(Some("/admin")), None);
        assert_eq!(safe_next(Some("/orders")), None);
        assert_eq!(safe_next(None), None);
        assert_eq!(safe_next(Some("")), None);
    }
}

#[cfg(test)]
mod safe_next_product_tests {
    use super::safe_next;

    #[test]
    fn product_paths_are_allowed() {
        assert_eq!(
            safe_next(Some("/products/42")).as_deref(),
            Some("/products/42")
        );
    }

    #[test]
    fn product_like_paths_are_rejected() {
        assert_eq!(safe_next(Some("/products/")), None);
        assert_eq!(safe_next(Some("/products/abc")), None);
        assert_eq!(safe_next(Some("/products/1/reviews")), None);
        assert_eq!(safe_next(Some("/products/1?x=1")), None);
        assert_eq!(safe_next(Some("/products/some-slug")), None);
    }
}

#[cfg(test)]
mod safe_next_order_tests {
    use super::safe_next;

    #[test]
    fn order_detail_paths_are_allowed() {
        assert_eq!(
            safe_next(Some("/mypage/orders/12")).as_deref(),
            Some("/mypage/orders/12")
        );
    }

    #[test]
    fn order_like_paths_are_rejected() {
        assert_eq!(safe_next(Some("/mypage/orders/")), None);
        assert_eq!(safe_next(Some("/mypage/orders/abc")), None);
        assert_eq!(safe_next(Some("/mypage/orders/1/edit")), None);
        assert_eq!(safe_next(Some("/mypage/orders/1?x=1")), None);
        assert_eq!(safe_next(Some("//mypage/orders/1")), None);
    }
}
