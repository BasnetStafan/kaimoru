//! Checkout controller.
//! The flow is three steps, each its own page: POST /checkout
//! validates the delivery form and moves on to the payment
//! step; POST /checkout/payment records the method and renders
//! the confirmation as its response; POST /checkout/confirm
//! places the order and redirects to the completion page.
//! Flow state lives in the session, never in hidden fields
//! beyond the CSRF token.

use axum::{
    Form, Router,
    extract::State,
    response::{IntoResponse, Redirect, Response},
    routing::{get, post},
};
use serde::Deserialize;
use sqlx::PgPool;
use tower_sessions::Session;

use super::{header_for, render};
use crate::messages::{COUPON_INVALID_MESSAGE, paid_method_label};
use crate::services::cart_service::{self, CartOwner};
use crate::services::coupon_service;
use crate::services::member_address_service;
use crate::services::order_service::{self, CheckoutFlow, PlaceError};
use crate::services::payment_service;
use crate::session;
use crate::session::{load_checkout_flow, save_checkout_flow, take_checkout_flow};
use crate::views::storefront::checkout::{
    CheckoutCompleteView, CheckoutConfirmView, CheckoutErrors, CheckoutPaymentView, CheckoutView,
    DeliveryDateOption, DeliverySlotOption, PREFECTURES, PaymentMethodOption,
};
use crate::views::storefront::payment::PaymentErrorView;

/// Routes owned by the checkout controller.
pub fn router() -> Router<PgPool> {
    Router::new()
        .route("/checkout", get(checkout_page).post(checkout_submit))
        .route(
            "/checkout/payment",
            get(checkout_payment_page).post(checkout_payment_submit),
        )
        .route("/checkout/confirm", post(checkout_confirm))
        .route("/checkout/complete", get(checkout_complete))
}

/// The checkout form: delivery fields plus which button was used.
#[derive(Deserialize)]
pub struct CheckoutForm {
    #[serde(default)]
    intent: String,
    #[serde(default)]
    email: String,
    #[serde(default)]
    family_name: String,
    #[serde(default)]
    given_name: String,
    #[serde(default)]
    postal_code: String,
    #[serde(default)]
    prefecture: String,
    #[serde(default)]
    city: String,
    #[serde(default)]
    address_line: String,
    #[serde(default)]
    building: String,
    #[serde(default)]
    phone: String,
    #[serde(default)]
    delivery_date: String,
    #[serde(default)]
    delivery_slot: String,
    #[serde(default)]
    noshi: String,
    #[serde(default)]
    noshi_name: String,
    #[serde(default)]
    gift_message: String,
    #[serde(default)]
    coupon_code: String,
    #[serde(default)]
    coupon_select: String,
    /// The address-book pick: a saved address's id, or "new"
    /// for the typed form. Absent when the member has no book.
    #[serde(default)]
    delivery_choice: String,
    /// The save-to-address-book checkbox (present when checked).
    #[serde(default)]
    save_address: Option<String>,
}

impl CheckoutForm {
    /// Converts the posted fields into flow state (keeping any
    /// previously applied coupon unless this submit changes it,
    /// and the payment method chosen on the payment step).
    fn to_flow(
        &self,
        keep_coupons: Vec<String>,
        keep_method: crate::models::PaymentMethod,
        keep_method_chosen: bool,
        keep_points: i64,
    ) -> CheckoutFlow {
        CheckoutFlow {
            email: self.email.trim().to_string(),
            family_name: self.family_name.trim().to_string(),
            given_name: self.given_name.trim().to_string(),
            postal_code: self.postal_code.trim().to_string(),
            prefecture: self.prefecture.trim().to_string(),
            city: self.city.trim().to_string(),
            address_line: self.address_line.trim().to_string(),
            building: self.building.trim().to_string(),
            phone: self.phone.trim().to_string(),
            delivery_date: self.delivery_date.trim().to_string(),
            delivery_slot: self.delivery_slot.trim().to_string(),
            noshi: self.noshi.trim().to_string(),
            noshi_name: self.noshi_name.trim().to_string(),
            gift_message: self.gift_message.trim().to_string(),
            coupon_codes: keep_coupons,
            payment_method: keep_method,
            payment_method_chosen: keep_method_chosen,
            save_address: false,
            points_to_use: keep_points,
        }
    }
}

/// Everything a checkout render needs, loaded once per request:
/// the member (if any), the cart owner derived from them, and the
/// cart lines. (One render used to resolve the user up to four
/// times and load the lines twice.)
struct CheckoutContext {
    /// Cart owner derived from the user (or the guest token).
    owner: CartOwner,
    /// The logged-in member, if any.
    user: Option<crate::models::User>,
    /// The owner's cart lines, unfiltered.
    lines: Vec<crate::models::CartLine>,
}

