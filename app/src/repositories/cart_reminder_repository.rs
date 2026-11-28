//! Cart reminder data: idle member carts, their lines
//! with live stock, and the reminded-fingerprint record.

use chrono::{DateTime, Utc};
use sqlx::PgPool;

use crate::error::AppError;

/// One idle member cart: its owner and last touch.
pub struct IdleCart {
    /// The member who owns the cart.
    pub user_id: i64,
    /// When the cart last changed.
    pub last_touch: DateTime<Utc>,
}

/// One cart line with everything the reminder checks and
/// prints.
#[derive(sqlx::FromRow)]
pub struct ReminderLine {
    /// Variant id (fingerprint input).
    pub variant_id: i64,
    /// Quantity in the cart (fingerprint input).
    pub quantity: i32,
    /// Product name for the mail.
    pub product_name: String,
    /// Whether the product tracks stock at all.
    pub track_stock: bool,
    /// The variant's stock, when tracked.
    pub variant_stock: i32,
    /// Whether the variant is purchasable.
    pub variant_enabled: bool,
    /// The product's visibility status.
    pub product_status: String,
}

/// Members whose cart has been untouched for at least
/// `delay_hours` and is non-empty (guest carts have no
/// durable email address and are out of scope by design).
pub async fn idle_member_carts(pool: &PgPool, delay_hours: i64) -> Result<Vec<IdleCart>, AppError> {
    let rows = sqlx::query_as::<_, (i64, DateTime<Utc>)>(
        "SELECT user_id, MAX(updated_at) FROM cart_items \
         WHERE user_id IS NOT NULL \
         GROUP BY user_id \
         HAVING MAX(updated_at) < now() - make_interval(hours => $1::int) \
         ORDER BY user_id",
    )
    .bind(delay_hours)
    .fetch_all(pool)
    .await
    .map_err(|e| AppError::db("idle carts", e))?;
    Ok(rows
        .into_iter()
        .map(|(user_id, last_touch)| IdleCart {
            user_id,
            last_touch,
        })
        .collect())
}

/// The member's current cart lines.
pub async fn cart_lines(pool: &PgPool, user_id: i64) -> Result<Vec<ReminderLine>, AppError> {
    sqlx::query_as::<_, ReminderLine>(
        "SELECT ci.variant_id, ci.quantity, p.name AS product_name, \
                p.track_stock, pv.stock AS variant_stock, \
                pv.is_enabled AS variant_enabled, p.status AS product_status \
         FROM cart_items ci \
         JOIN product_variants pv ON pv.id = ci.variant_id \
         JOIN products p ON p.id = pv.product_id \
         WHERE ci.user_id = $1 ORDER BY ci.id",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await
    .map_err(|e| AppError::db("reminder cart lines", e))
}

/// When the member last placed an order, when they have.
pub async fn last_order_at(pool: &PgPool, user_id: i64) -> Result<Option<DateTime<Utc>>, AppError> {
    sqlx::query_scalar("SELECT MAX(created_at) FROM orders WHERE user_id = $1")
        .bind(user_id)
        .fetch_one(pool)
        .await
        .map_err(|e| AppError::db("last order at", e))
}

/// The fingerprint already reminded for this member.
pub async fn reminded_fingerprint(pool: &PgPool, user_id: i64) -> Result<Option<String>, AppError> {
    sqlx::query_scalar("SELECT cart_fingerprint FROM cart_reminders WHERE user_id = $1")
        .bind(user_id)
        .fetch_optional(pool)
        .await
        .map_err(|e| AppError::db("reminded fingerprint", e))
}

/// Records that this cart state has been reminded.
pub async fn record_reminder(
    pool: &PgPool,
    user_id: i64,
    fingerprint: &str,
) -> Result<(), AppError> {
    sqlx::query(
        "INSERT INTO cart_reminders (user_id, cart_fingerprint) VALUES ($1, $2) \
         ON CONFLICT (user_id) DO UPDATE \
         SET cart_fingerprint = EXCLUDED.cart_fingerprint, reminded_at = now()",
    )
    .bind(user_id)
    .bind(fingerprint)
    .execute(pool)
    .await
    .map_err(|e| AppError::db("record cart reminder", e))?;
    Ok(())
}

/// The member's marketing consent state: the subscription
/// row's status, when a row exists at all.
pub async fn subscription_status(pool: &PgPool, user_id: i64) -> Result<Option<String>, AppError> {
    sqlx::query_scalar("SELECT status FROM email_subscriptions WHERE user_id = $1")
        .bind(user_id)
        .fetch_optional(pool)
        .await
        .map_err(|e| AppError::db("subscription status", e))
}

/// One idle guest cart with its captured checkout contact.
pub struct GuestIdleCart {
    /// The guest cart's session owner token.
    pub session_id: String,
    /// When the cart last changed.
    pub last_touch: DateTime<Utc>,
    /// The captured email, PII-encrypted at rest.
    pub email: String,
    /// The reminded fingerprint for this cart, when a
    /// reminder already went out for an earlier state.
    pub cart_fingerprint: Option<String>,
}

/// Stores (or refreshes) the email a guest typed at
/// checkout against their cart's session owner. A fresh
/// checkout entry always re-arms the contact: the address
/// comes back, the fingerprint tombstone stays.
pub async fn upsert_guest_contact(
    pool: &PgPool,
    session_id: &str,
    email: &str,
    email_bidx: &str,
) -> Result<(), AppError> {
    sqlx::query(
        "INSERT INTO guest_cart_contacts (session_id, email, email_bidx) VALUES ($1, $2, $3) \
         ON CONFLICT (session_id) DO UPDATE \
         SET email = EXCLUDED.email, email_bidx = EXCLUDED.email_bidx, updated_at = now()",
    )
    .bind(session_id)
    .bind(email)
    .bind(email_bidx)
    .execute(pool)
    .await
    .map_err(|e| AppError::db("upsert guest contact", e))?;
    Ok(())
}

/// Deletes a guest contact outright (the cart converted
/// to an order, or the address can never qualify).
pub async fn delete_guest_contact<'e, E>(executor: E, session_id: &str) -> Result<(), sqlx::Error>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    sqlx::query("DELETE FROM guest_cart_contacts WHERE session_id = $1")
        .bind(session_id)
        .execute(executor)
        .await?;
    Ok(())
}

