//! MCP order draft approval: the human half of MCP
//! Phase 3.
//!
//! An MCP client freezes a cart into a draft and hands
//! the person an approval URL. This page is where the
//! order is actually placed: the person checks the
//! frozen lines, the delivery address and the totals,
//! picks a payment method, and the pay click runs the
//! agent settlement's exact sequence — verify the
//! binding, bind the final state, place through
//! [`order_service::place_order`], consume the draft —
//! so an MCP draft and the in-store agent's settlement
//! are the same discipline on two doors. The client
//! itself never places an order and never sees this
//! page's session.

use axum::{
    Form, Router,
    extract::{Path, State},
    response::{IntoResponse, Redirect, Response},
    routing::{get, post},
};
use serde::Deserialize;
use sqlx::PgPool;
use tower_sessions::Session;

use super::{header_for, render};
use crate::messages::ORDER_PROCESSING_FAILED_NOTICE;
use crate::models::PaymentMethod;
use crate::repositories::mcp_order_draft_repository::McpOrderDraftRow;
use crate::services::cart_service::CartOwner;
use crate::services::order_service::{self, PlaceError};
use crate::services::{agent_approval_service, mcp_order_draft_service, payment_service};
use crate::session;
use crate::views::storefront::checkout::PaymentMethodOption;
use crate::views::storefront::mcp_draft::OrderDraftView;
use crate::views::storefront::payment::PaymentErrorView;

/// Routes owned by the draft approval controller.
pub fn router() -> Router<PgPool> {
    Router::new()
        .route("/orders/draft/{token}", get(draft_page))
        .route("/orders/draft/{token}/pay", post(draft_pay))
}

/// The explanation for a token that names no draft: the
/// same answer whatever the reason (unknown, mistyped,
/// or a draft whose row is gone) — the token is a bearer
/// credential and its failures are not enumerated.
const INVALID_LINK_NOTICE: &str = "この注文リンクは無効か、有効期限が切れています。";

/// The pay form: the chosen payment method's slug.
#[derive(Debug, Deserialize)]
pub struct DraftPayForm {
    /// The payment method the person picked.
    #[serde(default)]
    pub payment_method: String,
}

/// GET /orders/draft/{token}: the approval page. A live
/// draft shows its lines, address, totals and the pay
/// form; a dead one shows why — the states the agent
/// settlement explains in chat, on a page.
async fn draft_page(
    session: Session,
    State(pool): State<PgPool>,
    Path(token): Path<String>,
) -> Response {
    let draft = match load_draft(&pool, &token).await {
        Ok(draft) => draft,
        Err(_) => return server_error(),
    };
    let Some(draft) = draft else {
        return state_page(&pool, &session, &token, INVALID_LINK_NOTICE).await;
    };
    match draft.status.as_str() {
        "consumed" => state_page(&pool, &session, &token, "この注文はすでに完了しています。").await,
        "expired" => {
            state_page(
                &pool,
                &session,
                &token,
                &agent_approval_service::expired_notice(),
            )
            .await
        }
        "void" => {
            state_page(
                &pool,
                &session,
                &token,
                agent_approval_service::CHANGED_NOTICE,
            )
            .await
        }
        _ => match mcp_order_draft_service::verify_for_pay(&pool, &draft).await {
            Ok((lines, totals)) => {
                payable_page(
                    &pool,
                    &session,
                    &token,
                    &draft,
                    lines,
                    totals,
                    String::new(),
                )
                .await
            }
            Err(refusal) => state_page(&pool, &session, &token, &refusal.message).await,
        },
    }
}