/// Loads the [`CheckoutContext`] for this request.
async fn load_context(pool: &PgPool, session: &Session) -> CheckoutContext {
    let user = session::current_user(pool, session).await;
    let owner = match &user {
        Some(user) => CartOwner::User(user.id),
        None => match cart_service::existing_guest_token(session).await {
            Some(token) => CartOwner::Session(token),
            None => CartOwner::Session(String::new()),
        },
    };
    // A failed cart load reads as an empty cart here — the pages
    // then redirect to /cart — but it is logged, so an outage is
    // visible instead of masquerading as empty carts.
    let lines = match cart_service::lines(pool, &owner).await {
        Ok(lines) => lines,
        Err(e) => {
            tracing::error!("checkout cart load failed: {e}");
            Vec::new()
        }
    };
    CheckoutContext { owner, user, lines }
}

/// GET /checkout: the checkout form. An empty cart goes back to
/// the cart page; members start from their registered address,
/// returning visitors from their saved flow state.
async fn checkout_page(session: Session, State(pool): State<PgPool>) -> Response {
    let ctx = load_context(&pool, &session).await;
    if ctx.lines.iter().all(|l| !l.purchasable()) {
        return Redirect::to("/cart").into_response();
    }
    let mut values = match load_checkout_flow(&session).await {
        Some(flow) => flow,
        None => match &ctx.user {
            Some(user) => CheckoutFlow::from_user(user),
            None => CheckoutFlow::default(),
        },
    };
    if let Some(user) = &ctx.user {
        values.email = user.email.clone();
        // A saved flow can predate the login and hold blank
        // delivery fields; the registered address fills those,
        // and typed values win (see CheckoutFlow).
        values.fill_empty_from_user(user);
    }
    build_checkout_view(
        &pool,
        &session,
        values,
        CheckoutErrors::default(),
        None,
        None,
        &ctx,
    )
    .await
}

/// Builds the checkout view, evaluating the flow's coupon against
/// the current cart so the totals are always live.
async fn build_checkout_view(
    pool: &PgPool,
    session: &Session,
    values: CheckoutFlow,
    errors: CheckoutErrors,
    coupon_error: Option<String>,
    error_message: Option<String>,
    ctx: &CheckoutContext,
) -> Response {
    let owner = &ctx.owner;
    let lines: Vec<_> = ctx
        .lines
        .iter()
        .filter(|l| l.purchasable())
        .cloned()
        .collect();
    let is_member = ctx.user.is_some();
    // The engine evaluates the flow's entered codes plus the
    // automatic discounts against the live cart (Coupon
    // Shopify-Core spec): one applied coupon per class.
    let outcome =
        coupon_service::evaluate(pool, owner.user_id(), &lines, &values.coupon_codes).await;
    let applied_coupons = outcome.applied.clone();
    // The coupon dropdown's options: the member's held, unused
    // coupons, read through the same service as the mypage coupon
    // section. Only the unapplied form renders the dropdown; a
    // failed read is logged and offers none — the typed field
    // keeps working alone.
    let held_coupons: Vec<crate::models::MemberCoupon> = match &ctx.user {
        Some(user) => match coupon_service::member_coupons(pool, user.id).await {
            Ok(coupons) => coupons
                .into_iter()
                .filter(|c| !c.is_used() && c.is_available() && !outcome.contains(&c.code))
                .collect(),
            Err(e) => {
                tracing::error!("checkout coupon list failed: {e}");
                Vec::new()
            }
        },
        _ => Vec::new(),
    };
    // The address book's picker data: the member's saved
    // addresses, default first. A failed read is logged and
    // shows no picker — the typed form keeps working alone.
    let saved_addresses: Vec<crate::models::MemberAddress> = match &ctx.user {
        Some(user) => match member_address_service::list(pool, user.id).await {
            Ok(list) => list,
            Err(e) => {
                tracing::error!("checkout address list failed: {e}");
                Vec::new()
            }
        },
        None => Vec::new(),
    };
    let tax = crate::services::tax_service::params_today_or_fallback(pool).await;
    // The delivery step prices with the method the customer
    // has actually chosen (the
    // EC-CUBE/Amazon pattern): until the payment step is
    // completed the totals carry no payment-method fee, so
    // the COD fee never precedes its selection — it enters
    // at the payment step and is itemised at confirmation.
    // The one exception is the forced case: when COD is
    // the only offered method (KOMOJU unavailable) its fee
    // is a settled fact of the order, not a choice, and
    // shows from this step.
    let offered = payment_service::offered_methods();
    let sole_method = (offered.len() == 1).then(|| offered[0]);
    let totals_method = values.chosen_method().or(sole_method);
    let settings = crate::services::shop_settings_service::load(pool).await;
    let totals = cart_service::totals(
        &lines,
        outcome.discount_yen(),
        outcome.shipping_discount_yen,
        totals_method,
        tax,
        settings.fee_settings(),
    );
    // The delivery picker's options, generated for today (JST)
    // at render: the window dates with their weekday labels,
    // and the slot catalogue with the carried choice marked.
    let (earliest_off, latest_off) = settings.delivery_offsets();
    let holidays = crate::services::holiday_service::upcoming(pool).await;
    let delivery_dates: Vec<DeliveryDateOption> =
        crate::models::delivery::delivery_dates_with_holidays(
            crate::models::jst::jst_today(),
            earliest_off,
            latest_off,
            &holidays,
        )
        .into_iter()
        .map(|day| {
            let value = day.format("%Y-%m-%d").to_string();
            DeliveryDateOption {
                selected: values.delivery_date.trim() == value,
                label: crate::models::delivery::delivery_date_label(day),
                value,
            }
        })
        .collect();
    let current_slot = if values.delivery_slot.trim().is_empty() {
        "none"
    } else {
        values.delivery_slot.trim()
    };
    let delivery_slots: Vec<DeliverySlotOption> = crate::models::delivery::DELIVERY_SLOTS
        .iter()
        .filter(|slot| !settings.slot_disabled(slot.code))
        .map(|slot| DeliverySlotOption {
            code: slot.code.to_string(),
            label: slot.label.to_string(),
            selected: slot.code == current_slot,
        })
        .collect();
    render(CheckoutView {
        header: header_for(pool, session, false).await,
        values,
        errors,
        is_member,
        lines,
        totals,
        applied_coupons,
        held_coupons,
        coupon_error,
        error_message,
        prefectures: &PREFECTURES,
        delivery_dates,
        delivery_slots,
        saved_addresses,
    })
}

