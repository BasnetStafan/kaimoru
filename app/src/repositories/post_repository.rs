//! Blog post repository: `posts` and `post_revisions`
//! (CMS blog spec, docs/specs/CMS_SPEC.md).
//!
//! The storefront reads carry the ONE liveness predicate —
//! published, with a `published_at` that has arrived — in
//! SQL, the same rule [`crate::models::Post::is_live`]
//! states in Rust, so a draft, a scheduled post and an
//! unknown slug are the same `None` on every surface and
//! their 404s stay indistinguishable.

use sqlx::PgPool;

use crate::error::AppError;
use crate::models::{NewPost, Post, PostRevision, PostRevisionSummary, PostSummary, PostView};

/// The revision retention cap: only the newest
/// this many snapshots per post are kept.
pub const REVISION_CAP: i64 = 50;

/// The post column set, bound once so every read agrees
/// about the shape.
const POST_COLUMNS: &str = "id, slug, title, body_md, format, excerpt, meta_description, \
    cover_image_path, category_id, author_admin_id, status, published_at, created_at, updated_at";

/// The post column set with the `posts` alias, for the
/// admin list read: bare `id` / `slug` would be ambiguous
/// in the join. In its summary shape `body_md` rides only
/// when the stored excerpt is
/// empty — the one case where a list reader can touch the
/// body at all, since [`crate::models::Post::excerpt_or_derived`]
/// derives from it then and prefers the stored excerpt
/// otherwise (and the save path stores excerpts trimmed,
/// so an empty stored excerpt is exactly the derivation
/// case). A list never renders a body; shipping every
/// post's full text to build an index is waste this set
/// avoids without changing any output.
const POST_COLUMNS_P_SUMMARY: &str = "p.id, p.slug, p.title, \
    CASE WHEN p.excerpt = '' THEN p.body_md ELSE '' END AS body_md, p.format, p.excerpt, \
    p.meta_description, p.cover_image_path, p.category_id, p.author_admin_id, p.status, \
    p.published_at, p.created_at, p.updated_at";

/// The liveness predicate, spelled once for every
/// storefront query (see the module doc).
const LIVE_WHERE: &str =
    "status = 'published' AND published_at IS NOT NULL AND published_at <= now()";

/// The storefront read shape: the post plus its category's
/// name and slug and its author's display name.
const VIEW_COLUMNS: &str = "p.id, p.slug, p.title, p.body_md, p.format, p.excerpt, \
    p.meta_description, p.cover_image_path, p.category_id, p.author_admin_id, p.status, \
    p.published_at, p.created_at, p.updated_at, \
    c.name AS category_name, c.slug AS category_slug, a.display_name AS author_name";

/// The summary variant of [`VIEW_COLUMNS`] for the list
/// reads (the index, the archives, the feed): the same
/// body rule as [`POST_COLUMNS_P_SUMMARY`] — the full text
/// rides only for posts whose excerpt must be derived
/// from it.
const VIEW_COLUMNS_SUMMARY: &str = "p.id, p.slug, p.title, \
    CASE WHEN p.excerpt = '' THEN p.body_md ELSE '' END AS body_md, p.format, p.excerpt, \
    p.meta_description, p.cover_image_path, p.category_id, p.author_admin_id, p.status, \
    p.published_at, p.created_at, p.updated_at, \
    c.name AS category_name, c.slug AS category_slug, a.display_name AS author_name";

/// The joins every storefront read shares.
const VIEW_FROM: &str = "FROM posts p \
    LEFT JOIN post_categories c ON c.id = p.category_id \
    JOIN admins a ON a.id = p.author_admin_id";

