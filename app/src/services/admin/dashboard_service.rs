//! Admin dashboard reads.
//!
//! The read assembly behind the dashboard: today's counters, the
//! latest orders, the low-stock list and the 7-day sales series.
//! Every figure comes straight from the repository aggregates —
//! day boundaries are Japan time, computed in SQL — and a failed
//! query propagates as an error: the page renders real numbers
//! or fails, never invented zeros.

use sqlx::PgPool;

use crate::error::AppError;
use crate::models::{AdminOrderSummary, AdminProduct, SalesDay};
use crate::repositories::{admin::order_repository, admin::product_repository};

/// Orders placed today (Japan time), cancelled included.
pub async fn count_orders_today(pool: &PgPool) -> Result<i64, AppError> {
    order_repository::count_orders_today(pool).await
}

/// Sales total for today (Japan time), cancelled excluded.
pub async fn sales_today(pool: &PgPool) -> Result<i64, AppError> {
    order_repository::sales_today(pool).await
}

/// Orders waiting to ship （受付済み + 準備中）.
pub async fn count_unshipped(pool: &PgPool) -> Result<i64, AppError> {
    order_repository::count_unshipped(pool).await
}

/// The newest orders, for the dashboard's latest-orders
/// table.
pub async fn latest_orders(pool: &PgPool, limit: i64) -> Result<Vec<AdminOrderSummary>, AppError> {
    order_repository::latest_orders(pool, limit).await
}

/// Public products at or below the low-stock threshold,
/// least stock first.
pub async fn low_stock_products(
    pool: &PgPool,
    threshold: i64,
) -> Result<Vec<AdminProduct>, AppError> {
    product_repository::low_stock_products(pool, threshold).await
}

/// Per-day sales for the last `days` Japan-calendar days,
/// oldest first, cancelled orders excluded.
pub async fn sales_last_days(pool: &PgPool, days: i64) -> Result<Vec<SalesDay>, AppError> {
    order_repository::sales_last_days(pool, days).await
}