/// POST /checkout: coupon apply/remove re-renders the form with
/// live totals; the confirm intent validates the delivery fields
/// and moves on to the payment step.
async fn checkout_submit(
    session: Session,
    State(pool): State<PgPool>,
    Form(form): Form<CheckoutForm>,
) -> Response {
    let ctx = load_context(&pool, &session).await;
    let owner = &ctx.owner;
    let lines = &ctx.lines;
    if lines.iter().all(|l| !l.purchasable()) {
        return Redirect::to("/cart").into_response();
    }
    let user = &ctx.user;
    let is_member = user.is_some();
    let previous = load_checkout_flow::<CheckoutFlow>(&session).await;
    let (keep_coupons, keep_method, keep_method_chosen, keep_points) = match &previous {
        Some(f) => (
            f.coupon_codes.clone(),
            f.payment_method,
            f.payment_method_chosen,
            f.points_to_use,
        ),
        None => (
            Vec::new(),
            crate::models::PaymentMethod::default(),
            false,
            0,
        ),
    };
    let mut flow = form.to_flow(keep_coupons, keep_method, keep_method_chosen, keep_points);
    if let Some(user) = user {
        flow.email = user.email.clone();
    }
    // Availability coercion: a method the hosted gateway
    // cannot settle right now (integration off or misconfigured)
    // falls back to cash
    // on delivery — the customer is never carried into a payment
    // we cannot take.
    if !payment_service::hosted_available() && flow.payment_method.is_hosted() {
        flow.payment_method = crate::models::PaymentMethod::Cod;
    }

    // A targeted coupon removal posts remove_coupon:CODE; the
    // match below keys on the bare verb.
    let intent_verb = if form.intent.starts_with("remove_coupon") {
        "remove_coupon"
    } else {
        form.intent.as_str()
    };
    match intent_verb {
        "apply_coupon" => {
            // Either/or: a typed code wins; the dropdown's
            // selection is the fallback when the field is empty.
            // The selection is never trusted — it runs through
            // the engine exactly like a typed code, so a forged
            // value fails with the same generic error.
            let typed = form.coupon_code.trim().to_uppercase();
            let code = if !typed.is_empty() {
                typed
            } else {
                form.coupon_select.trim().to_uppercase()
            };
            if code.is_empty() {
                save_checkout_flow(&session, &flow).await;
                return build_checkout_view(
                    &pool,
                    &session,
                    flow,
                    CheckoutErrors::default(),
                    None,
                    None,
                    &ctx,
                )
                .await;
            }
            // The code applies when the engine slots it
            // alongside the codes already carried: its class
            // must be free (or held by an automatic discount it
            // displaces) and every condition must pass.
            let mut trial = flow.coupon_codes.clone();
            if !trial.contains(&code) {
                trial.push(code.clone());
            }
            let outcome = coupon_service::evaluate(&pool, owner.user_id(), lines, &trial).await;
            if outcome.contains(&code) {
                flow.coupon_codes = trial;
                save_checkout_flow(&session, &flow).await;
                build_checkout_view(
                    &pool,
                    &session,
                    flow,
                    CheckoutErrors::default(),
                    None,
                    None,
                    &ctx,
                )
                .await
            } else {
                save_checkout_flow(&session, &flow).await;
                build_checkout_view(
                    &pool,
                    &session,
                    flow,
                    CheckoutErrors::default(),
                    Some(COUPON_INVALID_MESSAGE.to_string()),
                    None,
                    &ctx,
                )
                .await
            }
        }
        "remove_coupon" => {
            // A targeted removal carries its code in the
            // intent value (remove_coupon:CODE); a bare
            // remove_coupon clears every entered code.
            match form.intent.strip_prefix("remove_coupon:") {
                Some(code) => {
                    let code = code.trim().to_uppercase();
                    flow.coupon_codes.retain(|c| c != &code);
                }
                None => flow.coupon_codes.clear(),
            }
            save_checkout_flow(&session, &flow).await;
            build_checkout_view(
                &pool,
                &session,
                flow,
                CheckoutErrors::default(),
                None,
                None,
                &ctx,
            )
            .await
        }
        _ => {
            // Address-book pick (Address Book spec §5.3): a
            // member who chose a saved address submits its id;
            // the values are copied from the book under the
            // member's ownership and replace the typed fields.
            // A missing, deleted or foreign id returns to the
            // delivery step with one message, and the order is
            // never placed from a failed pick.
            let choice = form.delivery_choice.trim();
            if is_member && !choice.is_empty() && choice != "new" {
                let picked = match choice.parse::<i64>() {
                    Ok(address_id) => member_address_service::find_owned(
                        &pool,
                        user.as_ref().map(|u| u.id).unwrap_or_default(),
                        address_id,
                    )
                    .await
                    .ok()
                    .flatten(),
                    Err(_) => None,
                };
                match picked {
                    Some(address) => {
                        flow.family_name = address.family_name.clone();
                        flow.given_name = address.given_name.clone();
                        flow.postal_code = address.postal_code.clone();
                        flow.prefecture = address.prefecture.clone();
                        flow.city = address.city.clone();
                        flow.address_line = address.address_line.clone();
                        flow.building = address.building.clone().unwrap_or_default();
                        flow.phone = address.phone;
                    }
                    None => {
                        save_checkout_flow(&session, &flow).await;
                        return build_checkout_view(
                            &pool,
                            &session,
                            flow,
                            CheckoutErrors::default(),
                            None,
                            Some(crate::messages::RESELECT_DELIVERY_MESSAGE.to_string()),
                            &ctx,
                        )
                        .await;
                    }
                }
            }
            // The save checkbox counts only on the typed path:
            // a picked address is already in the book.
            flow.save_address =
                is_member && form.save_address.is_some() && (choice.is_empty() || choice == "new");
            let settings = crate::services::shop_settings_service::load(&pool).await;
            let holidays = crate::services::holiday_service::upcoming(&pool).await;
            let errors = order_service::validate_delivery(&flow, is_member, &settings, &holidays);
            if !errors.is_empty() {
                save_checkout_flow(&session, &flow).await;
                return build_checkout_view(
                    &pool,
                    &session,
                    flow,
                    CheckoutErrors::from_field_errors(&errors),
                    None,
                    None,
                    &ctx,
                )
                .await;
            }
            // A guest's typed email is captured against
            // their cart here — the one moment it exists —
            // so the cart reminder can reach an abandoned
            // guest cart later (C3 completion). Members
            // need no capture: their address is the account's.
            if !is_member && let Some(session_key) = owner.session_key() {
                crate::services::cart_reminder_service::capture_guest_contact(
                    &pool,
                    session_key,
                    &flow.email,
                )
                .await;
            }
            save_checkout_flow(&session, &flow).await;
            // Delivery is settled; the payment step is its own page.
            Redirect::to("/checkout/payment").into_response()
        }
    }
}

