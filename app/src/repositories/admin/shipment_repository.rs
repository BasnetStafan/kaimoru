//! Split-shipment persistence: parcels of an order,
//! their line quantities, and the shipped-so-far sums the
//! split rules derive from. Every query is parameterised.

use sqlx::{PgConnection, PgPool};

use crate::models::order::OrderLine;

/// One parcel of an order.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct ShipmentRow {
    /// Shipment ID.
    pub id: i64,
    /// The order it belongs to.
    pub order_id: i64,
    /// Carrier code (may be empty while the parcel is open).
    pub carrier: String,
    /// Tracking number (empty while open).
    pub tracking_number: String,
    /// When the parcel shipped; `None` while open.
    pub shipped_at: Option<chrono::DateTime<chrono::Utc>>,
    /// When its notification mail was stamped sent.
    pub mail_sent_at: Option<chrono::DateTime<chrono::Utc>>,
    /// Creation timestamp.
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// One line of a parcel.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct ShipmentItemRow {
    /// The order line the quantity belongs to.
    pub order_item_id: i64,
    /// Product name snapshot.
    pub product_name: String,
    /// Variant label snapshot.
    pub variant_label: String,
    /// Quantity in this parcel.
    pub quantity: i32,
}

const SHIPMENT_COLUMNS: &str = "id, order_id, carrier, tracking_number, \
     shipped_at, mail_sent_at, created_at";

/// Every shipment of one order, oldest first.
pub async fn list_for_order(pool: &PgPool, order_id: i64) -> Result<Vec<ShipmentRow>, sqlx::Error> {
    sqlx::query_as::<_, ShipmentRow>(&format!(
        "SELECT {SHIPMENT_COLUMNS} FROM order_shipments \
         WHERE order_id = $1 ORDER BY id"
    ))
    .bind(order_id)
    .fetch_all(pool)
    .await
}

/// One shipment by ID.
pub async fn find(pool: &PgPool, shipment_id: i64) -> Result<Option<ShipmentRow>, sqlx::Error> {
    sqlx::query_as::<_, ShipmentRow>(&format!(
        "SELECT {SHIPMENT_COLUMNS} FROM order_shipments WHERE id = $1"
    ))
    .bind(shipment_id)
    .fetch_optional(pool)
    .await
}

/// The lines of one shipment.
pub async fn items_for_shipment(
    pool: &PgPool,
    shipment_id: i64,
) -> Result<Vec<ShipmentItemRow>, sqlx::Error> {
    sqlx::query_as::<_, ShipmentItemRow>(
        "SELECT si.order_item_id, oi.product_name, oi.variant_label, \
                si.quantity \
         FROM order_shipment_items si \
         JOIN order_items oi ON oi.id = si.order_item_id \
         WHERE si.shipment_id = $1 ORDER BY si.order_item_id",
    )
    .bind(shipment_id)
    .fetch_all(pool)
    .await
}

/// The order's lines as mail-ready details for one
/// shipment: quantities are the parcel's, not the order's.
pub async fn mail_lines_for_shipment(
    pool: &PgPool,
    shipment_id: i64,
) -> Result<Vec<OrderLine>, sqlx::Error> {
    sqlx::query_as::<_, OrderLine>(
        "SELECT oi.product_id, oi.product_name, oi.variant_label, \
                oi.unit_price_yen, si.quantity, oi.tax_class_code, \
                oi.rate_percent \
         FROM order_shipment_items si \
         JOIN order_items oi ON oi.id = si.order_item_id \
         WHERE si.shipment_id = $1 ORDER BY si.order_item_id",
    )
    .bind(shipment_id)
    .fetch_all(pool)
    .await
}

/// Inserts an open shipment in the caller's transaction.
pub async fn insert_shipment(conn: &mut PgConnection, order_id: i64) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar::<_, i64>("INSERT INTO order_shipments (order_id) VALUES ($1) RETURNING id")
        .bind(order_id)
        .fetch_one(&mut *conn)
        .await
}

/// Adds one line to a shipment in the caller's transaction.
pub async fn insert_item(
    conn: &mut PgConnection,
    shipment_id: i64,
    order_item_id: i64,
    quantity: i32,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO order_shipment_items \
         (shipment_id, order_item_id, quantity) VALUES ($1, $2, $3)",
    )
    .bind(shipment_id)
    .bind(order_item_id)
    .bind(quantity)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// The order's lines (id, ordered quantity) in a
/// transaction.
pub async fn order_items_in_tx(
    conn: &mut PgConnection,
    order_id: i64,
) -> Result<Vec<(i64, i32)>, sqlx::Error> {
    sqlx::query_as::<_, (i64, i32)>(
        "SELECT id, quantity FROM order_items WHERE order_id = $1 ORDER BY id",
    )
    .bind(order_id)
    .fetch_all(&mut *conn)
    .await
}

