//! Admin controllers: the back office surface at `/admin`.
//!
//! Controllers own HTTP concerns only: request input, status
//! codes and template rendering. Business rules live in the
//! shared services. Handlers in this tree run under the admin
//! session, never the member session, and every controller
//! here except `auth` takes the `CurrentAdmin` extractor, so
//! an unauthenticated request never reaches a handler: a
//! member account compromise cannot become an admin
//! compromise.

pub mod account;
pub mod admins;
pub mod analytics;
pub mod api_keys;
pub mod audit;
pub mod auth;
pub mod categories;
pub mod content_pages;
pub mod coupons;
pub mod customers;
pub mod dashboard;
pub mod holidays;
pub mod home_content;
pub mod inquiries;
pub mod mail;
pub mod menus;
pub mod notices;
pub mod orders;
pub mod payments;
pub mod products;
pub mod returns;
pub mod reviews;
pub mod rfm;
pub mod segments;
pub mod shop_settings;
pub mod tags;
pub mod tax;
pub mod totp;

use askama::Template;
use axum::{
    Router,
    http::StatusCode,
    response::{Html, IntoResponse, Response},
};
use sqlx::PgPool;
use tower_sessions::Session;

use crate::models::{
    AREA_CONTENT, AREA_MAIL, AREA_PRODUCTS, AREA_SALES, AREA_SETTINGS, AREA_STORE, AdminIdentity,
};
use crate::session::SESSION_FLASH;

/// Combines every admin controller router into the `/admin`
/// subtree. Unmatched admin paths land on the styled admin
/// 404 via a wildcard route rather than a router fallback:
/// Axum allows only ONE fallback per merged router, and the
/// storefront's router already owns it — a wildcard route
/// matches after every exact admin route, which is exactly
/// the fallback's job for this subtree.
pub fn router() -> Router<PgPool> {
    Router::new()
        .merge(auth::router())
        .merge(dashboard::router())
        .merge(products::router())
        .merge(orders::router())
        .merge(payments::router())
        .merge(reviews::router())
        .merge(returns::router())
        .merge(segments::router())
        .merge(rfm::router())
        .merge(notices::router())
        .merge(categories::router())
        .merge(content_pages::router())
        .merge(home_content::router())
        .merge(menus::router())
        .merge(tags::router())
        .merge(coupons::router())
        .merge(customers::router())
        .merge(audit::router())
        .merge(analytics::router())
        .merge(admins::router())
        .merge(account::router())
        .merge(totp::router())
        .merge(api_keys::router())
        .merge(shop_settings::router())
        .merge(holidays::router())
        .merge(tax::router())
        .merge(inquiries::router())
        .merge(mail::router())
        .route(
            "/admin/{*rest}",
            axum::routing::get(not_found).post(not_found),
        )
}

/// Splits a bracketed form key into its parts:
/// `axes[0][options][2][value]` becomes
/// `["axes", "0", "options", "2", "value"]`. Shared by the
/// controllers whose forms post indexed field names (the
/// product form's variation axes, the menus form's rows).
pub fn key_parts(key: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let (head, mut rest) = match key.find('[') {
        Some(i) => (&key[..i], &key[i..]),
        None => return vec![key.to_string()],
    };
    parts.push(head.to_string());
    while rest.starts_with('[') {
        match rest.find(']') {
            Some(close) => {
                parts.push(rest[1..close].to_string());
                rest = &rest[close + 1..];
            }
            None => break,
        }
    }
    parts
}

/// The sidebar layout state every admin page carries: who is
/// operating, and the one-time flash message when one is set.
/// Mirrors the storefront's `Header` struct on the other surface.
pub struct AdminLayout {
    /// The signed-in admin's display name for the sidebar.
    pub admin_name: String,
    /// One-time message set by the previous action, if any.
    pub flash: Option<String>,
    /// Asset cache-busting fingerprint, for the layout's CSS/JS URLs.
    pub asset_fingerprint: &'static str,
    /// Open inquiries, for the sidebar badge (Contact
    /// spec §3.5): the obligation is visible on every
    /// admin page, not just the dashboard.
    pub open_inquiries: i64,
    /// Whether the signed-in admin may use the sales area:
    /// the sidebar renders only the groups the admin holds
    /// (the permission guard is the enforcement; this is the
    /// presentation of the same answer).
    pub can_sales: bool,
    /// Whether the signed-in admin may use the products area.
    pub can_products: bool,
    /// Whether the signed-in admin may use the content area.
    pub can_content: bool,
    /// Whether the signed-in admin may use the mail area.
    pub can_mail: bool,
    /// Whether the signed-in admin may use the store area.
    pub can_store: bool,
    /// Whether the signed-in admin may use the settings area.
    pub can_settings: bool,
    /// Whether the signed-in admin is an owner: the account
    /// management entry renders only for owners.
    pub is_owner: bool,
}

