//! Payment reconciliation.
//!
//! The read-only 決済照合 page: the CLI reconciliation
//! script's three questions as a screen, with order links.
//! Gated to the sales area by the permission guard.

use axum::{Router, extract::State, response::Response, routing::get};
use sqlx::PgPool;
use tower_sessions::Session;

use super::{db_error, layout_for, render};
use crate::models::PaymentMethod;
use crate::models::jst::jst_datetime_label;
use crate::pricing;
use crate::services::admin::reconciliation_service;
use crate::session::CurrentAdmin;
use crate::views::admin::payments::{CapturedRow, FlaggedRow, OpenPaymentRow, PaymentsView};

/// Route: the reconciliation page.
pub fn router() -> Router<PgPool> {
    Router::new().route("/admin/payments", get(page))
}

/// A payment method code's display label.
fn method_label(code: &str) -> String {
    PaymentMethod::from_slug(code)
        .map(|m| m.label().to_string())
        .unwrap_or_else(|| code.to_string())
}

/// Renders the reconciliation page.
pub async fn page(
    State(pool): State<PgPool>,
    CurrentAdmin(admin): CurrentAdmin,
    session: Session,
) -> Response {
    let (open, captured, flagged) = match reconciliation_service::load(&pool).await {
        Ok(lists) => lists,
        Err(e) => return db_error("payments", e),
    };
    render(PaymentsView {
        layout: layout_for(&pool, &admin, &session).await,
        nav: "payments",
        open: open
            .iter()
            .map(|p| OpenPaymentRow {
                order_number: crate::models::order::order_number_for(
                    p.order_id,
                    p.order_created_at,
                ),
                order_url: format!("/admin/orders/{}", p.order_id),
                method_label: method_label(&p.method),
                status_label: crate::models::payment::status_label_for_status(&p.status)
                    .to_string(),
                amount: pricing::display_price(p.amount_yen),
                created_label: jst_datetime_label(p.created_at),
                deadline_label: p
                    .payment_deadline
                    .map(jst_datetime_label)
                    .unwrap_or_else(|| "—".to_string()),
            })
            .collect(),
        captured: captured
            .iter()
            .map(|c| CapturedRow {
                method_label: method_label(&c.method),
                count: c.count,
                net: pricing::display_price(c.net_yen),
            })
            .collect(),
        flagged: flagged
            .iter()
            .map(|p| FlaggedRow {
                order_number: crate::models::order::order_number_for(
                    p.order_id,
                    p.order_created_at,
                ),
                order_url: format!("/admin/orders/{}", p.order_id),
                status_label: crate::models::payment::status_label_for_status(&p.status)
                    .to_string(),
                flag_note: p.flag_note.clone(),
                updated_label: jst_datetime_label(p.updated_at),
            })
            .collect(),
    })
}
