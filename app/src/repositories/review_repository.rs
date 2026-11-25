//! Review repository.
//!
//! Owns every review query. Ownership and eligibility are enforced
//! inside the SQL — a query either filters by the session's
//! user_id itself or it does not run; no caller loads a row
//! unfiltered and checks afterwards.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use sqlx::PgPool;

use crate::models::review::{RatingAggregate, Review, ValidReview};

/// Sort orders for the product page review list.
///
/// The ORDER BY text is chosen from this fixed list in Rust;
/// the request's sort parameter only ever selects a variant,
/// never SQL text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReviewSort {
    /// Newest first (the default).
    Newest,
    /// Highest rated first; ties fall back to newest first.
    Highest,
    /// Lowest rated first; ties fall back to newest first.
    Lowest,
}

impl ReviewSort {
    /// The query-string value this sort travels as.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Newest => "new",
            Self::Highest => "high",
            Self::Lowest => "low",
        }
    }

    /// The fixed ORDER BY fragment for this sort. The id
    /// tiebreak keeps the order deterministic across pages.
    fn order_by(&self) -> &'static str {
        match self {
            Self::Newest => "created_at DESC, id DESC",
            Self::Highest => "rating DESC, created_at DESC, id DESC",
            Self::Lowest => "rating ASC, created_at DESC, id DESC",
        }
    }
}

/// Parses the request's sort value; anything unknown is the
/// default, newest first.
pub fn parse_sort(value: Option<&str>) -> ReviewSort {
    match value {
        Some("high") => ReviewSort::Highest,
        Some("low") => ReviewSort::Lowest,
        _ => ReviewSort::Newest,
    }
}

const REVIEW_COLUMNS: &str = "id, product_id, user_id, order_id, rating, comment, \
     created_at, updated_at, hidden_at, shop_reply, shop_reply_at";

/// The order that qualifies this member to review this
/// product: one of their own DELIVERED orders containing the
/// product in its lines (owner decision: reviews
/// only after delivery — a received, preparing, shipped or
/// cancelled order never qualifies). The newest such order is
/// recorded on the review.
pub async fn qualifying_order_id(
    pool: &PgPool,
    user_id: i64,
    product_id: i64,
) -> Result<Option<i64>, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT o.id FROM orders o \
         JOIN order_items oi ON oi.order_id = o.id \
         WHERE o.user_id = $1 AND oi.product_id = $2 AND o.status = 'delivered' \
         ORDER BY o.id DESC LIMIT 1",
    )
    .bind(user_id)
    .bind(product_id)
    .fetch_optional(pool)
    .await
}

/// The member's own review of this product, when it exists.
pub async fn find_own(
    pool: &PgPool,
    user_id: i64,
    product_id: i64,
) -> Result<Option<Review>, sqlx::Error> {
    sqlx::query_as::<_, Review>(&format!(
        "SELECT {REVIEW_COLUMNS} FROM reviews WHERE user_id = $1 AND product_id = $2"
    ))
    .bind(user_id)
    .bind(product_id)
    .fetch_optional(pool)
    .await
}

/// The member's own reviews among several products, as a
/// product_id → review_id map (the order detail page's buttons).
pub async fn find_own_for_products(
    pool: &PgPool,
    user_id: i64,
    product_ids: &[i64],
) -> Result<HashMap<i64, i64>, sqlx::Error> {
    if product_ids.is_empty() {
        return Ok(HashMap::new());
    }
    let rows: Vec<(i64, i64)> = sqlx::query_as(
        "SELECT product_id, id FROM reviews WHERE user_id = $1 AND product_id = ANY($2)",
    )
    .bind(user_id)
    .bind(product_ids)
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().collect())
}

/// One review by id, but only when it belongs to this member.
///
/// A foreign id and a missing id return the same None, so the
/// caller answers both with the same styled 404 and a foreign
/// id never confirms that the review exists.
pub async fn find_by_id_for_user(
    pool: &PgPool,
    review_id: i64,
    user_id: i64,
) -> Result<Option<Review>, sqlx::Error> {
    sqlx::query_as::<_, Review>(&format!(
        "SELECT {REVIEW_COLUMNS} FROM reviews WHERE id = $1 AND user_id = $2"
    ))
    .bind(review_id)
    .bind(user_id)
    .fetch_optional(pool)
    .await
}

