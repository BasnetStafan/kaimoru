//! Saved segments: validation, audited create/delete, and
//! the live membership preview the list renders.

use sqlx::PgPool;

use crate::error::AppError;
use crate::messages::NON_NEGATIVE_INTEGER_MESSAGE;
use crate::repositories::admin::segment_repository::{self, CustomerSegment};
use crate::services::admin::audit_service;

/// The segment form as typed; empty numeric/date fields
/// mean "no condition".
pub struct SegmentInput {
    /// Segment name.
    pub name: String,
    /// Required tag, when any.
    pub tag: String,
    /// Min spend text.
    pub min_total_spend_yen: String,
    /// Min orders text.
    pub min_order_count: String,
    /// Last-order-on-or-before date text.
    pub last_order_before: String,
    /// Last-order-on-or-after date text.
    pub last_order_after: String,
    /// Min RFM recency band text (1-5).
    pub min_r_score: String,
    /// Min RFM frequency band text (1-5).
    pub min_f_score: String,
    /// Min RFM monetary band text (1-5).
    pub min_m_score: String,
}

/// Validates the form into a definition. Every problem
/// reports at once, keyed by field name.
pub fn validate(input: &SegmentInput) -> Result<CustomerSegment, Vec<(String, String)>> {
    let mut errors: Vec<(String, String)> = Vec::new();
    let name = input.name.trim().to_string();
    if name.is_empty() {
        errors.push(("name".into(), "セグメント名を入力してください。".into()));
    }
    if name.chars().count() > 100 {
        errors.push((
            "name".into(),
            "セグメント名は100文字以内で入力してください。".into(),
        ));
    }
    let parse_yen = |raw: &str, field: &str, errors: &mut Vec<(String, String)>| {
        let raw = raw.trim();
        if raw.is_empty() {
            return None;
        }
        match raw.parse::<i64>() {
            Ok(v) if v >= 0 => Some(v),
            _ => {
                errors.push((field.into(), NON_NEGATIVE_INTEGER_MESSAGE.into()));
                None
            }
        }
    };
    let parse_count = |raw: &str, field: &str, errors: &mut Vec<(String, String)>| {
        let raw = raw.trim();
        if raw.is_empty() {
            return None;
        }
        match raw.parse::<i32>() {
            Ok(v) if v >= 0 => Some(v),
            _ => {
                errors.push((field.into(), NON_NEGATIVE_INTEGER_MESSAGE.into()));
                None
            }
        }
    };
    let parse_date = |raw: &str, field: &str, errors: &mut Vec<(String, String)>| {
        let raw = raw.trim();
        if raw.is_empty() {
            return None;
        }
        match chrono::NaiveDate::parse_from_str(raw, "%Y-%m-%d") {
            Ok(d) => Some(d),
            Err(_) => {
                errors.push((field.into(), "日付の形式で入力してください。".into()));
                None
            }
        }
    };
    let parse_band = |raw: &str, field: &str, errors: &mut Vec<(String, String)>| {
        let raw = raw.trim();
        if raw.is_empty() {
            return None;
        }
        match raw.parse::<i32>() {
            Ok(v) if (1..=5).contains(&v) => Some(v),
            _ => {
                errors.push((
                    field.into(),
                    "RFMスコアは1から5の整数で入力してください。".into(),
                ));
                None
            }
        }
    };
    let min_total_spend_yen = parse_yen(
        &input.min_total_spend_yen,
        "min_total_spend_yen",
        &mut errors,
    );
    let min_order_count = parse_count(&input.min_order_count, "min_order_count", &mut errors);
    let last_order_before = parse_date(&input.last_order_before, "last_order_before", &mut errors);
    let last_order_after = parse_date(&input.last_order_after, "last_order_after", &mut errors);
    let min_r_score = parse_band(&input.min_r_score, "min_r_score", &mut errors);
    let min_f_score = parse_band(&input.min_f_score, "min_f_score", &mut errors);
    let min_m_score = parse_band(&input.min_m_score, "min_m_score", &mut errors);
    if let (Some(before), Some(after)) = (last_order_before, last_order_after)
        && before < after
    {
        errors.push((
            "last_order_before".into(),
            "最終注文日の上限は下限以降の日付にしてください。".into(),
        ));
    }
    if errors.is_empty() {
        Ok(CustomerSegment {
            id: 0,
            name,
            tag: input.tag.trim().to_string(),
            min_total_spend_yen,
            min_order_count,
            last_order_before,
            last_order_after,
            min_r_score,
            min_f_score,
            min_m_score,
        })
    } else {
        Err(errors)
    }
}

/// Creates a segment, audited. Returns the new id.
pub async fn create_segment(
    pool: &PgPool,
    admin_id: i64,
    segment: &CustomerSegment,
) -> Result<i64, AppError> {
    let mut tx = pool.begin().await.map_err(|e| AppError::db("begin", e))?;
    let id = segment_repository::insert_segment(&mut tx, segment).await?;
    audit_service::record(&mut tx, admin_id, "segment.create", "segment", Some(id)).await?;
    tx.commit().await.map_err(|e| AppError::db("commit", e))?;
    Ok(id)
}

/// Deletes a segment definition, audited. Refused while
/// any campaign targets the segment: deleting it would
/// orphan the campaign's stored audience.
pub async fn delete_segment(pool: &PgPool, admin_id: i64, id: i64) -> Result<bool, AppError> {
    if crate::repositories::mail_repository::count_campaigns_for_segment(pool, id).await? > 0 {
        return Err(AppError::Conflict(
            "このセグメントはメールキャンペーンの宛先に使われているため削除できません".to_string(),
        ));
    }
    let mut tx = pool.begin().await.map_err(|e| AppError::db("begin", e))?;
    if !segment_repository::delete_segment(&mut tx, id).await? {
        return Ok(false);
    }
    audit_service::record(&mut tx, admin_id, "segment.delete", "segment", Some(id)).await?;
    tx.commit().await.map_err(|e| AppError::db("commit", e))?;
    Ok(true)
}

/// The list rows: each definition with its live member
/// count.
pub async fn list_with_counts(pool: &PgPool) -> Result<Vec<(CustomerSegment, i64)>, AppError> {
    segment_repository::list_with_counts(pool).await
}
