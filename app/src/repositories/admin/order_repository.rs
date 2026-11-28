//! Admin-side order queries.
//!
//! The dashboard aggregates, the order list and the fulfilment
//! writes. Day boundaries are Japan time computed in SQL
//! (`Asia/Tokyo`), matching the spec and immune to the process
//! timezone. Status writes run inside the caller's transaction
//! together with the stock restore and the audit row.

use sqlx::PgPool;

use crate::error::AppError;
use crate::models::{AdminOrderListRow, AdminOrderSummary, SalesDay};
use crate::pii::decrypt_field;

/// Orders placed today (Japan time), cancelled included — the
/// dashboard's 本日の注文. Cancellation is a state, not an
/// un-placement.
pub async fn count_orders_today(pool: &PgPool) -> Result<i64, AppError> {
    // The JST date is cast to a naive timestamp before `AT TIME
    // ZONE`: applied to a bare date, PostgreSQL turns the date
    // into a timestamptz at midnight in the SESSION timezone and
    // shifts zones, landing the boundary at 09:00 UTC instead of
    // JST midnight. The cast makes it Tokyo wall time.
    sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM orders \
         WHERE created_at >= ((now() AT TIME ZONE 'Asia/Tokyo')::date)::timestamp \
                              AT TIME ZONE 'Asia/Tokyo'",
    )
    .fetch_one(pool)
    .await
    .map_err(|e| AppError::db("count orders today", e))
}

/// Sales total for today (Japan time), cancelled orders
/// excluded — the dashboard's 本日の売上.
pub async fn sales_today(pool: &PgPool) -> Result<i64, AppError> {
    // `SUM` over a bigint column returns numeric in PostgreSQL and
    // sqlx will not decode numeric into i64, so the sum is cast
    // back to bigint in SQL — money stays integer yen end to end.
    // The day boundary casts exactly as in [`count_orders_today`].
    sqlx::query_scalar::<_, i64>(
        "SELECT COALESCE(SUM(total_yen)::bigint, 0) FROM orders \
         WHERE status <> 'cancelled' \
           AND created_at >= ((now() AT TIME ZONE 'Asia/Tokyo')::date)::timestamp \
                              AT TIME ZONE 'Asia/Tokyo'",
    )
    .fetch_one(pool)
    .await
    .map_err(|e| AppError::db("sales today", e))
}

/// Orders waiting to ship （受付済み + 準備中） — the dashboard's
/// 未発送 card and the order list's `unshipped` filter.
pub async fn count_unshipped(pool: &PgPool) -> Result<i64, AppError> {
    sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM orders WHERE status IN ('received', 'preparing')",
    )
    .fetch_one(pool)
    .await
    .map_err(|e| AppError::db("count unshipped orders", e))
}

/// Composes the displayed member name from its stored
/// parts, decrypting each: SQL string concatenation over
/// the ciphertext columns would render garbage (see
/// [`crate::pii`]). `None` when the order has no member.
fn compose_member_name(family: Option<String>, given: Option<String>) -> Option<String> {
    match (family, given) {
        (Some(family), Some(given)) => Some(format!(
            "{} {}",
            decrypt_field(&family),
            decrypt_field(&given)
        )),
        _ => None,
    }
}

/// One row of the dashboard's latest-orders query: the member
/// name arrives as its two stored parts (ciphertext) and is
/// composed after decryption.
#[derive(sqlx::FromRow)]
struct LatestOrderRow {
    id: i64,
    member_family_name: Option<String>,
    member_given_name: Option<String>,
    total_yen: i64,
    status: String,
    created_at: chrono::DateTime<chrono::Utc>,
}

