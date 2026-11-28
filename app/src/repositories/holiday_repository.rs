//! Shop holiday storage: the registered closed days.

use chrono::NaiveDate;
use sqlx::PgPool;

use crate::error::AppError;

/// One registered holiday, for the admin list.
pub struct HolidayRow {
    /// The closed day.
    pub date: NaiveDate,
    /// The operator's label (a name or a reason); may be empty.
    pub label: String,
}

/// Returns every registered holiday from `from` onward, in
/// date order: the window the delivery math can ever see.
pub async fn list_from(pool: &PgPool, from: NaiveDate) -> Result<Vec<NaiveDate>, AppError> {
    sqlx::query_scalar::<_, NaiveDate>(
        "SELECT holiday_date FROM shop_holidays WHERE holiday_date >= $1 ORDER BY holiday_date",
    )
    .bind(from)
    .fetch_all(pool)
    .await
    .map_err(|e| AppError::db("list holidays", e))
}

/// Returns all registered holidays, for the admin page.
pub async fn list_all(pool: &PgPool) -> Result<Vec<HolidayRow>, AppError> {
    sqlx::query_as::<_, (NaiveDate, String)>(
        "SELECT holiday_date, label FROM shop_holidays ORDER BY holiday_date",
    )
    .fetch_all(pool)
    .await
    .map(|rows| {
        rows.into_iter()
            .map(|(date, label)| HolidayRow { date, label })
            .collect()
    })
    .map_err(|e| AppError::db("list all holidays", e))
}

/// Registers one holiday inside the caller's transaction.
/// Returns false when the date was already registered.
pub async fn insert(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    date: NaiveDate,
    label: &str,
) -> Result<bool, AppError> {
    let n = sqlx::query(
        "INSERT INTO shop_holidays (holiday_date, label) VALUES ($1, $2) \
         ON CONFLICT (holiday_date) DO NOTHING",
    )
    .bind(date)
    .bind(label)
    .execute(&mut **tx)
    .await
    .map_err(|e| AppError::db("insert holiday", e))?;
    Ok(n.rows_affected() == 1)
}

/// Removes one holiday inside the caller's transaction.
/// Returns false when no such date was registered.
pub async fn delete(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    date: NaiveDate,
) -> Result<bool, AppError> {
    let n = sqlx::query("DELETE FROM shop_holidays WHERE holiday_date = $1")
        .bind(date)
        .execute(&mut **tx)
        .await
        .map_err(|e| AppError::db("delete holiday", e))?;
    Ok(n.rows_affected() == 1)
}
