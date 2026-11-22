//! Admin shop holidays: the operator's closed days.
//!
//! A holiday moves the delivery promise (the earliest date
//! counts working days only, and a holiday is never offered
//! as a delivery date), so changes are audited under the
//! shop-settings target and the storefront cache is dropped
//! on every write.

use chrono::NaiveDate;
use sqlx::PgPool;

use crate::error::AppError;
use crate::repositories::holiday_repository::{self, HolidayRow};
use crate::services::admin::audit_service;
use crate::services::holiday_service;
use crate::validation::FieldError;

/// Longest label the form accepts, in characters.
const LABEL_MAX_CHARS: usize = 50;

/// Returns every registered holiday for the admin page.
pub async fn list(pool: &PgPool) -> Result<Vec<HolidayRow>, AppError> {
    holiday_repository::list_all(pool).await
}

/// Registers one holiday and audits it. A date already
/// registered is the field error, not a silent no-op: the
/// operator should learn their list already had it.
pub async fn add(
    pool: &PgPool,
    admin_id: i64,
    date_raw: &str,
    label: &str,
) -> Result<(), Vec<FieldError>> {
    let label = label.trim();
    let mut errors = Vec::new();
    let date = match NaiveDate::parse_from_str(date_raw.trim(), "%Y-%m-%d") {
        Ok(d) => Some(d),
        Err(_) => {
            errors.push(FieldError::new(
                "holiday_date",
                "日付の形式が正しくありません。",
            ));
            None
        }
    };
    if label.chars().count() > LABEL_MAX_CHARS {
        errors.push(FieldError::new(
            "label",
            &format!("名称は{LABEL_MAX_CHARS}文字以内で入力してください。"),
        ));
    }
    let Some(date) = date else {
        return Err(errors);
    };
    if !errors.is_empty() {
        return Err(errors);
    }
    let mut tx = pool.begin().await.map_err(|e| vec![db_field(e)])?;
    let inserted = holiday_repository::insert(&mut tx, date, label)
        .await
        .map_err(|e| vec![db_field(e)])?;
    if !inserted {
        return Err(vec![FieldError::new(
            "holiday_date",
            "その日はすでに登録されています。",
        )]);
    }
    audit_service::record(
        &mut tx,
        admin_id,
        "shop_holiday.create",
        "shop_settings",
        Some(1),
    )
    .await
    .map_err(|e| vec![db_field(e)])?;
    tx.commit().await.map_err(|e| vec![db_field(e)])?;
    holiday_service::invalidate_cache();
    Ok(())
}

/// Removes one holiday and audits it; removing a date that
/// is not registered is a plain false, not an error.
pub async fn remove(pool: &PgPool, admin_id: i64, date: NaiveDate) -> Result<bool, AppError> {
    let mut tx = pool.begin().await.map_err(|e| AppError::db("begin", e))?;
    let removed = holiday_repository::delete(&mut tx, date).await?;
    if removed {
        audit_service::record(
            &mut tx,
            admin_id,
            "shop_holiday.delete",
            "shop_settings",
            Some(1),
        )
        .await?;
    }
    tx.commit().await.map_err(|e| AppError::db("commit", e))?;
    if removed {
        holiday_service::invalidate_cache();
    }
    Ok(removed)
}

/// Wraps a database failure as a form-level error: the add
/// form has nowhere else to say it.
fn db_field(e: impl std::fmt::Display) -> FieldError {
    tracing::error!("holiday write failed: {e}");
    FieldError::new(
        "holiday_date",
        "保存できませんでした。時間をおいて再度お試しください。",
    )
}
