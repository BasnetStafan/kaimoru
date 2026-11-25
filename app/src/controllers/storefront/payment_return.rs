//! Payment return controller.
//!
//! KOMOJU sends the customer's browser back here after the
//! hosted page. The redirect itself proves nothing — a customer
//! can arrive here having paid, having abandoned, or by typing
//! the URL — so this handler re-fetches the session from KOMOJU
//! server-side, applies what it says through the one status
//! path, and only then decides what to show.
//!
//! Correlation is scoped: the session KOMOJU names must belong
//! to the order this browser session just placed. Anything else
//! earns the generic "confirming" page — never another order's
//! state, never an error that confirms an order exists.

use axum::{
    Router,
    extract::{Query, State},
    response::{IntoResponse, Redirect, Response},
    routing::get,
};
use serde::Deserialize;
use sqlx::PgPool;
use tower_sessions::Session;

use super::{header_for, render};
use crate::models::PaymentMethod;
use crate::repositories::payment_repository;
use crate::services::order_service;
use crate::services::payment_service;
use crate::session;
use crate::views::storefront::payment::{
    PaymentBlockView, PaymentInstructionsView, PaymentPendingView,
};

/// Routes owned by the payment return controller.
pub fn router() -> Router<PgPool> {
    Router::new().route("/checkout/payment/return", get(payment_return))
}

/// The query KOMOJU appends to the return URL.
#[derive(Deserialize)]
struct ReturnQuery {
    /// The session the customer is returning from.
    session_id: Option<String>,
}

/// GET /checkout/payment/return: reconcile, then show the
/// state the reconciliation produced.
async fn payment_return(
    session: Session,
    State(pool): State<PgPool>,
    Query(query): Query<ReturnQuery>,
) -> Response {
    let header = header_for(&pool, &session, false).await;
    let is_member = session::current_user(&pool, &session).await.is_some();
    let last_order_id = session
        .get::<i64>(session::SESSION_LAST_ORDER)
        .await
        .ok()
        .flatten();

    // Scoped correlation: the named session must be the payment
    // session of the order this browser just placed.
    let payment = match (&query.session_id, last_order_id) {
        (Some(session_id), Some(order_id)) => {
            match payment_repository::find_by_session_id(&pool, session_id).await {
                Ok(Some(payment)) if payment.order_id == order_id => Some(payment),
                Ok(_) => None,
                Err(e) => {
                    tracing::error!("payment return lookup failed: {e}");
                    None
                }
            }
        }
        _ => None,
    };
    let Some(payment) = payment else {
        // Uncorrelated: the generic confirming page. It refreshes
        // itself; the webhook settles the truth in the meantime.
        return render(PaymentPendingView {
            header,
            order_number: None,
            is_member,
        });
    };

    // Re-fetch before believing anything: the session's payment
    // object, applied through the one path. A fetch failure is
    // not fatal — the stored state still renders an honest page.
    if let Some(gateway) = payment_service::gateway_for(&payment.provider)
        && let Err(e) = payment_service::sync_from_provider(&pool, &gateway, &payment).await
    {
        tracing::warn!(payment_id = payment.id, "payment return sync failed: {e}");
    }
    let payment = payment_repository::find_by_id(&pool, payment.id)
        .await
        .ok()
        .flatten()
        .unwrap_or(payment);
    let order = order_service::find_order(&pool, payment.order_id)
        .await
        .ok()
        .flatten();

    if payment.is_paid() {
        return Redirect::to("/checkout/complete").into_response();
    }
    match (payment.method_kind(), order) {
        (PaymentMethod::Konbini, Some(order)) if payment.instructions_url.is_some() => {
            render(PaymentInstructionsView {
                header,
                payment: PaymentBlockView::from_payment(&payment, &order.status),
                order,
                is_member,
            })
        }
        (_, Some(order)) => render(PaymentPendingView {
            header,
            order_number: Some(order.order_number()),
            is_member,
        }),
        _ => render(PaymentPendingView {
            header,
            order_number: None,
            is_member,
        }),
    }
}