/// Quantities already assigned to shipments, per order line:
/// (order_item_id, assigned quantity).
pub async fn assigned_quantities_in_tx(
    conn: &mut PgConnection,
    order_id: i64,
) -> Result<Vec<(i64, i32)>, sqlx::Error> {
    sqlx::query_as::<_, (i64, i32)>(
        "SELECT si.order_item_id, SUM(si.quantity)::int \
         FROM order_shipment_items si \
         JOIN order_shipments s ON s.id = si.shipment_id \
         WHERE s.order_id = $1 GROUP BY si.order_item_id",
    )
    .bind(order_id)
    .fetch_all(&mut *conn)
    .await
}

/// One shipment by ID, inside the caller's transaction.
pub async fn find_in_tx(
    conn: &mut PgConnection,
    shipment_id: i64,
) -> Result<Option<ShipmentRow>, sqlx::Error> {
    sqlx::query_as::<_, ShipmentRow>(&format!(
        "SELECT {SHIPMENT_COLUMNS} FROM order_shipments WHERE id = $1"
    ))
    .bind(shipment_id)
    .fetch_optional(&mut *conn)
    .await
}

/// How many of the order's shipments are still open,
/// inside the caller's transaction (so a just-marked
/// shipment in the same transaction already counts as
/// shipped).
pub async fn open_count_in_tx(conn: &mut PgConnection, order_id: i64) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM order_shipments \
         WHERE order_id = $1 AND shipped_at IS NULL",
    )
    .bind(order_id)
    .fetch_one(&mut *conn)
    .await
}

/// The order's lines with names, pool-level (the detail
/// page's split form): (item id, product name, variant
/// label, ordered quantity).
pub async fn order_items_named(
    pool: &PgPool,
    order_id: i64,
) -> Result<Vec<(i64, String, String, i32)>, sqlx::Error> {
    sqlx::query_as::<_, (i64, String, String, i32)>(
        "SELECT id, product_name, variant_label, quantity \
         FROM order_items WHERE order_id = $1 ORDER BY id",
    )
    .bind(order_id)
    .fetch_all(pool)
    .await
}

/// Quantities already assigned to shipments, per order
/// line, pool-level.
pub async fn assigned_quantities(
    pool: &PgPool,
    order_id: i64,
) -> Result<Vec<(i64, i32)>, sqlx::Error> {
    sqlx::query_as::<_, (i64, i32)>(
        "SELECT si.order_item_id, SUM(si.quantity)::int \
         FROM order_shipment_items si \
         JOIN order_shipments s ON s.id = si.shipment_id \
         WHERE s.order_id = $1 GROUP BY si.order_item_id",
    )
    .bind(order_id)
    .fetch_all(pool)
    .await
}

/// Marks a shipment shipped with its carrier and tracking
/// number in the caller's transaction.
pub async fn mark_shipped(
    conn: &mut PgConnection,
    shipment_id: i64,
    carrier: &str,
    tracking_number: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE order_shipments SET carrier = $2, \
                tracking_number = $3, shipped_at = now() \
         WHERE id = $1 AND shipped_at IS NULL",
    )
    .bind(shipment_id)
    .bind(carrier)
    .bind(tracking_number)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// Whether the order has any shipment rows at all (the
/// single-shipment path applies only when it has none).
pub async fn order_has_shipments<'e, E>(executor: E, order_id: i64) -> Result<bool, sqlx::Error>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS (SELECT 1 FROM order_shipments WHERE order_id = $1)",
    )
    .bind(order_id)
    .fetch_one(executor)
    .await
}

/// The order's oldest still-open shipment, if any — the
/// CSV import's landing target.
pub async fn open_shipment_for_order(
    pool: &PgPool,
    order_id: i64,
) -> Result<Option<ShipmentRow>, sqlx::Error> {
    sqlx::query_as::<_, ShipmentRow>(&format!(
        "SELECT {SHIPMENT_COLUMNS} FROM order_shipments \
         WHERE order_id = $1 AND shipped_at IS NULL ORDER BY id LIMIT 1"
    ))
    .bind(order_id)
    .fetch_optional(pool)
    .await
}

/// Stamps a shipment's notification as sent; `false` when
/// a racing send stamped first (the claim that keeps one
/// mail per parcel).
pub async fn stamp_mail_sent(pool: &PgPool, shipment_id: i64) -> Result<bool, sqlx::Error> {
    let rows = sqlx::query(
        "UPDATE order_shipments SET mail_sent_at = now() \
         WHERE id = $1 AND mail_sent_at IS NULL",
    )
    .bind(shipment_id)
    .execute(pool)
    .await?;
    Ok(rows.rows_affected() == 1)
}
