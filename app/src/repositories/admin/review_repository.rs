//! Admin-side review queries: the moderation list, the
//! reversible hide, the shop reply and the hard delete. The
//! mutations run inside the caller's transaction with their
//! audit rows.

use sqlx::PgPool;

use crate::error::AppError;
use crate::models::AdminReviewRow;

/// One page of the moderation list, newest first, optionally
/// narrowed to one product, plus the total count.
pub async fn list_reviews(
    pool: &PgPool,
    product_id: Option<i64>,
    limit: i64,
    offset: i64,
) -> Result<(Vec<AdminReviewRow>, i64), AppError> {
    let rows = sqlx::query_as::<_, AdminReviewRowWithTotal>(
        "SELECT r.id, r.product_id, p.name AS product_name, r.rating, \
                r.comment, r.created_at, (r.hidden_at IS NOT NULL) AS hidden, \
                r.shop_reply, COUNT(*) OVER () AS total \
         FROM reviews r JOIN products p ON p.id = r.product_id \
         WHERE ($1::bigint IS NULL OR r.product_id = $1) \
         ORDER BY r.created_at DESC, r.id DESC \
         LIMIT $2 OFFSET $3",
    )
    .bind(product_id)
    .bind(limit)
    .bind(offset)
    .fetch_all(pool)
    .await
    .map_err(|e| AppError::db("list admin reviews", e))?;
    // A page past the end returns no rows to carry the total;
    // only then is a separate count worth its round trip.
    if rows.is_empty() && offset > 0 {
        let total = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM reviews r \
             WHERE ($1::bigint IS NULL OR r.product_id = $1)",
        )
        .bind(product_id)
        .fetch_one(pool)
        .await
        .map_err(|e| AppError::db("count admin reviews", e))?;
        return Ok((Vec::new(), total));
    }
    let total = rows.first().map(|r| r.total).unwrap_or(0);
    Ok((
        rows.into_iter()
            .map(|r| AdminReviewRow {
                id: r.id,
                product_id: r.product_id,
                product_name: r.product_name,
                rating: r.rating,
                // The column is NOT NULL DEFAULT '', so a
                // rating-only review decodes as Some(""); the
                // model's None means "no comment", so the empty
                // string collapses to None at this mapping.
                comment: r.comment.filter(|c| !c.is_empty()),
                created_at: r.created_at,
                hidden: r.hidden,
                shop_reply: r.shop_reply,
            })
            .collect(),
        total,
    ))
}

/// [`AdminReviewRow`] plus the pagination total.
#[derive(sqlx::FromRow)]
struct AdminReviewRowWithTotal {
    id: i64,
    product_id: i64,
    product_name: String,
    rating: f64,
    comment: Option<String>,
    created_at: chrono::DateTime<chrono::Utc>,
    hidden: bool,
    shop_reply: String,
    total: i64,
}

/// Deletes one review inside the caller's transaction and
/// returns the product it belonged to, so the caller can
/// confirm existence and write the audit row. `None` when the
/// review does not exist.
pub async fn delete_review(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    review_id: i64,
) -> Result<Option<i64>, AppError> {
    sqlx::query_scalar::<_, i64>("DELETE FROM reviews WHERE id = $1 RETURNING product_id")
        .bind(review_id)
        .fetch_optional(&mut **tx)
        .await
        .map_err(|e| AppError::db("delete review", e))
}

/// Sets or clears the hidden stamp inside the caller's
/// transaction. Returns whether the review exists; the
/// caller audits only a real change of state, checked
/// against the previous stamp by the service.
pub async fn set_hidden(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    review_id: i64,
    hidden: bool,
) -> Result<Option<bool>, AppError> {
    let previous: Option<Option<chrono::DateTime<chrono::Utc>>> =
        sqlx::query_scalar("SELECT hidden_at FROM reviews WHERE id = $1 FOR UPDATE")
            .bind(review_id)
            .fetch_optional(&mut **tx)
            .await
            .map_err(|e| AppError::db("lock review for hide", e))?;
    let Some(previous) = previous else {
        return Ok(None);
    };
    let was_hidden = previous.is_some();
    if was_hidden != hidden {
        sqlx::query(
            "UPDATE reviews SET hidden_at = CASE WHEN $2 THEN now() ELSE NULL END \
             WHERE id = $1",
        )
        .bind(review_id)
        .bind(hidden)
        .execute(&mut **tx)
        .await
        .map_err(|e| AppError::db("set review hidden", e))?;
    }
    Ok(Some(was_hidden))
}

/// Saves the shop reply inside the caller's transaction
/// (empty text clears it). Returns whether the review
/// exists.
pub async fn set_reply(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    review_id: i64,
    reply: &str,
) -> Result<bool, AppError> {
    let affected =
        sqlx::query("UPDATE reviews SET shop_reply = $2, shop_reply_at = now() WHERE id = $1")
            .bind(review_id)
            .bind(reply)
            .execute(&mut **tx)
            .await
            .map_err(|e| AppError::db("set review reply", e))?;
    Ok(affected.rows_affected() > 0)
}