/// The newest orders, for the dashboard's latest-10 table.
pub async fn latest_orders(pool: &PgPool, limit: i64) -> Result<Vec<AdminOrderSummary>, AppError> {
    let rows = sqlx::query_as::<_, LatestOrderRow>(
        "SELECT o.id, u.family_name AS member_family_name, \
                u.given_name AS member_given_name, \
                o.total_yen, o.status, o.created_at \
         FROM orders o LEFT JOIN users u ON u.id = o.user_id \
         ORDER BY o.created_at DESC, o.id DESC LIMIT $1",
    )
    .bind(limit)
    .fetch_all(pool)
    .await
    .map_err(|e| AppError::db("latest orders", e))?;
    Ok(rows
        .into_iter()
        .map(|r| AdminOrderSummary {
            id: r.id,
            member_name: compose_member_name(r.member_family_name, r.member_given_name),
            total_yen: r.total_yen,
            status: r.status,
            created_at: r.created_at,
        })
        .collect())
}

/// Per-day sales for the last `days` Japan-calendar days,
/// oldest first, cancelled orders excluded. Days with no sales
/// are generated so the chart shows zero-height bars.
pub async fn sales_last_days(pool: &PgPool, days: i64) -> Result<Vec<SalesDay>, AppError> {
    // A date subtracts an integer but not a bigint, so the bound
    // day count is narrowed to integer; the `SUM` casts back to
    // bigint as in [`sales_today`].
    //
    // The aggregate runs over the requested window only: the
    // half-open JST range predicate on created_at is what the
    // orders_created_idx index serves, so the planner scans the
    // window's orders instead of the whole table (the previous
    // form joined every order on its JST date expression, which
    // no index can serve). The generated days are joined after,
    // so zero-sale days still render. Result-equivalent to the
    // previous form (verified by EXCEPT diff).
    sqlx::query_as::<_, SalesDay>(
        "WITH days AS ( \
            SELECT generate_series( \
                (now() AT TIME ZONE 'Asia/Tokyo')::date - ($1 - 1)::integer, \
                (now() AT TIME ZONE 'Asia/Tokyo')::date, interval '1 day')::date AS day \
         ), agg AS ( \
            SELECT (created_at AT TIME ZONE 'Asia/Tokyo')::date AS day, \
                   SUM(total_yen)::bigint AS total_yen \
            FROM orders \
            WHERE status <> 'cancelled' \
              AND created_at >= ((now() AT TIME ZONE 'Asia/Tokyo')::date - ($1 - 1)::integer)::timestamp AT TIME ZONE 'Asia/Tokyo' \
              AND created_at < ((now() AT TIME ZONE 'Asia/Tokyo')::date + 1)::timestamp AT TIME ZONE 'Asia/Tokyo' \
            GROUP BY 1 \
         ) \
         SELECT days.day AS day, COALESCE(agg.total_yen, 0)::bigint AS total_yen \
         FROM days LEFT JOIN agg ON agg.day = days.day \
         ORDER BY days.day",
    )
    .bind(days)
    .fetch_all(pool)
    .await
    .map_err(|e| AppError::db("sales last days", e))
}

