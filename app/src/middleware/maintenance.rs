//! Maintenance-mode gate: while the shop is closed
//! for maintenance, the storefront and the public APIs
//! answer 503 with the shop's message, and only the admin
//! (which must stay reachable to reopen), the payment
//! webhooks (KOMOJU events must keep settling — losing
//! them would strand paid orders), the health check and
//! static assets stay open. The check rides the settings
//! cache the admin save invalidates, so a toggle takes
//! effect immediately without a per-request query.

use axum::extract::{Request, State};
use axum::middleware::Next;
use axum::response::{Html, IntoResponse, Response};
use sqlx::PgPool;

use crate::services::shop_settings_service;

/// The closed page's message when the operator left the
/// setting empty.
const DEFAULT_MESSAGE: &str = "ただいまメンテナンス中のため、ご利用いただけません。ご不便をおかけしますが、しばらく経ってから再度アクセスしてください。";

/// Middleware: passes the request through unless
/// maintenance mode is on.
pub async fn gate(State(pool): State<PgPool>, req: Request, next: Next) -> Response {
    let settings = shop_settings_service::load(&pool).await;
    if !settings.maintenance_mode {
        return next.run(req).await;
    }
    let path = req.uri().path().to_string();
    if path.starts_with("/api/") {
        return (
            axum::http::StatusCode::SERVICE_UNAVAILABLE,
            axum::Json(serde_json::json!({
                "error": "maintenance",
                "message": "ただいまメンテナンス中です",
            })),
        )
            .into_response();
    }
    let message = if settings.maintenance_message.trim().is_empty() {
        DEFAULT_MESSAGE.to_string()
    } else {
        settings.maintenance_message.clone()
    };
    let body = format!(
        "<!DOCTYPE html>\n<html lang=\"ja\">\n<head>\n<meta charset=\"utf-8\">\n\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n\
         <title>メンテナンス中 | {}</title>\n\
         <style>body{{font-family:'Hiragino Kaku Gothic ProN','Yu Gothic','Noto Sans JP',sans-serif;margin:4rem auto;max-width:34rem;padding:0 1rem;line-height:1.8;background:#FAF8F4;color:#232825}}</style>\n\
         </head>\n<body>\n<h1>ただいまメンテナンス中です</h1>\n<p>{}</p>\n</body>\n</html>",
        askama_escape(&settings.shop_name),
        askama_escape(&message),
    );
    (axum::http::StatusCode::SERVICE_UNAVAILABLE, Html(body)).into_response()
}

/// Minimal HTML escaping for the two interpolated texts
/// (the page is hand-built, not templated).
fn askama_escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}