/// The purchasable lines and the live totals for a flow: its
/// coupon evaluated against the current cart. Shared by the
/// payment step's render, its limit check and the confirmation.
async fn flow_totals(
    pool: &PgPool,
    ctx: &CheckoutContext,
    flow: &CheckoutFlow,
    method: Option<crate::models::PaymentMethod>,
) -> (
    Vec<crate::models::CartLine>,
    cart_service::CartTotals,
    coupon_service::CouponOutcome,
) {
    let lines: Vec<_> = ctx
        .lines
        .iter()
        .filter(|l| l.purchasable())
        .cloned()
        .collect();
    let outcome =
        coupon_service::evaluate(pool, ctx.owner.user_id(), &lines, &flow.coupon_codes).await;
    let tax = crate::services::tax_service::params_today_or_fallback(pool).await;
    let fees = crate::services::shop_settings_service::load(pool)
        .await
        .fee_settings();
    let totals = cart_service::totals(
        &lines,
        outcome.discount_yen(),
        outcome.shipping_discount_yen,
        method,
        tax,
        fees,
    );
    (lines, totals, outcome)
}

/// The payment step's form: the chosen method's slug, and
/// the points the member wants to spend (EC V2 core 1).
#[derive(Deserialize)]
pub struct PaymentMethodForm {
    /// The chosen payment method's slug.
    #[serde(default)]
    payment_method: String,
    /// Points to spend, as typed (empty = none).
    #[serde(default)]
    points_to_use: Option<String>,
}

