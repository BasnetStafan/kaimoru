//! Tax administration: the class/rate overview and registering
//! a new dated rate.
//!
//! Rates are never edited in place: a change is a new row with
//! its own effective date, so orders keep resolving the rate of
//! their own date and historical breakdowns never move.

use sqlx::PgPool;

use crate::error::AppError;
use crate::messages::TAX_CLASS_REQUIRED_MESSAGE;
use crate::repositories::tax_repository;
use crate::services::admin::audit_service;
use crate::validation::FieldError;

/// One tax class with its full rate history, newest first.
pub struct ClassRates {
    /// The class.
    pub class: crate::models::tax::TaxClass,
    /// Its rates, newest effective date first.
    pub rates: Vec<crate::models::tax::TaxRate>,
}

/// The overview's data: every class with its rate history.
pub async fn overview(pool: &PgPool) -> Result<Vec<ClassRates>, AppError> {
    let classes = tax_repository::list_classes(pool).await?;
    let mut out = Vec::new();
    for class in classes {
        let rates = tax_repository::rates_of_class(pool, class.id).await?;
        out.push(ClassRates { class, rates });
    }
    Ok(out)
}

/// The add-rate form as submitted.
pub struct RateFormInput {
    /// Tax class ID, as typed.
    pub tax_class_id: String,
    /// Rate percent, as typed.
    pub rate_percent: String,
    /// Effective-from date (`YYYY-MM-DD`), as typed.
    pub effective_from: String,
}

/// Validates and registers one new rate, with its audit row,
/// in one transaction. A rate for a (class, date) that already
/// exists is refused here — never an upsert — so an accidental
/// double-submit reads as an error, not a silent rewrite.
pub async fn add_rate(pool: &PgPool, admin_id: i64, input: &RateFormInput) -> Result<(), AppError> {
    let mut errors = Vec::new();
    let class_id = input.tax_class_id.trim().parse::<i64>().ok();
    let classes = tax_repository::list_classes(pool).await?;
    let class = class_id.and_then(|id| classes.iter().find(|c| c.id == id));
    if class.is_none() {
        errors.push(FieldError::new("tax_class_id", TAX_CLASS_REQUIRED_MESSAGE));
    }
    let rate = match input.rate_percent.trim().parse::<i32>() {
        Ok(v) if (0..=100).contains(&v) => Some(v),
        _ => {
            errors.push(FieldError::new(
                "rate_percent",
                "税率は0〜100の整数で入力してください",
            ));
            None
        }
    };
    let from = match chrono::NaiveDate::parse_from_str(input.effective_from.trim(), "%Y-%m-%d") {
        Ok(d) => Some(d),
        Err(_) => {
            errors.push(FieldError::new(
                "effective_from",
                "適用開始日を入力してください",
            ));
            None
        }
    };
    if !errors.is_empty() {
        return Err(AppError::Validation(errors));
    }
    // The unwraps hold by the gate above: each of the three
    // values is None only on a path that pushed its field
    // error, so an empty error list proves all three Some.
    let (class_id, rate, from) = (class_id.unwrap(), rate.unwrap(), from.unwrap());
    let exists = tax_repository::rate_exists(pool, class_id, from).await?;
    if exists {
        return Err(AppError::Validation(vec![FieldError::new(
            "effective_from",
            "この適用開始日の税率は既に登録されています",
        )]));
    }
    let mut tx = pool.begin().await?;
    tax_repository::insert_rate(&mut tx, class_id, rate, from).await?;
    audit_service::record(
        &mut tx,
        admin_id,
        "tax_rate.create",
        "tax_class",
        Some(class_id),
    )
    .await?;
    tx.commit().await?;
    // A registered rate applies at once: drop the cached
    // display-side parameters.
    crate::services::tax_service::invalidate_params_cache();
    Ok(())
}
