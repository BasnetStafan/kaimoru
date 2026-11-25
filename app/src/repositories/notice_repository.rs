//! Notice repository: the top page news section and the admin
//! notices screen.

use chrono::NaiveDate;
use sqlx::PgPool;

use crate::error::AppError;
use crate::models::{AdminNotice, NoticeCandidate};

/// Returns every visible notice with its display window,
/// newest first, in the order the top page shows them.
///
/// The window itself is deliberately NOT evaluated here: the
/// service caches these rows and applies the window against
/// the current JST date on every request
/// ([`NoticeCandidate::is_live_on`]), so a notice appears and
/// expires on schedule even while cached. The displayed date
/// is the period start, falling back to the creation date.
pub async fn find_visible_candidates(pool: &PgPool) -> Result<Vec<NoticeCandidate>, sqlx::Error> {
    sqlx::query_as::<_, NoticeCandidate>(
        "SELECT to_char(COALESCE(display_from, (created_at AT TIME ZONE 'Asia/Tokyo')::date), \
                'YYYY/MM/DD') AS date_text, title, body, display_from, display_to \
         FROM notices \
         WHERE is_visible \
         ORDER BY COALESCE(display_from, (created_at AT TIME ZONE 'Asia/Tokyo')::date) DESC, \
                  id DESC",
    )
    .fetch_all(pool)
    .await
}

// ---- Admin side ----

/// The admin list's column set, bound once so the page query
/// and its fallback count can never disagree about the shape.
const ADMIN_COLUMNS: &str = "id, title, body, display_from, display_to, is_visible, updated_at";

/// One admin notice row plus the filtered total, carried by
/// the window function so a page costs one query.
#[derive(sqlx::FromRow)]
struct AdminNoticeWithTotal {
    /// Primary key.
    id: i64,
    /// Headline; empty only on rows predating the title column.
    title: String,
    /// Announcement text.
    body: String,
    /// Display period start; `None` shows immediately.
    display_from: Option<NaiveDate>,
    /// Display period end; `None` never expires.
    display_to: Option<NaiveDate>,
    /// The visibility flag.
    is_visible: bool,
    /// Last edit instant.
    updated_at: chrono::DateTime<chrono::Utc>,
    /// Filtered row total (window function).
    total: i64,
}

/// Splits the window-function total off admin notice rows.
fn with_total(rows: Vec<AdminNoticeWithTotal>) -> (Vec<AdminNotice>, i64) {
    let total = rows.first().map(|r| r.total).unwrap_or(0);
    (
        rows.into_iter()
            .map(|r| AdminNotice {
                id: r.id,
                title: r.title,
                body: r.body,
                display_from: r.display_from,
                display_to: r.display_to,
                is_visible: r.is_visible,
                updated_at: r.updated_at,
            })
            .collect(),
        total,
    )
}

/// Returns one page of the admin list, newest edit first, with
/// the filtered total.
///
/// `query` partial-matches the title; `status` is `visible`
/// （表示中） or `hidden` （非表示・期間外）, derived from the flag
/// and the period against `today` — never a stored column. The
/// predicate below and `notice_service::display_state` are
/// two spellings of one definition and must agree.
pub async fn list_admin(
    pool: &PgPool,
    pattern: Option<&str>,
    status: Option<&str>,
    today: NaiveDate,
    limit: i64,
    offset: i64,
) -> Result<(Vec<AdminNotice>, i64), AppError> {
    let rows = sqlx::query_as::<_, AdminNoticeWithTotal>(&format!(
        "SELECT {ADMIN_COLUMNS}, COUNT(*) OVER () AS total \
         FROM notices \
         WHERE ($2::text IS NULL OR title ILIKE $2) \
           AND ($3::text IS NULL \
                OR ($3 = 'visible' AND is_visible \
                    AND (display_from IS NULL OR display_from <= $1) \
                    AND (display_to IS NULL OR display_to >= $1)) \
                OR ($3 = 'hidden' AND NOT (is_visible \
                    AND (display_from IS NULL OR display_from <= $1) \
                    AND (display_to IS NULL OR display_to >= $1)))) \
         ORDER BY updated_at DESC, id DESC \
         LIMIT $4 OFFSET $5"
    ))
    .bind(today)
    .bind(pattern)
    .bind(status)
    .bind(limit)
    .bind(offset)
    .fetch_all(pool)
    .await
    .map_err(|e| AppError::db("list notices (admin)", e))?;
    // A page past the end returns no rows to carry the total;
    // only then is a separate count worth its round trip.
    if rows.is_empty() && offset > 0 {
        let total = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM notices \
             WHERE ($2::text IS NULL OR title ILIKE $2) \
               AND ($3::text IS NULL \
                    OR ($3 = 'visible' AND is_visible \
                        AND (display_from IS NULL OR display_from <= $1) \
                        AND (display_to IS NULL OR display_to >= $1)) \
                    OR ($3 = 'hidden' AND NOT (is_visible \
                        AND (display_from IS NULL OR display_from <= $1) \
                        AND (display_to IS NULL OR display_to >= $1))))",
        )
        .bind(today)
        .bind(pattern)
        .bind(status)
        .fetch_one(pool)
        .await
        .map_err(|e| AppError::db("count notices (admin)", e))?;
        return Ok((Vec::new(), total));
    }
    Ok(with_total(rows))
}

