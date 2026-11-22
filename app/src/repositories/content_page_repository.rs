//! Content page repository: `content_pages` and
//! `content_revisions` (CMS spec C-2).

use sqlx::PgPool;

use crate::error::AppError;
use crate::models::{
    ContentPage, ContentPageSummary, ContentRevision, ContentRevisionSummary, NewContentPage,
};

/// The revision retention cap: only the newest
/// this many snapshots per page are kept.
pub const REVISION_CAP: i64 = 50;

/// The page column set, bound once so every read agrees about
/// the shape.
const PAGE_COLUMNS: &str =
    "id, slug, kind, title, body_md, format, meta_description, status, created_at, updated_at";

/// Returns every page for the admin list, system pages first
/// (in seed order), then custom pages in creation order, each
/// with the name of the admin who recorded its newest revision
/// when one exists.
pub async fn list_all(pool: &PgPool) -> Result<Vec<ContentPageSummary>, AppError> {
    sqlx::query_as::<_, ContentPageSummary>(&format!(
        "SELECT {PAGE_COLUMNS}, \
                (SELECT a.display_name FROM content_revisions r \
                 JOIN admins a ON a.id = r.edited_by \
                 WHERE r.page_id = p.id ORDER BY r.id DESC LIMIT 1) AS last_editor_name \
         FROM content_pages p \
         ORDER BY (kind = 'custom'), id"
    ))
    .fetch_all(pool)
    .await
    .map_err(|e| AppError::db("list content pages", e))
}

/// Returns one page by ID, or `None` when it does not exist.
pub async fn find_by_id(pool: &PgPool, id: i64) -> Result<Option<ContentPage>, AppError> {
    sqlx::query_as::<_, ContentPage>(&format!(
        "SELECT {PAGE_COLUMNS} FROM content_pages WHERE id = $1"
    ))
    .bind(id)
    .fetch_optional(pool)
    .await
    .map_err(|e| AppError::db("find content page", e))
}

