//! Return requests screen.

use askama::Template;

use crate::controllers::admin::AdminLayout;
use crate::models::return_request;
use crate::repositories::admin::return_repository::ReturnRequestRow;

/// One return row, display-ready.
pub struct ReturnRow {
    /// Request id (status form target).
    pub id: i64,
    /// Order id (detail link).
    pub order_id: i64,
    /// Printed order number.
    pub order_number: String,
    /// Order total, formatted.
    pub order_total: String,
    /// Reason text.
    pub reason: String,
    /// Current status label.
    pub status_label: &'static str,
    /// Reachable next statuses (code, label).
    pub next: Vec<(String, String)>,
    /// Created label, JST.
    pub created_label: String,
    /// Whether this request is at the inspected step —
    /// the page then points at the order's refund action.
    pub show_refund_hint: bool,
}

impl From<&ReturnRequestRow> for ReturnRow {
    fn from(row: &ReturnRequestRow) -> Self {
        Self {
            id: row.id,
            order_id: row.order_id,
            order_number: crate::models::order::order_number_for(
                row.order_id,
                row.order_created_at,
            ),
            order_total: crate::pricing::display_price(row.order_total_yen),
            reason: row.reason.clone(),
            status_label: return_request::status_label(&row.status),
            next: return_request::legal_next_statuses(&row.status)
                .iter()
                .map(|code| {
                    (
                        code.to_string(),
                        return_request::status_label(code).to_string(),
                    )
                })
                .collect(),
            created_label: crate::models::jst::jst_datetime_label(row.created_at),
            show_refund_hint: row.status == "inspected",
        }
    }
}

/// The returns list page.
#[derive(Template)]
#[template(path = "admin/pages/returns.html")]
pub struct ReturnListView {
    /// Sidebar layout state.
    pub layout: AdminLayout,
    /// Active navigation key.
    pub nav: &'static str,
    /// The requests, newest first.
    pub rows: Vec<ReturnRow>,
}
