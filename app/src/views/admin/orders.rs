//! Order list and detail views.

use askama::Template;

use crate::controllers::admin::AdminLayout;
use crate::models::Order;
use crate::models::order::OrderLineDetail;
use crate::views::admin::Page;

/// One status filter chip of the order list: a GET link carrying
/// the search term, with the live count of orders in its state.
pub struct OrderChip {
    /// Chip URL: this status filter, search preserved.
    pub url: String,
    /// Japanese label.
    pub label: &'static str,
    /// Live count over all orders (search and chip ignored — the
    /// counts describe the whole queue).
    pub count: i64,
    /// Whether this chip is the active filter.
    pub active: bool,
}

/// One row of the order list, display-ready.
pub struct OrderRow {
    /// Order ID for the detail link.
    pub id: i64,
    /// Formatted order number.
    pub number: String,
    /// Buyer display name, or ゲスト.
    pub buyer: String,
    /// Buying member's ID for the customer-detail link;
    /// `None` for a guest order (ゲスト renders unlinked).
    pub member_id: Option<i64>,
    /// Total quantity across the order's lines （商品点数）.
    pub item_count: i64,
    /// Formatted total.
    pub total: String,
    /// The unified payment label （支払い状況）: the COD
    /// derivation for cash orders, the payment status wording
    /// for KOMOJU ones — the shared derivation the detail page
    /// also renders.
    pub payment_label: String,
    /// The payment badge's tone class, from
    /// `payment_badge_class_for_parts`.
    pub payment_badge_class: &'static str,
    /// Whether the order is cancelled (the row renders muted).
    pub cancelled: bool,
    /// Admin status label.
    pub status_label: &'static str,
    /// Raw status value (the badge class hook).
    pub status: String,
    /// Placement date/time, `YYYY/MM/DD HH:mm` JST.
    pub placed: String,
    /// Whether the order came through the shopping agent's
    /// settlement (the row renders an エージェント badge).
    pub agent_placed: bool,
}

/// The admin order list page.
#[derive(Template)]
#[template(path = "admin/pages/orders/list.html")]
pub struct OrderListView {
    /// Sidebar layout state.
    pub layout: AdminLayout,
    /// Active navigation key.
    pub nav: &'static str,
    /// The current list URL (filters included): bulk actions
    /// post it as their return address, so the operator lands
    /// back on the exact list they were working in.
    pub return_url: String,
    /// Status filter chips, in order.
    pub chips: Vec<OrderChip>,
    /// The active status filter value, for the search form's
    /// hidden field (searching keeps the active chip).
    pub status: String,
    /// The order-number search text, as typed.
    pub q: String,
    /// Placed-from date text (`YYYY-MM-DD` or empty).
    pub from: String,
    /// Placed-to date text (`YYYY-MM-DD` or empty).
    pub to: String,
    /// Customer name fragment, echoed back into the form.
    pub customer: String,
    /// The active payment filter value (empty = all states).
    pub payment: String,
    /// The rows of this page.
    pub orders: Vec<OrderRow>,
    /// Pagination state.
    pub page: Page,
    /// Carriers the label-CSV export offers: the catalogue
    /// minus the ones the shop disabled in its settings.
    pub label_carriers: Vec<BulkCarrierOption>,
}

/// One row of the bulk status change confirmation: the order
/// as the preview found it, and whether the target is a legal
/// next state from there (the execute step re-checks against
/// the live state regardless — this flag is presentation).
pub struct BulkConfirmRow {
    /// Order ID.
    pub id: i64,
    /// Formatted order number.
    pub number: String,
    /// Buyer display name, or ゲスト.
    pub buyer: String,
    /// The order's current status label.
    pub current_label: &'static str,
    /// The status the form was rendered against (reposted as
    /// the execute step's staleness guard).
    pub expected: String,
    /// Whether the target is reachable from the current state.
    pub applicable: bool,
}

/// One carrier choice of the bulk confirmation's ship form.
pub struct BulkCarrierOption {
    /// Stored carrier code.
    pub code: &'static str,
    /// Japanese label.
    pub label: &'static str,
}

/// The bulk status change confirmation page: what will change,
/// for how many orders, before anything is written.
#[derive(Template)]
#[template(path = "admin/pages/orders/bulk_confirm.html")]
pub struct BulkConfirmView {
    /// Sidebar layout state.
    pub layout: AdminLayout,
    /// Active navigation key.
    pub nav: &'static str,
    /// The orders the operator selected, as found.
    pub rows: Vec<BulkConfirmRow>,
    /// How many of the rows the target can actually reach.
    pub applicable_count: usize,
    /// The target status value (reposted by the execute form).
    pub target: String,
    /// The target's admin label.
    pub target_label: &'static str,
    /// Whether the target is the ship transition (the carrier
    /// choice shows only then).
    pub is_shipped: bool,
    /// The carrier catalogue for the ship form.
    pub carriers: Vec<BulkCarrierOption>,
    /// The list URL both buttons return to.
    pub return_url: String,
    /// Selected orders past the bulk cap that this action
    /// leaves out, announced rather than silently dropped.
    pub truncated: usize,
}

