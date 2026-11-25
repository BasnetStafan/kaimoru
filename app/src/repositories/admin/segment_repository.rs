//! Saved customer segments: definitions plus the live
//! membership evaluation shared by the admin preview and
//! the campaign audience.
//!
//! Conditions are AND-ed; an unset condition does not
//! filter. Spend and order counts exclude cancelled orders.
//! "Last order" conditions compare the JST calendar date of
//! the member's newest non-cancelled order; a member with
//! no orders never matches a last-order condition (there is
//! no last order to compare).

use sqlx::PgPool;

use crate::error::AppError;

/// One saved segment definition.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct CustomerSegment {
    /// Primary key.
    pub id: i64,
    /// Operator-facing name.
    pub name: String,
    /// Required tag; empty = no tag condition.
    pub tag: String,
    /// Minimum lifetime spend (yen), when set.
    pub min_total_spend_yen: Option<i64>,
    /// Minimum order count, when set.
    pub min_order_count: Option<i32>,
    /// Last order must be on/before this date, when set.
    pub last_order_before: Option<chrono::NaiveDate>,
    /// Last order must be on/after this date, when set.
    pub last_order_after: Option<chrono::NaiveDate>,
}

/// Every segment, newest first.
pub async fn list_segments(pool: &PgPool) -> Result<Vec<CustomerSegment>, AppError> {
    sqlx::query_as::<_, CustomerSegment>(
        "SELECT id, name, tag, min_total_spend_yen, min_order_count, \
                last_order_before, last_order_after \
         FROM customer_segments ORDER BY id DESC",
    )
    .fetch_all(pool)
    .await
    .map_err(|e| AppError::db("list segments", e))
}

