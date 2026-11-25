//! Admin review moderation.
//!
//! Three tools, mildest first: hide (reversible — the review
//! leaves the storefront and its aggregates but stays listed
//! here), the shop reply (the shop's own public words under
//! the review), and deletion (permanent). Each change and its
//! audit row commit together. Aggregates need no maintenance
//! — ratings are computed from live rows at read time, so a
//! hidden or deleted review's effect disappears on its own.

use sqlx::PgPool;

use crate::error::AppError;
use crate::models::AdminReviewRow;
use crate::repositories::admin::review_repository;
use crate::services::admin::audit_service;

/// Returns one page of the moderation list, newest first,
/// optionally narrowed to one product, plus the total count.
pub async fn list_reviews(
    pool: &PgPool,
    product_id: Option<i64>,
    limit: i64,
    offset: i64,
) -> Result<(Vec<AdminReviewRow>, i64), AppError> {
    review_repository::list_reviews(pool, product_id, limit, offset).await
}

/// Deletes one review. Returns `false` when the review does
/// not exist (already deleted, or a crafted ID) so the caller
/// can answer with the styled 404.
pub async fn delete_review(pool: &PgPool, admin_id: i64, review_id: i64) -> Result<bool, AppError> {
    let mut tx = pool.begin().await.map_err(|e| AppError::db("begin", e))?;
    let product_id = review_repository::delete_review(&mut tx, review_id).await?;
    let Some(_product_id) = product_id else {
        return Ok(false);
    };
    audit_service::record(
        &mut tx,
        admin_id,
        "review.delete",
        "review",
        Some(review_id),
    )
    .await?;
    tx.commit().await.map_err(|e| AppError::db("commit", e))?;
    Ok(true)
}

/// Hides or unhides one review. Audits only an actual change
/// of state; returns `false` when the review does not exist.
pub async fn set_hidden(
    pool: &PgPool,
    admin_id: i64,
    review_id: i64,
    hidden: bool,
) -> Result<bool, AppError> {
    let mut tx = pool.begin().await.map_err(|e| AppError::db("begin", e))?;
    let Some(was_hidden) = review_repository::set_hidden(&mut tx, review_id, hidden).await? else {
        return Ok(false);
    };
    if was_hidden != hidden {
        audit_service::record(
            &mut tx,
            admin_id,
            if hidden {
                "review.hide"
            } else {
                "review.unhide"
            },
            "review",
            Some(review_id),
        )
        .await?;
    }
    tx.commit().await.map_err(|e| AppError::db("commit", e))?;
    Ok(true)
}

/// The shop reply's length ceiling, in characters.
pub const REPLY_MAX_CHARS: usize = 1000;

/// Saves the shop reply (empty clears it), audited. Returns
/// the validation error message, or `None` on success.
pub async fn set_reply(
    pool: &PgPool,
    admin_id: i64,
    review_id: i64,
    reply: &str,
) -> Result<Option<String>, AppError> {
    let reply = reply.trim();
    if reply.chars().count() > REPLY_MAX_CHARS {
        return Ok(Some(format!(
            "店舗からの返信は{REPLY_MAX_CHARS}文字以内で入力してください"
        )));
    }
    let mut tx = pool.begin().await.map_err(|e| AppError::db("begin", e))?;
    if !review_repository::set_reply(&mut tx, review_id, reply).await? {
        return Ok(Some("レビューが見つかりません".to_string()));
    }
    audit_service::record(&mut tx, admin_id, "review.reply", "review", Some(review_id)).await?;
    tx.commit().await.map_err(|e| AppError::db("commit", e))?;
    Ok(None)
}