/// POST /orders/draft/{token}/pay: the pay click.
/// Verifies the draft (pending, in window, still bound
/// to the live cart), binds it to the final state — the
/// lines as they stand, the method-dependent totals,
/// the method, the address — then places the order
/// through the storefront's own machinery and consumes
/// the draft onto it. Hosted methods continue to the
/// provider's page exactly like a human checkout; COD
/// completes here.
async fn draft_pay(
    session: Session,
    State(pool): State<PgPool>,
    Path(token): Path<String>,
    Form(form): Form<DraftPayForm>,
) -> Response {
    let draft = match load_draft(&pool, &token).await {
        Ok(draft) => draft,
        Err(_) => return server_error(),
    };
    let Some(draft) = draft else {
        return state_page(&pool, &session, &token, INVALID_LINK_NOTICE).await;
    };
    if draft.status == "consumed" {
        // A repeated submit (double click, refresh of the
        // POST) lands on the completion page when this
        // session placed the order, like the checkout
        // confirm's own repeat answer.
        if session
            .get::<i64>(session::SESSION_LAST_ORDER)
            .await
            .ok()
            .flatten()
            .is_some_and(|id| Some(id) == draft.order_id)
        {
            return Redirect::to("/checkout/complete").into_response();
        }
        return state_page(&pool, &session, &token, "この注文はすでに完了しています。").await;
    }
    let (lines, _standing) = match mcp_order_draft_service::verify_for_pay(&pool, &draft).await {
        Ok(state) => state,
        Err(refusal) => return state_page(&pool, &session, &token, &refusal.message).await,
    };
    let Some(method) = PaymentMethod::from_slug(form.payment_method.trim())
        .filter(|method| payment_service::offered_methods().contains(method))
    else {
        return payable_page(
            &pool,
            &session,
            &token,
            &draft,
            lines.clone(),
            recompute_totals(&pool, &lines, None).await,
            "お支払い方法を選んでください。".to_string(),
        )
        .await;
    };
    let totals = recompute_totals(&pool, &lines, Some(method)).await;
    if let Some(max) = method.max_total_yen()
        && totals.total_yen > max
    {
        return payable_page(
            &pool,
            &session,
            &token,
            &draft,
            lines,
            totals,
            crate::messages::payment_method_limit_message(method.label(), max),
        )
        .await;
    }
    let Some(address) = mcp_order_draft_service::address_of(&draft) else {
        tracing::error!("mcp order draft {} carries an unreadable address", draft.id);
        return server_error();
    };
    let flow = address.to_checkout_flow(method);
    match mcp_order_draft_service::bind_final(&pool, &draft, &lines, &totals, method, &flow).await {
        Ok(true) => {}
        Ok(false) => {
            return state_page(
                &pool,
                &session,
                &token,
                "この注文案は無効になっています。お手数ですが、もう一度はじめからお試しください。",
            )
            .await;
        }
        Err(e) => {
            tracing::error!("mcp order draft bind failed: {e}");
            return state_page(&pool, &session, &token, ORDER_PROCESSING_FAILED_NOTICE).await;
        }
    }
    // The bound figures double as the placement's expected
    // fingerprint: bind_final read the cart unlocked, so
    // any drift between that read and placement's locked
    // read refuses with PriceChanged instead of placing
    // under a stale approval — the settlement's own rule.
    let bound_fingerprint = agent_approval_service::fingerprint_for(&lines, &totals);
    let owner = CartOwner::Session(draft.cart_id.clone());
    match order_service::place_order(
        &pool,
        &owner,
        &flow,
        crate::models::order::PlacedVia::Agent,
        Some(&bound_fingerprint),
    )
    .await
    {
        Ok(order) => {
            match mcp_order_draft_service::consume(&pool, draft.id, order.id).await {
                Ok(true) => {}
                Ok(false) => {
                    tracing::error!(
                        "mcp order draft {} did not consume onto order {}",
                        draft.id,
                        order.id
                    );
                }
                Err(e) => {
                    tracing::error!("mcp order draft consume failed: {e}");
                }
            }
            if let Err(e) = session.insert(session::SESSION_LAST_ORDER, order.id).await {
                tracing::error!("failed to record last order: {e}");
            }
            if !method.settles_without_session(order.payable_yen()) {
                match payment_service::start_session(&pool, order.id).await {
                    Ok(url) => Redirect::to(&url).into_response(),
                    Err(e) => {
                        tracing::error!("mcp draft payment session start failed: {e}");
                        render(PaymentErrorView {
                            header: header_for(&pool, &session, false).await,
                            order_number: order.order_number(),
                            is_member: session::current_user(&pool, &session).await.is_some(),
                        })
                    }
                }
            } else {
                order_service::spawn_confirmation_mail(pool.clone(), order.id);
                Redirect::to("/checkout/complete").into_response()
            }
        }
        Err(PlaceError::StockShortage) => {
            tracing::warn!("mcp order draft placement hit a stock shortage");
            let totals = recompute_totals(&pool, &lines, None).await;
            payable_page(
                &pool,
                &session,
                &token,
                &draft,
                lines,
                totals,
                "在庫が不足している商品があります。カートの内容をご確認ください。".to_string(),
            )
            .await
        }
        Err(PlaceError::Undeliverable(name)) => {
            let totals = recompute_totals(&pool, &lines, None).await;
            payable_page(
                &pool,
                &session,
                &token,
                &draft,
                lines,
                totals,
                format!("「{name}」は現在お届けできない商品のため、注文を完了できませんでした。"),
            )
            .await
        }
        Err(PlaceError::DeliveryInvalid(message)) => {
            let totals = recompute_totals(&pool, &lines, None).await;
            payable_page(&pool, &session, &token, &draft, lines, totals, message).await
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
                state_page(
                    &pool,
                    &session,
                    &token,
                    agent_approval_service::CHANGED_NOTICE,
                )
                .await
            }
        }
        Err(PlaceError::PaymentMethodLimit(message)) => {
            let totals = recompute_totals(&pool, &lines, None).await;
            payable_page(&pool, &session, &token, &draft, lines, totals, message).await
        }
        Err(PlaceError::CouponInvalid | PlaceError::PriceChanged | PlaceError::Db(_)) => {
            // CouponInvalid is unreachable (a draft flow
            // carries no coupon); PriceChanged means the
            // figures drifted between the binding read and
            // placement's locked read — nothing was placed
            // and the draft stays unconsumed. Both explain
            // themselves the same way, as does a store
            // failure (already logged by the service).
            state_page(&pool, &session, &token, ORDER_PROCESSING_FAILED_NOTICE).await
        }
    }
}

