//! View structs for the cart pages and fragments.

use askama::Template;

use super::agent_settlement::AgentSettlementView;
use super::header::Header;
use crate::models::CartLine;
use crate::services::cart_service::CartTotals;

/// Renders pages/cart.html, the cart page.
#[derive(Template)]
#[template(path = "storefront/pages/cart.html")]
pub struct CartView {
    /// Shared header state.
    pub header: Header,
    /// The cart contents and totals (the swappable main fragment).
    pub main: CartMainView,
}

/// The cart-main fragment's data: rows plus totals. The partial
/// reads everything through `main`, so the full page and the
/// htmx fragment share one template.
pub struct CartMainView {
    /// The cart lines, oldest first.
    pub lines: Vec<CartLine>,
    /// Computed totals for the lines.
    pub totals: CartTotals,
    /// Whether the cart belongs to a guest: the cart page then
    /// explains that logging in keeps the cart, because a guest
    /// cart dies with the session.
    pub is_guest: bool,
    /// The agent settlement state mirrored
    /// under the lines, when the cart holds agent lines.
    pub settlement: Option<AgentSettlementView>,
    /// One-shot page notice (the reorder's summary), taken
    /// from the session by the full page render only;
    /// fragment renders always carry `None`.
    pub notice: Option<String>,
    /// The shop's free-shipping threshold, for the progress
    /// bar: a settings value, carried in because the totals
    /// computation itself stays settings-free.
    pub free_shipping_threshold_yen: i64,
    /// The earliest delivery date's label under the shop's
    /// offsets, precomputed by the controller.
    pub earliest_delivery_label: String,
}

impl CartMainView {
    /// Product subtotal formatted for display.
    pub fn formatted_subtotal(&self) -> String {
        crate::pricing::display_price(self.totals.subtotal_yen)
    }

    /// Shipping fee formatted for display.
    pub fn formatted_shipping(&self) -> String {
        crate::pricing::display_price(self.totals.shipping_yen)
    }

    /// COD fee formatted for display.
    pub fn formatted_cod_fee(&self) -> String {
        crate::pricing::display_price(self.totals.cod_fee_yen)
    }

    /// Grand total formatted for display.
    pub fn formatted_total(&self) -> String {
        crate::pricing::display_price(self.totals.total_yen)
    }

    /// The total tax contained in the grand total, formatted.
    pub fn formatted_tax_total(&self) -> String {
        crate::pricing::display_price(self.totals.tax_total_yen)
    }

    /// Whether at least one line can still be bought. When none
    /// can — every line off sale or sold out — the checkout call
    /// to action renders disabled instead of bouncing the visitor
    /// back to this very page, and the quantity steppers are
    /// already disabled per line by the same `purchasable` rule.
    pub fn has_purchasable(&self) -> bool {
        self.lines.iter().any(|line| line.purchasable())
    }

    /// Progress toward free shipping, 0-100, for the cart page
    /// progress bar.
    pub fn shipping_percent(&self) -> i64 {
        if self.free_shipping_threshold_yen <= 0
            || self.totals.subtotal_yen >= self.free_shipping_threshold_yen
        {
            100
        } else {
            self.totals.subtotal_yen * 100 / self.free_shipping_threshold_yen
        }
    }

    /// Amount missing for free shipping, formatted.
    pub fn formatted_remaining(&self) -> String {
        crate::pricing::display_price(self.totals.free_shipping_remaining_yen)
    }

    /// The delivery-window line for the summary: the earliest
    /// selectable delivery date, from the same window the
    /// checkout picker offers. Shown only while something in
    /// the cart can still be bought.
    pub fn delivery_window_line(&self) -> Option<String> {
        if !self.has_purchasable() {
            return None;
        }
        Some(format!(
            "お届け希望日は、ご注文手続きで最短{}から指定できます",
            self.earliest_delivery_label
        ))
    }
}

/// Renders parts/cart_main.html on its own, for htmx swaps.
#[derive(Template)]
#[template(path = "storefront/parts/cart_main.html")]
pub struct CartMainFragment {
    /// The fragment data.
    pub main: CartMainView,
}

/// Renders parts/cart_toast.html: the add-to-cart confirmation
/// toast. The add endpoint returns it out-of-band; the layout
/// writes the same markup inline when a stored flash renders
/// on a full page.
#[derive(Template)]
#[template(path = "storefront/parts/cart_toast.html")]
pub struct CartToastView {
    /// Confirmation text, product name included.
    pub message: String,
}

/// Renders parts/cart_badge.html: the header cart count badge.
/// Cart endpoints return it with an out-of-band swap marker so
/// the header count updates without a page reload.
#[derive(Template)]
#[template(path = "storefront/parts/cart_badge.html")]
pub struct CartBadgeView {
    /// Total quantity in the visitor's cart.
    pub count: i64,
    /// Whether this copy is the out-of-band update (hx-swap-oob).
    pub oob: bool,
}
