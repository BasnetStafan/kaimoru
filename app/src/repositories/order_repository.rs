//! Order repository.
//! Placement writes run inside the order service's transaction;
//! these functions take the transaction as their executor.

use sqlx::PgPool;

use crate::models::order::OrderLineDetail;
use crate::models::{Order, OrderLine};
use crate::pii::{decrypt_order, encrypt_field, encrypt_field_opt};
use crate::repositories::variant_repository::LABEL_SQL;

/// The fields of a new order, computed by the order service from
/// the cart and the delivery form. Nothing here comes from the
/// request unchecked: totals are recomputed server-side.
pub struct NewOrder {
    /// Ordering member, or `None` for a guest order.
    pub user_id: Option<i64>,
    /// Contact email for the confirmation mail.
    pub email: String,
    /// Delivery family name.
    pub family_name: String,
    /// Delivery given name.
    pub given_name: String,
    /// Delivery postal code.
    pub postal_code: String,
    /// Delivery prefecture.
    pub prefecture: String,
    /// Delivery city and ward.
    pub city: String,
    /// Delivery street address.
    pub address_line: String,
    /// Delivery building, when given.
    pub building: Option<String>,
    /// Delivery phone number.
    pub phone: String,
    /// Product subtotal in yen.
    pub subtotal_yen: i64,
    /// Coupon discount in yen.
    pub discount_yen: i64,
    /// Shipping fee in yen (net of any shipping coupon).
    pub shipping_yen: i64,
    /// The shipping fee a free-shipping coupon waived.
    pub shipping_discount_yen: i64,
    /// Cash-on-delivery fee in yen.
    pub cod_fee_yen: i64,
    /// Grand total in yen.
    pub total_yen: i64,
    /// Coupon code applied, when any.
    pub coupon_code: Option<String>,
    /// Requested delivery date; `None` for no preference.
    pub delivery_date: Option<chrono::NaiveDate>,
    /// Requested delivery slot code.
    pub delivery_slot: String,
    /// The slot label snapshotted at placement.
    pub delivery_slot_label: String,
    /// Which door the order entered through, stamped at
    /// placement (see [`crate::models::order::PlacedVia`]).
    pub placed_via: crate::models::order::PlacedVia,
    /// Gift noshi code (catalogue in [`crate::models::gift`]).
    pub noshi_code: String,
    /// The noshi label snapshotted at placement.
    pub noshi_label: String,
    /// The gift name inscription, when given (encrypted at
    /// insert like the order's other personal fields).
    pub noshi_name: Option<String>,
    /// The gift message-card text, when given (encrypted at
    /// insert like the order's other personal fields).
    pub gift_message: Option<String>,
    /// Points the member spends on this order (1 pt = ¥1).
    pub points_used_yen: i64,
}

/// Inserts the order row and returns its ID.
pub async fn insert_order<'e, E>(executor: E, order: &NewOrder) -> Result<i64, sqlx::Error>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    sqlx::query_scalar(
        "INSERT INTO orders (user_id, email, family_name, given_name, postal_code, \
         prefecture, city, address_line, building, phone, subtotal_yen, discount_yen, \
         shipping_yen, shipping_discount_yen, cod_fee_yen, total_yen, coupon_code, \
         delivery_date, delivery_slot, delivery_slot_label, placed_via, noshi_code, \
         noshi_label, noshi_name, gift_message, points_used_yen) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, \
         $17, $18, $19, $20, $21, $22, $23, $24, $25, $26) \
         RETURNING id",
    )
    .bind(order.user_id)
    .bind(encrypt_field(&order.email))
    .bind(encrypt_field(&order.family_name))
    .bind(encrypt_field(&order.given_name))
    .bind(encrypt_field(&order.postal_code))
    .bind(encrypt_field(&order.prefecture))
    .bind(encrypt_field(&order.city))
    .bind(encrypt_field(&order.address_line))
    .bind(encrypt_field_opt(order.building.as_deref()))
    .bind(encrypt_field(&order.phone))
    .bind(order.subtotal_yen)
    .bind(order.discount_yen)
    .bind(order.shipping_yen)
    .bind(order.shipping_discount_yen)
    .bind(order.cod_fee_yen)
    .bind(order.total_yen)
    .bind(&order.coupon_code)
    .bind(order.delivery_date)
    .bind(&order.delivery_slot)
    .bind(&order.delivery_slot_label)
    .bind(order.placed_via.as_str())
    .bind(&order.noshi_code)
    .bind(&order.noshi_label)
    .bind(encrypt_field_opt(order.noshi_name.as_deref()))
    .bind(encrypt_field_opt(order.gift_message.as_deref()))
    .bind(order.points_used_yen)
    .fetch_one(executor)
    .await
}

