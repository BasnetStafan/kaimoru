//! Storefront controllers: the member-facing shop.
//!
//! Controllers own HTTP concerns only: request input, status
//! codes and template rendering. Business rules live in the
//! shared services. Every handler in this tree runs under the
//! member session. Each feature has its own controller file.

pub mod addresses;
pub mod agent;
pub mod cart;
pub mod category_index;
pub mod checkout;
pub mod contact;
pub mod favorites;
pub mod home;
pub mod login;
pub mod mypage;
pub mod order_lookup;
pub mod pages;
pub mod password_reset;
pub mod payment_return;
pub mod points;
pub mod product;
pub mod reviews;
pub mod search;
pub mod section;
pub mod signup;
pub mod sns;
pub mod special;
pub mod totp;
pub mod unsubscribe;

use askama::Template;
use axum::{
    Router,
    http::StatusCode,
    response::{Html, IntoResponse, Response},
};
use sqlx::PgPool;
use tower_sessions::Session;

use crate::messages::PAGE_NOT_FOUND_MESSAGE;
use crate::services::{
    cart_service, home_content_service, menu_service, order_service, review_service,
    shop_settings_service,
};
use crate::session as auth_helpers;
use crate::views::storefront::header::Header;

/// Combines every controller router into one.
pub fn router() -> Router<PgPool> {
    Router::new()
        .merge(home::router())
        .merge(special::router())
        .merge(category_index::router())
        .merge(signup::router())
        .merge(login::router())
        .merge(mypage::router())
        .merge(order_lookup::router())
        .merge(favorites::router())
        .merge(points::router())
        .merge(totp::router())
        .merge(sns::router())
        .merge(addresses::router())
        .merge(contact::router())
        .merge(password_reset::router())
        .merge(search::router())
        .merge(section::router())
        .merge(cart::router())
        .merge(agent::router())
        .merge(checkout::router())
        .merge(payment_return::router())
        .merge(product::router())
        .merge(reviews::router())
        .merge(unsubscribe::router())
        .merge(pages::router())
        .fallback(not_found_fallback)
}

/// Returns the fallback for unmatched paths: the styled not-found
/// page, never the framework's empty default 404.
///
/// The header here is the guest header with no database work
/// behind it. The fallback answers every garbage URL the internet
/// sends, and building the full header used to run three queries
/// per junk request; a missing page gains nothing from a
/// personalised header either.
pub async fn not_found_fallback() -> Response {
    let view = crate::views::storefront::errors::PageNotFoundView {
        header: Header::guest(),
    };
    let body = match view.render() {
        Ok(html) => html,
        Err(_) => PAGE_NOT_FOUND_MESSAGE.to_string(),
    };
    (StatusCode::NOT_FOUND, Html(body)).into_response()
}

/// Renders a template, or the styled Japanese 500 page
/// when rendering fails (this tail
/// used to answer a plain English "Internal Server
/// Error"). The fallback is the panic handler's page with
/// the guest header: no database work behind it, because
/// a render failure may ride on a broken database path.
/// If even that page fails to render, a fixed Japanese
/// sentence is the last resort.
pub fn render<T: Template>(view: T) -> Response {
    match view.render() {
        Ok(html) => Html(html).into_response(),
        Err(e) => {
            tracing::error!("Failed to render template: {e}");
            let fallback = crate::views::storefront::errors::ServerErrorView {
                header: crate::views::storefront::header::Header::guest(),
            };
            let body = fallback
                .render()
                .unwrap_or_else(|_| "システムエラーが発生しました".to_string());
            (StatusCode::INTERNAL_SERVER_ERROR, Html(body)).into_response()
        }
    }
}

/// Builds the printable receipt view for one order: the
/// shared tail of the member receipt page and the guest
/// lookup's receipt post. Callers have already proven
/// ownership (the member session, or the lookup's
/// number+email pair); this only assembles. A failed read
/// is an error, never a half-printed receipt.
pub async fn receipt_view(
    pool: &PgPool,
    order: &crate::models::Order,
) -> Result<crate::views::storefront::receipt::ReceiptView, crate::error::AppError> {
    use chrono::Datelike;
    let lines = crate::services::order_service::order_lines_with_images(pool, order.id).await?;
    let tax_breakdown =
        crate::repositories::order_repository::tax_breakdowns(pool, order.id).await?;
    let shop = crate::services::shop_settings_service::load(pool).await;
    let today = crate::models::jst::jst_today();
    Ok(crate::views::storefront::receipt::ReceiptView {
        asset_fingerprint: crate::util::asset_fingerprint::asset_fingerprint(),
        shop_name: shop.shop_name.clone(),
        registration_number: shop.invoice_registration_number.clone(),
        contact_email: if shop.contact_email.is_empty() {
            None
        } else {
            Some(shop.contact_email)
        },
        issued_label: format!("{}年{}月{}日", today.year(), today.month(), today.day()),
        order: order.clone(),
        lines,
        tax_breakdown,
    })
}