/// GET /checkout/payment: the dedicated payment step. It needs
/// the delivery step's saved flow; without one it sends the
/// visitor back to the delivery form.
async fn checkout_payment_page(session: Session, State(pool): State<PgPool>) -> Response {
    let ctx = load_context(&pool, &session).await;
    if ctx.lines.iter().all(|l| !l.purchasable()) {
        return Redirect::to("/cart").into_response();
    }
    let Some(mut flow) = load_checkout_flow::<CheckoutFlow>(&session).await else {
        return Redirect::to("/checkout").into_response();
    };
    // Availability coercion, the same rule as the delivery
    // submit: a method the hosted gateway cannot settle right
    // now (the integration is off or misconfigured) falls back
    // to COD.
    if !payment_service::hosted_available() && flow.payment_method.is_hosted() {
        flow.payment_method = crate::models::PaymentMethod::Cod;
    }
    // A one-shot notice (a placement refused over moved
    // figures) rides the payment step's message slot, once.
    let notice = session
        .get::<String>(session::SESSION_CHECKOUT_NOTICE)
        .await
        .ok()
        .flatten();
    if notice.is_some() {
        let _ = session
            .remove::<String>(session::SESSION_CHECKOUT_NOTICE)
            .await;
    }
    build_payment_view(&pool, &session, flow, notice, &ctx, None).await
}

/// Builds the payment view: the offered methods, and the live
/// totals the method choice moves.
async fn build_payment_view(
    pool: &PgPool,
    session: &Session,
    flow: CheckoutFlow,
    payment_error: Option<String>,
    ctx: &CheckoutContext,
    points_error: Option<String>,
) -> Response {
    // The summary follows the checked radio, chosen or not:
    // the fee row and the grand total describe that method.
    let (lines, totals, _outcome) = flow_totals(pool, ctx, &flow, Some(flow.payment_method)).await;
    let settings = crate::services::shop_settings_service::load(pool).await;
    let fees = settings.fee_settings();
    // The points section exists only for a member while the
    // programme is on; its balance is read fresh (a stale
    // balance would promise points the member no longer has
    // — placement re-proves it under lock regardless).
    let points_balance = match (&ctx.user, settings.points_settings().enabled) {
        (Some(user), true) => crate::services::point_service::balance(pool, user.id)
            .await
            .ok(),
        _ => None,
    };
    let payment_options: Vec<PaymentMethodOption> = payment_service::offered_methods()
        .into_iter()
        .map(|method| PaymentMethodOption {
            slug: method.slug(),
            label: method.label(),
            note: method.note(fees, payment_service::active_gateway_name()),
            checked: method == flow.payment_method,
        })
        .collect();
    // The two grand totals the summary's method-switch script
    // swaps between: identical except for the COD fee.
    let fee_if_cod = if totals.subtotal_yen > 0 {
        fees.cod_fee_yen
    } else {
        0
    };
    let grand_total_cod =
        crate::pricing::display_price(totals.total_yen - totals.cod_fee_yen + fee_if_cod);
    let grand_total_online = crate::pricing::display_price(totals.total_yen - totals.cod_fee_yen);
    render(CheckoutPaymentView {
        header: header_for(pool, session, false).await,
        flow,
        lines,
        totals,
        payment_options,
        payment_error,
        grand_total_cod,
        grand_total_online,
        cod_fee_amount: crate::pricing::display_price(fee_if_cod),
        points_balance,
        points_error,
    })
}