/// Returns one page of the admin order list plus the total count.
///
/// `status` is one of the fixed filter values (`unshipped` =
/// received + preparing) or `None` for all; `number` partial-
/// matches the displayed order number, formatted exactly as
/// `Order::order_number` computes it. `payment` narrows by the
/// payment state: `awaiting` (KOMOJU still pending — the
/// 入金待ち chip), `paid`, `refunded` (incl. partial) or
/// `failed` (incl. expired). `placed_from` / `placed_to` bound
/// the order instant half-open, already converted to UTC by
/// the caller; `customer` partial-matches the member name or
/// the delivery recipient name. All filters compose, and the
/// paged query and the fallback count share one WHERE text so
/// they can never disagree. Each row carries the total item
/// quantity across its lines （商品点数） and the order's
/// payment provider/status for the unified label.
///
/// The customer-name match is the one filter NOT in the SQL:
/// the name columns hold ciphertext (see [`crate::pii`]), so
/// with a customer term the remaining filters run in SQL, the
/// rows are decrypted, and the name match + paging run here.
#[allow(clippy::too_many_arguments)]
pub async fn list_orders(
    pool: &PgPool,
    status: Option<&str>,
    number: Option<String>,
    payment: Option<&str>,
    placed_from: Option<chrono::DateTime<chrono::Utc>>,
    placed_to: Option<chrono::DateTime<chrono::Utc>>,
    customer: Option<String>,
    limit: i64,
    offset: i64,
) -> Result<(Vec<AdminOrderListRow>, i64), AppError> {
    // Shared by the page query (binds $1..=$7) and the count
    // query (binds $1..=$5): status, number, payment state,
    // placed range.
    const WHERE: &str = "($1::text IS NULL \
            OR ($1 = 'unshipped' AND o.status IN ('received', 'preparing')) \
            OR o.status = $1) \
      AND ($2::text IS NULL OR \
           ('AS-' || to_char(o.created_at AT TIME ZONE 'Asia/Tokyo', 'YYYYMMDD') \
            || '-' || LPAD(o.id::text, 4, '0')) ILIKE $2) \
      AND ($3::text IS NULL \
           OR ($3 = 'awaiting' AND p.provider = 'komoju' AND p.status = 'pending_payment') \
           OR ($3 = 'paid' AND p.status = 'paid') \
           OR ($3 = 'refunded' AND p.status IN ('refunded', 'partially_refunded')) \
           OR ($3 = 'failed' AND p.status IN ('failed', 'expired'))) \
      AND ($4::timestamptz IS NULL OR o.created_at >= $4) \
      AND ($5::timestamptz IS NULL OR o.created_at < $5)";
    const SELECT: &str = "SELECT o.id, o.user_id, \
                u.family_name AS member_family_name, \
                u.given_name AS member_given_name, \
                o.family_name AS recipient_family_name, \
                o.given_name AS recipient_given_name, \
                o.total_yen, o.status, o.created_at, o.placed_via, \
                COALESCE((SELECT SUM(oi.quantity) FROM order_items oi \
                          WHERE oi.order_id = o.id), 0)::bigint AS item_count, \
                p.provider AS payment_provider, p.status AS payment_status, \
                COUNT(*) OVER () AS total \
         FROM orders o LEFT JOIN users u ON u.id = o.user_id \
         LEFT JOIN payments p ON p.order_id = o.id";
    let searching = customer.is_some();
    let page_sql = if searching {
        format!("{SELECT} WHERE {WHERE} ORDER BY o.created_at DESC, o.id DESC")
    } else {
        format!("{SELECT} WHERE {WHERE} ORDER BY o.created_at DESC, o.id DESC LIMIT $6 OFFSET $7")
    };
    let mut fetch = sqlx::query_as::<_, AdminOrderListRowWithTotal>(&page_sql)
        .bind(status)
        .bind(&number)
        .bind(payment)
        .bind(placed_from)
        .bind(placed_to);
    if !searching {
        fetch = fetch.bind(limit).bind(offset);
    }
    let rows = fetch
        .fetch_all(pool)
        .await
        .map_err(|e| AppError::db("list admin orders", e))?;
    if let Some(term) = &customer {
        let needle = term.to_lowercase();
        let matched: Vec<AdminOrderListRow> = rows
            .into_iter()
            .filter_map(|r| {
                let member_name =
                    compose_member_name(r.member_family_name.clone(), r.member_given_name.clone());
                let recipient = format!(
                    "{} {}",
                    decrypt_field(&r.recipient_family_name),
                    decrypt_field(&r.recipient_given_name)
                );
                let hit = member_name
                    .as_deref()
                    .is_some_and(|n| n.to_lowercase().contains(&needle))
                    || recipient.to_lowercase().contains(&needle);
                hit.then(|| to_list_row(r, member_name))
            })
            .collect();
        let total = matched.len() as i64;
        let page = matched
            .into_iter()
            .skip(offset.max(0) as usize)
            .take(limit.max(0) as usize)
            .collect();
        return Ok((page, total));
    }
    // A page past the end returns no rows to carry the total;
    // only then is a separate count worth its round trip.
    if rows.is_empty() && offset > 0 {
        let count_sql = format!(
            "SELECT COUNT(*) FROM orders o \
             LEFT JOIN users u ON u.id = o.user_id \
             LEFT JOIN payments p ON p.order_id = o.id \
             WHERE {WHERE}"
        );
        let total = sqlx::query_scalar::<_, i64>(&count_sql)
            .bind(status)
            .bind(&number)
            .bind(payment)
            .bind(placed_from)
            .bind(placed_to)
            .fetch_one(pool)
            .await
            .map_err(|e| AppError::db("count admin orders", e))?;
        return Ok((Vec::new(), total));
    }
    let total = rows.first().map(|r| r.total).unwrap_or(0);
    Ok((
        rows.into_iter()
            .map(|r| {
                let member_name =
                    compose_member_name(r.member_family_name.clone(), r.member_given_name.clone());
                to_list_row(r, member_name)
            })
            .collect(),
        total,
    ))
}

