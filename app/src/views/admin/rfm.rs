//! RFM analytics view.

use askama::Template;

use crate::controllers::admin::AdminLayout;
use crate::views::admin::Page;

/// One scored member row, display-ready.
pub struct RfmRow {
    /// The member.
    pub user_id: i64,
    /// Full name, decrypted.
    pub name: String,
    /// Account email, decrypted.
    pub email: String,
    /// Days since the newest counted order.
    pub r_days: i32,
    /// Counted order count.
    pub f_count: i64,
    /// Net monetary total, formatted.
    pub m_display: String,
    /// The newest counted order's date, `YYYY/MM/DD`.
    pub last_order_label: String,
    /// Recency band, 1-5.
    pub r_band: i32,
    /// Frequency band, 1-5.
    pub f_band: i32,
    /// Monetary band, 1-5.
    pub m_band: i32,
    /// The named segment's label.
    pub segment_label: String,
}

/// One named-segment summary row.
pub struct SegmentSummaryRow {
    /// The segment's label.
    pub label: String,
    /// Members in it.
    pub count: i64,
}

/// The RFM analytics page.
#[derive(Template)]
#[template(path = "admin/pages/rfm.html")]
pub struct RfmView {
    /// Sidebar layout state.
    pub layout: AdminLayout,
    /// Active navigation key.
    pub nav: &'static str,
    /// Scored population size.
    pub population: i64,
    /// Named-segment counts, first-match order.
    pub segments: Vec<SegmentSummaryRow>,
    /// Top members by monetary total (the leaderboard).
    pub leaders: Vec<RfmRow>,
    /// The member table's page, in the chosen sort.
    pub rows: Vec<RfmRow>,
    /// Pagination state for the member table.
    pub page: Page,
    /// Sort tab flags: (monetary, frequency, recency).
    pub tab_m: bool,
    /// Frequency tab active.
    pub tab_f: bool,
    /// Recency tab active.
    pub tab_r: bool,
}