/// Records the points an order earned (the shipment hook
/// grants them and stamps the order in the same
/// transaction, so the member and admin order views read
/// the figure from the order itself).
/// One order line's points facts, for the shipment-time
/// grant (points override layer): the line amount in yen
/// and its snapshotted effective earn percent — `None`
/// on lines written before the snapshot existed.
pub async fn line_point_facts<'e, E>(
    executor: E,
    order_id: i64,
) -> Result<Vec<(i64, Option<i32>)>, sqlx::Error>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    sqlx::query_as::<_, (i64, Option<i32>)>(
        "SELECT unit_price_yen * quantity, points_earn_percent \
         FROM order_items WHERE order_id = $1 ORDER BY id",
    )
    .bind(order_id)
    .fetch_all(executor)
    .await
}

pub async fn set_points_earned<'e, E>(
    executor: E,
    order_id: i64,
    points: i64,
) -> Result<(), sqlx::Error>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    sqlx::query("UPDATE orders SET points_earned = $2 WHERE id = $1")
        .bind(order_id)
        .bind(points)
        .execute(executor)
        .await
        .map(|_| ())
}

/// One variant row locked for order placement: the authoritative
/// stock, read under `FOR UPDATE`, with the product's name, price
/// and status joined in for the snapshot and the sale checks.
#[derive(sqlx::FromRow)]
pub struct LockedVariant {
    /// Variant ID.
    pub variant_id: i64,
    /// Product the variant belongs to.
    pub product_id: i64,
    /// Current product name (frozen onto the order line).
    pub name: String,
    /// Current unit price in yen (frozen onto the order line):
    /// the variant's own price when set, else the product's
    /// effective (sale-aware) price.
    pub price_yen: i64,
    /// Current variant stock, against which the order validates.
    pub stock: i32,
    /// Publication status, so placement can refuse a line whose
    /// product was drafted or archived after it entered the cart.
    pub status: String,
    /// The product's tax class, snapshotted onto the order line.
    pub tax_class_id: i64,
    /// Whether the product tracks stock: untracked lines are
    /// never decremented.
    pub track_stock: bool,
    /// The product's points earn-rate override (points
    /// override layer): `None` earns at the shop rate.
    pub points_earn_percent: Option<i32>,
    /// Whether the product sells past zero stock.
    pub allow_backorder: bool,
    /// Whether the combination is enabled: a disabled combination
    /// is skipped like an off-sale line.
    pub is_enabled: bool,
    /// Whether the product's publication window is open now.
    pub publish_open: bool,
    /// The combination label built from its axes and options
    /// (frozen onto the order line); empty for the default variant.
    pub variant_label: String,
}

/// Locks every variant row of an order in one query, ordered by
/// ID: a fixed lock order keeps concurrent placements from
/// deadlocking, and one round trip replaces per-line lock queries.
pub async fn lock_variants(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    variant_ids: &[i64],
) -> Result<Vec<LockedVariant>, sqlx::Error> {
    sqlx::query_as::<_, LockedVariant>(&format!(
        "SELECT v.id AS variant_id, v.product_id, p.name, \
                COALESCE(v.price_yen, {EFFECTIVE}) AS price_yen, v.stock, \
                p.status, {LABEL_SQL} AS variant_label, p.tax_class_id, \
                p.track_stock, p.allow_backorder, p.points_earn_percent, v.is_enabled, \
                ((p.publish_starts_at IS NULL OR p.publish_starts_at <= now()) \
                  AND (p.publish_ends_at IS NULL OR p.publish_ends_at > now())) AS publish_open \
         FROM product_variants v \
         JOIN products p ON p.id = v.product_id \
         WHERE v.id = ANY($1) ORDER BY v.id FOR UPDATE OF v",
        EFFECTIVE = crate::repositories::EFFECTIVE_PRICE_SQL
    ))
    .bind(variant_ids)
    .fetch_all(&mut **tx)
    .await
}

