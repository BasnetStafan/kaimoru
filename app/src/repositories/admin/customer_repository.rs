//! Admin-side customer queries.
//!
//! Privacy rules are enforced here, at the query: the SELECT lists
//! name only the columns the admin screens show — `password_hash`
//! is never selected. There is deliberately no way to suspend or
//! delete a member in V1.

use sqlx::PgPool;

use crate::error::AppError;
use crate::models::{AdminCustomer, AdminCustomerDetail, Order};
use crate::pii::{decrypt_customer_detail, decrypt_field, decrypt_order};
use crate::repositories::order_repository::ORDER_COLUMNS;

/// Returns one page of the customers list, newest
/// registrations first, each with the member's purchase
/// aggregates — order count, total spent and last order date
/// over non-cancelled orders, the same definition the detail
/// screen uses, so the two screens can never disagree — plus
/// the total. `query` is the raw search text, matched
/// partially and case-insensitively against the kanji name
/// (either part, or the two joined with or without the space
/// the pages render), the kana name the same way, and the
/// email, or `None` for the unfiltered list.
///
/// The match runs in Rust over decrypted values: those
/// columns hold ciphertext (see [`crate::pii`]), so the SQL
/// ILIKE this list used before encryption cannot see the
/// text it matched. With a query, the member set is read
/// whole and filtered here — an admin-scale scan, with a
/// dedicated search index as the future path if the member
/// base ever outgrows it.
pub async fn list_customers(
    pool: &PgPool,
    query: Option<String>,
    limit: i64,
    offset: i64,
) -> Result<(Vec<AdminCustomer>, i64), AppError> {
    const SELECT: &str = "SELECT u.id, u.family_name, u.given_name, \
                u.family_name_kana, u.given_name_kana, u.email, u.created_at, \
                u.withdrawn_at, \
                COUNT(o.id) FILTER (WHERE o.status <> 'cancelled') AS order_count, \
                COALESCE(SUM(o.total_yen) FILTER (WHERE o.status <> 'cancelled'), 0)::bigint \
                    AS total_spent_yen, \
                MAX(o.created_at) FILTER (WHERE o.status <> 'cancelled') AS last_order_at, \
                COUNT(*) OVER () AS total \
         FROM users u LEFT JOIN orders o ON o.user_id = u.id \
         GROUP BY u.id \
         ORDER BY u.created_at DESC, u.id DESC";
    let searching = query.is_some();
    let sql = if searching {
        SELECT.to_string()
    } else {
        format!("{SELECT} LIMIT $1 OFFSET $2")
    };
    let mut fetch = sqlx::query_as::<_, AdminCustomerWithTotal>(&sql);
    if !searching {
        fetch = fetch.bind(limit).bind(offset);
    }
    let mut rows = fetch
        .fetch_all(pool)
        .await
        .map_err(|e| AppError::db("list customers", e))?;
    for row in &mut rows {
        row.family_name = decrypt_field(&row.family_name);
        row.given_name = decrypt_field(&row.given_name);
        row.family_name_kana = decrypt_field(&row.family_name_kana);
        row.given_name_kana = decrypt_field(&row.given_name_kana);
        row.email = decrypt_field(&row.email);
    }
    if let Some(term) = &query {
        let needle = term.to_lowercase();
        rows.retain(|row| customer_row_matches(row, &needle));
        let total = rows.len() as i64;
        let page = rows
            .into_iter()
            .skip(offset.max(0) as usize)
            .take(limit.max(0) as usize)
            .map(to_customer)
            .collect();
        return Ok((page, total));
    }
    // A page past the end returns no rows to carry the total;
    // only then is a separate count worth its round trip.
    if rows.is_empty() && offset > 0 {
        let total = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM users u")
            .fetch_one(pool)
            .await
            .map_err(|e| AppError::db("count customers", e))?;
        return Ok((Vec::new(), total));
    }
    let total = rows.first().map(|r| r.total).unwrap_or(0);
    Ok((rows.into_iter().map(to_customer).collect(), total))
}