/// Converts one decrypted list row to the screen model.
fn to_list_row(r: AdminOrderListRowWithTotal, member_name: Option<String>) -> AdminOrderListRow {
    AdminOrderListRow {
        id: r.id,
        user_id: r.user_id,
        member_name,
        total_yen: r.total_yen,
        status: r.status,
        created_at: r.created_at,
        placed_via: r.placed_via,
        item_count: r.item_count,
        payment_provider: r.payment_provider,
        payment_status: r.payment_status,
    }
}

/// Returns how many orders sit in each fulfilment status, over
/// all orders — the order list's filter-chip counts. The counts
/// describe the whole queue: they deliberately ignore the current
/// search term and active chip, like Shopify's view tabs. One
/// GROUP BY round trip; the caller derives the `unshipped`
/// shortcut from the per-status counts.
pub async fn status_counts(pool: &PgPool) -> Result<Vec<(String, i64)>, AppError> {
    sqlx::query_as::<_, (String, i64)>("SELECT status, COUNT(*) FROM orders GROUP BY status")
        .fetch_all(pool)
        .await
        .map_err(|e| AppError::db("admin order status counts", e))
}

/// [`AdminOrderListRow`] plus the pagination total.
#[derive(sqlx::FromRow)]
struct AdminOrderListRowWithTotal {
    id: i64,
    user_id: Option<i64>,
    member_family_name: Option<String>,
    member_given_name: Option<String>,
    recipient_family_name: String,
    recipient_given_name: String,
    total_yen: i64,
    status: String,
    created_at: chrono::DateTime<chrono::Utc>,
    placed_via: String,
    item_count: i64,
    payment_provider: Option<String>,
    payment_status: Option<String>,
    total: i64,
}

/// The current status of one order, read inside the caller's
/// transaction with the row locked: two admins working the same
/// order cannot both pass the legality check on a stale state.
pub async fn current_status(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    order_id: i64,
) -> Result<Option<String>, AppError> {
    sqlx::query_scalar::<_, String>("SELECT status FROM orders WHERE id = $1 FOR UPDATE")
        .bind(order_id)
        .fetch_optional(&mut **tx)
        .await
        .map_err(|e| AppError::db("current order status", e))
}

/// Moves one order to a new fulfilment status inside the
/// caller's transaction. The service has already verified the
/// transition against [`legal_next_statuses`]; this is the
/// write only. The transition into `delivered` also stamps
/// `delivered_at`: this is the one
/// write point every status path funnels through, so the
/// stamp cannot be missed by a caller.
///
/// [`legal_next_statuses`]: crate::models::order::legal_next_statuses
pub async fn set_order_status(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    order_id: i64,
    status: &str,
) -> Result<(), AppError> {
    sqlx::query(
        "UPDATE orders SET status = $2, \
         delivered_at = CASE WHEN $2 = 'delivered' THEN now() ELSE delivered_at END \
         WHERE id = $1",
    )
    .bind(order_id)
    .bind(status)
    .execute(&mut **tx)
    .await
    .map_err(|e| AppError::db("set order status", e))?;
    Ok(())
}

