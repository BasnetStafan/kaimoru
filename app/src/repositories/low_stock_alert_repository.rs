//! Low-stock alert record: which products the operator has
//! already been mailed about on a given day.

use sqlx::PgPool;

use crate::error::AppError;

/// Returns the subset of `product_ids` already alerted on
/// `date`: the worker mails only the rest.
pub async fn alerted_on(
    pool: &PgPool,
    product_ids: &[i64],
    date: chrono::NaiveDate,
) -> Result<Vec<i64>, AppError> {
    sqlx::query_scalar::<_, i64>(
        "SELECT product_id FROM low_stock_alerts \
         WHERE alert_date = $1 AND product_id = ANY($2)",
    )
    .bind(date)
    .bind(product_ids)
    .fetch_all(pool)
    .await
    .map_err(|e| AppError::db("low stock alerted lookup", e))
}

/// Records one product as alerted on `date`. The primary key
/// makes a racing second record a no-op rather than an error.
pub async fn record(
    pool: &PgPool,
    product_id: i64,
    date: chrono::NaiveDate,
) -> Result<(), AppError> {
    sqlx::query(
        "INSERT INTO low_stock_alerts (product_id, alert_date) VALUES ($1, $2) \
         ON CONFLICT (product_id, alert_date) DO NOTHING",
    )
    .bind(product_id)
    .bind(date)
    .execute(pool)
    .await
    .map_err(|e| AppError::db("record low stock alert", e))?;
    Ok(())
}
