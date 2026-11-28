//! Category CSV import/export for the admin.
//!
//! The column map is the locked five (ID, name, slug,
//! parent ID, description). Two form fields the map does
//! not carry — display order and the active flag — are
//! carried forward from the stored category on update (and
//! default on create), so an export → import round-trip
//! never silently reorders or deactivates a category.
//! Writes go through `validate_category` → `create_category`
//! / `update_category`, so the parent-cycle guard and the
//! product name-snapshot sync behave exactly as the form's.

use sqlx::PgPool;

use crate::config::CONFIG;
use crate::error::AppError;
use crate::messages::csv_row_limit_message;
use crate::repositories::csv_import_batch_repository;
use crate::services::admin::category_service::{self, CategoryFormInput, ValidatedCategory};
use crate::util::csv_transfer::{BatchReport, LineError, decode_bytes, parse_rows, write_csv};

/// The fixed category column set, in file order.
pub const CATEGORY_HEADERS: [&str; 5] = [
    "カテゴリーID",
    "カテゴリー名",
    "スラッグ",
    "親カテゴリーID",
    "説明",
];

/// Renders the export file: every category, in ID order.
pub async fn export_csv(pool: &PgPool) -> Result<Vec<u8>, AppError> {
    let categories = category_service::parent_options(pool, None).await?;
    let mut sorted = categories;
    sorted.sort_by_key(|c| c.id);
    let rows: Vec<Vec<String>> = sorted
        .iter()
        .map(|c| {
            vec![
                c.id.to_string(),
                c.name.clone(),
                c.slug.clone(),
                c.parent_id.map(|id| id.to_string()).unwrap_or_default(),
                c.description.clone(),
            ]
        })
        .collect();
    Ok(write_csv(&CATEGORY_HEADERS, &rows))
}

/// One category's validated write, ready for the apply step.
pub struct ValidRow {
    /// Target category ID; `None` creates a new category.
    pub category_id: Option<i64>,
    /// The validated form, exactly as the form path writes it.
    pub valid: ValidatedCategory,
    /// The stored name before the write (the update path
    /// syncs product snapshots only when the name moves).
    pub previous_name: String,
}

/// Validates one upload end to end: returns the applicable
/// writes plus the preview report. One row is one category —
/// a row with any error applies not at all.
pub async fn validate_upload(
    pool: &PgPool,
    text: &str,
) -> Result<(Vec<ValidRow>, BatchReport), AppError> {
    let mut report = BatchReport::default();
    let rows = match parse_rows(text, &CATEGORY_HEADERS) {
        Ok(rows) => rows,
        Err(e) => {
            report.errors.push(e);
            return Ok((Vec::new(), report));
        }
    };
    if rows.len() > CONFIG.csv_max_rows.max(0) as usize {
        report.errors.push(LineError::new(
            1,
            csv_row_limit_message(CONFIG.csv_max_rows),
        ));
        return Ok((Vec::new(), report));
    }

    let mut valid_rows: Vec<ValidRow> = Vec::new();
    let mut seen_ids: Vec<i64> = Vec::new();
    for row in &rows {
        let id_cell = row.get(0).trim();
        let existing = if id_cell.is_empty() {
            None
        } else {
            match id_cell.parse::<i64>() {
                Ok(id) if id > 0 => {
                    if seen_ids.contains(&id) {
                        report.errors.push(LineError::new(
                            row.line,
                            format!("カテゴリーID {id} が重複しています"),
                        ));
                        continue;
                    }
                    seen_ids.push(id);
                    match category_service::find_category(pool, id).await? {
                        Some(c) => Some(c),
                        None => {
                            report.errors.push(LineError::new(
                                row.line,
                                format!("カテゴリーID {id} のカテゴリーが存在しません"),
                            ));
                            continue;
                        }
                    }
                }
                _ => {
                    report.errors.push(LineError::new(
                        row.line,
                        format!("カテゴリーID「{id_cell}」が正しくありません"),
                    ));
                    continue;
                }
            }
        };
        // Fields the CSV map does not carry ride forward
        // from the stored record (see the module doc).
        let input = CategoryFormInput {
            name: row.get(1).to_string(),
            slug: row.get(2).to_string(),
            description: row.get(4).to_string(),
            parent_id: row.get(3).trim().to_string(),
            display_order: existing
                .as_ref()
                .map(|c| c.display_order.to_string())
                .unwrap_or_default(),
            is_active: existing.as_ref().map(|c| c.is_active).unwrap_or(true),
        };
        match category_service::validate_category(pool, &input, existing.as_ref().map(|c| c.id))
            .await
        {
            Ok(valid) => {
                if existing.is_some() {
                    report.update_count += 1;
                } else {
                    report.new_count += 1;
                }
                valid_rows.push(ValidRow {
                    category_id: existing.as_ref().map(|c| c.id),
                    valid,
                    previous_name: existing.map(|c| c.name).unwrap_or_default(),
                });
            }
            Err(field_errors) => {
                for fe in field_errors {
                    report.errors.push(LineError::new(row.line, fe.message));
                }
            }
        }
    }
    report.errors.sort_by_key(|e| e.line);
    Ok((valid_rows, report))
}

