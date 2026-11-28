//! Payment reconciliation view.

use askama::Template;

use crate::controllers::admin::AdminLayout;

/// One open-payment row.
pub struct OpenPaymentRow {
    /// Order number label, linked.
    pub order_number: String,
    /// Order detail URL.
    pub order_url: String,
    /// Method label.
    pub method_label: String,
    /// Status label.
    pub status_label: String,
    /// Amount, formatted.
    pub amount: String,
    /// Creation time in JST.
    pub created_label: String,
    /// Deadline in JST, or a dash.
    pub deadline_label: String,
}

/// One captured-total row.
pub struct CapturedRow {
    /// Method label.
    pub method_label: String,
    /// Payment count.
    pub count: i64,
    /// Net captured, formatted.
    pub net: String,
}

/// One flagged-payment row.
pub struct FlaggedRow {
    /// Order number label, linked.
    pub order_number: String,
    /// Order detail URL.
    pub order_url: String,
    /// Status label.
    pub status_label: String,
    /// The flag note.
    pub flag_note: String,
    /// Last update in JST.
    pub updated_label: String,
}

/// The payment reconciliation page.
#[derive(Template)]
#[template(path = "admin/pages/payments.html")]
pub struct PaymentsView {
    /// Sidebar layout state.
    pub layout: AdminLayout,
    /// Active navigation key.
    pub nav: &'static str,
    /// Open payments, oldest first.
    pub open: Vec<OpenPaymentRow>,
    /// Captured totals per method.
    pub captured: Vec<CapturedRow>,
    /// Flagged payments.
    pub flagged: Vec<FlaggedRow>,
}
