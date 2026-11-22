//! The admin dashboard.
//!
//! Read-only assembly: five stat cards, the latest 10 orders,
//! the low-stock list (public products at or below their
//! low-stock threshold), and the 7-day sales chart. Day
//! boundaries are Asia/Tokyo, computed in SQL by the
//! repository.

use axum::{Router, extract::State, response::Response, routing::get};
use chrono::Datelike;
use sqlx::PgPool;
use tower_sessions::Session;

use super::{db_error, layout_for, render};
use crate::models::order::admin_status_label;
use crate::pricing;
use crate::services::admin::dashboard_service;
use crate::session::CurrentAdmin;
use crate::views::admin::dashboard::{DashboardView, LatestOrderRow, SalesBar};

/// The low-stock list's threshold is the typed config value
/// `low_stock_default` (products may override it per product).
/// How many days the sales chart shows.
const CHART_DAYS: i64 = 7;

/// Route: the dashboard is the admin root.
pub fn router() -> Router<PgPool> {
    Router::new().route("/admin", get(dashboard))
}

/// Renders the dashboard.
pub async fn dashboard(
    State(pool): State<PgPool>,
    CurrentAdmin(admin): CurrentAdmin,
    session: Session,
) -> Response {
    // Every figure on this page comes from the database, so a
    // failed query is a 500, never a fake zero. Swallowing these
    // errors is what let broken sales queries ship unnoticed: the
    // page rendered ¥0 and empty bars with a 200 until 2026-11-02.
    let orders_today = match dashboard_service::count_orders_today(&pool).await {
        Ok(value) => value,
        Err(e) => return db_error("dashboard", e),
    };
    let sales = match dashboard_service::sales_today(&pool).await {
        Ok(value) => value,
        Err(e) => return db_error("dashboard", e),
    };
    let unshipped = match dashboard_service::count_unshipped(&pool).await {
        Ok(value) => value,
        Err(e) => return db_error("dashboard", e),
    };
    let low_stock =
        match dashboard_service::low_stock_products(&pool, crate::config::CONFIG.low_stock_default)
            .await
        {
            Ok(value) => value,
            Err(e) => return db_error("dashboard", e),
        };
    let latest = match dashboard_service::latest_orders(&pool, 10).await {
        Ok(value) => value,
        Err(e) => return db_error("dashboard", e),
    };
    let days = match dashboard_service::sales_last_days(&pool, CHART_DAYS).await {
        Ok(value) => value,
        Err(e) => return db_error("dashboard", e),
    };

    let jst = crate::models::jst::jst();
    let today = crate::models::jst::jst_today();
    let latest_rows = latest
        .iter()
        .map(|o| LatestOrderRow {
            number: crate::models::order::order_number_for(o.id, o.created_at),
            id: o.id,
            buyer: o
                .member_name
                .clone()
                .unwrap_or_else(|| "ゲスト".to_string()),
            total: pricing::display_price(o.total_yen),
            status_label: admin_status_label(&o.status),
            time: o.created_at.with_timezone(&jst).format("%H:%M").to_string(),
        })
        .collect();
    let max = days.iter().map(|d| d.total_yen).max().unwrap_or(0);
    let bars = days
        .iter()
        .map(|d| SalesBar {
            label: format!("{}/{}（{}）", d.day.month(), d.day.day(), weekday_ja(d.day)),
            height_pct: if max > 0 {
                (d.total_yen * 100 / max).max(if d.total_yen > 0 { 4 } else { 0 })
            } else {
                0
            },
            amount: pricing::display_price(d.total_yen),
            is_today: d.day == today,
        })
        .collect();

    // The ops status panel degrades per figure (its service
    // documents why), so it loads after the fail-closed
    // figures and never blocks them.
    let ops = crate::services::admin::ops_status_service::load(&pool).await;
    let ops_backup_label = match &ops.backup {
        Some(b) => format!("{}（{}、{}）", b.filename, b.modified_label, b.size_label),
        None => "確認できません".to_string(),
    };
    let ops_mail_label = format!(
        "キャンペーン待ち {}件・失敗 {}件（実行中ジョブ {}件）／ 発送メール待ち {}件・失敗 {}件",
        ops.mail_waiting,
        ops.mail_failed,
        ops.mail_jobs_running,
        ops.shipping_waiting,
        ops.shipping_failed
    );
    let ops_sweep_label = match &ops.sweep_last_run_label {
        Some(label) => format!("最終実行 {label}"),
        None => "記録なし".to_string(),
    };
    let ops_llm_label = if ops.llm_up {
        format!("稼働中（{}）", ops.llm_model)
    } else {
        format!("応答なし（{}）", ops.llm_model)
    };
    render(DashboardView {
        layout: layout_for(&pool, &admin, &session).await,
        nav: "dashboard",
        ops_backup_label,
        ops_mail_label,
        ops_sweep_label,
        ops_llm_label,
        today_label: format!(
            "{}年{}月{}日（{}）",
            today.year(),
            today.month(),
            today.day(),
            weekday_ja(today)
        ),
        orders_today,
        sales_today: pricing::display_price(sales),
        unshipped,
        open_inquiries: crate::repositories::inquiry_repository::count_by_status(&pool, "open")
            .await
            .unwrap_or(0),
        low_stock_count: low_stock.len(),
        low_stock_default: crate::config::CONFIG.low_stock_default,
        latest: latest_rows,
        low_stock,
        bars,
    })
}

/// The Japanese weekday character for a date.
fn weekday_ja(day: chrono::NaiveDate) -> &'static str {
    match day.weekday() {
        chrono::Weekday::Mon => "月",
        chrono::Weekday::Tue => "火",
        chrono::Weekday::Wed => "水",
        chrono::Weekday::Thu => "木",
        chrono::Weekday::Fri => "金",
        chrono::Weekday::Sat => "土",
        chrono::Weekday::Sun => "日",
    }
}
