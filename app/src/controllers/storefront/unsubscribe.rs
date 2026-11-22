//! Unsubscribe controller.
//! The no-login 配信停止 link printed in every marketing
//! mail: the token in the URL is the credential, GET shows
//! the confirmation and POST applies the opt-out. Only
//! marketing mail stops; transactional mail is unaffected
//! and the page says so.

use askama::Template;
use axum::{
    Router,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::get,
};
use sqlx::PgPool;
use tower_sessions::Session;

use super::{header_for, render};
use crate::messages::PAGE_NOT_FOUND_MESSAGE;
use crate::services::mail_subscription_service;
use crate::views::storefront::unsubscribe::{
    UnsubscribeConfirmView, UnsubscribeDoneView, UnsubscribeInvalidView,
};

/// Routes owned by the unsubscribe controller.
pub fn router() -> Router<PgPool> {
    Router::new().route("/unsubscribe/{token}", get(confirm_page).post(apply))
}

/// GET /unsubscribe/{token}: the confirmation page. An
/// unknown token gets the dedicated invalid-link page
/// (still a 404): it explains the link is the problem
/// without revealing anything about any other token
///.
async fn confirm_page(
    session: Session,
    State(pool): State<PgPool>,
    Path(token): Path<String>,
) -> Response {
    match mail_subscription_service::find_by_token(&pool, &token).await {
        Ok(Some(subscription)) => {
            if subscription.is_subscribed() {
                render(UnsubscribeConfirmView {
                    header: header_for(&pool, &session, false).await,
                    token,
                })
            } else {
                render(UnsubscribeDoneView {
                    header: header_for(&pool, &session, false).await,
                    already: true,
                })
            }
        }
        Ok(None) => invalid_token(&pool, &session).await,
        Err(e) => {
            tracing::error!("unsubscribe lookup failed: {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, "Internal Server Error").into_response()
        }
    }
}

/// POST /unsubscribe/{token}: applies the opt-out.
async fn apply(
    session: Session,
    State(pool): State<PgPool>,
    Path(token): Path<String>,
) -> Response {
    match mail_subscription_service::unsubscribe_by_token(&pool, &token).await {
        Ok(Some(transitioned)) => render(UnsubscribeDoneView {
            header: header_for(&pool, &session, false).await,
            already: !transitioned,
        }),
        Ok(None) => invalid_token(&pool, &session).await,
        Err(e) => {
            tracing::error!("unsubscribe apply failed: {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, "Internal Server Error").into_response()
        }
    }
}

/// The invalid-link page for an unknown token, with a
/// 404 status: the visitor followed a mail link, so the
/// page names the link as the problem and points at the
/// recovery paths (the newest mail's link, or the mypage
/// setting for members).
async fn invalid_token(pool: &PgPool, session: &Session) -> Response {
    let view = UnsubscribeInvalidView {
        header: header_for(pool, session, false).await,
    };
    let body = match view.render() {
        Ok(html) => html,
        Err(_) => PAGE_NOT_FOUND_MESSAGE.to_string(),
    };
    (StatusCode::NOT_FOUND, axum::response::Html(body)).into_response()
}