/// One segment by id.
pub async fn find_segment(pool: &PgPool, id: i64) -> Result<Option<CustomerSegment>, AppError> {
    sqlx::query_as::<_, CustomerSegment>(
        "SELECT id, name, tag, min_total_spend_yen, min_order_count, \
                last_order_before, last_order_after \
         FROM customer_segments WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(pool)
    .await
    .map_err(|e| AppError::db("find segment", e))
}

/// Inserts a definition inside the caller's transaction.
pub async fn insert_segment(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    segment: &CustomerSegment,
) -> Result<i64, AppError> {
    sqlx::query_scalar(
        "INSERT INTO customer_segments \
            (name, tag, min_total_spend_yen, min_order_count, \
             last_order_before, last_order_after) \
         VALUES ($1, $2, $3, $4, $5, $6) RETURNING id",
    )
    .bind(&segment.name)
    .bind(&segment.tag)
    .bind(segment.min_total_spend_yen)
    .bind(segment.min_order_count)
    .bind(segment.last_order_before)
    .bind(segment.last_order_after)
    .fetch_one(&mut **tx)
    .await
    .map_err(|e| AppError::db("insert segment", e))
}

/// Deletes a definition inside the caller's transaction.
pub async fn delete_segment(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    id: i64,
) -> Result<bool, AppError> {
    let affected = sqlx::query("DELETE FROM customer_segments WHERE id = $1")
        .bind(id)
        .execute(&mut **tx)
        .await
        .map_err(|e| AppError::db("delete segment", e))?;
    Ok(affected.rows_affected() > 0)
}

/// The membership predicate, shared by the count and the id
/// list so the preview and the campaign can never disagree.
/// Placeholders: $1 tag, $2 min spend, $3 min orders,
/// $4 last-order-before, $5 last-order-after.
const MATCH_WHERE: &str = "u.withdrawn_at IS NULL \
    AND ($1 = '' OR EXISTS (SELECT 1 FROM customer_tags t \
         WHERE t.user_id = u.id AND t.tag = $1)) \
    AND ($2::bigint IS NULL OR (SELECT COALESCE(SUM(o.total_yen), 0) \
         FROM orders o WHERE o.user_id = u.id AND o.status <> 'cancelled') >= $2) \
    AND ($3::int IS NULL OR (SELECT COUNT(*) FROM orders o \
         WHERE o.user_id = u.id AND o.status <> 'cancelled') >= $3) \
    AND ($4::date IS NULL OR (SELECT (MAX(o.created_at) AT TIME ZONE 'Asia/Tokyo')::date \
         FROM orders o WHERE o.user_id = u.id AND o.status <> 'cancelled') <= $4) \
    AND ($5::date IS NULL OR (SELECT (MAX(o.created_at) AT TIME ZONE 'Asia/Tokyo')::date \
         FROM orders o WHERE o.user_id = u.id AND o.status <> 'cancelled') >= $5)";

/// [`MATCH_WHERE`] in join form: the segment is the row
/// `s` of an outer query instead of bound parameters, so
/// every segment's count can resolve in one grouped query.
/// The two texts state the same predicate — keep them in
/// step.
const MATCH_ON: &str = "u.withdrawn_at IS NULL \
    AND (s.tag = '' OR EXISTS (SELECT 1 FROM customer_tags t \
         WHERE t.user_id = u.id AND t.tag = s.tag)) \
    AND (s.min_total_spend_yen IS NULL OR (SELECT COALESCE(SUM(o.total_yen), 0) \
         FROM orders o WHERE o.user_id = u.id AND o.status <> 'cancelled') >= s.min_total_spend_yen) \
    AND (s.min_order_count IS NULL OR (SELECT COUNT(*) FROM orders o \
         WHERE o.user_id = u.id AND o.status <> 'cancelled') >= s.min_order_count) \
    AND (s.last_order_before IS NULL OR (SELECT (MAX(o.created_at) AT TIME ZONE 'Asia/Tokyo')::date \
         FROM orders o WHERE o.user_id = u.id AND o.status <> 'cancelled') <= s.last_order_before) \
    AND (s.last_order_after IS NULL OR (SELECT (MAX(o.created_at) AT TIME ZONE 'Asia/Tokyo')::date \
         FROM orders o WHERE o.user_id = u.id AND o.status <> 'cancelled') >= s.last_order_after)";

/// Binds a segment's conditions onto a query in
/// [`MATCH_WHERE`] order.
macro_rules! bind_segment {
    ($q:expr, $s:expr) => {
        $q.bind($s.tag.clone())
            .bind($s.min_total_spend_yen)
            .bind($s.min_order_count)
            .bind($s.last_order_before)
            .bind($s.last_order_after)
    };
}

/// How many members currently match.
pub async fn count_matching(pool: &PgPool, segment: &CustomerSegment) -> Result<i64, AppError> {
    let sql = format!("SELECT COUNT(*) FROM users u WHERE {MATCH_WHERE}");
    bind_segment!(sqlx::query_scalar::<_, i64>(&sql), segment)
        .fetch_one(pool)
        .await
        .map_err(|e| AppError::db("count segment members", e))
}

/// One row of the bulk count read: a segment plus its
/// live member count.
#[derive(sqlx::FromRow)]
struct SegmentCountRow {
    /// The segment definition.
    #[sqlx(flatten)]
    segment: CustomerSegment,
    /// How many members currently match it.
    member_count: i64,
}

/// Every segment with its live member count, newest
/// first, in ONE grouped query — the list screen's read.
/// The per-segment [`count_matching`] loop this replaces
/// cost one round trip per saved segment.
pub async fn list_with_counts(pool: &PgPool) -> Result<Vec<(CustomerSegment, i64)>, AppError> {
    let sql = format!(
        "SELECT s.id, s.name, s.tag, s.min_total_spend_yen, s.min_order_count, \
                s.last_order_before, s.last_order_after, COUNT(u.id) AS member_count \
         FROM customer_segments s \
         LEFT JOIN users u ON {MATCH_ON} \
         GROUP BY s.id ORDER BY s.id DESC"
    );
    let rows = sqlx::query_as::<_, SegmentCountRow>(&sql)
        .fetch_all(pool)
        .await
        .map_err(|e| AppError::db("list segments with counts", e))?;
    Ok(rows
        .into_iter()
        .map(|row| (row.segment, row.member_count))
        .collect())
}

/// The member ids currently matching — the campaign
/// audience source.
pub async fn matching_member_ids(
    pool: &PgPool,
    segment: &CustomerSegment,
) -> Result<Vec<i64>, AppError> {
    let sql = format!("SELECT u.id FROM users u WHERE {MATCH_WHERE} ORDER BY u.id");
    bind_segment!(sqlx::query_scalar::<_, i64>(&sql), segment)
        .fetch_all(pool)
        .await
        .map_err(|e| AppError::db("list segment members", e))
}
