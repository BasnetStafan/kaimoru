//! Dashboard view.
//!
//! Five stat cards, the latest orders, the low-stock list, and a
//! 7-day sales chart drawn with CSS bars only — heights are
//! precomputed percentages, because a hand-rolled chart keeps the
//! page free of chart libraries and their supply chain.

use askama::Template;

use crate::controllers::admin::AdminLayout;
use crate::models::AdminProduct;

/// One row of the latest-orders table, display-ready.
pub struct LatestOrderRow {
    /// Formatted order number.
    pub number: String,
    /// Order ID for the detail link.
    pub id: i64,
    /// Buyer display name, or ゲスト.
    pub buyer: String,
    /// Formatted total.
    pub total: String,
    /// Admin status label.
    pub status_label: &'static str,
    /// Placement time, `HH:mm` JST.
    pub time: String,
}

/// One bar of the 7-day sales chart.
pub struct SalesBar {
    /// Day label, `M/d` plus the weekday in parentheses.
    pub label: String,
    /// Bar height as a percentage of the week's best day.
    pub height_pct: i64,
    /// The day's sales, formatted.
    pub amount: String,
    /// Whether this bar is today (highlighted).
    pub is_today: bool,
}

/// The dashboard page.
#[derive(Template)]
#[template(path = "admin/pages/dashboard.html")]
pub struct DashboardView {
    /// Sidebar layout state.
    pub layout: AdminLayout,
    /// Active navigation key.
    pub nav: &'static str,
    /// Today's date line, e.g. `2026年11月2日（月）`.
    pub today_label: String,
    /// Orders placed today, cancelled included.
    pub orders_today: i64,
    /// Today's sales total, formatted; cancelled excluded.
    pub sales_today: String,
    /// Orders in received + preparing.
    pub unshipped: i64,
    /// Open inquiries (Contact spec §3.5 tile).
    pub open_inquiries: i64,
    /// How many public products are at or below the threshold.
    pub low_stock_count: usize,
    /// The configured default low-stock threshold, for the
    /// section note: a product's own threshold wins when set,
    /// so no single number is the whole rule.
    pub low_stock_default: i64,
    /// The latest 10 orders.
    pub latest: Vec<LatestOrderRow>,
    /// Public products at or below the low-stock threshold.
    pub low_stock: Vec<AdminProduct>,
    /// The 7 chart bars, oldest first.
    pub bars: Vec<SalesBar>,
    /// Ops status: newest backup label, or the unknown text.
    pub ops_backup_label: String,
    /// Ops status: mail backlog line.
    pub ops_mail_label: String,
    /// Ops status: payment sweep line.
    pub ops_sweep_label: String,
    /// Ops status: agent (LLM) health line.
    pub ops_llm_label: String,
}
