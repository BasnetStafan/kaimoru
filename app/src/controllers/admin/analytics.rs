//! Sales analytics.
//!
//! The 売上分析 page: period order/sales totals with the
//! previous period's comparison, product and category
//! rankings, and the member repeat rate. Gated to the sales
//! area by the permission guard (see
//! `crate::middleware::admin_permission_guard`).

use axum::{
    Router,
    extract::{Query, State},
    response::Response,
    routing::get,
};
use serde::Deserialize;
use sqlx::PgPool;
use tower_sessions::Session;

use super::{db_error, layout_for, render};
use crate::pricing;
use crate::services::admin::analytics_service::{self, PeriodKind};
use crate::session::CurrentAdmin;
use crate::views::admin::analytics::{AnalyticsView, CategoryRankRow, ProductRankRow, delta_label};

/// Route: the analytics page.
pub fn router() -> Router<PgPool> {
    Router::new().route("/admin/analytics", get(page))
}

/// The period query: `period` picks the tab, `from`/`to`
/// carry the custom span.
#[derive(Deserialize)]
pub struct AnalyticsQuery {
    /// Tab key: this_month (default) / last_month / custom.
    period: Option<String>,
    /// Custom span start (`YYYY-MM-DD`).
    from: Option<String>,
    /// Custom span end (`YYYY-MM-DD`, inclusive).
    to: Option<String>,
}

/// A date span's label, e.g. `2026年11月1日〜11月9日`: the
/// span is [from, to), so the last shown day is `to - 1`.
fn span_label(from: chrono::NaiveDate, to: chrono::NaiveDate) -> String {
    let last = to - chrono::Duration::days(1);
    format!(
        "{}年{}月{}日〜{}月{}日",
        from.year(),
        from.month(),
        from.day(),
        last.month(),
        last.day()
    )
}

use chrono::Datelike;

/// Renders the analytics page.
pub async fn page(
    State(pool): State<PgPool>,
    CurrentAdmin(admin): CurrentAdmin,
    session: Session,
    Query(query): Query<AnalyticsQuery>,
) -> Response {
    let kind = match query.period.as_deref() {
        Some("last_month") => PeriodKind::LastMonth,
        Some("custom") => PeriodKind::Custom,
        _ => PeriodKind::ThisMonth,
    };
    let from_raw = query.from.as_deref().unwrap_or("");
    let to_raw = query.to.as_deref().unwrap_or("");
    let period =
        analytics_service::resolve(kind, from_raw, to_raw, crate::models::jst::jst_today());
    let data = match analytics_service::load(&pool, &period).await {
        Ok(data) => data,
        Err(e) => return db_error("analytics", e),
    };
    let repeat_label = if data.member_buyers > 0 {
        let pct = data.member_repeaters as f64 * 100.0 / data.member_buyers as f64;
        format!(
            "{pct:.1}%（会員 {}人中 {}人が2回以上）",
            data.member_buyers, data.member_repeaters
        )
    } else {
        "—（期間中の会員注文なし）".to_string()
    };
    render(AnalyticsView {
        layout: layout_for(&pool, &admin, &session).await,
        nav: "analytics",
        tab_this_month: period.kind == PeriodKind::ThisMonth,
        tab_last_month: period.kind == PeriodKind::LastMonth,
        tab_custom: period.kind == PeriodKind::Custom,
        span_label: span_label(period.from, period.to),
        prev_span_label: span_label(period.prev_from, period.prev_to),
        from_value: if period.kind == PeriodKind::Custom {
            period.from.format("%Y-%m-%d").to_string()
        } else {
            from_raw.to_string()
        },
        to_value: if period.kind == PeriodKind::Custom {
            (period.to - chrono::Duration::days(1))
                .format("%Y-%m-%d")
                .to_string()
        } else {
            to_raw.to_string()
        },
        orders: group(data.orders),
        sales: pricing::display_price(data.sales_yen),
        orders_delta: delta_label(data.orders, data.prev_orders),
        sales_delta: delta_label(data.sales_yen, data.prev_sales_yen),
        prev_orders: group(data.prev_orders),
        prev_sales: pricing::display_price(data.prev_sales_yen),
        products: data
            .products
            .iter()
            .enumerate()
            .map(|(i, r)| ProductRankRow {
                rank: i + 1,
                name: r.product_name.clone(),
                url: format!("/admin/products/{}/edit", r.product_id),
                quantity: group(r.quantity),
                amount: pricing::display_price(r.amount_yen),
            })
            .collect(),
        categories: data
            .categories
            .iter()
            .enumerate()
            .map(|(i, r)| CategoryRankRow {
                rank: i + 1,
                name: r.category.clone(),
                quantity: group(r.quantity),
                amount: pricing::display_price(r.amount_yen),
            })
            .collect(),
        repeat_label,
    })
}

/// Groups an integer with thousands separators, the plain
/// count format the dashboard cards use.
fn group(value: i64) -> String {
    let digits = value.abs().to_string();
    let mut out = String::new();
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    if value < 0 { format!("-{out}") } else { out }
}