/// Returns one page of the admin list in the requested
/// derived state (`draft` / `scheduled` / `live`; `None`
/// for every state), newest save first, each post with its
/// category name, its author and its newest revision's
/// editor when one exists.
pub async fn list_admin(
    pool: &PgPool,
    state: Option<&str>,
    limit: i64,
    offset: i64,
) -> Result<Vec<PostSummary>, AppError> {
    let state_where = match state {
        Some("draft") => "p.status = 'draft'",
        Some("scheduled") => {
            "p.status = 'published' AND (p.published_at IS NULL OR p.published_at > now())"
        }
        Some("live") => {
            "p.status = 'published' AND p.published_at IS NOT NULL AND p.published_at <= now()"
        }
        _ => "TRUE",
    };
    sqlx::query_as::<_, PostSummary>(&format!(
        "SELECT {POST_COLUMNS_P_SUMMARY}, \
                c.name AS category_name, a.display_name AS author_name, \
                (SELECT a2.display_name FROM post_revisions r \
                 JOIN admins a2 ON a2.id = r.edited_by \
                 WHERE r.post_id = p.id ORDER BY r.id DESC LIMIT 1) AS last_editor_name \
         FROM posts p \
         LEFT JOIN post_categories c ON c.id = p.category_id \
         JOIN admins a ON a.id = p.author_admin_id \
         WHERE {state_where} \
         ORDER BY p.updated_at DESC, p.id DESC LIMIT $1 OFFSET $2"
    ))
    .bind(limit)
    .bind(offset)
    .fetch_all(pool)
    .await
    .map_err(|e| AppError::db("list posts", e))
}

/// Counts the admin list for one derived state (the same
/// filter [`list_admin`] applies).
pub async fn count_admin(pool: &PgPool, state: Option<&str>) -> Result<i64, AppError> {
    let state_where = match state {
        Some("draft") => "status = 'draft'",
        Some("scheduled") => {
            "status = 'published' AND (published_at IS NULL OR published_at > now())"
        }
        Some("live") => {
            "status = 'published' AND published_at IS NOT NULL AND published_at <= now()"
        }
        _ => "TRUE",
    };
    sqlx::query_scalar::<_, i64>(&format!("SELECT count(*) FROM posts WHERE {state_where}"))
        .fetch_one(pool)
        .await
        .map_err(|e| AppError::db("count posts", e))
}

/// Returns one post by ID, or `None` when it does not exist.
pub async fn find_by_id(pool: &PgPool, id: i64) -> Result<Option<Post>, AppError> {
    sqlx::query_as::<_, Post>(&format!("SELECT {POST_COLUMNS} FROM posts WHERE id = $1"))
        .bind(id)
        .fetch_optional(pool)
        .await
        .map_err(|e| AppError::db("find post", e))
}

