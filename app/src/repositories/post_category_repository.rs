//! Blog post category repository: `post_categories`
//! (CMS blog spec, docs/specs/CMS_SPEC.md).
//!
//! The blog's own small vocabulary — a post about shipping
//! is not a product category, so posts never read the
//! product taxonomy. Deleting a category leaves its posts
//! uncategorised: the foreign key's ON DELETE SET NULL is
//! the whole mechanism.

use sqlx::PgPool;

use crate::error::AppError;
use crate::models::PostCategory;

/// The category column set, bound once so every read
/// agrees about the shape.
const CATEGORY_COLUMNS: &str = "id, name, slug, created_at, updated_at";

/// Returns every category in registration order — the
/// admin list and the post form's select share this order.
pub async fn list_all(pool: &PgPool) -> Result<Vec<PostCategory>, AppError> {
    sqlx::query_as::<_, PostCategory>(&format!(
        "SELECT {CATEGORY_COLUMNS} FROM post_categories ORDER BY id"
    ))
    .fetch_all(pool)
    .await
    .map_err(|e| AppError::db("list post categories", e))
}

/// Returns one category by ID, or `None`.
pub async fn find_by_id(pool: &PgPool, id: i64) -> Result<Option<PostCategory>, AppError> {
    sqlx::query_as::<_, PostCategory>(&format!(
        "SELECT {CATEGORY_COLUMNS} FROM post_categories WHERE id = $1"
    ))
    .bind(id)
    .fetch_optional(pool)
    .await
    .map_err(|e| AppError::db("find post category", e))
}

/// Returns one category by slug, or `None` — the archive's
/// lookup; whether it holds live posts is the service's
/// question, answered by the post queries.
pub async fn find_by_slug(pool: &PgPool, slug: &str) -> Result<Option<PostCategory>, AppError> {
    sqlx::query_as::<_, PostCategory>(&format!(
        "SELECT {CATEGORY_COLUMNS} FROM post_categories WHERE slug = $1"
    ))
    .bind(slug)
    .fetch_optional(pool)
    .await
    .map_err(|e| AppError::db("find post category by slug", e))
}

/// Whether a category other than `exclude` already owns
/// this slug (`exclude` is the category being edited).
pub async fn slug_taken(pool: &PgPool, slug: &str, exclude: Option<i64>) -> Result<bool, AppError> {
    let row: Option<(i64,)> = sqlx::query_as(
        "SELECT id FROM post_categories WHERE slug = $1 AND ($2::bigint IS NULL OR id <> $2)",
    )
    .bind(slug)
    .bind(exclude)
    .fetch_optional(pool)
    .await
    .map_err(|e| AppError::db("check post category slug", e))?;
    Ok(row.is_some())
}

/// Categories holding at least one live post, for the
/// sitemap: an archive page with nothing public on it is
/// not advertised.
pub async fn list_with_live_posts(pool: &PgPool) -> Result<Vec<PostCategory>, AppError> {
    sqlx::query_as::<_, PostCategory>(&format!(
        "SELECT {CATEGORY_COLUMNS} FROM post_categories c \
         WHERE EXISTS (SELECT 1 FROM posts p WHERE p.category_id = c.id \
                       AND p.status = 'published' AND p.published_at IS NOT NULL \
                       AND p.published_at <= now()) \
         ORDER BY c.slug ASC"
    ))
    .fetch_all(pool)
    .await
    .map_err(|e| AppError::db("list post categories with live posts", e))
}

/// Inserts a category inside the caller's transaction and
/// returns its new ID.
pub async fn insert(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    name: &str,
    slug: &str,
) -> Result<i64, AppError> {
    let row: (i64,) =
        sqlx::query_as("INSERT INTO post_categories (name, slug) VALUES ($1, $2) RETURNING id")
            .bind(name)
            .bind(slug)
            .fetch_one(&mut **tx)
            .await
            .map_err(|e| AppError::db("insert post category", e))?;
    Ok(row.0)
}

/// Returns the category's current `updated_at` with the
/// row locked: the edit form's staleness guard, mirroring
/// the tags'.
pub async fn updated_at_for_update(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    id: i64,
) -> Result<Option<chrono::DateTime<chrono::Utc>>, AppError> {
    let value = sqlx::query_scalar::<_, chrono::DateTime<chrono::Utc>>(
        "SELECT updated_at FROM post_categories WHERE id = $1 FOR UPDATE",
    )
    .bind(id)
    .fetch_optional(&mut **tx)
    .await?;
    Ok(value)
}

/// Stores a category's fields inside the caller's
/// transaction. `updated_at` is stamped by the trigger.
pub async fn update(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    id: i64,
    name: &str,
    slug: &str,
) -> Result<(), AppError> {
    sqlx::query("UPDATE post_categories SET name = $1, slug = $2 WHERE id = $3")
        .bind(name)
        .bind(slug)
        .bind(id)
        .execute(&mut **tx)
        .await
        .map_err(|e| AppError::db("update post category", e))?;
    Ok(())
}

/// Deletes a category inside the caller's transaction;
/// its posts keep living, uncategorised (the foreign
/// key's SET NULL).
pub async fn delete(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    id: i64,
) -> Result<u64, AppError> {
    let result = sqlx::query("DELETE FROM post_categories WHERE id = $1")
        .bind(id)
        .execute(&mut **tx)
        .await
        .map_err(|e| AppError::db("delete post category", e))?;
    Ok(result.rows_affected())
}