/// One order line to insert, with its frozen snapshots.
#[derive(Clone)]
pub struct NewOrderLine {
    /// Product being ordered.
    pub product_id: i64,
    /// Variant (combination) being ordered.
    pub variant_id: i64,
    /// Product name as it was when ordered.
    pub product_name: String,
    /// Combination label as it was when ordered; empty for the
    /// default variant.
    pub variant_label: String,
    /// Unit price in yen as it was when ordered.
    pub unit_price_yen: i64,
    /// Quantity ordered.
    pub quantity: i32,
    /// Tax class code snapshot of the order date.
    pub tax_class_code: String,
    /// Tax rate percent snapshot of the order date.
    pub rate_percent: i32,
    /// Effective points earn percent snapshot of the order
    /// date (the product's override, else the shop rate).
    pub points_earn_percent: i32,
}

/// Inserts all of an order's lines in one multi-row statement.
///
/// Only the placeholder numbers are formatted into the SQL — they
/// are internal constants; every value is a bound parameter.
pub async fn insert_lines(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    order_id: i64,
    lines: &[NewOrderLine],
) -> Result<(), sqlx::Error> {
    if lines.is_empty() {
        return Ok(());
    }
    let mut sql = String::from(
        "INSERT INTO order_items (order_id, product_id, variant_id, product_name, variant_label, unit_price_yen, quantity, tax_class_code, rate_percent, points_earn_percent) VALUES ",
    );
    for i in 0..lines.len() {
        if i > 0 {
            sql.push_str(", ");
        }
        let base = i * 10;
        sql.push_str(&format!(
            "(${}, ${}, ${}, ${}, ${}, ${}, ${}, ${}, ${}, ${})",
            base + 1,
            base + 2,
            base + 3,
            base + 4,
            base + 5,
            base + 6,
            base + 7,
            base + 8,
            base + 9,
            base + 10
        ));
    }
    let mut query = sqlx::query(&sql);
    for line in lines {
        query = query
            .bind(order_id)
            .bind(line.product_id)
            .bind(line.variant_id)
            .bind(&line.product_name)
            .bind(&line.variant_label)
            .bind(line.unit_price_yen)
            .bind(line.quantity)
            .bind(&line.tax_class_code)
            .bind(line.rate_percent)
            .bind(line.points_earn_percent);
    }
    query.execute(&mut **tx).await.map(|_| ())
}

/// Decrements the stock of every ordered variant in one statement:
/// a VALUES list joined against the variants table. Callers lock
/// the rows first and have checked that stock suffices, so this
/// never goes negative. Only placeholder numbers are formatted
/// into the SQL; the values are bound parameters.
pub async fn decrement_stocks(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    lines: &[NewOrderLine],
) -> Result<(), sqlx::Error> {
    if lines.is_empty() {
        return Ok(());
    }
    let mut values = String::new();
    for i in 0..lines.len() {
        if i > 0 {
            values.push_str(", ");
        }
        let base = i * 2;
        values.push_str(&format!("(${}::bigint, ${}::int)", base + 1, base + 2));
    }
    let sql = format!(
        "UPDATE product_variants v SET stock = v.stock - x.qty, updated_at = now() \
         FROM (VALUES {values}) AS x(id, qty) WHERE v.id = x.id"
    );
    let mut query = sqlx::query(&sql);
    for line in lines {
        query = query.bind(line.variant_id).bind(line.quantity);
    }
    query.execute(&mut **tx).await.map(|_| ())
}

/// Every column of the orders table, named explicitly: `SELECT *`
/// would silently change shape the day a column is added.
pub(crate) const ORDER_COLUMNS: &str = "id, user_id, email, family_name, given_name, postal_code, \
    prefecture, city, address_line, building, phone, subtotal_yen, discount_yen, \
    shipping_yen, shipping_discount_yen, cod_fee_yen, total_yen, coupon_code, status, created_at, \
    delivery_date, delivery_slot, delivery_slot_label, carrier, tracking_number, \
    shipped_mail_sent_at, placed_via, noshi_code, noshi_label, noshi_name, gift_message, \
    points_used_yen, points_earned";

/// Locks one order row inside the caller's transaction and
/// returns it. Payment status application and the admin
/// status change take this lock before touching the payment
/// row — the store-wide lock order is order, then payment,
/// so the webhook, the sweep and the back office cannot
/// deadlock each other over the same money.
pub async fn lock_by_id(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    order_id: i64,
) -> Result<Option<Order>, sqlx::Error> {
    let mut order = sqlx::query_as::<_, Order>(&format!(
        "SELECT {ORDER_COLUMNS} FROM orders WHERE id = $1 FOR UPDATE"
    ))
    .bind(order_id)
    .fetch_optional(&mut **tx)
    .await?;
    if let Some(order) = &mut order {
        decrypt_order(order);
    }
    Ok(order)
}