/// Returns the rating aggregates for a set of products, in one
/// grouped query however many products the page shows — never one
/// query per card. Products without reviews are absent from the
/// map; callers read absence as no reviews.
pub async fn aggregate_for_products(
    pool: &PgPool,
    product_ids: &[i64],
) -> Result<HashMap<i64, RatingAggregate>, sqlx::Error> {
    if product_ids.is_empty() {
        return Ok(HashMap::new());
    }
    let rows: Vec<(i64, Option<f64>, i64)> = sqlx::query_as(
        "SELECT product_id, AVG(rating)::float8, COUNT(*) FROM reviews \
         WHERE product_id = ANY($1) AND hidden_at IS NULL GROUP BY product_id",
    )
    .bind(product_ids)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(product_id, avg, count)| (product_id, RatingAggregate::from_raw(avg, count)))
        .collect())
}

/// How many reviews gave each rating value for this product,
/// as (rating, count) rows — ratings are half steps, so the
/// caller builds the fixed ten-row distribution from these.
pub async fn distribution(pool: &PgPool, product_id: i64) -> Result<Vec<(f64, i64)>, sqlx::Error> {
    sqlx::query_as(
        "SELECT rating, COUNT(*) FROM reviews \
         WHERE product_id = $1 AND hidden_at IS NULL GROUP BY rating",
    )
    .bind(product_id)
    .fetch_all(pool)
    .await
}

/// One row of the paged list query: the review columns plus the
/// window-counted total (a tuple of struct and count does not decode).
#[derive(sqlx::FromRow)]
struct ReviewPageRow {
    id: i64,
    product_id: i64,
    user_id: i64,
    order_id: i64,
    rating: f64,
    comment: String,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    hidden_at: Option<DateTime<Utc>>,
    shop_reply: String,
    shop_reply_at: Option<DateTime<Utc>>,
    total_count: i64,
}

impl From<ReviewPageRow> for Review {
    fn from(row: ReviewPageRow) -> Self {
        Self {
            id: row.id,
            product_id: row.product_id,
            user_id: row.user_id,
            order_id: row.order_id,
            rating: row.rating,
            comment: row.comment,
            created_at: row.created_at,
            updated_at: row.updated_at,
            hidden_at: row.hidden_at,
            shop_reply: row.shop_reply,
            shop_reply_at: row.shop_reply_at,
        }
    }
}

/// Returns one page of a product's reviews in the requested order,
/// with the total across all pages — page rows and count in one
/// query via `COUNT(*) OVER ()`.
pub async fn find_by_product(
    pool: &PgPool,
    product_id: i64,
    sort: ReviewSort,
    limit: i64,
    offset: i64,
) -> Result<(Vec<Review>, i64), sqlx::Error> {
    let sql = format!(
        "SELECT {REVIEW_COLUMNS}, COUNT(*) OVER () AS total_count FROM reviews \
         WHERE product_id = $1 AND hidden_at IS NULL ORDER BY {} LIMIT $2 OFFSET $3",
        sort.order_by()
    );
    let rows: Vec<ReviewPageRow> = sqlx::query_as(&sql)
        .bind(product_id)
        .bind(limit)
        .bind(offset)
        .fetch_all(pool)
        .await?;
    if let Some(first) = rows.first() {
        let total = first.total_count;
        return Ok((rows.into_iter().map(Review::from).collect(), total));
    }
    if offset == 0 {
        return Ok((Vec::new(), 0));
    }
    // A page past the end returns no rows to carry the total;
    // only then is a separate count worth its round trip.
    let total: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM reviews WHERE product_id = $1 AND hidden_at IS NULL",
    )
    .bind(product_id)
    .fetch_one(pool)
    .await?;
    Ok((Vec::new(), total))
}

/// Inserts a review for a purchase already proven eligible.
///
/// A uniqueness failure (two posts racing past the eligibility
/// check) surfaces as the database error; the service maps it to
/// the duplicate message.
pub async fn insert(
    pool: &PgPool,
    user_id: i64,
    product_id: i64,
    order_id: i64,
    valid: &ValidReview,
) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar(
        "INSERT INTO reviews (product_id, user_id, order_id, rating, comment) \
         VALUES ($1, $2, $3, $4, $5) RETURNING id",
    )
    .bind(product_id)
    .bind(user_id)
    .bind(order_id)
    .bind(valid.rating)
    .bind(&valid.comment)
    .fetch_one(pool)
    .await
}

