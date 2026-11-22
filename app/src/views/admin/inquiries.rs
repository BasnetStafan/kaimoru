//! View types for the admin inquiry inbox (Contact spec
//! §3.5): a status-filtered list and a detail page with
//! the status/memo form, the reply form, the reply
//! confirm step and the reply history.

use askama::Template;

use crate::controllers::admin::AdminLayout;
use crate::models::{Inquiry, InquiryReply};
use crate::views::admin::Page;

/// One status filter chip with its count.
pub struct InquiryChip {
    /// Chip label.
    pub label: String,
    /// Filter URL.
    pub url: String,
    /// Row count under this filter.
    pub count: i64,
    /// Whether this chip is the current filter.
    pub active: bool,
}

/// `GET /admin/inquiries`: the inbox list.
#[derive(Template)]
#[template(path = "admin/pages/inquiries/list.html")]
pub struct InquiryListView {
    /// Sidebar layout state.
    pub layout: AdminLayout,
    /// Active navigation key.
    pub nav: &'static str,
    /// Status chips with counts.
    pub chips: Vec<InquiryChip>,
    /// This page's rows.
    pub rows: Vec<Inquiry>,
    /// Pagination state for the shared part.
    pub page: Page,
}

/// Values preserved on the detail page's forms.
#[derive(Default)]
pub struct ReplyValues {
    /// Reply subject.
    pub subject: String,
    /// Reply body.
    pub body: String,
}

/// `GET /admin/inquiries/{id}`: the detail page.
#[derive(Template)]
#[template(path = "admin/pages/inquiries/detail.html")]
pub struct InquiryDetailView {
    /// Sidebar layout state.
    pub layout: AdminLayout,
    /// Active navigation key.
    pub nav: &'static str,
    /// The inquiry.
    pub inquiry: Inquiry,
    /// The submitting member's admin link, when the
    /// inquiry is tied to an account: (user id, email).
    pub member: Option<(i64, String)>,
    /// The matched order's admin link, when the typed
    /// order number matches a real order: (order id,
    /// order number).
    pub matched_order: Option<(i64, String)>,
    /// Replies already sent, oldest first.
    pub replies: Vec<InquiryReply>,
    /// Reply form values (prefilled or preserved).
    pub reply: ReplyValues,
    /// Reply validation errors, per field.
    pub reply_errors: ReplyErrors,
    /// Memo validation error, when the save failed.
    pub memo_error: Option<String>,
    /// One-time form ticket guarding the status/memo form.
    pub form_ticket: String,
}

/// Reply form errors, one slot per field.
#[derive(Default)]
pub struct ReplyErrors {
    /// Subject error.
    pub subject: Option<String>,
    /// Body error.
    pub body: Option<String>,
}

/// The reply confirm step (mail cannot be unsent, so
/// sending takes one deliberate extra click, spec §5.4).
#[derive(Template)]
#[template(path = "admin/pages/inquiries/reply_confirm.html")]
pub struct InquiryReplyConfirmView {
    /// Sidebar layout state.
    pub layout: AdminLayout,
    /// Active navigation key.
    pub nav: &'static str,
    /// The inquiry id.
    pub inquiry_id: i64,
    /// The inquiry reference.
    pub reference: String,
    /// The recipient address.
    pub to_email: String,
    /// The subject about to be sent.
    pub subject: String,
    /// The body about to be sent.
    pub body: String,
}