/// Loads one order by ID.
pub async fn find_by_id(pool: &PgPool, order_id: i64) -> Result<Option<Order>, sqlx::Error> {
    let mut order =
        sqlx::query_as::<_, Order>(&format!("SELECT {ORDER_COLUMNS} FROM orders WHERE id = $1"))
            .bind(order_id)
            .fetch_optional(pool)
            .await?;
    if let Some(order) = &mut order {
        decrypt_order(order);
    }
    Ok(order)
}

/// Returns orders in any of `statuses`, newest first, capped
/// at `limit`: the label export's candidate list. PII fields
/// are decrypted exactly like a single-order load.
pub async fn find_by_statuses(
    pool: &PgPool,
    statuses: &[&str],
    limit: i64,
) -> Result<Vec<Order>, sqlx::Error> {
    let mut orders = sqlx::query_as::<_, Order>(&format!(
        "SELECT {ORDER_COLUMNS} FROM orders WHERE status = ANY($1) \
         ORDER BY created_at DESC, id DESC LIMIT $2"
    ))
    .bind(statuses)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    for order in &mut orders {
        decrypt_order(order);
    }
    Ok(orders)
}

/// Loads an order's lines in placement order.
/// Returns the orders for `ids` that exist, each decrypted
/// like [`find_by_id`]. Result order is unspecified: a
/// caller that presented a selection order re-imposes it.
pub async fn find_by_ids(pool: &PgPool, ids: &[i64]) -> Result<Vec<Order>, sqlx::Error> {
    let mut orders = sqlx::query_as::<_, Order>(&format!(
        "SELECT {ORDER_COLUMNS} FROM orders WHERE id = ANY($1)"
    ))
    .bind(ids)
    .fetch_all(pool)
    .await?;
    for order in &mut orders {
        decrypt_order(order);
    }
    Ok(orders)
}

pub async fn lines_of(pool: &PgPool, order_id: i64) -> Result<Vec<OrderLine>, sqlx::Error> {
    sqlx::query_as::<_, OrderLine>(
        "SELECT product_id, product_name, variant_label, unit_price_yen, quantity, tax_class_code, rate_percent \
         FROM order_items WHERE order_id = $1 ORDER BY id",
    )
    .bind(order_id)
    .fetch_all(pool)
    .await
}

/// One row of the bulk line read: the line plus the order
/// it belongs to.
#[derive(sqlx::FromRow)]
struct OrderLineRow {
    /// The order the line belongs to.
    order_id: i64,
    /// The line itself.
    #[sqlx(flatten)]
    line: OrderLine,
}

/// The lines of many orders in one query: (order id, line)
/// pairs ordered by order, then by the line order
/// [`lines_of`] presents within one order.
pub async fn lines_for_orders(
    pool: &PgPool,
    order_ids: &[i64],
) -> Result<Vec<(i64, OrderLine)>, sqlx::Error> {
    let rows = sqlx::query_as::<_, OrderLineRow>(
        "SELECT order_id, product_id, product_name, variant_label, unit_price_yen, quantity, tax_class_code, rate_percent \
         FROM order_items WHERE order_id = ANY($1) ORDER BY order_id, id",
    )
    .bind(order_ids)
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(|r| (r.order_id, r.line)).collect())
}

/// One order line as the reorder flow reads it: the frozen
/// snapshot plus the variant id recorded at placement
/// (`None` on orders placed before variants were recorded).
#[derive(sqlx::FromRow)]
pub struct ReorderRow {
    /// Product ID.
    pub product_id: i64,
    /// Variant ID recorded at placement.
    pub variant_id: Option<i64>,
    /// Product name as it was when ordered.
    pub product_name: String,
    /// Combination label as it was when ordered.
    pub variant_label: String,
    /// Quantity ordered.
    pub quantity: i32,
}

/// The lines of one order for the reorder flow: the snapshot
/// names for the skip report, the recorded variant ids for
/// the cart adds.
pub async fn reorder_rows(pool: &PgPool, order_id: i64) -> Result<Vec<ReorderRow>, sqlx::Error> {
    sqlx::query_as::<_, ReorderRow>(
        "SELECT product_id, variant_id, product_name, variant_label, quantity \
         FROM order_items WHERE order_id = $1 ORDER BY id",
    )
    .bind(order_id)
    .fetch_all(pool)
    .await
}