/// Updates the member's own review; ownership is in the WHERE
/// clause, so a foreign id updates nothing. Returns whether a
/// row changed.
/// Replaces a review's rating and comment, but only when
/// the row still carries `expected` as its `updated_at`:
/// the form-ticket guard as a compare-and-swap in the WHERE
/// clause, atomic in the single statement. A row that moved
/// after the edit form was rendered matches nothing, and
/// the service tells that apart from a missing row.
pub async fn update(
    pool: &PgPool,
    review_id: i64,
    user_id: i64,
    valid: &ValidReview,
    expected: chrono::DateTime<chrono::Utc>,
) -> Result<bool, sqlx::Error> {
    let result = sqlx::query(
        "UPDATE reviews SET rating = $1, comment = $2, updated_at = now() \
         WHERE id = $3 AND user_id = $4 AND updated_at = $5",
    )
    .bind(valid.rating)
    .bind(&valid.comment)
    .bind(review_id)
    .bind(user_id)
    .bind(expected)
    .execute(pool)
    .await?;
    Ok(result.rows_affected() > 0)
}

/// Hard-deletes the member's own review; ownership is in the
/// WHERE clause, so a foreign id deletes nothing. Returns whether
/// a row was removed. There is no soft delete in v1: the row is
/// gone and aggregates recompute from what remains.
pub async fn delete(pool: &PgPool, review_id: i64, user_id: i64) -> Result<bool, sqlx::Error> {
    let result = sqlx::query("DELETE FROM reviews WHERE id = $1 AND user_id = $2")
        .bind(review_id)
        .bind(user_id)
        .execute(pool)
        .await?;
    Ok(result.rows_affected() > 0)
}

/// Finds one review by id, whoever wrote it. The helpful
/// endpoint needs the row itself (to check it belongs to
/// the product in the URL); member-scoped reads stay on
/// [`find_by_id_for_user`].
pub async fn find_by_id(pool: &PgPool, review_id: i64) -> Result<Option<Review>, sqlx::Error> {
    sqlx::query_as(&format!(
        "SELECT {REVIEW_COLUMNS} FROM reviews WHERE id = $1"
    ))
    .bind(review_id)
    .fetch_optional(pool)
    .await
}

/// Records one helpful vote. The (review, voter) unique
/// pair is the duplicate protection: a repeat insert is a
/// no-op, reported as `false` (already voted).
pub async fn record_helpful_vote(
    pool: &PgPool,
    review_id: i64,
    voter_key: &str,
) -> Result<bool, sqlx::Error> {
    let result = sqlx::query(
        "INSERT INTO review_helpful_votes (review_id, voter_key) VALUES ($1, $2) \
         ON CONFLICT (review_id, voter_key) DO NOTHING",
    )
    .bind(review_id)
    .bind(voter_key)
    .execute(pool)
    .await?;
    Ok(result.rows_affected() == 1)
}

/// The helpful count of one review.
pub async fn helpful_count(pool: &PgPool, review_id: i64) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar("SELECT COUNT(*) FROM review_helpful_votes WHERE review_id = $1")
        .bind(review_id)
        .fetch_one(pool)
        .await
}

/// Helpful counts for a page of reviews in one query:
/// reviews without votes are absent from the map (count 0).
pub async fn helpful_counts(
    pool: &PgPool,
    review_ids: &[i64],
) -> Result<std::collections::HashMap<i64, i64>, sqlx::Error> {
    let rows: Vec<(i64, i64)> = sqlx::query_as(
        "SELECT review_id, COUNT(*) FROM review_helpful_votes \
         WHERE review_id = ANY($1) GROUP BY review_id",
    )
    .bind(review_ids)
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().collect())
}

/// Which of these reviews this voter already marked
/// helpful, in one query.
pub async fn helpful_voted(
    pool: &PgPool,
    review_ids: &[i64],
    voter_key: &str,
) -> Result<std::collections::HashSet<i64>, sqlx::Error> {
    let rows: Vec<i64> = sqlx::query_scalar(
        "SELECT review_id FROM review_helpful_votes \
         WHERE review_id = ANY($1) AND voter_key = $2",
    )
    .bind(review_ids)
    .bind(voter_key)
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().collect())
}
