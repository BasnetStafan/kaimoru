//! Cart controller.
//! Cart interactions are htmx fragments: adding answers with the
//! confirmation toast and the header badge, both out-of-band (the
//! add buttons themselves never change); quantity and remove swap
//! the cart page's main area. Nothing here reloads a page.

use axum::{
    Form, Router,
    extract::State,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde::Deserialize;
use sqlx::PgPool;
use tower_sessions::Session;

use super::{RenderHtml, header_for, render};
use crate::services::cart_service::{self, AddOutcome, CartOwner};
use crate::services::product_service;
use crate::views::storefront::cart::{
    CartBadgeView, CartMainFragment, CartMainView, CartToastView, CartView,
};

/// Routes owned by the cart controller.
pub fn router() -> Router<PgPool> {
    Router::new()
        .route("/cart", get(cart_page))
        .route("/cart/items", post(add_item))
        .route("/cart/items/update", post(update_item))
        .route("/cart/items/remove", post(remove_item))
}

/// Form posted by every add-to-cart button. The detail page's
/// picker posts the chosen `variant_id`; cards post none (the
/// server resolves a single-combination product's variant).
/// Values arrive as typed and are parsed by the handler (the
/// app-wide form convention), so a crafted value gets the
/// handler's own answers instead of a form-parsing rejection.
#[derive(Deserialize)]
pub struct AddForm {
    product_id: String,
    variant_id: Option<String>,
    #[serde(default)]
    quantity: String,
}

/// Form posted by the cart page stepper and remove buttons:
/// lines are addressed by variant, the cart's line identity.
#[derive(Deserialize)]
pub struct LineForm {
    variant_id: String,
    #[serde(default)]
    quantity: String,
}

/// GET /cart: the cart page.
async fn cart_page(session: Session, State(pool): State<PgPool>) -> Response {
    let header = header_for(&pool, &session, false).await;
    // The cart is this page: a failed load is a 500, never an
    // empty-looking cart.
    match main_fragment(&pool, &session).await {
        Ok(mut main) => {
            main.notice = session
                .remove::<String>(crate::session::SESSION_CART_NOTICE)
                .await
                .ok()
                .flatten();
            render(CartView { header, main })
        }
        Err(e) => {
            tracing::error!("cart page failed: {e}");
            (
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                "Internal Server Error",
            )
                .into_response()
        }
    }
}

/// Builds the cart-main fragment state for the current visitor.
async fn main_fragment(
    pool: &PgPool,
    session: &Session,
) -> Result<CartMainView, crate::error::AppError> {
    let owner = cart_service::owner_for_read(pool, session).await;
    let lines = cart_service::lines(pool, &owner).await?;
    let tax = crate::services::tax_service::params_today_or_fallback(pool).await;
    let settings = crate::services::shop_settings_service::load(pool).await;
    let fees = settings.fee_settings();
    // No payment method exists on the cart page: the totals
    // carry no cash-on-delivery fee until one is chosen.
    let totals = cart_service::totals(&lines, 0, 0, None, tax, fees);
    // The agent settlement state mirrors the
    // chat thread's from the same server-side resolution,
    // so a quantity change here re-renders it in its new
    // (voided) state in the same swap. On this surface it
    // is display-only: the settlement itself lives in chat.
    let settlement = super::agent::load_settlement_view(pool, session, false).await;
    let (earliest_off, latest_off) = settings.delivery_offsets();
    Ok(CartMainView {
        is_guest: matches!(owner, CartOwner::Session(_)),
        lines,
        totals,
        settlement,
        notice: None,
        free_shipping_threshold_yen: fees.free_shipping_threshold_yen,
        earliest_delivery_label: crate::models::delivery::earliest_delivery_label_with_holidays(
            crate::models::jst::jst_today(),
            earliest_off,
            latest_off,
            &crate::services::holiday_service::upcoming(pool).await,
        ),
    })
}

/// Renders the add answer: the out-of-band confirmation toast
/// (on success) plus the out-of-band header badge. Both add
/// forms use `hx-swap="none"`, so out-of-band fragments are the
/// whole visible answer and the buttons themselves never change.
async fn add_response(message: Option<String>, count: i64) -> Response {
    let badge = CartBadgeView { count, oob: true };
    let body = match message {
        Some(message) => format!(
            "{}{}",
            CartToastView { message }.render_html().unwrap_or_default(),
            badge.render_html().unwrap_or_default()
        ),
        None => badge.render_html().unwrap_or_default(),
    };
    axum::response::Html(body).into_response()
}

/// POST /cart/items: adds to the cart and answers with the
/// confirmation toast and the updated header badge, both
/// out-of-band. The toast is success-only: a capped, sold-out or
/// failed add updates the badge and shows nothing.
async fn add_item(
    session: Session,
    State(pool): State<PgPool>,
    Form(form): Form<AddForm>,
) -> Response {
    let owner = cart_service::owner_for(&pool, &session).await;
    // An ID that does not parse names nothing: the product
    // answer is the same 404 as a missing product, and an
    // unparseable variant is no variant at all. An empty
    // variant field is the picker's "no choice" state.
    let Ok(product_id) = form.product_id.parse::<i64>() else {
        return (axum::http::StatusCode::NOT_FOUND, "Not Found").into_response();
    };
    let variant_id = match form.variant_id.as_deref().map(str::trim) {
        None | Some("") => None,
        Some(raw) => match raw.parse::<i64>() {
            Ok(id) => Some(id),
            Err(_) => {
                return (axum::http::StatusCode::NOT_FOUND, "Not Found").into_response();
            }
        },
    };
    let quantity: i32 = form.quantity.parse().unwrap_or(1);
    let product = match product_service::detail(&pool, product_id).await {
        Ok(Some(product)) => product,
        Ok(None) => return (axum::http::StatusCode::NOT_FOUND, "Not Found").into_response(),
        Err(e) => {
            tracing::error!(product_id = product_id, "add-to-cart lookup failed: {e}");
            return (
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                "Internal Server Error",
            )
                .into_response();
        }
    };
    match cart_service::add_to_cart(&pool, &owner, product_id, variant_id, quantity).await {
        Ok(AddOutcome::Added(count)) => {
            let mut message = format!("{}をカートに追加しました", product.name);
            // The detail buy box can add several at once; the
            // count is part of the confirmation there.
            let qty = quantity.max(1);
            if qty > 1 {
                message.push_str(&format!("（{qty}点）"));
            }
            // The toast lives only in this answer: nothing is
            // stored for later pages, so navigating away never
            // shows the confirmation a second time.
            add_response(Some(message), count).await
        }
        Ok(AddOutcome::Capped(count)) => add_response(None, count).await,
        Ok(AddOutcome::SoldOut) => {
            add_response(
                None,
                cart_service::count_for_header(&pool, &session, None).await,
            )
            .await
        }
        Ok(AddOutcome::NotFound) => {
            (axum::http::StatusCode::NOT_FOUND, "Not Found").into_response()
        }
        Err(e) => {
            tracing::error!("Add to cart failed: {e}");
            add_response(
                None,
                cart_service::count_for_header(&pool, &session, None).await,
            )
            .await
        }
    }
}

/// POST /cart/items/update: stepper changes, answered with the
/// whole cart-main fragment and the badge.
async fn update_item(
    session: Session,
    State(pool): State<PgPool>,
    Form(form): Form<LineForm>,
) -> Response {
    let owner = cart_service::owner_for(&pool, &session).await;
    // An unparseable variant ID addresses no line: the answer
    // is the same re-render a failed update already produces.
    let Ok(variant_id) = form.variant_id.parse::<i64>() else {
        return main_response(&pool, &session).await;
    };
    let quantity: i32 = form.quantity.parse().unwrap_or(0);
    if let Err(e) = cart_service::update_quantity(&pool, &owner, variant_id, quantity).await {
        tracing::error!("Cart update failed: {e}");
    }
    main_response(&pool, &session).await
}

/// POST /cart/items/remove: removes a line, same response shape.
async fn remove_item(
    session: Session,
    State(pool): State<PgPool>,
    Form(form): Form<LineForm>,
) -> Response {
    let owner = cart_service::owner_for(&pool, &session).await;
    let Ok(variant_id) = form.variant_id.parse::<i64>() else {
        return main_response(&pool, &session).await;
    };
    if let Err(e) = cart_service::remove_line(&pool, &owner, variant_id).await {
        tracing::error!("Cart remove failed: {e}");
    }
    main_response(&pool, &session).await
}

/// Renders the cart-main fragment plus the out-of-band badge. A
/// failed reload after a mutation is logged and the fragment
/// renders empty rather than failing the request — the mutation
/// itself already reported its own errors.
async fn main_response(pool: &PgPool, session: &Session) -> Response {
    let main = match main_fragment(pool, session).await {
        Ok(main) => main,
        Err(e) => {
            tracing::error!("cart fragment reload failed: {e}");
            CartMainView {
                is_guest: true,
                settlement: None,
                notice: None,
                lines: Vec::new(),
                totals: cart_service::totals(
                    &[],
                    0,
                    0,
                    None,
                    crate::services::tax_service::TaxParams::fallback(),
                    crate::models::FeeSettings::fallback(),
                ),
                free_shipping_threshold_yen: crate::models::FeeSettings::fallback()
                    .free_shipping_threshold_yen,
                earliest_delivery_label: crate::models::delivery::earliest_delivery_label(
                    crate::models::jst::jst_today(),
                ),
            }
        }
    };
    let count = cart_service::count_for_header(pool, session, None).await;
    let badge = CartBadgeView { count, oob: true };
    let fragment = CartMainFragment { main };
    let body = format!(
        "{}{}",
        fragment.render_html().unwrap_or_default(),
        badge.render_html().unwrap_or_default()
    );
    axum::response::Html(body).into_response()
}
