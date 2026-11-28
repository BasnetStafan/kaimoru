//! View structs for the payment pages and the shared payment
//! block: the block is rendered identically on the konbini
//! instructions page and the member's order detail.

use askama::Template;

use super::header::Header;
use crate::models::payment::payment_status_label_for;
use crate::models::{Order, Payment};

/// The data of parts/payment_block.html: how an order is being
/// paid for and where that payment stands. A plain struct like
/// the product card — the part is included by pages holding a
/// `payment` field of this type.
pub struct PaymentBlockView {
    /// The method's customer-facing name.
    pub method_label: String,
    /// The unified payment status wording.
    pub status_label: String,
    /// The amount formatted.
    pub formatted_amount: String,
    /// The konbini deadline, long JST form, when one is set.
    pub deadline: Option<String>,
    /// The convenience store chain, when KOMOJU reported one.
    pub store: Option<String>,
    /// The hosted instructions page, when issued.
    pub instructions_url: Option<String>,
}

impl PaymentBlockView {
    /// Builds the block from an order's payment row.
    pub fn from_payment(payment: &Payment, order_status: &str) -> Self {
        Self {
            method_label: payment.method_label().to_string(),
            status_label: payment_status_label_for(order_status, Some(payment)),
            formatted_amount: payment.formatted_amount(),
            deadline: payment.deadline_long(),
            store: payment.store_label().map(str::to_string),
            instructions_url: payment.instructions_url.clone(),
        }
    }
}

/// Renders pages/checkout/payment_instructions.html: the
/// konbini payment instructions for a just-placed order.
#[derive(Template)]
#[template(path = "storefront/pages/checkout/payment_instructions.html")]
pub struct PaymentInstructionsView {
    /// Shared header state.
    pub header: Header,
    /// The placed order.
    pub order: Order,
    /// The payment block (method, deadline, instructions link).
    pub payment: PaymentBlockView,
    /// Whether the buyer is a member (shows the history link).
    pub is_member: bool,
}

impl PaymentInstructionsView {
    /// Grand total formatted for display.
    pub fn formatted_total(&self) -> String {
        crate::pricing::display_price(self.order.total_yen)
    }
}

/// Renders pages/checkout/payment_pending.html: the honest
/// "we cannot confirm your payment yet" page — shown when the
/// return cannot be correlated, and while a card payment is
/// still settling. It refreshes itself periodically.
#[derive(Template)]
#[template(path = "storefront/pages/checkout/payment_pending.html")]
pub struct PaymentPendingView {
    /// Shared header state.
    pub header: Header,
    /// The order number, when the return could be correlated
    /// to this session's order.
    pub order_number: Option<String>,
    /// Whether the buyer is a member (shows the history link).
    pub is_member: bool,
}

/// Renders pages/checkout/payment_error.html: placement
/// succeeded but the payment could not be started. The order
/// stands; this page says exactly that and nothing more.
#[derive(Template)]
#[template(path = "storefront/pages/checkout/payment_error.html")]
pub struct PaymentErrorView {
    /// Shared header state.
    pub header: Header,
    /// The placed order's number.
    pub order_number: String,
    /// Whether the buyer is a member (shows the history link).
    pub is_member: bool,
}