/// One notice by ID, for the admin edit form.
pub async fn find_admin(pool: &PgPool, id: i64) -> Result<Option<AdminNotice>, AppError> {
    sqlx::query_as::<_, AdminNotice>(&format!(
        "SELECT {ADMIN_COLUMNS} FROM notices WHERE id = $1"
    ))
    .bind(id)
    .fetch_optional(pool)
    .await
    .map_err(|e| AppError::db("find notice (admin)", e))
}

/// Creates one notice inside the caller's transaction.
pub async fn create_admin(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    title: &str,
    body: &str,
    display_from: Option<NaiveDate>,
    display_to: Option<NaiveDate>,
    is_visible: bool,
) -> Result<i64, AppError> {
    sqlx::query_scalar::<_, i64>(
        "INSERT INTO notices (title, body, display_from, display_to, is_visible) \
         VALUES ($1, $2, $3, $4, $5) RETURNING id",
    )
    .bind(title)
    .bind(body)
    .bind(display_from)
    .bind(display_to)
    .bind(is_visible)
    .fetch_one(&mut **tx)
    .await
    .map_err(|e| AppError::db("create notice", e))
}

/// The notice's current `updated_at`: the stamp an edit
/// form ticket is minted with at render time. (The notice
/// row struct does not carry the stamp, so it is read on
/// its own, like the tag's.)
pub async fn updated_at(
    pool: &PgPool,
    id: i64,
) -> Result<Option<chrono::DateTime<chrono::Utc>>, AppError> {
    let value = sqlx::query_scalar::<_, chrono::DateTime<chrono::Utc>>(
        "SELECT updated_at FROM notices WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?;
    Ok(value)
}

/// Returns the notice's current `updated_at` with the row
/// locked: the edit form's staleness guard compares its
/// form ticket's stamp against this value inside the
/// update transaction (the product guard's shape).
pub async fn updated_at_for_update(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    id: i64,
) -> Result<Option<chrono::DateTime<chrono::Utc>>, AppError> {
    let value = sqlx::query_scalar::<_, chrono::DateTime<chrono::Utc>>(
        "SELECT updated_at FROM notices WHERE id = $1 FOR UPDATE",
    )
    .bind(id)
    .fetch_optional(&mut **tx)
    .await?;
    Ok(value)
}

/// Updates one notice inside the caller's transaction, stamping
/// `updated_at` in the statement.
#[allow(clippy::too_many_arguments)]
pub async fn update_admin(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    id: i64,
    title: &str,
    body: &str,
    display_from: Option<NaiveDate>,
    display_to: Option<NaiveDate>,
    is_visible: bool,
    admin_id: i64,
) -> Result<(), AppError> {
    sqlx::query(
        "UPDATE notices SET title = $2, body = $3, display_from = $4, \
                display_to = $5, is_visible = $6, updated_at = now(), updated_by = $7 \
         WHERE id = $1",
    )
    .bind(id)
    .bind(title)
    .bind(body)
    .bind(display_from)
    .bind(display_to)
    .bind(is_visible)
    .bind(admin_id)
    .execute(&mut **tx)
    .await
    .map_err(|e| AppError::db("update notice", e))?;
    Ok(())
}

/// Deletes one notice inside the caller's transaction and returns
/// how many rows were removed, so the caller can audit only an
/// actual deletion (idempotent delete).
pub async fn delete_admin(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    id: i64,
) -> Result<u64, AppError> {
    let res = sqlx::query("DELETE FROM notices WHERE id = $1")
        .bind(id)
        .execute(&mut **tx)
        .await
        .map_err(|e| AppError::db("delete notice", e))?;
    Ok(res.rows_affected())
}

/// Content groups (same title AND body) held by more than one
/// notice, with their sizes — the admin list's duplicate
/// marker. Display only: rows are never merged or removed.
pub async fn duplicate_counts(pool: &PgPool) -> Result<Vec<(String, String, i64)>, AppError> {
    sqlx::query_as::<_, (String, String, i64)>(
        "SELECT title, body, COUNT(*) FROM notices \
         GROUP BY title, body HAVING COUNT(*) > 1",
    )
    .fetch_all(pool)
    .await
    .map_err(|e| AppError::db("notice duplicate counts", e))
}

/// How many OTHER notices are set to show the same title and
/// body (visible flag on) — the duplicate-on-save warning's
/// ground truth. Whether the twins are inside their display
/// windows today is the list's top-3 preview's business; this
/// count is about what the operator has armed to show.
pub async fn count_visible_same_content(
    pool: &PgPool,
    title: &str,
    body: &str,
    exclude_id: Option<i64>,
) -> Result<i64, AppError> {
    sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM notices \
         WHERE title = $1 AND body = $2 AND is_visible \
           AND ($3::bigint IS NULL OR id <> $3)",
    )
    .bind(title)
    .bind(body)
    .bind(exclude_id)
    .fetch_one(pool)
    .await
    .map_err(|e| AppError::db("count same-content notices", e))
}
