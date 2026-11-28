//! Return requests: the admin list, creation, and the
//! status walk; plus the member-facing latest status.

use sqlx::PgPool;

use crate::error::AppError;

/// One return request with its order's display facts.
#[derive(sqlx::FromRow)]
pub struct ReturnRequestRow {
    /// Primary key.
    pub id: i64,
    /// The order being returned.
    pub order_id: i64,
    /// The order's creation time (order number input).
    pub order_created_at: chrono::DateTime<chrono::Utc>,
    /// The order's total, yen.
    pub order_total_yen: i64,
    /// The order's fulfilment status.
    pub order_status: String,
    /// The originating inquiry, when born from one.
    pub inquiry_id: Option<i64>,
    /// The customer's stated reason.
    pub reason: String,
    /// Workflow status code.
    pub status: String,
    /// When the request was created.
    pub created_at: chrono::DateTime<chrono::Utc>,
}

const RETURN_COLUMNS: &str = "r.id, r.order_id, o.created_at AS order_created_at, \
     o.total_yen AS order_total_yen, o.status AS order_status, r.inquiry_id, \
     r.reason, r.status, r.created_at";

/// Every return request, newest first.
pub async fn list_returns(pool: &PgPool) -> Result<Vec<ReturnRequestRow>, AppError> {
    sqlx::query_as::<_, ReturnRequestRow>(&format!(
        "SELECT {RETURN_COLUMNS} FROM return_requests r \
         JOIN orders o ON o.id = r.order_id ORDER BY r.id DESC"
    ))
    .fetch_all(pool)
    .await
    .map_err(|e| AppError::db("list returns", e))
}

/// One request by id.
pub async fn find_return(pool: &PgPool, id: i64) -> Result<Option<ReturnRequestRow>, AppError> {
    sqlx::query_as::<_, ReturnRequestRow>(&format!(
        "SELECT {RETURN_COLUMNS} FROM return_requests r \
         JOIN orders o ON o.id = r.order_id WHERE r.id = $1"
    ))
    .bind(id)
    .fetch_optional(pool)
    .await
    .map_err(|e| AppError::db("find return", e))
}

/// Whether an open (non-terminal) request already exists
/// for the inquiry — one inquiry opens at most one workflow.
pub async fn open_request_for_inquiry(
    pool: &PgPool,
    inquiry_id: i64,
) -> Result<Option<i64>, AppError> {
    sqlx::query_scalar(
        "SELECT id FROM return_requests \
         WHERE inquiry_id = $1 AND status NOT IN ('refunded', 'rejected')",
    )
    .bind(inquiry_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| AppError::db("open request for inquiry", e))
}

/// Inserts a request inside the caller's transaction.
pub async fn insert_return(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    order_id: i64,
    inquiry_id: Option<i64>,
    reason: &str,
) -> Result<i64, AppError> {
    sqlx::query_scalar(
        "INSERT INTO return_requests (order_id, inquiry_id, reason) \
         VALUES ($1, $2, $3) RETURNING id",
    )
    .bind(order_id)
    .bind(inquiry_id)
    .bind(reason)
    .fetch_one(&mut **tx)
    .await
    .map_err(|e| AppError::db("insert return", e))
}

/// Moves the status inside the caller's transaction.
pub async fn set_status(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    id: i64,
    status: &str,
    expected: &str,
) -> Result<bool, AppError> {
    let affected = sqlx::query(
        "UPDATE return_requests SET status = $2, updated_at = now() \
         WHERE id = $1 AND status = $3",
    )
    .bind(id)
    .bind(status)
    .bind(expected)
    .execute(&mut **tx)
    .await
    .map_err(|e| AppError::db("set return status", e))?;
    Ok(affected.rows_affected() > 0)
}

/// The newest request's status for one order — the member
/// order detail's line.
pub async fn latest_status_for_order(
    pool: &PgPool,
    order_id: i64,
) -> Result<Option<String>, AppError> {
    sqlx::query_scalar(
        "SELECT status FROM return_requests WHERE order_id = $1 ORDER BY id DESC LIMIT 1",
    )
    .bind(order_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| AppError::db("latest return status", e))
}