/// Builds the shared header state for one request.
///
/// With `take_flash`, a stored one-time message is read and removed,
/// so it is shown exactly once. The pages that land a completed
/// flow take it: the top page (sign-up and login land there),
/// the login page, and the withdraw, SNS and TOTP settings
/// pages.
pub async fn header_for(pool: &PgPool, session: &Session, take_flash: bool) -> Header {
    let user = auth_helpers::current_user(pool, session).await;
    header_for_user(pool, session, take_flash, user).await
}

/// Builds the shared header state around a user the caller
/// already resolved, for handlers whose own logic needed the
/// session user first (the product detail page): resolving it
/// again here would repeat the same users lookup on one request.
pub async fn header_for_user(
    pool: &PgPool,
    session: &Session,
    take_flash: bool,
    user: Option<crate::models::User>,
) -> Header {
    let cart_count = cart_service::count_for_header(pool, session, user.as_ref()).await;
    // The band comes from the active coupon, cached briefly by
    // the service; no active coupon means no band.
    let band = order_service::band_parts(pool).await;
    let welcome = if take_flash {
        session
            .remove::<String>(auth_helpers::SESSION_FLASH)
            .await
            .ok()
            .flatten()
    } else {
        None
    };
    // The shop's words come from the operator's settings row
    // through the service's brief cache; a save invalidates
    // it, so new words are still live on the next page.
    let shop = shop_settings_service::load(pool).await;
    let mut header = Header::build(
        user.as_ref(),
        welcome,
        cart_count,
        band.text,
        band.amount,
        shop,
    );
    // The announcement bar (CMS C-3) rides in the shared
    // header state; only the top page renders it. The
    // service caches the slot
    // rows; whether the bar is inside its display window is
    // decided on this request, never cached.
    header.announcement = home_content_service::load(pool)
        .await
        .live_announcement()
        .map(|parts| crate::views::storefront::header::AnnouncementBar {
            text: parts.text,
            link_label: parts.link_label,
            link_url: parts.link_url,
        });
    // The layout's menus (CMS C-4) ride in the shared header
    // state too. The service caches the rows; visibility is
    // a stored property, filtered here at render.
    let menus = menu_service::load(pool).await;
    header.header_menu = menus
        .visible_items(crate::models::HEADER_KEY)
        .iter()
        .map(crate::views::storefront::header::MenuLink::from)
        .collect();
    header.footer_menu = menus
        .visible_items(crate::models::FOOTER_KEY)
        .iter()
        .map(crate::views::storefront::header::MenuLink::from)
        .collect();
    header
}

/// Renders a template to an HTML string, for handlers that must
/// concatenate fragments (the cart's button + out-of-band badge)
/// instead of returning one rendered page.
pub trait RenderHtml {
    /// Renders this template, mirroring [`render`]'s contract.
    fn render_html(&self) -> Result<String, askama::Error>;
}

impl<T: Template> RenderHtml for T {
    fn render_html(&self) -> Result<String, askama::Error> {
        self.render()
    }
}

/// Builds display cards for a list of products, attaching each
/// product's review aggregate in one grouped query for the whole
/// list — never one query per card. The viewing member's
/// favourite marks ride along the same way (one bulk query);
/// guests (`viewer` is `None`) get unmarked cards.
pub async fn product_cards(
    pool: &PgPool,
    viewer: Option<i64>,
    products: Vec<crate::models::Product>,
) -> Vec<crate::views::storefront::product_card::ProductCard> {
    let ids: Vec<i64> = products.iter().map(|p| p.id).collect();
    let aggregates = review_service::aggregate_map(pool, &ids).await;
    let favorites = crate::services::favorite_service::favorite_id_set(pool, viewer, &ids).await;
    products
        .into_iter()
        .map(|product| {
            let rating = aggregates.get(&product.id).copied().unwrap_or_default();
            let is_favorite = favorites.contains(&product.id);
            crate::views::storefront::product_card::ProductCard {
                product,
                rating,
                is_favorite,
                is_member: viewer.is_some(),
                action: crate::views::storefront::product_card::ProductCardAction::Storefront,
            }
        })
        .collect()
}
