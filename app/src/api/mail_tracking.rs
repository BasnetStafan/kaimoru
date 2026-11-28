//! Campaign open/click tracking endpoints (Wave 3 C2
//! completion).
//!
//! These are mail-client facing, not pages: the open
//! endpoint answers a 1x1 GIF, the click endpoint a 302
//! to the URL stored for its token. Tokens are random
//! ids that reveal nothing about the recipient. An
//! unknown token is a forgery or a stale client and
//! answers 404; the click endpoint NEVER honours a
//! target supplied by the request, so it cannot be used
//! as an open redirect. The endpoints carry no session
//! and sit outside the maintenance gate (see
//! [`crate::routes`]): a click in an already-delivered
//! mail must keep working while the shop is maintained.

use axum::Router;
use axum::extract::{Path, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use sqlx::PgPool;

use crate::repositories::mail_repository;

/// The routes owned by this module.
pub fn router() -> Router<PgPool> {
    Router::new()
        .route("/mail/track/open/{token}", get(open_pixel))
        .route("/mail/track/click/{token}", get(click_redirect))
}

/// A 1x1 transparent GIF.
const PIXEL_GIF: &[u8] = &[
    0x47, 0x49, 0x46, 0x38, 0x39, 0x61, 0x01, 0x00, 0x01, 0x00, 0x80, 0x00, 0x00, 0xff, 0xff, 0xff,
    0x00, 0x00, 0x00, 0x21, 0xf9, 0x04, 0x01, 0x00, 0x00, 0x00, 0x00, 0x2c, 0x00, 0x00, 0x00, 0x00,
    0x01, 0x00, 0x01, 0x00, 0x00, 0x02, 0x02, 0x44, 0x01, 0x00, 0x3b,
];

/// GET /mail/track/open/{token}: records the open and
/// answers the pixel with no-store headers, so a mail
/// client's cache can never swallow a real open.
async fn open_pixel(State(pool): State<PgPool>, Path(token): Path<String>) -> Response {
    match mail_repository::record_open(&pool, &token).await {
        Ok(true) => (
            StatusCode::OK,
            [
                (header::CONTENT_TYPE, "image/gif"),
                (header::CACHE_CONTROL, "no-store, no-cache, must-revalidate"),
                (header::PRAGMA, "no-cache"),
            ],
            PIXEL_GIF,
        )
            .into_response(),
        Ok(false) => StatusCode::NOT_FOUND.into_response(),
        Err(e) => {
            tracing::error!("record open failed: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

/// GET /mail/track/click/{token}: records the click and
/// redirects to the URL stored for the token at send
/// time — the only target it can ever produce.
async fn click_redirect(State(pool): State<PgPool>, Path(token): Path<String>) -> Response {
    match mail_repository::record_click(&pool, &token).await {
        Ok(Some(url)) => (StatusCode::FOUND, [(header::LOCATION, url)]).into_response(),
        Ok(None) => StatusCode::NOT_FOUND.into_response(),
        Err(e) => {
            tracing::error!("record click failed: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}
