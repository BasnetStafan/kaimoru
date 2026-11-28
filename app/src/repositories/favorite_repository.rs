//! Persistence for member favourites.
//!
//! Every statement is scoped by `user_id`: ownership is the
//! WHERE clause itself, the same pattern as the address book,
//! so another member's marks are unreachable by construction.

use sqlx::PgPool;

use crate::models::Favorite;

/// Adds a favourite mark; an existing mark is left as it is
/// (the insert is idempotent, so a repeated toggle-on or a
/// double submit can never duplicate or fail).
pub async fn add(pool: &PgPool, user_id: i64, product_id: i64) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO favorites (user_id, product_id) VALUES ($1, $2) \
         ON CONFLICT DO NOTHING",
    )
    .bind(user_id)
    .bind(product_id)
    .execute(pool)
    .await
    .map(|_| ())
}

/// Removes a favourite mark; returns whether one was removed.
pub async fn remove(pool: &PgPool, user_id: i64, product_id: i64) -> Result<bool, sqlx::Error> {
    let rows = sqlx::query("DELETE FROM favorites WHERE user_id = $1 AND product_id = $2")
        .bind(user_id)
        .bind(product_id)
        .execute(pool)
        .await?;
    Ok(rows.rows_affected() > 0)
}

/// Whether the member has favourited the product.
pub async fn is_favorite(
    pool: &PgPool,
    user_id: i64,
    product_id: i64,
) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(SELECT 1 FROM favorites WHERE user_id = $1 AND product_id = $2)",
    )
    .bind(user_id)
    .bind(product_id)
    .fetch_one(pool)
    .await
}

/// The subset of `product_ids` the member has favourited, in
/// one query for a whole page of cards — never one query per
/// card.
pub async fn favorite_ids_among(
    pool: &PgPool,
    user_id: i64,
    product_ids: &[i64],
) -> Result<Vec<i64>, sqlx::Error> {
    sqlx::query_scalar::<_, i64>(
        "SELECT product_id FROM favorites WHERE user_id = $1 AND product_id = ANY($2)",
    )
    .bind(user_id)
    .bind(product_ids)
    .fetch_all(pool)
    .await
}

/// All of a member's favourite marks, newest first (the
/// favourites page's order).
pub async fn list_for_user(pool: &PgPool, user_id: i64) -> Result<Vec<Favorite>, sqlx::Error> {
    sqlx::query_as::<_, Favorite>(
        "SELECT user_id, product_id, created_at FROM favorites \
         WHERE user_id = $1 ORDER BY created_at DESC, product_id DESC",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await
}

/// How many favourites the member holds.
pub async fn count_for_user(pool: &PgPool, user_id: i64) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar("SELECT COUNT(*) FROM favorites WHERE user_id = $1")
        .bind(user_id)
        .fetch_one(pool)
        .await
}

/// Deletes every favourite of one member (account withdrawal):
/// favourites are personal data with no remaining business
/// purpose, the address book's principle.
pub async fn delete_all_for_user(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    user_id: i64,
) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM favorites WHERE user_id = $1")
        .bind(user_id)
        .execute(&mut **tx)
        .await
        .map(|_| ())
}