/// Returns one post by ID with its row locked for the
/// caller's transaction (the restore's read, mirroring the
/// content pages).
pub async fn find_by_id_for_update(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    id: i64,
) -> Result<Option<Post>, AppError> {
    sqlx::query_as::<_, Post>(&format!(
        "SELECT {POST_COLUMNS} FROM posts WHERE id = $1 FOR UPDATE"
    ))
    .bind(id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(|e| AppError::db("find post for update", e))
}

/// Whether any post already owns this slug.
pub async fn slug_taken(pool: &PgPool, slug: &str) -> Result<bool, AppError> {
    let row: Option<(i64,)> = sqlx::query_as("SELECT id FROM posts WHERE slug = $1")
        .bind(slug)
        .fetch_optional(pool)
        .await
        .map_err(|e| AppError::db("check post slug", e))?;
    Ok(row.is_some())
}

/// Returns the live post for a slug in the storefront read
/// shape, or `None` — a draft, a scheduled post and an
/// unknown slug are the same `None` here.
pub async fn find_live_by_slug(pool: &PgPool, slug: &str) -> Result<Option<PostView>, AppError> {
    sqlx::query_as::<_, PostView>(&format!(
        "SELECT {VIEW_COLUMNS} {VIEW_FROM} WHERE p.slug = $1 AND {LIVE_WHERE}"
    ))
    .bind(slug)
    .fetch_optional(pool)
    .await
    .map_err(|e| AppError::db("find live post", e))
}

/// Returns one page of live posts, newest publication
/// first; `category_id` narrows to one category's archive.
pub async fn list_live(
    pool: &PgPool,
    category_id: Option<i64>,
    limit: i64,
    offset: i64,
) -> Result<Vec<PostView>, AppError> {
    sqlx::query_as::<_, PostView>(&format!(
        "SELECT {VIEW_COLUMNS_SUMMARY} {VIEW_FROM} \
         WHERE {LIVE_WHERE} AND ($1::bigint IS NULL OR p.category_id = $1) \
         ORDER BY p.published_at DESC, p.id DESC LIMIT $2 OFFSET $3"
    ))
    .bind(category_id)
    .bind(limit)
    .bind(offset)
    .fetch_all(pool)
    .await
    .map_err(|e| AppError::db("list live posts", e))
}

/// Counts live posts (the index's total); `category_id`
/// narrows to one category's archive.
pub async fn count_live(pool: &PgPool, category_id: Option<i64>) -> Result<i64, AppError> {
    sqlx::query_scalar::<_, i64>(&format!(
        "SELECT count(*) FROM posts \
         WHERE {LIVE_WHERE} AND ($1::bigint IS NULL OR category_id = $1)"
    ))
    .bind(category_id)
    .fetch_one(pool)
    .await
    .map_err(|e| AppError::db("count live posts", e))
}

/// Returns the newest `limit` live posts for the RSS feed.
pub async fn list_live_for_feed(pool: &PgPool, limit: i64) -> Result<Vec<PostView>, AppError> {
    sqlx::query_as::<_, PostView>(&format!(
        "SELECT {VIEW_COLUMNS_SUMMARY} {VIEW_FROM} \
         WHERE {LIVE_WHERE} ORDER BY p.published_at DESC, p.id DESC LIMIT $1"
    ))
    .bind(limit)
    .fetch_all(pool)
    .await
    .map_err(|e| AppError::db("list feed posts", e))
}

/// Slugs and update stamps of every live post, for the
/// sitemap.
pub async fn list_live_for_sitemap(
    pool: &PgPool,
) -> Result<Vec<(String, chrono::DateTime<chrono::Utc>)>, AppError> {
    sqlx::query_as::<_, (String, chrono::DateTime<chrono::Utc>)>(&format!(
        "SELECT slug, updated_at FROM posts WHERE {LIVE_WHERE} ORDER BY slug ASC"
    ))
    .fetch_all(pool)
    .await
    .map_err(|e| AppError::db("list sitemap posts", e))
}

/// Inserts a post inside the caller's transaction and
/// returns its new ID.
pub async fn insert(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    post: &NewPost<'_>,
) -> Result<i64, AppError> {
    let row: (i64,) = sqlx::query_as(
        "INSERT INTO posts (slug, title, body_md, format, excerpt, meta_description, \
                cover_image_path, category_id, author_admin_id, status, published_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11) RETURNING id",
    )
    .bind(post.slug)
    .bind(post.title)
    .bind(post.body_md)
    .bind(post.format)
    .bind(post.excerpt)
    .bind(post.meta_description)
    .bind(post.cover_image_path)
    .bind(post.category_id)
    .bind(post.author_admin_id)
    .bind(post.status)
    .bind(post.published_at)
    .fetch_one(&mut **tx)
    .await
    .map_err(|e| AppError::db("insert post", e))?;
    Ok(row.0)
}

/// Returns the post's current `updated_at` with the row
/// locked: the edit form's staleness guard compares its
/// form ticket's stamp against this value inside the
/// update transaction.
pub async fn updated_at_for_update(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    id: i64,
) -> Result<Option<chrono::DateTime<chrono::Utc>>, AppError> {
    let value = sqlx::query_scalar::<_, chrono::DateTime<chrono::Utc>>(
        "SELECT updated_at FROM posts WHERE id = $1 FOR UPDATE",
    )
    .bind(id)
    .fetch_optional(&mut **tx)
    .await?;
    Ok(value)
}

/// Stores a post's editable fields inside the caller's
/// transaction. The slug and the author are not parameters:
/// neither ever changes after creation. `updated_at` is
/// stamped by the table's trigger.
#[allow(clippy::too_many_arguments)]
pub async fn update(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    id: i64,
    title: &str,
    body_md: &str,
    format: &str,
    excerpt: &str,
    meta_description: &str,
    cover_image_path: Option<&str>,
    category_id: Option<i64>,
    status: &str,
    published_at: Option<chrono::DateTime<chrono::Utc>>,
) -> Result<(), AppError> {
    sqlx::query(
        "UPDATE posts SET title = $1, body_md = $2, format = $3, excerpt = $4, \
                meta_description = $5, cover_image_path = $6, category_id = $7, \
                status = $8, published_at = $9 \
         WHERE id = $10",
    )
    .bind(title)
    .bind(body_md)
    .bind(format)
    .bind(excerpt)
    .bind(meta_description)
    .bind(cover_image_path)
    .bind(category_id)
    .bind(status)
    .bind(published_at)
    .bind(id)
    .execute(&mut **tx)
    .await
    .map_err(|e| AppError::db("update post", e))?;
    Ok(())
}

/// Stores a post's word fields from a revision inside the
/// caller's transaction (the restore): a snapshot holds
/// words only, so the status, category, cover and schedule
/// are not parameters and never move here.
pub async fn update_words(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    id: i64,
    title: &str,
    body_md: &str,
    format: &str,
    excerpt: &str,
    meta_description: &str,
) -> Result<(), AppError> {
    sqlx::query(
        "UPDATE posts SET title = $1, body_md = $2, format = $3, excerpt = $4, \
                meta_description = $5 \
         WHERE id = $6",
    )
    .bind(title)
    .bind(body_md)
    .bind(format)
    .bind(excerpt)
    .bind(meta_description)
    .bind(id)
    .execute(&mut **tx)
    .await
    .map_err(|e| AppError::db("restore post words", e))?;
    Ok(())
}

/// Deletes a post inside the caller's transaction; its
/// revisions go with it (the foreign key cascades).
pub async fn delete(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    id: i64,
) -> Result<(), AppError> {
    sqlx::query("DELETE FROM posts WHERE id = $1")
        .bind(id)
        .execute(&mut **tx)
        .await
        .map_err(|e| AppError::db("delete post", e))?;
    Ok(())
}

/// Snapshots a post's current word fields as a revision
/// inside the caller's transaction. `edited_by` is the
/// admin whose save or restore is recording the snapshot.
pub async fn insert_revision(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    post: &Post,
    edited_by: i64,
) -> Result<(), AppError> {
    sqlx::query(
        "INSERT INTO post_revisions \
         (post_id, title, body_md, format, meta_description, excerpt, edited_by) \
         VALUES ($1, $2, $3, $4, $5, $6, $7)",
    )
    .bind(post.id)
    .bind(&post.title)
    .bind(&post.body_md)
    .bind(&post.format)
    .bind(&post.meta_description)
    .bind(&post.excerpt)
    .bind(edited_by)
    .execute(&mut **tx)
    .await
    .map_err(|e| AppError::db("insert post revision", e))?;
    Ok(())
}

/// Prunes a post's revisions to the newest [`REVISION_CAP`]
/// inside the caller's transaction.
pub async fn prune_revisions(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    post_id: i64,
) -> Result<(), AppError> {
    sqlx::query(
        "DELETE FROM post_revisions \
         WHERE post_id = $1 AND id NOT IN \
               (SELECT id FROM post_revisions WHERE post_id = $1 ORDER BY id DESC LIMIT $2)",
    )
    .bind(post_id)
    .bind(REVISION_CAP)
    .execute(&mut **tx)
    .await
    .map_err(|e| AppError::db("prune post revisions", e))?;
    Ok(())
}

/// Returns a post's revisions, newest first, each with its
/// recording admin's display name.
pub async fn list_revisions(
    pool: &PgPool,
    post_id: i64,
) -> Result<Vec<PostRevisionSummary>, AppError> {
    sqlx::query_as::<_, PostRevisionSummary>(
        "SELECT r.id, r.post_id, r.title, r.body_md, r.format, r.meta_description, r.excerpt, \
                r.edited_by, r.created_at, a.display_name AS editor_name \
         FROM post_revisions r JOIN admins a ON a.id = r.edited_by \
         WHERE r.post_id = $1 ORDER BY r.id DESC",
    )
    .bind(post_id)
    .fetch_all(pool)
    .await
    .map_err(|e| AppError::db("list post revisions", e))
}

/// Returns one revision of one post; a revision ID from
/// another post is the same `None` as a missing one.
pub async fn find_revision(
    pool: &PgPool,
    post_id: i64,
    revision_id: i64,
) -> Result<Option<PostRevision>, AppError> {
    sqlx::query_as::<_, PostRevision>(
        "SELECT id, post_id, title, body_md, format, meta_description, excerpt, edited_by, \
                created_at \
         FROM post_revisions WHERE post_id = $1 AND id = $2",
    )
    .bind(post_id)
    .bind(revision_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| AppError::db("find post revision", e))
}

/// [`find_revision`] inside a caller-owned transaction.
pub async fn find_revision_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    post_id: i64,
    revision_id: i64,
) -> Result<Option<PostRevision>, AppError> {
    sqlx::query_as::<_, PostRevision>(
        "SELECT id, post_id, title, body_md, format, meta_description, excerpt, edited_by, \
                created_at \
         FROM post_revisions WHERE post_id = $1 AND id = $2",
    )
    .bind(post_id)
    .bind(revision_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(|e| AppError::db("find post revision", e))
}