/// Builds the layout state for one admin page, taking the flash
/// message out of the admin session so it shows exactly once.
pub async fn layout_for(pool: &PgPool, admin: &AdminIdentity, session: &Session) -> AdminLayout {
    let flash = session.remove::<String>(SESSION_FLASH).await.ok().flatten();
    let open_inquiries = crate::repositories::inquiry_repository::count_by_status(pool, "open")
        .await
        .unwrap_or(0);
    AdminLayout {
        admin_name: admin.display_name.clone(),
        flash,
        asset_fingerprint: crate::util::asset_fingerprint::asset_fingerprint(),
        open_inquiries,
        can_sales: admin.allows(AREA_SALES),
        can_products: admin.allows(AREA_PRODUCTS),
        can_content: admin.allows(AREA_CONTENT),
        can_mail: admin.allows(AREA_MAIL),
        can_store: admin.allows(AREA_STORE),
        can_settings: admin.allows(AREA_SETTINGS),
        is_owner: admin.is_owner(),
    }
}

/// Returns the list URL a POST should send the operator back
/// to, taken from the form's `return_url` field: list pages
/// render their current tab/filter/page URL into action forms
/// so list-driven work (bulk actions, row deletes) returns to
/// the exact list the operator was working in. Only an
/// admin-surface path is honoured — anything else (missing,
/// external, scheme-bearing) falls back to `fallback`.
pub fn safe_return_url(pairs: &[(String, String)], fallback: &str) -> String {
    let raw = pairs
        .iter()
        .find(|(k, _)| k == "return_url")
        .map(|(_, v)| v.trim())
        .unwrap_or("");
    if raw.starts_with("/admin/") && !raw.contains("://") && !raw.contains('\\') {
        raw.to_string()
    } else {
        fallback.to_string()
    }
}

/// Stores a one-time message in the admin session; the next
/// page render shows it once (the Post/Redirect/Get pattern the
/// member side uses for its flash messages).
pub async fn set_flash(session: &Session, message: &str) {
    let _ = session.insert(SESSION_FLASH, message.to_string()).await;
}

/// Renders a template, or the styled Japanese 500 page
/// when rendering fails (this tail
/// used to answer a plain English "Internal Server
/// Error"). The fallback is the fixed standalone 500
/// document, which depends on nothing dynamic.
pub fn render<T: Template>(view: T) -> Response {
    match view.render() {
        Ok(html) => Html(html).into_response(),
        Err(e) => {
            tracing::error!("Failed to render admin template: {e}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Html(crate::views::admin::notfound::bare_500_html()),
            )
                .into_response()
        }
    }
}

/// Returns a database failure as a response: the shared tail of
/// the admin handlers. The cause was logged at the repository
/// (AppError::db); this line names the surface area — pass the
/// area whose log line must be kept (`"products"`, `"customer
/// load"`, ...). The admin sees the same styled 500
/// everywhere, never a page of invented zeros.
pub fn db_error(area: &str, e: crate::error::AppError) -> Response {
    tracing::error!("admin {area} failed: {e}");
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Html(crate::views::admin::notfound::bare_500_html()),
    )
        .into_response()
}

/// Resolves a list's page request into the triple every admin
/// list handler needs: the page number to show (the request
/// clamped to at least 1), the page size to fetch at, and the
/// row offset to fetch from. A future list calls this with its
/// own page size; only the arithmetic lives here. The display
/// clamp for a page past the end stays in the Page view, as
/// before.
pub fn pagination(page: Option<&str>, per_page: i64) -> (i64, i64, i64) {
    // The page number arrives as typed and is parsed here:
    // an unparseable value is page 1, never a framework
    // rejection of the whole request.
    let page_no = page.and_then(|raw| raw.parse().ok()).unwrap_or(1).max(1);
    (
        page_no,
        per_page,
        page_no.saturating_sub(1).saturating_mul(per_page),
    )
}

/// Renders the styled admin 404 for a missing record, inside
/// the admin layout. Handlers take ID path segments as text
/// and parse them by hand (the storefront convention), so a
/// non-numeric ID lands here too instead of surfacing as a
/// framework plain-text rejection.
/// the admin layout. Used by every admin controller so a
/// guessed ID and a deleted record look exactly the same.
pub async fn record_not_found(pool: &PgPool, admin: &AdminIdentity, session: &Session) -> Response {
    let view = crate::views::admin::notfound::NotFoundView {
        layout: layout_for(pool, admin, session).await,
        nav: "",
    };
    (StatusCode::NOT_FOUND, render(view)).into_response()
}

/// Fallback for unmatched `/admin` paths: the same styled 404
/// as a bare document (no admin session can be assumed here).
pub async fn not_found() -> Response {
    let body = crate::views::admin::notfound::bare_404_html();
    (StatusCode::NOT_FOUND, Html(body)).into_response()
}