/// One row of the paged history query: the order columns plus the
/// window-counted total (a tuple of struct and count does not decode).
#[derive(sqlx::FromRow)]
struct OrderPageRow {
    id: i64,
    user_id: Option<i64>,
    email: String,
    family_name: String,
    given_name: String,
    postal_code: String,
    prefecture: String,
    city: String,
    address_line: String,
    building: Option<String>,
    phone: String,
    subtotal_yen: i64,
    discount_yen: i64,
    shipping_yen: i64,
    shipping_discount_yen: i64,
    cod_fee_yen: i64,
    total_yen: i64,
    points_used_yen: i64,
    points_earned: i64,
    coupon_code: Option<String>,
    status: String,
    placed_via: String,
    created_at: chrono::DateTime<chrono::Utc>,
    delivery_date: Option<chrono::NaiveDate>,
    delivery_slot: String,
    delivery_slot_label: String,
    carrier: Option<String>,
    tracking_number: Option<String>,
    shipped_mail_sent_at: Option<chrono::DateTime<chrono::Utc>>,
    noshi_code: String,
    noshi_label: String,
    noshi_name: Option<String>,
    gift_message: Option<String>,
    total_count: i64,
    /// The first line's snapshotted product name.
    first_product_name: String,
    /// How many lines the order holds.
    item_kinds: i64,
}

impl From<OrderPageRow> for Order {
    fn from(row: OrderPageRow) -> Self {
        let mut order = Self {
            id: row.id,
            user_id: row.user_id,
            email: row.email,
            family_name: row.family_name,
            given_name: row.given_name,
            postal_code: row.postal_code,
            prefecture: row.prefecture,
            city: row.city,
            address_line: row.address_line,
            building: row.building,
            phone: row.phone,
            subtotal_yen: row.subtotal_yen,
            discount_yen: row.discount_yen,
            shipping_yen: row.shipping_yen,
            shipping_discount_yen: row.shipping_discount_yen,
            cod_fee_yen: row.cod_fee_yen,
            total_yen: row.total_yen,
            points_used_yen: row.points_used_yen,
            points_earned: row.points_earned,
            coupon_code: row.coupon_code,
            status: row.status,
            placed_via: row.placed_via,
            created_at: row.created_at,
            delivery_date: row.delivery_date,
            delivery_slot: row.delivery_slot,
            delivery_slot_label: row.delivery_slot_label,
            carrier: row.carrier,
            tracking_number: row.tracking_number,
            shipped_mail_sent_at: row.shipped_mail_sent_at,
            noshi_code: row.noshi_code,
            noshi_label: row.noshi_label,
            noshi_name: row.noshi_name,
            gift_message: row.gift_message,
        };
        decrypt_order(&mut order);
        order
    }
}

/// Returns one page of a member's order history, newest first,
/// plus the member's total order count.
///
/// Ownership is the WHERE clause itself, as everywhere here. Page
/// rows and total come back in one query via `COUNT(*) OVER ()`;
/// a page past the end falls back to a bare count so the caller
/// can clamp to the last page.
pub async fn history_for(
    pool: &PgPool,
    user_id: i64,
    limit: i64,
    offset: i64,
) -> Result<(Vec<crate::models::RecentOrder>, i64), sqlx::Error> {
    let rows = sqlx::query_as::<_, OrderPageRow>(&format!(
        "SELECT {ORDER_COLUMNS}, COUNT(*) OVER () AS total_count, \
         COALESCE(first_item.product_name, '') AS first_product_name, \
         COALESCE(kinds.item_kinds, 0) AS item_kinds \
         FROM orders \
         LEFT JOIN LATERAL (\
             SELECT product_name FROM order_items \
             WHERE order_id = orders.id ORDER BY id LIMIT 1\
         ) first_item ON true \
         LEFT JOIN LATERAL (\
             SELECT COUNT(*) AS item_kinds FROM order_items WHERE order_id = orders.id\
         ) kinds ON true \
         WHERE user_id = $1 ORDER BY created_at DESC, id DESC LIMIT $2 OFFSET $3"
    ))
    .bind(user_id)
    .bind(limit)
    .bind(offset)
    .fetch_all(pool)
    .await?;
    if let Some(first) = rows.first() {
        let total = first.total_count;
        return Ok((
            rows.into_iter()
                .map(|row| crate::models::RecentOrder {
                    first_product_name: row.first_product_name.clone(),
                    item_kinds: row.item_kinds,
                    order: Order::from(row),
                })
                .collect(),
            total,
        ));
    }
    if offset == 0 {
        return Ok((Vec::new(), 0));
    }
    let total: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM orders WHERE user_id = $1")
        .bind(user_id)
        .fetch_one(pool)
        .await?;
    Ok((Vec::new(), total))
}