/// The (variant, product, quantity) lines of one order in
/// variant-ID order — the restore-stock input on cancellation,
/// locked in the same ID order the checkout path uses. Quantity
/// decodes as i32: the column is INTEGER, and sqlx will not
/// widen it. The variant is `None` only on lines placed before
/// variants existed.
pub async fn order_line_quantities(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    order_id: i64,
) -> Result<Vec<(Option<i64>, i64, i32)>, AppError> {
    sqlx::query_as::<_, (Option<i64>, i64, i32)>(
        "SELECT variant_id, product_id, quantity FROM order_items \
         WHERE order_id = $1 ORDER BY variant_id NULLS LAST, product_id \
         FOR UPDATE",
    )
    .bind(order_id)
    .fetch_all(&mut **tx)
    .await
    .map_err(|e| AppError::db("order line quantities", e))
}

/// Returns one line's quantity to its variant's stock inside
/// the caller's transaction (the cancellation restore). A line
/// placed before variants existed names no variant: its units
/// go back to the product's default variant when that row still
/// exists, and are dropped when the product has since gained
/// axes (there is no honest row to credit).
pub async fn restore_stock(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    variant_id: Option<i64>,
    product_id: i64,
    quantity: i64,
) -> Result<(), AppError> {
    match variant_id {
        Some(variant_id) => {
            sqlx::query(
                "UPDATE product_variants SET stock = stock + $2, updated_at = now() \
                 WHERE id = $1",
            )
            .bind(variant_id)
            .bind(quantity)
            .execute(&mut **tx)
            .await
        }
        None => {
            sqlx::query(
                "UPDATE product_variants SET stock = stock + $2, updated_at = now() \
                 WHERE product_id = $1 AND option_signature = ''",
            )
            .bind(product_id)
            .bind(quantity)
            .execute(&mut **tx)
            .await
        }
    }
    .map_err(|e| AppError::db("restore stock", e))?;
    Ok(())
}

/// Order count and sales total for one JST period
/// `[from, to)` (UTC bounds the caller computed from JST
/// dates), cancelled orders excluded from both — the
/// analytics page's headline pair.
pub async fn period_totals(
    pool: &PgPool,
    from: chrono::DateTime<chrono::Utc>,
    to: chrono::DateTime<chrono::Utc>,
) -> Result<(i64, i64), AppError> {
    sqlx::query_as::<_, (i64, i64)>(
        "SELECT count(*)::bigint, COALESCE(sum(total_yen), 0)::bigint \
         FROM orders \
         WHERE status <> 'cancelled' AND created_at >= $1 AND created_at < $2",
    )
    .bind(from)
    .bind(to)
    .fetch_one(pool)
    .await
    .map_err(|e| AppError::db("period totals", e))
}

/// One product's sales in a period, for the ranking.
pub struct ProductSalesRow {
    /// The product.
    pub product_id: i64,
    /// The name snapshot from the order lines.
    pub product_name: String,
    /// Units sold.
    pub quantity: i64,
    /// Gross line amount in yen (unit price × quantity).
    pub amount_yen: i64,
}