/// Decodes and stores one upload as a preview batch.
pub async fn store_upload(
    pool: &PgPool,
    admin_id: i64,
    file_name: &str,
    bytes: &[u8],
) -> Result<i64, AppError> {
    let text = decode_bytes(bytes);
    let (_, report) = validate_upload(pool, &text).await?;
    csv_import_batch_repository::insert_batch(pool, "category", admin_id, file_name, &text, &report)
        .await
}

/// Applies one preview batch: re-validates, writes every
/// still-valid category, then flips the batch to applied
/// with its one audit row. Every write — the categories,
/// the batch flip and the audit rows — commits or rolls
/// back as ONE transaction: a row failing mid-batch leaves
/// nothing half-applied behind. Returns (created, updated).
pub async fn apply_batch(
    pool: &PgPool,
    admin_id: i64,
    batch_id: i64,
) -> Result<(usize, usize), AppError> {
    let batch = csv_import_batch_repository::find_batch(pool, batch_id)
        .await?
        .filter(|b| b.entity == "category" && b.status == "preview")
        .ok_or(AppError::NotFound)?;
    let (rows, _) = validate_upload(pool, &batch.raw_text).await?;
    let (mut created, mut updated) = (0usize, 0usize);
    let mut tx = pool.begin().await.map_err(|e| AppError::db("begin", e))?;
    for row in rows {
        match row.category_id {
            Some(id) => {
                category_service::update_category_tx(
                    &mut tx,
                    admin_id,
                    id,
                    &row.valid,
                    &row.previous_name,
                    None,
                )
                .await?;
                updated += 1;
            }
            None => {
                category_service::create_category_tx(&mut tx, admin_id, &row.valid).await?;
                created += 1;
            }
        }
    }
    csv_import_batch_repository::mark_applied(&mut tx, batch_id).await?;
    crate::services::admin::audit_service::record(
        &mut tx,
        admin_id,
        "category.csv_import",
        "category",
        None,
    )
    .await?;
    tx.commit().await.map_err(|e| AppError::db("commit", e))?;
    // The storefront navigation caches category names
    // briefly; drop the cache so the batch shows at once
    // (the per-row saves used to drop it after each row).
    crate::services::product_service::invalidate_categories_cache();
    Ok((created, updated))
}

/// Discards one preview batch without touching any category.
pub async fn discard_batch(pool: &PgPool, batch_id: i64) -> Result<(), AppError> {
    csv_import_batch_repository::find_batch(pool, batch_id)
        .await?
        .filter(|b| b.entity == "category" && b.status == "preview")
        .ok_or(AppError::NotFound)?;
    csv_import_batch_repository::mark_discarded(pool, batch_id).await
}