/// One row of the tracking import results page.
pub struct TrackingImportRow {
    /// File line the row came from.
    pub line: usize,
    /// The posted order number.
    pub number: String,
    /// Whether the row was applied.
    pub applied: bool,
    /// What happened, in the service's words.
    pub message: String,
}

/// The tracking import results page: every row's outcome,
/// applied or refused with its reason.
#[derive(Template)]
#[template(path = "admin/pages/orders/tracking_import.html")]
pub struct TrackingImportView {
    /// Sidebar layout state.
    pub layout: AdminLayout,
    /// Active navigation key.
    pub nav: &'static str,
    /// Per-row outcomes, in file order.
    pub rows: Vec<TrackingImportRow>,
    /// How many rows were applied.
    pub applied_count: usize,
}

/// One order's packing slip （納品書） on the bulk print page.
pub struct PackingSlip {
    /// The order the slip covers.
    pub order: crate::models::Order,
    /// Its snapshot lines.
    pub lines: Vec<OrderLineDetail>,
    /// The stored per-class tax breakdown.
    pub tax_breakdown: Vec<crate::models::tax::OrderTaxBreakdown>,
}

impl PackingSlip {
    /// The recipient line: the delivery name plus 様.
    pub fn recipient_label(&self) -> String {
        format!("{} {} 様", self.order.family_name, self.order.given_name)
    }

    /// The delivery address as one line.
    pub fn address_label(&self) -> String {
        format!(
            "〒{} {}{}{}{}",
            self.order.postal_code,
            self.order.prefecture,
            self.order.city,
            self.order.address_line,
            self.order.building.clone().unwrap_or_default()
        )
    }

    /// The summed 内消費税 of the breakdown, formatted.
    pub fn formatted_tax_total(&self) -> String {
        crate::pricing::display_tax_total(&self.tax_breakdown)
    }
}

/// The bulk packing-slip print page: one slip per selected
/// order, standalone like the receipt page (no admin chrome),
/// each slip starting on its own printed sheet.
#[derive(Template)]
#[template(path = "admin/pages/orders/packing_slips.html")]
pub struct PackingSlipsView {
    /// Asset fingerprint for the stylesheet/script URLs.
    pub asset_fingerprint: &'static str,
    /// Shop name from the shop settings.
    pub shop_name: String,
    /// Qualified-invoice registration number from the
    /// settings; empty renders no line at all.
    pub registration_number: String,
    /// Issue date label (today, JST).
    pub issued_label: String,
    /// The slips, in selection order.
    pub slips: Vec<PackingSlip>,
}

/// One selectable next state in the detail's status form.
pub struct NextStatus {
    /// Status value.
    pub value: &'static str,
    /// Admin label.
    pub label: &'static str,
}

/// One 変更履歴 row of the order detail: when a change happened,
/// which admin made it, and the action's Japanese label — the
/// audit diary's rows for this order, rendered with the viewer's
/// own label map.
pub struct OrderHistoryRow {
    /// When it happened, `YYYY/MM/DD HH:mm` JST.
    pub at: String,
    /// The acting admin's display name.
    pub admin_name: String,
    /// Japanese action label.
    pub action_label: String,
}

/// The admin order detail page.
#[derive(Template)]
#[template(path = "admin/pages/orders/detail.html")]
pub struct OrderDetailView {
    /// Sidebar layout state.
    pub layout: AdminLayout,
    /// Active navigation key.
    pub nav: &'static str,
    /// The order.
    pub order: Order,
    /// Its snapshot lines (with current thumbnails).
    pub lines: Vec<OrderLineDetail>,
    /// Admin status label of the current state.
    pub status_label: &'static str,
    /// Buyer display name, or ゲスト.
    pub buyer: String,
    /// Member ID for the customer link; `None` for guests.
    pub member_id: Option<i64>,
    /// Formatted placement date/time in the long Japanese form.
    pub placed: String,
    /// Legal next states for the status form (cancellation
    /// excluded — it has its own confirm form).
    pub next_statuses: Vec<NextStatus>,
    /// Whether cancellation is currently legal.
    pub can_cancel: bool,
    /// Whether the status form shows at all (terminal states
    /// show a note instead).
    pub has_actions: bool,
    /// A refused transition's form-level error, when the last
    /// POST attempted an illegal move.
    pub form_error: Option<String>,
    /// Derived cash-on-delivery collection state: 代引き（未収）
    /// while the parcel is on its way, 代引き（回収済み） once
    /// delivered, 代引き（対象外） when cancelled. There is no
    /// "mark as paid" action — collection is the physical event at
    /// the door, recorded by the 配達完了 transition itself.
    pub payment_status_label: String,
    /// The 変更履歴: this order's audit rows, newest first.
    /// Empty until an admin first changes the order.
    pub history: Vec<OrderHistoryRow>,
    /// The payment block, when the order has a payment row.
    /// Orders placed before payments existed render the COD
    /// wording the template falls back to.
    pub payment: Option<AdminPaymentBlock>,
    /// The payment's webhook event history, oldest first.
    pub payment_history: Vec<PaymentHistoryRow>,
    /// The order's stored per-class tax breakdown （税率ごとの
    /// 内訳）; empty for orders placed before tax classes.
    pub tax_breakdown: Vec<crate::models::tax::OrderTaxBreakdown>,
    /// Carrier codes the shop disabled in its settings: the
    /// ship form offers the rest (plus this order's stored
    /// carrier, which stays valid history).
    pub disabled_carriers: String,
    /// The order's split shipments; empty for the
    /// classic single-shipment orders.
    pub shipments: Vec<AdminShipmentRow>,
    /// Lines still unassigned to any shipment, for the
    /// split-create form; empty when nothing remains.
    pub split_lines: Vec<SplitLineRow>,
}