/// POST /checkout/payment: records the chosen method and renders
/// the confirmation page as this response.
async fn checkout_payment_submit(
    session: Session,
    State(pool): State<PgPool>,
    Form(form): Form<PaymentMethodForm>,
) -> Response {
    let ctx = load_context(&pool, &session).await;
    if ctx.lines.iter().all(|l| !l.purchasable()) {
        return Redirect::to("/cart").into_response();
    }
    let Some(mut flow) = load_checkout_flow::<CheckoutFlow>(&session).await else {
        return Redirect::to("/checkout").into_response();
    };
    // An unlisted value parses to nothing and lands on the
    // default: a forged method settles nothing.
    flow.payment_method =
        crate::models::PaymentMethod::from_slug(form.payment_method.trim()).unwrap_or_default();
    // The retired agent dummy method is not a posted choice
    // either: nothing in the flow sets it, so a forged slug
    // for it lands on the default like any unlisted value.
    if flow.payment_method.is_agent_test() {
        flow.payment_method = crate::models::PaymentMethod::default();
    }
    // Availability coercion: a method the hosted gateway
    // cannot settle right now falls back to cash on delivery —
    // the customer is never
    // carried into a payment we cannot take.
    if !payment_service::hosted_available() && flow.payment_method.is_hosted() {
        flow.payment_method = crate::models::PaymentMethod::Cod;
    }
    // From here the method is a real choice: the confirmation
    // (and any later step) charges the fee of the method picked.
    flow.payment_method_chosen = true;
    // Points (EC V2 core 1): a member's typed amount is
    // checked against the live balance here; placement
    // re-proves it under the ledger lock, so a balance that
    // moves after this screen still cannot overspend.
    // Guests, and a disabled programme, always carry zero.
    let points_on = crate::services::shop_settings_service::load(&pool)
        .await
        .points_settings()
        .enabled;
    match (&ctx.user, points_on) {
        (Some(user), true) => {
            let raw = form.points_to_use.as_deref().unwrap_or("").trim();
            let parsed = if raw.is_empty() {
                Ok(0)
            } else {
                raw.parse::<i64>().map_err(|_| ())
            };
            match parsed {
                Ok(value) if value >= 0 => {
                    flow.points_to_use = value;
                    let balance = crate::services::point_service::balance(&pool, user.id)
                        .await
                        .unwrap_or(0);
                    if value > balance {
                        save_checkout_flow(&session, &flow).await;
                        return build_payment_view(
                            &pool,
                            &session,
                            flow,
                            None,
                            &ctx,
                            Some("ポイント残高が不足しています。".to_string()),
                        )
                        .await;
                    }
                }
                _ => {
                    flow.points_to_use = 0;
                    save_checkout_flow(&session, &flow).await;
                    return build_payment_view(
                        &pool,
                        &session,
                        flow,
                        None,
                        &ctx,
                        Some("ポイントは0以上の整数で入力してください。".to_string()),
                    )
                    .await;
                }
            }
        }
        _ => flow.points_to_use = 0,
    }
    // The one method limit the checkout can check itself:
    // konbini's ceiling. Over it, the payment step re-renders
    // with the error under the method choice.
    let (purchasable, totals, outcome) =
        flow_totals(&pool, &ctx, &flow, flow.chosen_method()).await;
    if let Some(max) = flow.payment_method.max_total_yen()
        && totals.total_yen > max
    {
        save_checkout_flow(&session, &flow).await;
        return build_payment_view(
            &pool,
            &session,
            flow,
            Some(format!(
                "コンビニ決済は{}までのお支払いにご利用いただけます。",
                crate::pricing::display_price(max)
            )),
            &ctx,
            None,
        )
        .await;
    }
    save_checkout_flow(&session, &flow).await;
    // stamp the fingerprint of the very lines and
    // totals this confirmation is about to show. The placement
    // POST consumes it and place_order refuses to place at
    // any other figures. Fail CLOSED on a stamp error
    // without the stamp, placement
    // could not be bound to these figures, so the shopper
    // retries the payment step instead of confirming into
    // an unbound placement.
    if let Err(e) = session
        .insert(
            session::SESSION_CONFIRM_FINGERPRINT,
            crate::services::agent_approval_service::fingerprint_for(&purchasable, &totals),
        )
        .await
    {
        tracing::error!("checkout fingerprint stamp failed: {e}");
        return build_payment_view(
            &pool,
            &session,
            flow,
            None,
            &ctx,
            Some("処理に失敗しました。もう一度お試しください。".to_string()),
        )
        .await;
    }
    // Confirmation: the POST response IS the review page.
    let points_used_yen = flow.points_to_use.clamp(0, totals.total_yen);
    render(CheckoutConfirmView {
        header: header_for(&pool, &session, false).await,
        points_used_yen,
        payable_display: crate::pricing::display_price(totals.total_yen - points_used_yen),
        flow,
        lines: purchasable,
        totals,
        applied_coupons: outcome.applied.clone(),
        fees: crate::services::shop_settings_service::load(&pool)
            .await
            .fee_settings(),
        gateway_name: payment_service::active_gateway_name(),
    })
}

/// Sends a confirm POST back to the payment step with
/// the re-confirm notice: the shared landing of a figure
/// drift (`PlaceError::PriceChanged`) and of a missing or
/// unreadable fingerprint (D5's fail-closed read) — in
/// both cases nothing was written and the cart is
/// untouched, so the shopper simply re-confirms.
async fn refuse_for_reconfirm(session: &Session, flow: &CheckoutFlow) -> Response {
    save_checkout_flow(session, flow).await;
    let _ = session
        .insert(
            session::SESSION_CHECKOUT_NOTICE,
            "商品の価格や在庫状況が変わったため、合計金額が更新されました。お手数ですが、内容をご確認のうえ、再度お手続きください。",
        )
        .await;
    Redirect::to("/checkout/payment").into_response()
}

