//! Tax repository: classes and their dated rates.
//!
//! A rate applies from its `effective_from` until the day before
//! the next rate of the same class starts (or its own
//! `effective_to`, when set): resolution is by the order's date,
//! never by "the current row", so a future rate change can be
//! registered in advance without touching today's prices.

use chrono::NaiveDate;
use sqlx::PgPool;

use crate::error::AppError;
use crate::models::tax::{ClassRate, TaxClass, TaxRate};

/// Lists every tax class in display order.
pub async fn list_classes(pool: &PgPool) -> Result<Vec<TaxClass>, sqlx::Error> {
    sqlx::query_as::<_, TaxClass>("SELECT id, code, name FROM tax_classes ORDER BY id")
        .fetch_all(pool)
        .await
}

/// Resolves the rate of one class in force on `date`, in whole
/// percent; 0 when no rate row covers the date.
pub async fn rate_for_class_on(
    pool: &PgPool,
    tax_class_id: i64,
    date: NaiveDate,
) -> Result<i32, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT rate_percent FROM tax_rates \
         WHERE tax_class_id = $1 AND effective_from <= $2 \
           AND (effective_to IS NULL OR effective_to >= $2) \
         ORDER BY effective_from DESC LIMIT 1",
    )
    .bind(tax_class_id)
    .bind(date)
    .fetch_optional(pool)
    .await
    .map(|row| row.unwrap_or(0))
}

/// Resolves every class with its rate in force on `date`, in
/// one query: the per-class rate is a lateral subquery with
/// the same resolution rule as [`rate_for_class_on`], so the
/// caller pays one round-trip instead of one per class.
pub async fn class_rates_on(pool: &PgPool, date: NaiveDate) -> Result<Vec<ClassRate>, sqlx::Error> {
    let rows = sqlx::query_as::<_, (i64, String, String, i32)>(
        "SELECT c.id, c.code, c.name, COALESCE(r.rate_percent, 0) \
         FROM tax_classes c \
         LEFT JOIN LATERAL ( \
             SELECT rate_percent FROM tax_rates \
             WHERE tax_class_id = c.id AND effective_from <= $1 \
               AND (effective_to IS NULL OR effective_to >= $1) \
             ORDER BY effective_from DESC LIMIT 1 \
         ) r ON true \
         ORDER BY c.id",
    )
    .bind(date)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(id, code, name, rate_percent)| ClassRate {
            class: TaxClass { id, code, name },
            rate_percent,
        })
        .collect())
}

/// One class by ID.
pub async fn find_class(pool: &PgPool, id: i64) -> Result<Option<TaxClass>, sqlx::Error> {
    sqlx::query_as::<_, TaxClass>("SELECT id, code, name FROM tax_classes WHERE id = $1")
        .bind(id)
        .fetch_optional(pool)
        .await
}

/// Every rate of one class, newest effective date first — the
/// administration overview's history rows.
pub async fn rates_of_class(pool: &PgPool, class_id: i64) -> Result<Vec<TaxRate>, AppError> {
    sqlx::query_as::<_, TaxRate>(
        "SELECT id, tax_class_id, rate_percent, effective_from, effective_to \
         FROM tax_rates WHERE tax_class_id = $1 ORDER BY effective_from DESC, id DESC",
    )
    .bind(class_id)
    .fetch_all(pool)
    .await
    .map_err(|e| AppError::db("list class rates", e))
}

/// Whether a rate already exists for (class, effective date):
/// the add-rate flow refuses duplicates instead of upserting.
pub async fn rate_exists(
    pool: &PgPool,
    class_id: i64,
    effective_from: NaiveDate,
) -> Result<bool, AppError> {
    sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(SELECT 1 FROM tax_rates \
         WHERE tax_class_id = $1 AND effective_from = $2)",
    )
    .bind(class_id)
    .bind(effective_from)
    .fetch_one(pool)
    .await
    .map_err(|e| AppError::db("check rate exists", e))
}

/// Inserts one rate row inside the caller's transaction.
pub async fn insert_rate(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    class_id: i64,
    rate_percent: i32,
    effective_from: NaiveDate,
) -> Result<(), AppError> {
    sqlx::query(
        "INSERT INTO tax_rates (tax_class_id, rate_percent, effective_from) \
         VALUES ($1, $2, $3)",
    )
    .bind(class_id)
    .bind(rate_percent)
    .bind(effective_from)
    .execute(&mut **tx)
    .await
    .map_err(|e| AppError::db("insert tax rate", e))?;
    Ok(())
}
