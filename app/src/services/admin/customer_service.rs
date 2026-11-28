//! Admin customer reads, plus operator metadata writes.
//!
//! The read side of the member screens: the list, the detail, the
//! purchase totals and the order history. The only writes are
//! OPERATOR metadata — the internal memo and tags — added by C1:
//! there is still no suspend, profile-edit or delete path for
//! members, and the privacy rule lives in the repository below:
//! its queries never select the password hash.

use sqlx::PgPool;

use crate::error::AppError;
use crate::models::{AdminCustomer, AdminCustomerDetail, Order};
use crate::repositories::admin::customer_repository;
use crate::services::admin::audit_service;

/// Returns one page of the customers list, newest registrations
/// first, each with the purchase aggregates (non-cancelled order
/// count, total spent, last order date), plus the total. `query`
/// is the raw search text, matched partially and
/// case-insensitively over name, kana name and email, or `None`.
pub async fn list_customers(
    pool: &PgPool,
    query: Option<String>,
    limit: i64,
    offset: i64,
) -> Result<(Vec<AdminCustomer>, i64), AppError> {
    let (mut rows, total) = customer_repository::list_customers(pool, query, limit, offset).await?;
    // The RFM bands ride along from the live band map: the
    // list query pages over every member, scored or not, so
    // the bands (a population-relative computation) arrive
    // as a lookup, not a join.
    let bands = crate::repositories::admin::rfm_repository::band_map(pool).await?;
    for row in &mut rows {
        if let Some(&(r, f, m)) = bands.get(&row.id) {
            row.r_score = Some(r);
            row.f_score = Some(f);
            row.m_score = Some(m);
        }
    }
    Ok((rows, total))
}

/// Returns one member's full profile for the detail screen.
pub async fn find_customer_detail(
    pool: &PgPool,
    id: i64,
) -> Result<Option<AdminCustomerDetail>, AppError> {
    customer_repository::find_customer_detail(pool, id).await
}

/// Returns one member's order count and purchase total, both
/// excluding cancelled orders — the same figures the customers
/// list shows.
pub async fn customer_totals(pool: &PgPool, user_id: i64) -> Result<(i64, i64), AppError> {
    customer_repository::customer_totals(pool, user_id).await
}

/// Returns one member's orders, newest first — the detail
/// screen's history table.
pub async fn orders_for_customer(pool: &PgPool, user_id: i64) -> Result<Vec<Order>, AppError> {
    customer_repository::orders_for_customer(pool, user_id).await
}

// ------------------------------------------------------------------
// C1 operator metadata (memo + tags), audited. Operator data
// only — member credentials and profile are never touched.
// ------------------------------------------------------------------

/// Memo length ceiling, in characters.
pub const MEMO_MAX_CHARS: usize = 2000;
/// Tag length ceiling, in characters.
pub const TAG_MAX_CHARS: usize = 50;

/// The member's memo + tags for the detail page.
pub async fn customer_meta(pool: &PgPool, user_id: i64) -> Result<(String, Vec<String>), AppError> {
    Ok((
        customer_repository::customer_memo(pool, user_id).await?,
        customer_repository::customer_tags(pool, user_id).await?,
    ))
}

/// Saves the memo. Returns the validation message, or None.
pub async fn save_memo(
    pool: &PgPool,
    admin_id: i64,
    user_id: i64,
    memo: &str,
) -> Result<Option<String>, AppError> {
    let memo = memo.trim();
    if memo.chars().count() > MEMO_MAX_CHARS {
        return Ok(Some(format!(
            "メモは{MEMO_MAX_CHARS}文字以内で入力してください"
        )));
    }
    let mut tx = pool.begin().await.map_err(|e| AppError::db("begin", e))?;
    customer_repository::save_customer_memo(&mut tx, user_id, memo).await?;
    audit_service::record(&mut tx, admin_id, "customer.memo", "user", Some(user_id)).await?;
    tx.commit().await.map_err(|e| AppError::db("commit", e))?;
    Ok(None)
}

/// Adds a tag. Returns the validation/info message, or None.
pub async fn add_tag(
    pool: &PgPool,
    admin_id: i64,
    user_id: i64,
    tag: &str,
) -> Result<Option<String>, AppError> {
    let tag = tag.trim();
    if tag.is_empty() {
        return Ok(Some("タグを入力してください".to_string()));
    }
    if tag.chars().count() > TAG_MAX_CHARS {
        return Ok(Some(format!(
            "タグは{TAG_MAX_CHARS}文字以内で入力してください"
        )));
    }
    let mut tx = pool.begin().await.map_err(|e| AppError::db("begin", e))?;
    let added = customer_repository::add_customer_tag(&mut tx, user_id, tag).await?;
    if added {
        audit_service::record(&mut tx, admin_id, "customer.tag", "user", Some(user_id)).await?;
    }
    tx.commit().await.map_err(|e| AppError::db("commit", e))?;
    Ok(if added {
        None
    } else {
        Some("そのタグはすでに付いています".to_string())
    })
}

/// Removes a tag, audited when it existed.
pub async fn remove_tag(
    pool: &PgPool,
    admin_id: i64,
    user_id: i64,
    tag: &str,
) -> Result<(), AppError> {
    let mut tx = pool.begin().await.map_err(|e| AppError::db("begin", e))?;
    if customer_repository::remove_customer_tag(&mut tx, user_id, tag).await? {
        audit_service::record(&mut tx, admin_id, "customer.tag", "user", Some(user_id)).await?;
    }
    tx.commit().await.map_err(|e| AppError::db("commit", e))?;
    Ok(())
}
