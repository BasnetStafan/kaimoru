//! MCP order draft approval screens: the page a person
//! opens from a draft's approval URL to check the frozen
//! order and pay for it.

use askama::Template;

use super::checkout::PaymentMethodOption;
use super::header::Header;
use crate::models::CartLine;
use crate::services::cart_service::CartTotals;
use crate::services::mcp_order_draft_service::DraftAddress;

/// Renders pages/orders/draft.html: the draft approval
/// page. In the payable state it lists the live cart
/// lines, the delivery address the draft orders to, the
/// totals and the payment methods, with the approve-and-
/// pay form. In a terminal state (`payable` false) it
/// shows only the explanation: a void, expired, consumed
/// or unknown draft has nothing to pay.
#[derive(Template)]
#[template(path = "storefront/pages/orders/draft.html")]
pub struct OrderDraftView {
    /// Shared header state.
    pub header: Header,
    /// Whether the pay form is shown: the draft is
    /// pending and still binds the live cart.
    pub payable: bool,
    /// The pay form's target (this page's `/pay` sibling).
    pub pay_url: String,
    /// The explanation for a terminal state, or a
    /// page-level refusal on the payable state (a payment
    /// method ceiling, a placement refusal). Empty when
    /// there is nothing to explain.
    pub notice: String,
    /// The live purchasable lines the draft binds.
    pub lines: Vec<CartLine>,
    /// Totals without a payment method (no COD fee yet).
    pub totals: CartTotals,
    /// The delivery address the draft orders to.
    pub address: Option<DraftAddress>,
    /// The store's payment methods, checkout's own list.
    pub payment_options: Vec<PaymentMethodOption>,
    /// The cash-on-delivery fee, for the note under the
    /// totals: choosing COD adds exactly this.
    pub cod_fee_display: String,
    /// Subtotal formatted for display.
    pub subtotal_display: String,
    /// Shipping formatted for display.
    pub shipping_display: String,
    /// Grand total (before any COD fee) formatted.
    pub total_display: String,
}
