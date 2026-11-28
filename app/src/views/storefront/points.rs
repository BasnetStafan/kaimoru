//! Points page view (EC V2 core 1).
//!
//! The member's points home: the balance, the soonest
//! expiry, and the ledger history rendered as plain rows —
//! the service's movements, pre-labelled for display.

use askama::Template;

use crate::views::storefront::header::Header;

/// One history row: a ledger movement in display shape.
pub struct PointHistoryRow {
    /// When the movement happened (JST, long form).
    pub date_label: String,
    /// What the movement was （付与・利用・返還・取消・失効・調整）.
    pub kind_label: String,
    /// The signed amount, formatted (+1,000 / −300).
    pub points_display: String,
    /// The order the movement belongs to, when it has one.
    pub order_id: Option<i64>,
}

/// Renders pages/mypage/points.html: the member's points
/// balance and history.
#[derive(Template)]
#[template(path = "storefront/pages/mypage/points.html")]
pub struct PointsPageView {
    /// Shared header state.
    pub header: Header,
    /// Spendable balance right now.
    pub balance: i64,
    /// The soonest expiry, as "1,234 ポイント（2027年1月1日まで）"
    /// when any grant is live.
    pub next_expiry_label: Option<String>,
    /// Ledger history, newest first.
    pub rows: Vec<PointHistoryRow>,
}
