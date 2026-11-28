//! Tag CSV import/export for the admin.
//!
//! The column map matches the tags table exactly (ID, slug,
//! label, description). Writes go through `validate_tag` →
//! `create_tag` / `update_tag`, so the vocabulary rules —
//! slug shape, case-insensitive uniqueness of slug and
//! label — behave exactly as the form's. Slugs are Session
//! 5's matching key; an import renames one only when the
//! file says so, like the form.

use sqlx::PgPool;

use crate::config::CONFIG;
use crate::error::AppError;
use crate::messages::csv_row_limit_message;
use crate::repositories::csv_import_batch_repository;
use crate::services::admin::tag_service::{self, TagFormInput, ValidatedTag};
use crate::util::csv_transfer::{BatchReport, LineError, decode_bytes, parse_rows, write_csv};

/// The fixed tag column set, in file order.
pub const TAG_HEADERS: [&str; 4] = ["タグID", "スラッグ", "表示名", "説明"];

/// Renders the export file: every tag, in ID order.
pub async fn export_csv(pool: &PgPool) -> Result<Vec<u8>, AppError> {
    let tags = crate::services::admin::product_service::list_tags(pool).await?;
    let rows: Vec<Vec<String>> = tags
        .iter()
        .map(|t| {
            vec![
                t.id.to_string(),
                t.slug.clone(),
                t.label.clone(),
                t.description.clone(),
            ]
        })
        .collect();
    Ok(write_csv(&TAG_HEADERS, &rows))
}

/// One tag's validated write, ready for the apply step.
pub struct ValidRow {
    /// Target tag ID; `None` creates a new tag.
    pub tag_id: Option<i64>,
    /// The validated form, exactly as the form path writes it.
    pub valid: ValidatedTag,
}

/// Validates one upload end to end: returns the applicable
/// writes plus the preview report. One row is one tag — a
/// row with any error applies not at all.
pub async fn validate_upload(
    pool: &PgPool,
    text: &str,
) -> Result<(Vec<ValidRow>, BatchReport), AppError> {
    let mut report = BatchReport::default();
    let rows = match parse_rows(text, &TAG_HEADERS) {
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
        let existing_id = if id_cell.is_empty() {
            None
        } else {
            match id_cell.parse::<i64>() {
                Ok(id) if id > 0 => {
                    if seen_ids.contains(&id) {
                        report.errors.push(LineError::new(
                            row.line,
                            format!("タグID {id} が重複しています"),
                        ));
                        continue;
                    }
                    seen_ids.push(id);
                    match tag_service::find_tag(pool, id).await? {
                        Some(_) => Some(id),
                        None => {
                            report.errors.push(LineError::new(
                                row.line,
                                format!("タグID {id} のタグが存在しません"),
                            ));
                            continue;
                        }
                    }
                }
                _ => {
                    report.errors.push(LineError::new(
                        row.line,
                        format!("タグID「{id_cell}」が正しくありません"),
                    ));
                    continue;
                }
            }
        };
        let input = TagFormInput {
            label: row.get(2).to_string(),
            slug: row.get(1).to_string(),
            description: row.get(3).to_string(),
        };
        match tag_service::validate_tag(pool, &input, existing_id).await {
            Ok(valid) => {
                if existing_id.is_some() {
                    report.update_count += 1;
                } else {
                    report.new_count += 1;
                }
                valid_rows.push(ValidRow {
                    tag_id: existing_id,
                    valid,
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
    csv_import_batch_repository::insert_batch(pool, "tag", admin_id, file_name, &text, &report)
        .await
}

/// Applies one preview batch: re-validates, writes every
/// still-valid tag, then flips the batch to applied with
/// its one audit row. Returns (created, updated).
pub async fn apply_batch(
    pool: &PgPool,
    admin_id: i64,
    batch_id: i64,
) -> Result<(usize, usize), AppError> {
    let batch = csv_import_batch_repository::find_batch(pool, batch_id)
        .await?
        .filter(|b| b.entity == "tag" && b.status == "preview")
        .ok_or(AppError::NotFound)?;
    let (rows, _) = validate_upload(pool, &batch.raw_text).await?;
    let (mut created, mut updated) = (0usize, 0usize);
    // Every write — the tags, the batch flip and the audit
    // rows — commits or rolls back as ONE transaction: a
    // row failing mid-batch leaves nothing half-applied.
    let mut tx = pool.begin().await.map_err(|e| AppError::db("begin", e))?;
    for row in rows {
        match row.tag_id {
            Some(id) => {
                tag_service::update_tag_tx(&mut tx, admin_id, id, &row.valid, None).await?;
                updated += 1;
            }
            None => {
                tag_service::create_tag_tx(&mut tx, admin_id, &row.valid).await?;
                created += 1;
            }
        }
    }
    csv_import_batch_repository::mark_applied(&mut tx, batch_id).await?;
    crate::services::admin::audit_service::record(&mut tx, admin_id, "tag.csv_import", "tag", None)
        .await?;
    tx.commit().await.map_err(|e| AppError::db("commit", e))?;
    Ok((created, updated))
}

/// Discards one preview batch without touching any tag.
pub async fn discard_batch(pool: &PgPool, batch_id: i64) -> Result<(), AppError> {
    csv_import_batch_repository::find_batch(pool, batch_id)
        .await?
        .filter(|b| b.entity == "tag" && b.status == "preview")
        .ok_or(AppError::NotFound)?;
    csv_import_batch_repository::mark_discarded(pool, batch_id).await
}