/// Whether a decrypted customer row matches the search text:
/// a case-insensitive substring over the name parts, the two
/// joined forms the pages render, the kana name the same way,
/// and the email — the predicate the SQL ILIKE ran before
/// encryption.
fn customer_row_matches(row: &AdminCustomerWithTotal, needle: &str) -> bool {
    let fields = [
        row.family_name.clone(),
        row.given_name.clone(),
        format!("{}{}", row.family_name, row.given_name),
        format!("{} {}", row.family_name, row.given_name),
        row.family_name_kana.clone(),
        row.given_name_kana.clone(),
        format!("{}{}", row.family_name_kana, row.given_name_kana),
        format!("{} {}", row.family_name_kana, row.given_name_kana),
        row.email.clone(),
    ];
    fields.iter().any(|f| f.to_lowercase().contains(needle))
}

/// Converts a decrypted list row to the screen model.
fn to_customer(r: AdminCustomerWithTotal) -> AdminCustomer {
    AdminCustomer {
        id: r.id,
        family_name: r.family_name,
        given_name: r.given_name,
        email: r.email,
        created_at: r.created_at,
        order_count: r.order_count,
        total_spent_yen: r.total_spent_yen,
        last_order_at: r.last_order_at,
        withdrawn_at: r.withdrawn_at,
        r_score: None,
        f_score: None,
        m_score: None,
    }
}

/// [`AdminCustomer`] plus the pagination total.
#[derive(sqlx::FromRow)]
struct AdminCustomerWithTotal {
    id: i64,
    family_name: String,
    given_name: String,
    family_name_kana: String,
    given_name_kana: String,
    email: String,
    created_at: chrono::DateTime<chrono::Utc>,
    order_count: i64,
    total_spent_yen: i64,
    last_order_at: Option<chrono::DateTime<chrono::Utc>>,
    withdrawn_at: Option<chrono::DateTime<chrono::Utc>>,
    total: i64,
}

