//! Reads for the postal code table.
//!
//! The table is loaded from the Japan Post data by
//! `db/import_postal_codes.sh` and replaced wholesale
//! on every import; the app itself only reads it.

use sqlx::PgPool;

use crate::error::AppError;
use crate::models::PostalAddress;

/// Returns every town row for one postal code, ordered by
/// town name so a multi-town code answers in a stable order.
pub async fn find_by_code(
    pool: &PgPool,
    postal_code: &str,
) -> Result<Vec<PostalAddress>, AppError> {
    sqlx::query_as::<_, PostalAddress>(
        "SELECT prefecture, city, town FROM postal_codes \
         WHERE postal_code = $1 ORDER BY town",
    )
    .bind(postal_code)
    .fetch_all(pool)
    .await
    .map_err(|e| AppError::db("postal code lookup", e))
}