/// Guest carts (with a captured, still-held email) idle
/// for at least `delay_hours`.
pub async fn idle_guest_carts(
    pool: &PgPool,
    delay_hours: i64,
) -> Result<Vec<GuestIdleCart>, AppError> {
    let rows = sqlx::query_as::<_, (String, DateTime<Utc>, String, Option<String>)>(
        "SELECT ci.session_id, MAX(ci.updated_at), g.email, g.cart_fingerprint \
         FROM cart_items ci \
         JOIN guest_cart_contacts g ON g.session_id = ci.session_id \
         WHERE ci.user_id IS NULL AND ci.session_id IS NOT NULL AND g.email <> '' \
         GROUP BY ci.session_id, g.email, g.cart_fingerprint \
         HAVING MAX(ci.updated_at) < now() - make_interval(hours => $1::int) \
         ORDER BY ci.session_id",
    )
    .bind(delay_hours)
    .fetch_all(pool)
    .await
    .map_err(|e| AppError::db("idle guest carts", e))?;
    Ok(rows
        .into_iter()
        .map(
            |(session_id, last_touch, email, cart_fingerprint)| GuestIdleCart {
                session_id,
                last_touch,
                email,
                cart_fingerprint,
            },
        )
        .collect())
}

/// The guest cart's current lines (same shape as members).
pub async fn guest_cart_lines(
    pool: &PgPool,
    session_id: &str,
) -> Result<Vec<ReminderLine>, AppError> {
    sqlx::query_as::<_, ReminderLine>(
        "SELECT ci.variant_id, ci.quantity, p.name AS product_name, \
                p.track_stock, pv.stock AS variant_stock, \
                pv.is_enabled AS variant_enabled, p.status AS product_status \
         FROM cart_items ci \
         JOIN product_variants pv ON pv.id = ci.variant_id \
         JOIN products p ON p.id = pv.product_id \
         WHERE ci.session_id = $1 ORDER BY ci.id",
    )
    .bind(session_id)
    .fetch_all(pool)
    .await
    .map_err(|e| AppError::db("guest reminder cart lines", e))
}

/// Claims a guest cart state for reminding: records the
/// fingerprint and clears the stored email in the same
/// write, so the address is never held past its one use.
pub async fn claim_guest_reminder(
    pool: &PgPool,
    session_id: &str,
    fingerprint: &str,
) -> Result<(), AppError> {
    sqlx::query(
        "UPDATE guest_cart_contacts \
         SET cart_fingerprint = $2, email = '', email_bidx = '', updated_at = now() \
         WHERE session_id = $1",
    )
    .bind(session_id)
    .bind(fingerprint)
    .execute(pool)
    .await
    .map_err(|e| AppError::db("claim guest reminder", e))?;
    Ok(())
}

/// Drops contact rows whose cart no longer exists (the
/// cart expired or was swept): the address has nothing
/// left to attach to.
pub async fn delete_orphan_guest_contacts(pool: &PgPool) -> Result<(), AppError> {
    sqlx::query(
        "DELETE FROM guest_cart_contacts g \
         WHERE NOT EXISTS (SELECT 1 FROM cart_items ci WHERE ci.session_id = g.session_id)",
    )
    .execute(pool)
    .await
    .map_err(|e| AppError::db("delete orphan guest contacts", e))?;
    Ok(())
}

/// The account id registered for an email blind index,
/// when one exists (no credential fields are read here).
pub async fn member_id_by_bidx(pool: &PgPool, email_bidx: &str) -> Result<Option<i64>, AppError> {
    sqlx::query_scalar("SELECT id FROM users WHERE email_bidx = $1")
        .bind(email_bidx)
        .fetch_optional(pool)
        .await
        .map_err(|e| AppError::db("member id by email index", e))
}

/// The member's unsubscribe token, for the reminder
/// mail's tokenised unsubscribe link.
pub async fn subscription_token(pool: &PgPool, user_id: i64) -> Result<Option<String>, AppError> {
    sqlx::query_scalar("SELECT unsubscribe_token FROM email_subscriptions WHERE user_id = $1")
        .bind(user_id)
        .fetch_optional(pool)
        .await
        .map_err(|e| AppError::db("subscription token", e))
}
