//! RFM analytics.
//!
//! The RFM分析 page: per-member recency / frequency /
//! monetary bands over counted orders, the named-segment
//! summary, and the customer leaderboard. Read-only; the
//! scores are computed on view by the service. Gated to
//! the store area by the permission guard, with the
//! customers and segments surfaces it feeds.

use axum::{
    Router,
    extract::{Query, State},
    response::Response,
    routing::get,
};
use serde::Deserialize;
use sqlx::PgPool;
use tower_sessions::Session;

use super::{db_error, layout_for, pagination, render};
use crate::config::CONFIG;
use crate::pricing;
use crate::services::admin::rfm_service::{self, ScoredMember};
use crate::session::CurrentAdmin;
use crate::views::admin::rfm::{RfmRow, RfmView, SegmentSummaryRow};
use crate::views::admin::{Page, filter_query};

/// Route: the RFM page.
pub fn router() -> Router<PgPool> {
    Router::new().route("/admin/rfm", get(page))
}

/// Page query: `sort` picks the member table's order
/// (m = monetary, the default; f = frequency; r =
/// recency), `page` its page number.
#[derive(Deserialize, Default)]
pub struct RfmQuery {
    /// Sort key text.
    sort: Option<String>,
    /// Page number.
    page: Option<String>,
}

/// How many leaders the leaderboard shows.
const LEADERBOARD_LEN: usize = 10;

/// Shapes one scored member into a display row.
fn to_row(member: &ScoredMember) -> RfmRow {
    RfmRow {
        user_id: member.user_id,
        name: format!("{} {}", member.family_name, member.given_name),
        email: member.email.clone(),
        r_days: member.r_days,
        f_count: member.f_count,
        m_display: pricing::display_price(member.m_yen),
        last_order_label: member.last_order_date.format("%Y/%m/%d").to_string(),
        r_band: member.r_band,
        f_band: member.f_band,
        m_band: member.m_band,
        segment_label: member.segment.label().to_string(),
    }
}

/// Renders the RFM page.
pub async fn page(
    State(pool): State<PgPool>,
    CurrentAdmin(admin): CurrentAdmin,
    session: Session,
    Query(query): Query<RfmQuery>,
) -> Response {
    let sort = match query.sort.as_deref() {
        Some("f") => "f",
        Some("r") => "r",
        _ => "m",
    };
    let members = match rfm_service::scored_members(&pool).await {
        Ok(v) => v,
        Err(e) => return db_error("rfm", e),
    };
    let population = members.len() as i64;
    let segments = rfm_service::segment_counts(&members)
        .into_iter()
        .map(|(segment, count)| SegmentSummaryRow {
            label: segment.label().to_string(),
            count,
        })
        .collect();
    let mut by_m = members.clone();
    by_m.sort_by(|a, b| b.m_yen.cmp(&a.m_yen).then(a.user_id.cmp(&b.user_id)));
    let leaders = by_m.iter().take(LEADERBOARD_LEN).map(to_row).collect();
    let mut table = members;
    match sort {
        "f" => table.sort_by(|a, b| b.f_count.cmp(&a.f_count).then(a.user_id.cmp(&b.user_id))),
        "r" => table.sort_by(|a, b| a.r_days.cmp(&b.r_days).then(a.user_id.cmp(&b.user_id))),
        _ => table.sort_by(|a, b| b.m_yen.cmp(&a.m_yen).then(a.user_id.cmp(&b.user_id))),
    }
    let (page_no, limit, offset) = pagination(query.page.as_deref(), CONFIG.admin_per_page);
    let rows = table
        .iter()
        .skip(offset.max(0) as usize)
        .take(limit.max(0) as usize)
        .map(to_row)
        .collect();
    let base = Page::base_with("/admin/rfm", &filter_query(&[("sort", Some(sort))]));
    render(RfmView {
        layout: layout_for(&pool, &admin, &session).await,
        nav: "rfm",
        population,
        segments,
        leaders,
        rows,
        page: Page::new(page_no, population, limit, base),
        tab_m: sort == "m",
        tab_f: sort == "f",
        tab_r: sort == "r",
    })
}