/// One member's full profile for the detail screen —
/// [`AdminCustomerDetail`] has no password-hash field, and the
/// SELECT names exactly its columns.
pub async fn find_customer_detail(
    pool: &PgPool,
    id: i64,
) -> Result<Option<AdminCustomerDetail>, AppError> {
    let mut detail = sqlx::query_as::<_, AdminCustomerDetail>(
        "SELECT id, email, family_name, given_name, family_name_kana, \
                given_name_kana, phone, postal_code, prefecture, city, \
                address_line, building, birth_date, gender, mail_magazine, \
                created_at \
         FROM users WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(pool)
    .await
    .map_err(|e| AppError::db("find customer detail", e))?;
    if let Some(detail) = &mut detail {
        decrypt_customer_detail(detail);
    }
    Ok(detail)
}

/// Lists one member's orders, newest first — the detail screen's
/// history table. The column list is the shared ORDER_COLUMNS
/// constant — never `SELECT *`.
pub async fn orders_for_customer(pool: &PgPool, user_id: i64) -> Result<Vec<Order>, AppError> {
    let mut orders = sqlx::query_as::<_, Order>(&format!(
        "SELECT {ORDER_COLUMNS} FROM orders WHERE user_id = $1 ORDER BY created_at DESC, id DESC"
    ))
    .bind(user_id)
    .fetch_all(pool)
    .await
    .map_err(|e| AppError::db("orders for customer", e))?;
    for order in &mut orders {
        decrypt_order(order);
    }
    Ok(orders)
}

/// Returns one member's purchase figures: the order count and
/// the purchase total both exclude cancelled orders, so the
/// detail header shows exactly the numbers the customers list
/// shows for the same member.
pub async fn customer_totals(pool: &PgPool, user_id: i64) -> Result<(i64, i64), AppError> {
    // `SUM` over a bigint column returns numeric and sqlx will not
    // decode numeric into i64, so the sum is cast back to bigint —
    // money stays integer yen end to end. Without the cast this
    // query errored at runtime and the page showed a fictional ¥0.
    sqlx::query_as::<_, (i64, i64)>(
        "SELECT COUNT(*) FILTER (WHERE status <> 'cancelled'), \
                COALESCE(SUM(total_yen) FILTER (WHERE status <> 'cancelled'), 0)::bigint \
         FROM orders WHERE user_id = $1",
    )
    .bind(user_id)
    .fetch_one(pool)
    .await
    .map_err(|e| AppError::db("customer totals", e))
}

/// A member's withdrawal instant, for the admin detail
/// marker (Account Deletion spec D7). `None` both for an
/// active member and a missing one; the detail handler
/// only calls this for a member it already loaded.
pub async fn withdrawn_at(
    pool: &PgPool,
    id: i64,
) -> Result<Option<chrono::DateTime<chrono::Utc>>, AppError> {
    sqlx::query_scalar::<_, Option<chrono::DateTime<chrono::Utc>>>(
        "SELECT withdrawn_at FROM users WHERE id = $1",
    )
    .bind(id)
    .fetch_one(pool)
    .await
    .map_err(|e| AppError::db("customer withdrawn_at", e))
}

// ------------------------------------------------------------------
// C1 operator metadata: memo + tags. Operator data only —
// nothing here reads or writes member profile columns.
// ------------------------------------------------------------------

/// The operator memo for one member (empty when none).
pub async fn customer_memo(pool: &PgPool, user_id: i64) -> Result<String, AppError> {
    let memo: Option<String> =
        sqlx::query_scalar("SELECT memo FROM customer_notes WHERE user_id = $1")
            .bind(user_id)
            .fetch_optional(pool)
            .await
            .map_err(|e| AppError::db("read customer memo", e))?;
    Ok(memo.unwrap_or_default())
}

/// Saves the memo inside the caller's transaction (upsert).
pub async fn save_customer_memo(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    user_id: i64,
    memo: &str,
) -> Result<(), AppError> {
    sqlx::query(
        "INSERT INTO customer_notes (user_id, memo) VALUES ($1, $2) \
         ON CONFLICT (user_id) DO UPDATE SET memo = EXCLUDED.memo, updated_at = now()",
    )
    .bind(user_id)
    .bind(memo)
    .execute(&mut **tx)
    .await
    .map_err(|e| AppError::db("save customer memo", e))?;
    Ok(())
}

/// The member's tags, alphabetical.
pub async fn customer_tags(pool: &PgPool, user_id: i64) -> Result<Vec<String>, AppError> {
    sqlx::query_scalar("SELECT tag FROM customer_tags WHERE user_id = $1 ORDER BY tag")
        .bind(user_id)
        .fetch_all(pool)
        .await
        .map_err(|e| AppError::db("list customer tags", e))
}

/// Adds a tag inside the caller's transaction; false when
/// it was already present.
pub async fn add_customer_tag(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    user_id: i64,
    tag: &str,
) -> Result<bool, AppError> {
    let affected = sqlx::query(
        "INSERT INTO customer_tags (user_id, tag) VALUES ($1, $2) \
         ON CONFLICT DO NOTHING",
    )
    .bind(user_id)
    .bind(tag)
    .execute(&mut **tx)
    .await
    .map_err(|e| AppError::db("add customer tag", e))?;
    Ok(affected.rows_affected() > 0)
}

/// Removes a tag inside the caller's transaction.
pub async fn remove_customer_tag(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    user_id: i64,
    tag: &str,
) -> Result<bool, AppError> {
    let affected = sqlx::query("DELETE FROM customer_tags WHERE user_id = $1 AND tag = $2")
        .bind(user_id)
        .bind(tag)
        .execute(&mut **tx)
        .await
        .map_err(|e| AppError::db("remove customer tag", e))?;
    Ok(affected.rows_affected() > 0)
}