/// One split shipment on the admin order detail.
pub struct AdminShipmentRow {
    /// Position within the order (1-based).
    pub index: usize,
    /// Shipment ID for the ship form's action.
    pub id: i64,
    /// Its lines as one summary text.
    pub items_label: String,
    /// Carrier label when shipped (or set).
    pub carrier_label: String,
    /// Tracking number, when recorded.
    pub tracking_number: String,
    /// Whether the parcel has shipped.
    pub shipped: bool,
    /// Notification state label.
    pub mail_label: String,
}

/// One unassigned remainder line for the split-create form.
pub struct SplitLineRow {
    /// Order item ID (the form field suffix).
    pub item_id: i64,
    /// Product + variant label.
    pub label: String,
    /// Quantity not yet assigned to any shipment.
    pub remaining: i32,
}

/// The admin payment block on the order detail: everything the
/// operator needs about the money — method, provider, status,
/// identifiers, deadline, capture time, a human-attention flag,
/// and which payment actions this order allows.
pub struct AdminPaymentBlock {
    /// Whether a hosted gateway settles this payment.
    pub is_hosted: bool,
    /// The method's name.
    pub method_label: String,
    /// The provider's name for display.
    pub provider_label: String,
    /// The unified payment status wording.
    pub status_label: String,
    /// The amount formatted.
    pub formatted_amount: String,
    /// The refunded total formatted, when any refund stands.
    pub refunded: Option<String>,
    /// The refunded total in yen, raw: the refund form
    /// posts it back as its staleness `expected` value.
    pub refunded_yen: i64,
    /// The provider's session ID, when a session was created.
    pub session_id: Option<String>,
    /// The provider's payment ID, when the provider created
    /// the payment.
    pub payment_id: Option<String>,
    /// Konbini deadline, long JST form, when set.
    pub deadline: Option<String>,
    /// Capture instant, JST, when captured.
    pub captured_at: Option<String>,
    /// The operator flag note, when a human must look.
    pub flag_note: Option<String>,
    /// Whether the refund form renders (settled card/PayPay).
    pub can_refund: bool,
    /// Whether the sync button renders (a KOMOJU reference exists).
    pub can_sync: bool,
    /// Whether the konbini dashboard-refund note renders.
    pub konbini_refund_note: bool,
}

/// One webhook event in the detail's payment history.
pub struct PaymentHistoryRow {
    /// When the event was received, JST.
    pub at: String,
    /// The provider's event type word.
    pub event_type: String,
    /// Whether the event changed a status （適用 / 記録のみ).
    pub applied_label: String,
}

impl OrderDetailView {
    /// The summed 内消費税 of the order's breakdown, formatted.
    pub fn formatted_tax_total(&self) -> String {
        crate::pricing::display_tax_total(&self.tax_breakdown)
    }

    /// The delivery block's postal line.
    pub fn postal_line(&self) -> String {
        format!("〒{}", self.order.postal_code)
    }

    /// The delivery block's address lines joined for display.
    pub fn address_line(&self) -> String {
        let mut text = format!(
            "{} {} {}",
            self.order.prefecture, self.order.city, self.order.address_line
        );
        if let Some(building) = &self.order.building {
            text.push(' ');
            text.push_str(building);
        }
        text
    }

    /// The recipient name line.
    pub fn recipient(&self) -> String {
        format!("{} {} 様", self.order.family_name, self.order.given_name)
    }

    /// The ship form's carrier choices, with the order's
    /// stored carrier marked selected.
    pub fn carrier_options(&self) -> Vec<CarrierOption> {
        crate::models::carrier::CARRIERS
            .iter()
            .filter(|c| {
                self.order.carrier.as_deref() == Some(c.code)
                    || !self
                        .disabled_carriers
                        .split(',')
                        .any(|d| d.trim() == c.code)
            })
            .map(|c| CarrierOption {
                code: c.code,
                label: c.label,
                selected: self.order.carrier.as_deref() == Some(c.code),
            })
            .collect()
    }

    /// The tracking-number input's current value: the stored
    /// number, or empty when none was entered.
    pub fn tracking_number_value(&self) -> &str {
        self.order.tracking_number.as_deref().unwrap_or_default()
    }
}

/// One carrier choice in the ship form's select.
pub struct CarrierOption {
    /// Stable carrier code (the form value).
    pub code: &'static str,
    /// Japanese label.
    pub label: &'static str,
    /// Whether this is the order's stored carrier.
    pub selected: bool,
}