/// Returns one page by ID with its row locked for the
/// caller's transaction: a restore reads the page it is
/// about to snapshot and overwrite under this lock, so a
/// concurrent save cannot slip between the read and the
/// write and be silently lost.
pub async fn find_by_id_for_update(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    id: i64,
) -> Result<Option<ContentPage>, AppError> {
    sqlx::query_as::<_, ContentPage>(&format!(
        "SELECT {PAGE_COLUMNS} FROM content_pages WHERE id = $1 FOR UPDATE"
    ))
    .bind(id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(|e| AppError::db("find content page for update", e))
}

/// Returns one page by slug, whatever its status.
pub async fn find_by_slug(pool: &PgPool, slug: &str) -> Result<Option<ContentPage>, AppError> {
    sqlx::query_as::<_, ContentPage>(&format!(
        "SELECT {PAGE_COLUMNS} FROM content_pages WHERE slug = $1"
    ))
    .bind(slug)
    .fetch_optional(pool)
    .await
    .map_err(|e| AppError::db("find content page by slug", e))
}

/// Returns the page the storefront may serve for a slug: it
/// must exist AND be published — a draft and an unknown slug
/// are the same `None` here, which is what makes their 404s
/// indistinguishable.
pub async fn find_published_by_slug(
    pool: &PgPool,
    slug: &str,
) -> Result<Option<ContentPage>, AppError> {
    sqlx::query_as::<_, ContentPage>(&format!(
        "SELECT {PAGE_COLUMNS} FROM content_pages WHERE slug = $1 AND status = 'published'"
    ))
    .bind(slug)
    .fetch_optional(pool)
    .await
    .map_err(|e| AppError::db("find published content page", e))
}

/// Returns the published custom page for a slug, or `None`
/// when the slug is unknown, a draft — or a system page:
/// `/pages/{slug}` serves operator-created pages only, and a
/// system page must not gain a second URL there.
pub async fn find_published_custom_by_slug(
    pool: &PgPool,
    slug: &str,
) -> Result<Option<ContentPage>, AppError> {
    sqlx::query_as::<_, ContentPage>(&format!(
        "SELECT {PAGE_COLUMNS} FROM content_pages \
         WHERE slug = $1 AND status = 'published' AND kind = 'custom'"
    ))
    .bind(slug)
    .fetch_optional(pool)
    .await
    .map_err(|e| AppError::db("find published custom content page", e))
}

/// Whether any page already owns this slug.
pub async fn slug_taken(pool: &PgPool, slug: &str) -> Result<bool, AppError> {
    let row: Option<(i64,)> = sqlx::query_as("SELECT id FROM content_pages WHERE slug = $1")
        .bind(slug)
        .fetch_optional(pool)
        .await
        .map_err(|e| AppError::db("check content page slug", e))?;
    Ok(row.is_some())
}

/// Inserts a page inside the caller's transaction and returns
/// its new ID.
pub async fn insert(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    page: &NewContentPage<'_>,
) -> Result<i64, AppError> {
    let row: (i64,) = sqlx::query_as(
        "INSERT INTO content_pages (slug, kind, title, body_md, format, meta_description, status) \
         VALUES ($1, $2, $3, $4, $5, $6, $7) RETURNING id",
    )
    .bind(page.slug)
    .bind(page.kind)
    .bind(page.title)
    .bind(page.body_md)
    .bind(page.format)
    .bind(page.meta_description)
    .bind(page.status)
    .fetch_one(&mut **tx)
    .await
    .map_err(|e| AppError::db("insert content page", e))?;
    Ok(row.0)
}

/// Returns the page's current `updated_at` with the row
/// locked: the edit form's staleness guard compares its
/// form ticket's stamp against this value inside the
/// update transaction (the product guard's shape).
pub async fn updated_at_for_update(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    id: i64,
) -> Result<Option<chrono::DateTime<chrono::Utc>>, AppError> {
    let value = sqlx::query_scalar::<_, chrono::DateTime<chrono::Utc>>(
        "SELECT updated_at FROM content_pages WHERE id = $1 FOR UPDATE",
    )
    .bind(id)
    .fetch_optional(&mut **tx)
    .await?;
    Ok(value)
}

/// Stores a page's editable fields inside the caller's
/// transaction, stamping `updated_at`. The slug is not a
/// parameter: it never changes after creation.
#[allow(clippy::too_many_arguments)]
pub async fn update(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    id: i64,
    title: &str,
    body_md: &str,
    format: &str,
    meta_description: &str,
    status: &str,
    admin_id: i64,
) -> Result<(), AppError> {
    sqlx::query(
        "UPDATE content_pages SET title = $1, body_md = $2, format = $3, \
                meta_description = $4, status = $5, updated_at = now(), updated_by = $7 \
         WHERE id = $6",
    )
    .bind(title)
    .bind(body_md)
    .bind(format)
    .bind(meta_description)
    .bind(status)
    .bind(id)
    .bind(admin_id)
    .execute(&mut **tx)
    .await
    .map_err(|e| AppError::db("update content page", e))?;
    Ok(())
}

/// Deletes a page inside the caller's transaction; its
/// revisions go with it (the foreign key cascades). The
/// service refuses system pages and published pages before
/// this is ever reached.
pub async fn delete(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    id: i64,
) -> Result<(), AppError> {
    sqlx::query("DELETE FROM content_pages WHERE id = $1")
        .bind(id)
        .execute(&mut **tx)
        .await
        .map_err(|e| AppError::db("delete content page", e))?;
    Ok(())
}

/// Snapshots a page's current fields as a revision inside the
/// caller's transaction. `edited_by` is the admin whose save or
/// restore is recording the snapshot.
pub async fn insert_revision(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    page: &ContentPage,
    edited_by: i64,
) -> Result<(), AppError> {
    sqlx::query(
        "INSERT INTO content_revisions \
         (page_id, title, body_md, format, meta_description, edited_by) \
         VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(page.id)
    .bind(&page.title)
    .bind(&page.body_md)
    .bind(&page.format)
    .bind(&page.meta_description)
    .bind(edited_by)
    .execute(&mut **tx)
    .await
    .map_err(|e| AppError::db("insert content revision", e))?;
    Ok(())
}

/// Prunes a page's revisions to the newest [`REVISION_CAP`]
/// inside the caller's transaction — the same transaction as
/// the publish that created the overflow.
pub async fn prune_revisions(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    page_id: i64,
) -> Result<(), AppError> {
    sqlx::query(
        "DELETE FROM content_revisions \
         WHERE page_id = $1 AND id NOT IN \
               (SELECT id FROM content_revisions WHERE page_id = $1 ORDER BY id DESC LIMIT $2)",
    )
    .bind(page_id)
    .bind(REVISION_CAP)
    .execute(&mut **tx)
    .await
    .map_err(|e| AppError::db("prune content revisions", e))?;
    Ok(())
}

/// Returns a page's revisions, newest first, each with its
/// recording admin's display name.
pub async fn list_revisions(
    pool: &PgPool,
    page_id: i64,
) -> Result<Vec<ContentRevisionSummary>, AppError> {
    sqlx::query_as::<_, ContentRevisionSummary>(
        "SELECT r.id, r.page_id, r.title, r.body_md, r.format, r.meta_description, r.edited_by, \
                r.created_at, a.display_name AS editor_name \
         FROM content_revisions r JOIN admins a ON a.id = r.edited_by \
         WHERE r.page_id = $1 ORDER BY r.id DESC",
    )
    .bind(page_id)
    .fetch_all(pool)
    .await
    .map_err(|e| AppError::db("list content revisions", e))
}

/// Returns one revision of one page; a revision ID from
/// another page is the same `None` as a missing one, so a
/// guessed ID never reaches across pages.
pub async fn find_revision(
    pool: &PgPool,
    page_id: i64,
    revision_id: i64,
) -> Result<Option<ContentRevision>, AppError> {
    sqlx::query_as::<_, ContentRevision>(
        "SELECT id, page_id, title, body_md, format, meta_description, edited_by, created_at \
         FROM content_revisions WHERE page_id = $1 AND id = $2",
    )
    .bind(page_id)
    .bind(revision_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| AppError::db("find content revision", e))
}

/// [`find_revision`] inside a caller-owned transaction.
pub async fn find_revision_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    page_id: i64,
    revision_id: i64,
) -> Result<Option<ContentRevision>, AppError> {
    sqlx::query_as::<_, ContentRevision>(
        "SELECT id, page_id, title, body_md, format, meta_description, edited_by, created_at \
         FROM content_revisions WHERE page_id = $1 AND id = $2",
    )
    .bind(page_id)
    .bind(revision_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(|e| AppError::db("find content revision", e))
}
