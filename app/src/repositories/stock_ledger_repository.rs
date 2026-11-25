//! Stock ledger repository: the adjustment rows written with
//! every stock movement, and the admin history read over them.
//!
//! A row exists only when stock actually moved (tracked
//! products): placement decrements, cancellation and refund
//! restores, manual grid edits, and combinations created with
//! stock on hand.

use sqlx::PgPool;

use crate::error::AppError;

/// One stock adjustment row for the admin history view.
#[derive(Clone, sqlx::FromRow)]
pub struct StockAdjustment {
    /// Surrogate ID.
    pub id: i64,
    /// The combination whose stock moved.
    pub variant_id: i64,
    /// The combination's current label (axes/options); empty for
    /// the default variant.
    pub variant_label: String,
    /// Signed change: negative when stock left the shelf.
    pub delta: i32,
    /// Why: `order`, `cancel`, `refund`, `manual` or `initial`.
    pub reason: String,
    /// The order behind the movement, when it was an order's.
    pub order_id: Option<i64>,
    /// The admin behind a manual movement, when it was one.
    pub admin_id: Option<i64>,
    /// Free note: the CSV import tags its adjustments with
    /// the batch they came from; every other flow leaves it
    /// empty.
    pub note: String,
    /// When the movement happened.
    pub created_at: chrono::DateTime<chrono::Utc>,
}

impl StockAdjustment {
    /// The reason's Japanese label for the history view.
    pub fn reason_label(&self) -> &'static str {
        match self.reason.as_str() {
            "order" => "注文",
            "cancel" => "キャンセル戻し",
            "refund" => "返金戻し",
            "manual" => "手動調整",
            "initial" => "初期登録",
            _ => "その他",
        }
    }

    /// The movement time in JST display form.
    pub fn created_display(&self) -> String {
        crate::models::jst::jst_datetime_label(self.created_at)
    }
}

/// Writes one adjustment row inside the caller's transaction,
/// with the stock change it records.
pub async fn insert_adjustment(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    variant_id: i64,
    delta: i32,
    reason: &str,
    order_id: Option<i64>,
    admin_id: Option<i64>,
) -> Result<(), AppError> {
    if delta == 0 {
        return Ok(());
    }
    sqlx::query(
        "INSERT INTO stock_adjustments (variant_id, delta, reason, order_id, admin_id) \
         VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(variant_id)
    .bind(delta)
    .bind(reason)
    .bind(order_id)
    .bind(admin_id)
    .execute(&mut **tx)
    .await
    .map_err(|e| AppError::db("insert stock adjustment", e))?;
    Ok(())
}

/// One adjustment row for [`insert_adjustments`].
pub struct NewAdjustment<'a> {
    /// The combination whose stock moved.
    pub variant_id: i64,
    /// Signed change: negative when stock left the shelf.
    pub delta: i32,
    /// Why: `order`, `cancel`, `refund`, `manual` or `initial`.
    pub reason: &'a str,
    /// The order behind the movement, when it was an order's.
    pub order_id: Option<i64>,
    /// The admin behind a manual movement, when it was one.
    pub admin_id: Option<i64>,
    /// Free note stored on the row; `None` stores the
    /// column's empty default.
    pub note: Option<&'a str>,
}

/// Writes many adjustment rows in ONE insert inside the
/// caller's transaction. Zero deltas are skipped, exactly as
/// the single-row [`insert_adjustment`] skips them, so a
/// batched save records the same ledger as row-by-row writes.
pub async fn insert_adjustments(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    rows: &[NewAdjustment<'_>],
) -> Result<(), AppError> {
    let rows: Vec<&NewAdjustment<'_>> = rows.iter().filter(|r| r.delta != 0).collect();
    if rows.is_empty() {
        return Ok(());
    }
    let mut values = String::new();
    for i in 0..rows.len() {
        if i > 0 {
            values.push_str(", ");
        }
        let b = i * 6;
        values.push_str(&format!(
            "(${}::bigint, ${}::int, ${}::text, ${}::bigint, ${}::bigint, ${}::text)",
            b + 1,
            b + 2,
            b + 3,
            b + 4,
            b + 5,
            b + 6
        ));
    }
    let sql = format!(
        "INSERT INTO stock_adjustments (variant_id, delta, reason, order_id, admin_id, note) \
         VALUES {values}"
    );
    let mut query = sqlx::query(&sql);
    for row in rows {
        query = query
            .bind(row.variant_id)
            .bind(row.delta)
            .bind(row.reason)
            .bind(row.order_id)
            .bind(row.admin_id)
            .bind(row.note.unwrap_or(""));
    }
    query
        .execute(&mut **tx)
        .await
        .map_err(|e| AppError::db("insert stock adjustments", e))?;
    Ok(())
}

/// A product's adjustment history, newest first.
pub async fn list_for_product(
    pool: &PgPool,
    product_id: i64,
    limit: i64,
) -> Result<Vec<StockAdjustment>, AppError> {
    sqlx::query_as::<_, StockAdjustment>(&format!(
        "SELECT a.id, a.variant_id, {label} AS variant_label, a.delta, a.reason, \
         a.order_id, a.admin_id, a.note, a.created_at \
         FROM stock_adjustments a \
         JOIN product_variants v ON v.id = a.variant_id \
         WHERE v.product_id = $1 ORDER BY a.id DESC LIMIT $2",
        label = crate::repositories::variant_repository::LABEL_SQL
    ))
    .bind(product_id)
    .bind(limit)
    .fetch_all(pool)
    .await
    .map_err(|e| AppError::db("list stock adjustments", e))
}
