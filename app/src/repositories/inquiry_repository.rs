//! SQL for inquiries and their replies.
//! Every statement is parameterised; bodies and memos are
//! data and are never logged by callers.

use sqlx::{PgPool, Postgres, Transaction};

use crate::models::{Inquiry, InquiryReply};

/// Inserts one inquiry and returns its id (the 受付番号
/// derives from it).
pub async fn insert(
    pool: &PgPool,
    user_id: Option<i64>,
    name: &str,
    email: &str,
    category: &str,
    order_number: Option<&str>,
    body: &str,
) -> Result<i64, sqlx::Error> {
    let (id,): (i64,) = sqlx::query_as(
        "INSERT INTO inquiries (user_id, name, email, category, order_number, body)
         VALUES ($1, $2, $3, $4, $5, $6) RETURNING id",
    )
    .bind(user_id)
    .bind(name)
    .bind(email)
    .bind(category)
    .bind(order_number)
    .bind(body)
    .fetch_one(pool)
    .await?;
    Ok(id)
}

/// How many inquiries one email address submitted in the
/// last 24 hours — the per-email rate limit, counted in the
/// DB so it survives restarts.
pub async fn count_by_email_last_day(pool: &PgPool, email: &str) -> Result<i64, sqlx::Error> {
    let (count,): (i64,) = sqlx::query_as(
        "SELECT count(*) FROM inquiries
         WHERE lower(email) = lower($1) AND created_at > now() - INTERVAL '24 hours'",
    )
    .bind(email)
    .fetch_one(pool)
    .await?;
    Ok(count)
}

/// Counts per status, for the admin chips, dashboard tile
/// and sidebar badge.
pub async fn count_by_status(pool: &PgPool, status: &str) -> Result<i64, sqlx::Error> {
    let (count,): (i64,) = sqlx::query_as("SELECT count(*) FROM inquiries WHERE status = $1")
        .bind(status)
        .fetch_one(pool)
        .await?;
    Ok(count)
}

/// The total inquiry count, for the すべて chip.
pub async fn count_all(pool: &PgPool) -> Result<i64, sqlx::Error> {
    let (count,): (i64,) = sqlx::query_as("SELECT count(*) FROM inquiries")
        .fetch_one(pool)
        .await?;
    Ok(count)
}

const COLS: &str = "id, user_id, name, email, category, order_number, body,
                    status, admin_memo, created_at, updated_at";

/// One page of the admin list, newest first, optionally
/// filtered by status.
pub async fn list_page(
    pool: &PgPool,
    status: Option<&str>,
    limit: i64,
    offset: i64,
) -> Result<Vec<Inquiry>, sqlx::Error> {
    let sql = format!(
        "SELECT {COLS} FROM inquiries
         WHERE ($1::text IS NULL OR status = $1)
         ORDER BY id DESC LIMIT $2 OFFSET $3"
    );
    sqlx::query_as::<_, Inquiry>(&sql)
        .bind(status)
        .bind(limit)
        .bind(offset)
        .fetch_all(pool)
        .await
}

/// The filtered list's total, for pagination.
pub async fn count_filtered(pool: &PgPool, status: Option<&str>) -> Result<i64, sqlx::Error> {
    let (count,): (i64,) =
        sqlx::query_as("SELECT count(*) FROM inquiries WHERE ($1::text IS NULL OR status = $1)")
            .bind(status)
            .fetch_one(pool)
            .await?;
    Ok(count)
}

/// Loads one inquiry by id.
pub async fn find(pool: &PgPool, id: i64) -> Result<Option<Inquiry>, sqlx::Error> {
    let sql = format!("SELECT {COLS} FROM inquiries WHERE id = $1");
    sqlx::query_as::<_, Inquiry>(&sql)
        .bind(id)
        .fetch_optional(pool)
        .await
}

/// Loads one inquiry inside a transaction, locking its
/// row: the read the admin status/memo guard compares
/// before writing.
pub async fn find_for_update(
    tx: &mut Transaction<'_, Postgres>,
    id: i64,
) -> Result<Option<Inquiry>, sqlx::Error> {
    let sql = format!("SELECT {COLS} FROM inquiries WHERE id = $1 FOR UPDATE");
    sqlx::query_as::<_, Inquiry>(&sql)
        .bind(id)
        .fetch_optional(&mut **tx)
        .await
}

/// The status write inside a caller's transaction (the
/// guarded admin save).
pub async fn update_status_tx(
    tx: &mut Transaction<'_, Postgres>,
    id: i64,
    status: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE inquiries SET status = $1, updated_at = now() WHERE id = $2")
        .bind(status)
        .bind(id)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

/// The memo write inside a caller's transaction (the
/// guarded admin save).
pub async fn update_memo_tx(
    tx: &mut Transaction<'_, Postgres>,
    id: i64,
    memo: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE inquiries SET admin_memo = $1, updated_at = now() WHERE id = $2")
        .bind(memo)
        .bind(id)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

/// Sets the workflow status (freely settable, spec §3.5).
pub async fn update_status(pool: &PgPool, id: i64, status: &str) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE inquiries SET status = $1, updated_at = now() WHERE id = $2")
        .bind(status)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Stores one sent reply exactly as sent.
pub async fn insert_reply(
    pool: &PgPool,
    inquiry_id: i64,
    admin_id: i64,
    subject: &str,
    body: &str,
) -> Result<i64, sqlx::Error> {
    let (id,): (i64,) = sqlx::query_as(
        "INSERT INTO inquiry_replies (inquiry_id, admin_id, subject, body)
         VALUES ($1, $2, $3, $4) RETURNING id",
    )
    .bind(inquiry_id)
    .bind(admin_id)
    .bind(subject)
    .bind(body)
    .fetch_one(pool)
    .await?;
    Ok(id)
}

/// An inquiry's replies, oldest first (the 返信履歴 order).
pub async fn replies_of(pool: &PgPool, inquiry_id: i64) -> Result<Vec<InquiryReply>, sqlx::Error> {
    sqlx::query_as::<_, InquiryReply>(
        "SELECT id, inquiry_id, admin_id, subject, body, sent_at
         FROM inquiry_replies WHERE inquiry_id = $1 ORDER BY id",
    )
    .bind(inquiry_id)
    .fetch_all(pool)
    .await
}

/// Scrubs a member's identity off their inquiries at
/// withdrawal (Account Deletion spec 3.4's principle
/// applied to the contact feature): the support record
/// stays, the name and address become markers and the
/// member link is removed.
pub async fn scrub_for_user(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    user_id: i64,
    marker_email: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE inquiries SET name = '退会済み', email = $2, user_id = NULL \
         WHERE user_id = $1",
    )
    .bind(user_id)
    .bind(marker_email)
    .execute(&mut **tx)
    .await
    .map(|_| ())
}
