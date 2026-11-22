//! Sales analytics view.

use askama::Template;

use crate::controllers::admin::AdminLayout;

/// One product ranking row.
pub struct ProductRankRow {
    /// 1-based rank.
    pub rank: usize,
    /// Product name.
    pub name: String,
    /// Admin product page URL.
    pub url: String,
    /// Units sold, grouped.
    pub quantity: String,
    /// Line amount, formatted.
    pub amount: String,
}

/// One category ranking row.
pub struct CategoryRankRow {
    /// 1-based rank.
    pub rank: usize,
    /// Category name.
    pub name: String,
    /// Units sold, grouped.
    pub quantity: String,
    /// Line amount, formatted.
    pub amount: String,
}

/// Formats a period-over-period change: `+12.3%` / `-5.0%`,
/// or `—` when the previous period is zero (no honest rate
/// exists against nothing).
pub fn delta_label(current: i64, previous: i64) -> String {
    if previous <= 0 {
        return "—".to_string();
    }
    let pct = (current - previous) as f64 * 100.0 / previous as f64;
    format!("{pct:+.1}%")
}

/// The sales analytics page.
#[derive(Template)]
#[template(path = "admin/pages/analytics.html")]
pub struct AnalyticsView {
    /// Sidebar layout state.
    pub layout: AdminLayout,
    /// Active navigation key.
    pub nav: &'static str,
    /// The period tab keys' active flags: (this, last, custom).
    pub tab_this_month: bool,
    /// Last-month tab active.
    pub tab_last_month: bool,
    /// Custom tab active.
    pub tab_custom: bool,
    /// The current span's label, e.g. `2026年11月1日〜11月9日`.
    pub span_label: String,
    /// The previous span's label.
    pub prev_span_label: String,
    /// The custom form's from value.
    pub from_value: String,
    /// The custom form's to value.
    pub to_value: String,
    /// Orders in the period, grouped.
    pub orders: String,
    /// Sales in the period, formatted.
    pub sales: String,
    /// Orders delta label.
    pub orders_delta: String,
    /// Sales delta label.
    pub sales_delta: String,
    /// Previous-period orders, grouped.
    pub prev_orders: String,
    /// Previous-period sales, formatted.
    pub prev_sales: String,
    /// Product ranking rows.
    pub products: Vec<ProductRankRow>,
    /// Category ranking rows.
    pub categories: Vec<CategoryRankRow>,
    /// Repeat rate label, e.g. `23.5%（会員 34人中 8人）`.
    pub repeat_label: String,
}