/// One row of the recent-orders query: the order's own columns
/// plus its first line's name and line count, decoded flat
/// because the model pair is built by hand below.
#[derive(sqlx::FromRow)]
struct RecentOrderRow {
    /// The order's columns, decoded through [`Order`]'s own row
    /// mapping.
    #[sqlx(flatten)]
    order: Order,
    /// The first line's snapshotted product name.
    first_product_name: String,
    /// How many lines the order holds.
    item_kinds: i64,
}

/// A member's most recent orders with their line summaries
/// (the mypage hub's recent-orders block): one query, the
/// first line and the kind count folded in by lateral joins.
pub async fn recent_for_user(
    pool: &PgPool,
    user_id: i64,
    limit: i64,
) -> Result<Vec<crate::models::RecentOrder>, sqlx::Error> {
    let rows = sqlx::query_as::<_, RecentOrderRow>(&format!(
        "SELECT {ORDER_COLUMNS}, \
         COALESCE(first_item.product_name, '') AS first_product_name, \
         COALESCE(kinds.item_kinds, 0) AS item_kinds \
         FROM orders \
         LEFT JOIN LATERAL (\
             SELECT product_name FROM order_items \
             WHERE order_id = orders.id ORDER BY id LIMIT 1\
         ) first_item ON true \
         LEFT JOIN LATERAL (\
             SELECT COUNT(*) AS item_kinds FROM order_items WHERE order_id = orders.id\
         ) kinds ON true \
         WHERE user_id = $1 ORDER BY created_at DESC, id DESC LIMIT $2"
    ))
    .bind(user_id)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    let mut rows = rows;
    for row in &mut rows {
        decrypt_order(&mut row.order);
    }
    Ok(rows
        .into_iter()
        .map(|row| crate::models::RecentOrder {
            order: row.order,
            first_product_name: row.first_product_name,
            item_kinds: row.item_kinds,
        })
        .collect())
}

/// Finds one order only when it belongs to the given member:
/// ownership is enforced in the query itself, so another member's
/// order and a nonexistent ID are indistinguishable to the caller.
pub async fn find_by_id_for_user(
    pool: &PgPool,
    order_id: i64,
    user_id: i64,
) -> Result<Option<Order>, sqlx::Error> {
    let mut order = sqlx::query_as::<_, Order>(&format!(
        "SELECT {ORDER_COLUMNS} FROM orders WHERE id = $1 AND user_id = $2"
    ))
    .bind(order_id)
    .bind(user_id)
    .fetch_optional(pool)
    .await?;
    if let Some(order) = &mut order {
        decrypt_order(order);
    }
    Ok(order)
}

/// The order's lines with each product's current thumbnail
/// joined in for display; the snapshot columns stay authoritative
/// for name and price.
pub async fn lines_with_images(
    pool: &PgPool,
    order_id: i64,
) -> Result<Vec<OrderLineDetail>, sqlx::Error> {
    sqlx::query_as::<_, OrderLineDetail>(
        "SELECT oi.product_id, oi.product_name, oi.variant_label, oi.unit_price_yen, oi.quantity, oi.tax_class_code, oi.rate_percent, \
                NULLIF(p.image_path, '') AS image_path \
         FROM order_items oi \
         LEFT JOIN products p ON p.id = oi.product_id \
         WHERE oi.order_id = $1 ORDER BY oi.id",
    )
    .bind(order_id)
    .fetch_all(pool)
    .await
}

/// Writes the shipping details captured with the ship
/// transition, inside the caller's transaction: carrier and
/// tracking number are inputs to the transition itself, never
/// written by the mail-send step that follows it.
pub async fn set_shipping_details(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    order_id: i64,
    carrier: Option<&str>,
    tracking_number: Option<&str>,
) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE orders SET carrier = $1, tracking_number = $2 WHERE id = $3")
        .bind(carrier)
        .bind(tracking_number)
        .bind(order_id)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