/// POST /checkout/confirm: places the order. A repeated submit
/// (double click, refresh of the POST) finds the cart empty but
/// last_order_id set, and lands on the same completion page
/// instead of creating a second order.
async fn checkout_confirm(session: Session, State(pool): State<PgPool>) -> Response {
    // Take (not load) the flow: placement consumes it, so a
    // simultaneous second confirm cannot place a duplicate.
    let Some(flow) = take_checkout_flow::<CheckoutFlow>(&session).await else {
        // No checkout was started in this session.
        if session
            .get::<i64>(session::SESSION_LAST_ORDER)
            .await
            .ok()
            .flatten()
            .is_some()
        {
            return Redirect::to("/checkout/complete").into_response();
        }
        return Redirect::to("/cart").into_response();
    };
    let owner = cart_service::owner_for(&pool, &session).await;
    // The confirmation's fingerprint is one-shot: read and
    // remove it before placing, whatever the outcome. Fail
    // the stamp is the only
    // thing tying placement to the figures the shopper
    // confirmed, so a missing or unreadable stamp refuses
    // placement exactly like a figure drift — never a
    // silent downgrade to an unbound placement.
    let expected_fingerprint = match session
        .get::<String>(session::SESSION_CONFIRM_FINGERPRINT)
        .await
    {
        Ok(Some(fingerprint)) => fingerprint,
        Ok(None) => return refuse_for_reconfirm(&session, &flow).await,
        Err(e) => {
            tracing::error!("checkout fingerprint read failed; refusing placement: {e}");
            return refuse_for_reconfirm(&session, &flow).await;
        }
    };
    let _ = session
        .remove::<String>(session::SESSION_CONFIRM_FINGERPRINT)
        .await;
    match order_service::place_order(
        &pool,
        &owner,
        &flow,
        crate::models::order::PlacedVia::Storefront,
        Some(&expected_fingerprint),
    )
    .await
    {
        Ok(order) => {
            // The session bookkeeping lives here, not in the
            // service: placement is a pure database operation.
            // The completion target depends on the method: cash
            // on delivery completes as it always has; a KOMOJU
            // order continues to the hosted payment page, whose
            // session is created only now — after the placement
            // transaction has committed.
            let _ = session.insert(session::SESSION_LAST_ORDER, order.id).await;
            // The address-book save the member asked for at the
            // delivery step happens now — at placement, never
            // before, so an abandoned checkout saves nothing.
            // Only the at-cap outcome is surfaced, as a note the
            // completion page shows once.
            if flow.save_address
                && let Some(user_id) = owner.user_id()
                && member_address_service::save_from_checkout(&pool, user_id, &flow).await
                    == member_address_service::CheckoutSaveOutcome::AtCap
            {
                let _ = session
                    .insert(
                        session::SESSION_ADDRESS_SAVE_NOTE,
                        "アドレス帳の上限に達したため、住所は保存されませんでした",
                    )
                    .await;
            }
            // The session decision tests the PAYABLE amount
            // (total minus points spent), mirroring the
            // payment row write_order just created: a fully
            // points-covered order has nothing left to
            // collect, so it settles in place and takes the
            // completion path — KOMOJU cannot open a ¥0
            // session.
            if !flow
                .payment_method
                .settles_without_session(order.payable_yen())
            {
                return match payment_service::start_session(&pool, order.id).await {
                    Ok(session_url) => Redirect::to(&session_url).into_response(),
                    Err(e) => {
                        tracing::error!(
                            order_id = order.id,
                            "payment session start failed after placement: {e}"
                        );
                        render(PaymentErrorView {
                            header: header_for(&pool, &session, false).await,
                            order_number: order.order_number(),
                            is_member: session::current_user(&pool, &session).await.is_some(),
                        })
                    }
                };
            }
            order_service::spawn_confirmation_mail(pool.clone(), order.id);
            Redirect::to("/checkout/complete").into_response()
        }
        Err(PlaceError::PriceChanged) => {
            // Nothing was written and the cart is untouched:
            // back to the payment step to re-confirm at the
            // current figures.
            refuse_for_reconfirm(&session, &flow).await
        }
        Err(PlaceError::EmptyCart) => {
            if session
                .get::<i64>(session::SESSION_LAST_ORDER)
                .await
                .ok()
                .flatten()
                .is_some()
            {
                Redirect::to("/checkout/complete").into_response()
            } else {
                Redirect::to("/cart").into_response()
            }
        }
        Err(PlaceError::StockShortage) => {
            let ctx = load_context(&pool, &session).await;
            // Retryable: the flow goes back for another attempt.
            save_checkout_flow(&session, &flow).await;
            build_checkout_view(
                &pool,
                &session,
                flow,
                CheckoutErrors::default(),
                None,
                Some("在庫が不足しています。カートに戻ってご確認ください。".to_string()),
                &ctx,
            )
            .await
        }
        Err(PlaceError::CouponInvalid) => {
            let ctx = load_context(&pool, &session).await;
            let mut flow = flow;
            flow.coupon_codes.clear();
            save_checkout_flow(&session, &flow).await;
            build_checkout_view(
                &pool,
                &session,
                flow,
                CheckoutErrors::default(),
                Some(COUPON_INVALID_MESSAGE.to_string()),
                None,
                &ctx,
            )
            .await
        }
        Err(PlaceError::Undeliverable(product_name)) => {
            let ctx = load_context(&pool, &session).await;
            save_checkout_flow(&session, &flow).await;
            build_checkout_view(
                &pool,
                &session,
                flow,
                CheckoutErrors::default(),
                None,
                Some(format!(
                    "「{product_name}」はお届け先の都道府県へ配送できません。カートに戻ってご確認ください。"
                )),
                &ctx,
            )
            .await
        }
        Err(PlaceError::DeliveryInvalid(message)) => {
            let ctx = load_context(&pool, &session).await;
            // The request is re-asked on the delivery step: the
            // flow goes back with the message under the date
            // field and the picker rebuilt on the new window.
            // The carried values stay as the shopper left them —
            // nothing is silently moved to a valid date.
            save_checkout_flow(&session, &flow).await;
            build_checkout_view(
                &pool,
                &session,
                flow,
                CheckoutErrors {
                    delivery_date: Some(message),
                    ..Default::default()
                },
                None,
                None,
                &ctx,
            )
            .await
        }
        Err(PlaceError::PaymentMethodLimit(message)) => {
            // Placement refused the method's ceiling: nothing
            // was written. Back to the payment step with the
            // reason shown once, like the price-change path.
            save_checkout_flow(&session, &flow).await;
            let _ = session
                .insert(session::SESSION_CHECKOUT_NOTICE, message)
                .await;
            Redirect::to("/checkout/payment").into_response()
        }
        Err(PlaceError::Db(e)) => {
            let ctx = load_context(&pool, &session).await;
            save_checkout_flow(&session, &flow).await;
            tracing::error!("Order placement failed: {e}");
            build_checkout_view(
                &pool,
                &session,
                flow,
                CheckoutErrors::default(),
                None,
                Some("注文処理に失敗しました。しばらくしてから再度お試しください。".to_string()),
                &ctx,
            )
            .await
        }
    }
}