/// Best-selling products of a period by line amount,
/// cancelled orders excluded.
pub async fn product_ranking(
    pool: &PgPool,
    from: chrono::DateTime<chrono::Utc>,
    to: chrono::DateTime<chrono::Utc>,
    limit: i64,
) -> Result<Vec<ProductSalesRow>, AppError> {
    sqlx::query_as::<_, (i64, String, i64, i64)>(
        "SELECT oi.product_id, oi.product_name, \
                sum(oi.quantity)::bigint, \
                sum(oi.unit_price_yen * oi.quantity)::bigint \
         FROM order_items oi \
         JOIN orders o ON o.id = oi.order_id \
         WHERE o.status <> 'cancelled' AND o.created_at >= $1 AND o.created_at < $2 \
         GROUP BY oi.product_id, oi.product_name \
         ORDER BY sum(oi.unit_price_yen * oi.quantity) DESC, oi.product_id \
         LIMIT $3",
    )
    .bind(from)
    .bind(to)
    .bind(limit)
    .fetch_all(pool)
    .await
    .map(|rows| {
        rows.into_iter()
            .map(
                |(product_id, product_name, quantity, amount_yen)| ProductSalesRow {
                    product_id,
                    product_name,
                    quantity,
                    amount_yen,
                },
            )
            .collect()
    })
    .map_err(|e| AppError::db("product ranking", e))
}

/// One category's sales in a period, for the ranking. The
/// category is the product's current one (the name snapshot
/// kept in sync on the product row); a product whose
/// category was since deleted groups under its last name,
/// and a product with none under （未分類）.
pub struct CategorySalesRow {
    /// Category display name.
    pub category: String,
    /// Units sold.
    pub quantity: i64,
    /// Gross line amount in yen.
    pub amount_yen: i64,
}

/// Best-selling categories of a period by line amount,
/// cancelled orders excluded.
pub async fn category_ranking(
    pool: &PgPool,
    from: chrono::DateTime<chrono::Utc>,
    to: chrono::DateTime<chrono::Utc>,
    limit: i64,
) -> Result<Vec<CategorySalesRow>, AppError> {
    sqlx::query_as::<_, (String, i64, i64)>(
        "SELECT COALESCE(NULLIF(p.category, ''), '（未分類）'), \
                sum(oi.quantity)::bigint, \
                sum(oi.unit_price_yen * oi.quantity)::bigint \
         FROM order_items oi \
         JOIN orders o ON o.id = oi.order_id \
         JOIN products p ON p.id = oi.product_id \
         WHERE o.status <> 'cancelled' AND o.created_at >= $1 AND o.created_at < $2 \
         GROUP BY COALESCE(NULLIF(p.category, ''), '（未分類）') \
         ORDER BY sum(oi.unit_price_yen * oi.quantity) DESC \
         LIMIT $3",
    )
    .bind(from)
    .bind(to)
    .bind(limit)
    .fetch_all(pool)
    .await
    .map(|rows| {
        rows.into_iter()
            .map(|(category, quantity, amount_yen)| CategorySalesRow {
                category,
                quantity,
                amount_yen,
            })
            .collect()
    })
    .map_err(|e| AppError::db("category ranking", e))
}

/// Member buyer counts of a period: how many members ordered
/// at least once, and how many of them ordered at least
/// twice — the repeat-rate pair. Guests cannot be counted
/// honestly (their identity is a typed address, not an
/// account), so the rate is members-only by construction.
pub async fn member_buyer_counts(
    pool: &PgPool,
    from: chrono::DateTime<chrono::Utc>,
    to: chrono::DateTime<chrono::Utc>,
) -> Result<(i64, i64), AppError> {
    sqlx::query_as::<_, (i64, i64)>(
        "SELECT count(*)::bigint, \
                count(*) FILTER (WHERE c >= 2)::bigint \
         FROM ( \
             SELECT user_id, count(*) AS c FROM orders \
             WHERE user_id IS NOT NULL AND status <> 'cancelled' \
               AND created_at >= $1 AND created_at < $2 \
             GROUP BY user_id \
         ) t",
    )
    .bind(from)
    .bind(to)
    .fetch_one(pool)
    .await
    .map_err(|e| AppError::db("member buyer counts", e))
}
