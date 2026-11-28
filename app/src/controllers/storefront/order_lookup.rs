//! Guest order lookup controller.
//! The public status page for shoppers who checked out as
//! guests: the order number plus the email stored on the
//! order is the whole credential.
//!
//! Anti-enumeration is the design centre, as in the password
//! reset flow: a wrong number, a wrong date segment and a
//! wrong email all return the same form, the same single
//! message and the same status, so the page can never be
//! used to test which order numbers or addresses exist.
//! Failed matches feed the lookup throttle exactly like
//! failed logins feed the login throttle. Nothing about the
//! submitted pair is logged.

use std::net::SocketAddr;

use axum::{
    Form, Router,
    extract::{ConnectInfo, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
};
use serde::Deserialize;
use sqlx::PgPool;
use tower_sessions::Session;

use super::{header_for, render};
use crate::messages::{EMAIL_FORMAT_MESSAGE, TOO_MANY_REQUESTS_MESSAGE};
use crate::middleware::auth_throttle;
use crate::services::order_service;
use crate::validation::{FieldError, is_valid_email};
use crate::views::storefront::order_lookup::{
    GuestOrderView, LookupErrors, LookupValues, OrderLookupView,
};

/// The single message for every failed match: it never says
/// which half of the pair was wrong.
const MISMATCH_MESSAGE: &str =
    "注文番号またはメールアドレスが一致しませんでした。ご確認のうえ、もう一度お試しください。";

// The too-many-requests message, shared wording with the
// password reset flow.

/// Routes owned by the guest order lookup controller.
pub fn router() -> Router<PgPool> {
    Router::new()
        .route("/order-lookup", get(form_page).post(lookup_submit))
        .route("/order-lookup/receipt", axum::routing::post(receipt_submit))
}

/// Form posted by the lookup page.
#[derive(Deserialize)]
pub struct LookupForm {
    order_number: String,
    email: String,
}

/// GET /order-lookup: the blank lookup form.
async fn form_page(session: Session, State(pool): State<PgPool>) -> Response {
    render(OrderLookupView {
        header: header_for(&pool, &session, false).await,
        values: LookupValues::default(),
        errors: LookupErrors::default(),
        form_error: None,
    })
}

/// Renders the form again with preserved values and one
/// error shape — the shared tail of every refusal below.
async fn render_form(
    pool: &PgPool,
    session: &Session,
    values: LookupValues,
    errors: LookupErrors,
    form_error: Option<String>,
) -> Response {
    render(OrderLookupView {
        header: header_for(pool, session, false).await,
        values,
        errors,
        form_error,
    })
}

/// The outcome of the shared credential check behind the
/// lookup page and its receipt post.
enum LookupMatch {
    /// The pair matched this order.
    Matched(Box<crate::models::Order>),
    /// A refusal response, rendered exactly as a failed
    /// lookup renders it.
    Refused(Response),
}

/// The lookup's credential check, shared verbatim with the
/// receipt post so the receipt is no weaker an oracle:
/// shape errors answer as form errors (public rules, they
/// leak nothing), a locked pair or IP is refused up front,
/// and every well-formed mismatch records a throttle
/// failure and renders the one indistinguishable message.
async fn match_order(pool: &PgPool, session: &Session, form: &LookupForm, ip: &str) -> LookupMatch {
    let values = LookupValues {
        order_number: form.order_number.clone(),
        email: form.email.clone(),
    };
    // Shape first: a malformed pair is a form error, not a
    // match attempt. The shape rules are public knowledge, so
    // answering them leaks nothing — and they never reach the
    // throttle or the database.
    let mut errors = Vec::new();
    if crate::models::order::parse_order_number(&form.order_number).is_none() {
        errors.push(FieldError::new(
            "order_number",
            "注文番号の形式が正しくありません。",
        ));
    }
    if !is_valid_email(form.email.trim()) {
        errors.push(FieldError::new("email", EMAIL_FORMAT_MESSAGE));
    }
    if !errors.is_empty() {
        return LookupMatch::Refused(
            render_form(
                pool,
                session,
                values,
                LookupErrors::from_field_errors(&errors),
                None,
            )
            .await,
        );
    }
    // The pair is the credential: refuse up front while this
    // (email, IP) pair or IP is locked, before any lookup.
    if auth_throttle::lookup_is_blocked(&form.email, ip) {
        return LookupMatch::Refused(
            render_form(
                pool,
                session,
                values,
                LookupErrors::default(),
                Some(TOO_MANY_REQUESTS_MESSAGE.to_string()),
            )
            .await,
        );
    }
    let order =
        match order_service::find_order_for_lookup(pool, &form.order_number, &form.email).await {
            Ok(Some(order)) => order,
            Ok(None) => {
                auth_throttle::lookup_record_failure(&form.email, ip);
                return LookupMatch::Refused(
                    render_form(
                        pool,
                        session,
                        values,
                        LookupErrors::default(),
                        Some(MISMATCH_MESSAGE.to_string()),
                    )
                    .await,
                );
            }
            Err(e) => {
                tracing::error!("guest order lookup failed: {e}");
                return LookupMatch::Refused(
                    (StatusCode::INTERNAL_SERVER_ERROR, "Internal Server Error").into_response(),
                );
            }
        };
    auth_throttle::lookup_record_success(&form.email, ip);
    LookupMatch::Matched(Box::new(order))
}

/// POST /order-lookup: matches the pair and shows the order,
/// or returns the identical mismatch form for every failure.
async fn lookup_submit(
    session: Session,
    State(pool): State<PgPool>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Form(form): Form<LookupForm>,
) -> Response {
    let ip = crate::session::client_ip(&addr, &headers);
    let order = match match_order(&pool, &session, &form, &ip).await {
        LookupMatch::Matched(order) => *order,
        LookupMatch::Refused(response) => return response,
    };
    // The detail reads: a failure here is a 500, never a
    // half-rendered order.
    let lines = match order_service::order_lines_with_images(&pool, order.id).await {
        Ok(lines) => lines,
        Err(e) => {
            tracing::error!(order_id = order.id, "guest lookup lines failed: {e}");
            return (StatusCode::INTERNAL_SERVER_ERROR, "Internal Server Error").into_response();
        }
    };
    let (tax_breakdown, payment_row) = match order_service::completion_extras(&pool, order.id).await
    {
        Ok(extras) => extras,
        Err(e) => {
            tracing::error!(order_id = order.id, "guest lookup extras failed: {e}");
            return (StatusCode::INTERNAL_SERVER_ERROR, "Internal Server Error").into_response();
        }
    };
    let payment = payment_row.as_ref().map(|row| {
        crate::views::storefront::payment::PaymentBlockView::from_payment(row, &order.status)
    });
    let shipments = super::mypage::public_shipment_rows(&pool, order.id).await;
    render(GuestOrderView {
        header: header_for(&pool, &session, false).await,
        order,
        lines,
        payment,
        tax_breakdown,
        shipments,
    })
}

/// `POST /order-lookup/receipt`: the guest receipt （領収書）.
/// The form on the lookup result carries the same pair the
/// shopper just proved; the match is the lookup's own
/// ([`match_order`] — same throttle, same indistinguishable
/// refusals), and only a matched order renders a receipt.
async fn receipt_submit(
    session: Session,
    State(pool): State<PgPool>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Form(form): Form<LookupForm>,
) -> Response {
    let ip = crate::session::client_ip(&addr, &headers);
    let order = match match_order(&pool, &session, &form, &ip).await {
        LookupMatch::Matched(order) => *order,
        LookupMatch::Refused(response) => return response,
    };
    match super::receipt_view(&pool, &order).await {
        Ok(view) => render(view),
        Err(e) => {
            tracing::error!(order_id = order.id, "guest receipt failed: {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, "Internal Server Error").into_response()
        }
    }
}