/// Stamps a successful shipping-mail send — only when no send
/// has been stamped yet. The conditional update is the claim
/// that keeps the automatic send and any later resend from
/// double-sending: whichever send stamps first wins, and a
/// send that finds the stamp already set has been superseded.
///
/// Returns whether this call set the stamp.
pub async fn stamp_shipped_mail_sent(pool: &PgPool, order_id: i64) -> Result<bool, sqlx::Error> {
    let result = sqlx::query(
        "UPDATE orders SET shipped_mail_sent_at = now() \
         WHERE id = $1 AND shipped_mail_sent_at IS NULL",
    )
    .bind(order_id)
    .execute(pool)
    .await?;
    Ok(result.rows_affected() > 0)
}

/// The orders blocking a member's withdrawal (Account
/// Deletion spec 3.2): not yet shipped, or carrying a
/// payment still pending or a refund in flight. Generic
/// over the executor so the withdrawal transaction can
/// re-check under its own locks.
pub async fn list_blocking_for_user<'e, E>(
    executor: E,
    user_id: i64,
) -> Result<Vec<Order>, sqlx::Error>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    sqlx::query_as::<_, Order>(&format!(
        "SELECT {ORDER_COLUMNS} FROM orders o WHERE o.user_id = $1 AND ( \
             o.status IN ('received', 'preparing') \
             OR EXISTS (SELECT 1 FROM payments p WHERE p.order_id = o.id \
                        AND p.status IN ('pending_payment', 'partially_refunded')) \
         ) ORDER BY o.id"
    ))
    .bind(user_id)
    .fetch_all(executor)
    .await
    .map(|mut orders| {
        for order in &mut orders {
            decrypt_order(order);
        }
        orders
    })
}

/// Scrubs the delivery identity off a member's retained
/// orders at withdrawal (Account Deletion spec 3.4):
/// amounts, lines, statuses and payment rows keep the
/// business record; the person is replaced by markers.
pub async fn scrub_delivery_for_user(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    user_id: i64,
    marker_email: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE orders SET family_name = '退会済み', given_name = '', \
         postal_code = '', prefecture = '', city = '', address_line = '', \
         building = NULL, phone = '', email = $2 \
         WHERE user_id = $1",
    )
    .bind(user_id)
    .bind(marker_email)
    .execute(&mut **tx)
    .await
    .map(|_| ())
}

/// Inserts an order's per-class tax breakdown rows inside the
/// placement transaction. The rows decompose the order's total;
/// they never feed back into it.
pub async fn insert_tax_breakdowns(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    order_id: i64,
    breakdown: &[crate::models::tax::TaxBreakdown],
) -> Result<(), sqlx::Error> {
    for row in breakdown {
        sqlx::query(
            "INSERT INTO order_tax_breakdowns (order_id, tax_class_code, tax_class_name, \
             rate_percent, taxable_base_yen, tax_yen) VALUES ($1, $2, $3, $4, $5, $6)",
        )
        .bind(order_id)
        .bind(&row.tax_class_code)
        .bind(&row.tax_class_name)
        .bind(row.rate_percent)
        .bind(row.taxable_base_yen)
        .bind(row.tax_yen)
        .execute(&mut **tx)
        .await?;
    }
    Ok(())
}

/// Returns an order's stored tax breakdown, in class order;
/// empty for orders placed before tax classes existed.
pub async fn tax_breakdowns(
    pool: &PgPool,
    order_id: i64,
) -> Result<Vec<crate::models::tax::OrderTaxBreakdown>, sqlx::Error> {
    let mut rows = sqlx::query_as::<_, crate::models::tax::OrderTaxBreakdown>(
        "SELECT tax_class_code, tax_class_name, rate_percent, taxable_base_yen, tax_yen \
         FROM order_tax_breakdowns WHERE order_id = $1",
    )
    .bind(order_id)
    .fetch_all(pool)
    .await?;
    rows.sort_by_key(|r| crate::models::tax::class_rank(&r.tax_class_code));
    Ok(rows)
}

/// The summed tax of an order from its stored breakdown.
pub async fn tax_total(pool: &PgPool, order_id: i64) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT COALESCE(SUM(tax_yen), 0) FROM order_tax_breakdowns WHERE order_id = $1",
    )
    .bind(order_id)
    .fetch_one(pool)
    .await
}
