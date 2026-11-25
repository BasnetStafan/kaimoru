//! View struct for the printable receipt (領収書).
//!
//! A plain receipt. It prints the shop's qualified-invoice
//! registration number when one is registered in the shop
//! settings — and, exactly as before, no line at all
//! when none is: a number is never invented. What it does
//! carry regardless is the
//! full per-rate tax breakdown the order stored, so the
//! tax inside the tax-inclusive total is visible per class
//! （標準税率/軽減税率）, as the owner's tax-display rule asks.

use askama::Template;

use crate::models::Order;
use crate::models::order::OrderLineDetail;
use crate::models::tax::OrderTaxBreakdown;

/// Renders pages/receipt.html: a standalone print-first
/// page (no storefront chrome) reached from the member
/// order detail or the guest lookup result, always behind
/// the same ownership proof as the order itself.
#[derive(Template)]
#[template(path = "storefront/pages/receipt.html")]
pub struct ReceiptView {
    /// Asset fingerprint for the stylesheet/script URLs.
    pub asset_fingerprint: &'static str,
    /// Shop name from the shop settings.
    pub shop_name: String,
    /// Qualified-invoice registration number from the
    /// settings; empty renders no line at all.
    pub registration_number: String,
    /// Public contact address from the settings, when set.
    pub contact_email: Option<String>,
    /// Issue date label (today, JST).
    pub issued_label: String,
    /// The order the receipt covers.
    pub order: Order,
    /// The order's lines (snapshotted names and prices).
    pub lines: Vec<OrderLineDetail>,
    /// The order's stored per-class tax breakdown.
    pub tax_breakdown: Vec<OrderTaxBreakdown>,
}

impl ReceiptView {
    /// The recipient line: the delivery name plus 様.
    pub fn recipient_label(&self) -> String {
        format!("{} {} 様", self.order.family_name, self.order.given_name)
    }

    /// The summed 内消費税 of the breakdown, formatted.
    pub fn formatted_tax_total(&self) -> String {
        crate::pricing::display_tax_total(&self.tax_breakdown)
    }

    /// Whether any line falls under the reduced rate: the
    /// receipt then states the 軽減税率対象 note explicitly.
    pub fn has_reduced_rate(&self) -> bool {
        self.tax_breakdown
            .iter()
            .any(|row| row.tax_class_code == "reduced")
    }
}