/// GET /checkout/complete: shows the order named by the session's
/// last_order_id. Refreshing keeps showing the same order.
async fn checkout_complete(session: Session, State(pool): State<PgPool>) -> Response {
    let Some(order_id) = session
        .get::<i64>(session::SESSION_LAST_ORDER)
        .await
        .ok()
        .flatten()
    else {
        return Redirect::to("/").into_response();
    };
    // A database failure is logged and goes home like a missing
    // order: the completion page must never render a half-loaded
    // order.
    let order = match order_service::find_order(&pool, order_id).await {
        Ok(Some(order)) => order,
        Ok(None) => return Redirect::to("/").into_response(),
        Err(e) => {
            tracing::error!(order_id, "completion order load failed: {e}");
            return Redirect::to("/").into_response();
        }
    };
    let lines = match order_service::order_lines_with_images(&pool, order.id).await {
        Ok(lines) => lines,
        Err(e) => {
            tracing::error!(order_id, "completion lines load failed: {e}");
            return Redirect::to("/").into_response();
        }
    };
    // The tax breakdown and the payment row ride one
    // service call: a failure of either
    // goes home like a missing order, honouring the rule
    // above — the completion page must never render a
    // half-loaded order (a paid KOMOJU order shown without
    // its payment row would read as cash on delivery).
    let (tax_breakdown, payment_row) = match order_service::completion_extras(&pool, order.id).await
    {
        Ok(pair) => pair,
        Err(e) => {
            tracing::error!(order_id, "completion extras load failed: {e}");
            return Redirect::to("/").into_response();
        }
    };
    let is_member = session::current_user(&pool, &session).await.is_some();
    // The note beside the total: today's wording for cash on
    // delivery (and for orders that predate payment rows), the
    // method and its state for hosted-gateway orders (KOMOJU
    // or Stripe — the rail on the row decides, not the rail
    // configured now).
    let payment_note = match payment_row {
        Some(payment) if payment.provider == "agent" => paid_method_label(payment.method_label()),
        Some(payment) if payment.is_hosted() => {
            if payment.is_paid() {
                paid_method_label(payment.method_label())
            } else {
                format!("（{}・お支払い待ち）", payment.method_label())
            }
        }
        _ => "（代金引換・お届け時に現金でお支払いください）".to_string(),
    };
    // The address-book save note, when the placement skipped
    // a requested save at the book's cap: shown on this page
    // once, then cleared.
    let address_note = session
        .get::<String>(session::SESSION_ADDRESS_SAVE_NOTE)
        .await
        .ok()
        .flatten();
    if address_note.is_some() {
        let _ = session
            .remove::<String>(session::SESSION_ADDRESS_SAVE_NOTE)
            .await;
    }
    render(CheckoutCompleteView {
        header: header_for(&pool, &session, false).await,
        order,
        lines,
        is_member,
        payment_note,
        address_note,
        tax_breakdown,
    })
}