/// Loads the draft a token names, with lazy expiry
/// applied. A store failure is a 500, never a state
/// page: the refusal copy must only ever mean the draft
/// itself failed.
async fn load_draft(
    pool: &PgPool,
    token: &str,
) -> Result<Option<McpOrderDraftRow>, crate::error::AppError> {
    let draft = mcp_order_draft_service::find_for_page(pool, token).await;
    if let Err(e) = &draft {
        tracing::error!("mcp order draft load failed: {e}");
    }
    draft
}

/// The plain 500 the draft surface answers store
/// failures with.
fn server_error() -> Response {
    (
        axum::http::StatusCode::INTERNAL_SERVER_ERROR,
        "Internal Server Error",
    )
        .into_response()
}

/// Recomputes a cart state's totals, with a payment
/// method when one is chosen (its COD fee applies) and
/// without one before that — the cart service's one
/// totals computation, as every surface runs it.
async fn recompute_totals(
    pool: &PgPool,
    lines: &[crate::models::CartLine],
    method: Option<PaymentMethod>,
) -> crate::services::cart_service::CartTotals {
    let tax = crate::services::tax_service::params_today_or_fallback(pool).await;
    let fees = crate::services::shop_settings_service::load(pool)
        .await
        .fee_settings();
    crate::services::cart_service::totals(lines, 0, 0, method, tax, fees)
}

/// Renders the terminal-state page: the explanation and
/// nothing to pay.
async fn state_page(pool: &PgPool, session: &Session, token: &str, notice: &str) -> Response {
    render(OrderDraftView {
        header: header_for(pool, session, false).await,
        payable: false,
        pay_url: format!("/orders/draft/{token}/pay"),
        notice: notice.to_string(),
        lines: Vec::new(),
        totals: crate::services::cart_service::CartTotals::default(),
        address: None,
        payment_options: Vec::new(),
        cod_fee_display: String::new(),
        subtotal_display: String::new(),
        shipping_display: String::new(),
        total_display: String::new(),
    })
}

/// Renders the payable page: the live lines, the draft's
/// address, the no-method totals, and the store's payment
/// methods with cash on delivery preselected, as the
/// checkout's payment step defaults.
async fn payable_page(
    pool: &PgPool,
    session: &Session,
    token: &str,
    draft: &McpOrderDraftRow,
    lines: Vec<crate::models::CartLine>,
    totals: crate::services::cart_service::CartTotals,
    notice: String,
) -> Response {
    let settings = crate::services::shop_settings_service::load(pool).await;
    let fees = settings.fee_settings();
    let payment_options: Vec<PaymentMethodOption> = payment_service::offered_methods()
        .into_iter()
        .map(|method| PaymentMethodOption {
            slug: method.slug(),
            label: method.label(),
            note: method.note(fees, payment_service::active_gateway_name()),
            checked: method == PaymentMethod::Cod,
        })
        .collect();
    let subtotal_display = crate::pricing::display_price(totals.subtotal_yen);
    let shipping_display = crate::pricing::display_price(totals.shipping_yen);
    let total_display = crate::pricing::display_price(totals.total_yen);
    render(OrderDraftView {
        header: header_for(pool, session, false).await,
        payable: true,
        pay_url: format!("/orders/draft/{token}/pay"),
        notice,
        lines: lines
            .into_iter()
            .filter(|line| line.purchasable())
            .collect(),
        totals,
        address: mcp_order_draft_service::address_of(draft),
        payment_options,
        cod_fee_display: crate::pricing::display_price(fees.cod_fee_yen),
        subtotal_display,
        shipping_display,
        total_display,
    })
}
