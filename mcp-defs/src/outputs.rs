//! The MCP tools' own result types: the shapes only the
//! MCP surface returns — cart views under their handle,
//! the checkout handoff, order drafts and order status —
//! shared by both transports.

use schemars::JsonSchema;
use serde::Serialize;

use crate::catalog::{ApiCart, ApiCartLine};

/// One MCP cart as the cart tools return it: the API cart's
/// lines and totals, plus the handle they belong to. Totals
/// come from the store's one totals implementation — the
/// same numbers the cart page shows.
#[derive(Serialize, JsonSchema)]
pub struct McpCart {
    /// The cart handle.
    pub cart_id: String,
    /// The cart's lines, in cart order.
    pub lines: Vec<ApiCartLine>,
    /// Total units across all lines.
    pub item_count: i64,
    /// Sum of purchasable line totals, in yen.
    pub subtotal_yen: i64,
    /// Shipping fee in yen (free at the store's threshold).
    pub shipping_yen: i64,
    /// Grand total in yen: subtotal plus shipping.
    pub total_yen: i64,
}

impl McpCart {
    /// Wraps an API cart view under its handle.
    pub fn new(cart_id: String, cart: ApiCart) -> Self {
        Self {
            cart_id,
            lines: cart.lines,
            item_count: cart.item_count,
            subtotal_yen: cart.subtotal_yen,
            shipping_yen: cart.shipping_yen,
            total_yen: cart.total_yen,
        }
    }
}

/// The browser handoff the `get_checkout_link` tool returns:
/// a person opens the URL and the cart becomes theirs.
#[derive(Serialize, JsonSchema)]
pub struct McpCheckoutLink {
    /// The cart handle the link hands off.
    pub cart_id: String,
    /// The single-use claim URL to open in a browser.
    pub checkout_url: String,
    /// When the link stops working, as an RFC 3339 timestamp.
    pub expires_at: String,
}

/// One line of a draft, from its frozen snapshot: what
/// the approval page listed at creation (or, after the
/// pay click, in the final binding).
#[derive(Serialize, JsonSchema)]
pub struct McpDraftLine {
    /// The line's variant id.
    pub variant_id: i64,
    /// The line's product id.
    pub product_id: i64,
    /// Product name at draft time.
    pub name: String,
    /// Combination label; empty for a plain product.
    pub variant_label: String,
    /// Unit price in integer yen, tax included.
    pub unit_price_yen: i64,
    /// Units in the draft.
    pub quantity: i64,
    /// Line total in integer yen.
    pub line_total_yen: i64,
}

/// One order draft as the order tools return it: the
/// frozen lines and totals from its snapshot, its status,
/// and — at creation only — the approval URL a person
/// opens to approve and pay. The URL is never shown again
/// (only its digest is stored), so `approval_url` is
/// present exactly on the create answer.
#[derive(Serialize, JsonSchema)]
pub struct McpOrderDraft {
    /// The draft's id.
    pub draft_id: i64,
    /// `pending`, `consumed`, `expired` or `void`.
    pub status: String,
    /// The placed order's id, once the draft is consumed.
    pub order_id: Option<i64>,
    /// The browser approval URL (create answer only).
    pub approval_url: Option<String>,
    /// When the approval window closes, RFC 3339.
    pub expires_at: String,
    /// The draft's lines, from its snapshot.
    pub lines: Vec<McpDraftLine>,
    /// Sum of line totals in yen.
    pub subtotal_yen: i64,
    /// Shipping fee in yen.
    pub shipping_yen: i64,
    /// Grand total in yen.
    pub total_yen: i64,
}

/// One placed order's status as `get_order_status`
/// returns it: fulfilment status and shipment facts
/// only — never the buyer's details.
#[derive(Serialize, JsonSchema)]
pub struct McpOrderStatus {
    /// The order's id.
    pub order_id: i64,
    /// Fulfilment status: `received`, `preparing`,
    /// `shipped`, `delivered` or `cancelled`.
    pub status: String,
    /// Grand total in integer yen.
    pub total_yen: i64,
    /// When the order was placed, RFC 3339.
    pub created_at: String,
    /// When the order was delivered, RFC 3339, when it
    /// has been.
    pub delivered_at: Option<String>,
    /// Delivery carrier code, once shipped with one named.
    pub carrier: Option<String>,
    /// Carrier tracking number, once shipped with one.
    pub tracking_number: Option<String>,
}
