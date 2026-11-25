//! Reads and writes for the CSV import batch table.
//!
//! A batch row is the durable half of an import: the decoded
//! file text and its validation report survive the redirect
//! to the preview page, and the status flip to `applied` is
//! the record that the preview's confirm actually ran.

use sqlx::{FromRow, PgPool, Postgres, Transaction};

use crate::error::AppError;
use crate::util::csv_transfer::BatchReport;

/// One stored import batch.
#[derive(Debug, Clone, FromRow)]
pub struct CsvImportBatch {
    /// Primary key.
    pub id: i64,
    /// Entity kind: `product`, `category` or `tag`.
    pub entity: String,
    /// The admin who uploaded the file.
    pub admin_id: i64,
    /// The uploaded file's name, as sent by the browser.
    pub file_name: String,
    /// The decoded file text, re-parsed by the apply step.
    pub raw_text: String,
    /// The validation report JSON (a [`BatchReport`]).
    pub report: String,
    /// Lifecycle: `preview`, `applied` or `discarded`.
    pub status: String,
}

/// Stores one freshly validated upload as a `preview` batch
/// and returns its ID.
pub async fn insert_batch(
    pool: &PgPool,
    entity: &str,
    admin_id: i64,
    file_name: &str,
    raw_text: &str,
    report: &BatchReport,
) -> Result<i64, AppError> {
    let row: (i64,) = sqlx::query_as(
        "INSERT INTO csv_import_batches (entity, admin_id, file_name, raw_text, report) \
         VALUES ($1, $2, $3, $4, $5) RETURNING id",
    )
    .bind(entity)
    .bind(admin_id)
    .bind(file_name)
    .bind(raw_text)
    .bind(report.to_json())
    .fetch_one(pool)
    .await
    .map_err(|e| AppError::db("csv import batch insert", e))?;
    Ok(row.0)
}

/// Returns one batch by ID, whatever its status.
pub async fn find_batch(pool: &PgPool, id: i64) -> Result<Option<CsvImportBatch>, AppError> {
    sqlx::query_as::<_, CsvImportBatch>(
        "SELECT id, entity, admin_id, file_name, raw_text, report, status \
         FROM csv_import_batches WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(pool)
    .await
    .map_err(|e| AppError::db("csv import batch find", e))
}

/// Flips a batch to `applied`, stamping the time. Runs inside
/// the caller's transaction so the flip and the batch's audit
/// row land together — the apply itself (per-record service
/// calls) has already committed record by record, exactly
/// like a sequence of form submits.
pub async fn mark_applied(tx: &mut Transaction<'_, Postgres>, id: i64) -> Result<(), AppError> {
    sqlx::query(
        "UPDATE csv_import_batches SET status = 'applied', applied_at = now() WHERE id = $1",
    )
    .bind(id)
    .execute(&mut **tx)
    .await
    .map_err(|e| AppError::db("csv import batch apply mark", e))?;
    Ok(())
}

/// Flips a batch to `discarded` (the preview's 破棄 action);
/// the rows it would have written are never touched.
pub async fn mark_discarded(pool: &PgPool, id: i64) -> Result<(), AppError> {
    sqlx::query("UPDATE csv_import_batches SET status = 'discarded' WHERE id = $1")
        .bind(id)
        .execute(pool)
        .await
        .map_err(|e| AppError::db("csv import batch discard", e))?;
    Ok(())
}
