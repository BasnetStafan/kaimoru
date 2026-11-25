//! Tag repository: the tag vocabulary and the product tag links.
//!
//! Every query on the `tags` / `product_tags` tables lives here —
//! the admin product form's vocabulary reads, the tag management
//! page's CRUD, and the storefront's per-product tag reads. (The
//! product-form reads moved here from `product_repository`
//! when tag management landed, so the vocabulary has one home.)

use std::collections::HashMap;

use sqlx::PgPool;

use crate::error::AppError;
use crate::models::Tag;

/// The tags of several products at once, keyed by product
/// id — the catalog API resolves a whole page's tags in one
/// query instead of one query per product. Products without
/// tags are absent from the map.
pub async fn tags_for_products(
    pool: &PgPool,
    product_ids: &[i64],
) -> Result<HashMap<i64, Vec<Tag>>, AppError> {
    if product_ids.is_empty() {
        return Ok(HashMap::new());
    }
    // A flat row: sqlx maps one row to one struct, so the
    // product id rides alongside the tag's own columns and the
    // `Tag` is assembled in the loop below.
    #[derive(sqlx::FromRow)]
    struct ProductTagRow {
        product_id: i64,
        id: i64,
        slug: String,
        label: String,
        description: String,
    }
    let rows: Vec<ProductTagRow> = sqlx::query_as(&format!(
        "SELECT pt.product_id, {TAG_COLUMNS} FROM product_tags pt \
         JOIN tags ON tags.id = pt.tag_id \
         WHERE pt.product_id = ANY($1) ORDER BY pt.product_id, tags.label"
    ))
    .bind(product_ids)
    .fetch_all(pool)
    .await?;
    let mut by_product: HashMap<i64, Vec<Tag>> = HashMap::new();
    for row in rows {
        by_product.entry(row.product_id).or_default().push(Tag {
            id: row.id,
            slug: row.slug,
            label: row.label,
            description: row.description,
        });
    }
    Ok(by_product)
}

/// Every column [`Tag`] reads, named once.
const TAG_COLUMNS: &str = "id, slug, label, description";

/// The whole tag vocabulary, in seed order — the product form's
/// checkbox list, its validation vocabulary and the admin list
/// filter's options.
pub async fn list_tags(pool: &PgPool) -> Result<Vec<Tag>, AppError> {
    sqlx::query_as::<_, Tag>(&format!("SELECT {TAG_COLUMNS} FROM tags ORDER BY id"))
        .fetch_all(pool)
        .await
        .map_err(|e| AppError::db("list tags", e))
}

/// The tag IDs currently attached to one product.
pub async fn product_tag_ids(pool: &PgPool, product_id: i64) -> Result<Vec<i64>, AppError> {
    sqlx::query_scalar::<_, i64>(
        "SELECT tag_id FROM product_tags WHERE product_id = $1 ORDER BY tag_id",
    )
    .bind(product_id)
    .fetch_all(pool)
    .await
    .map_err(|e| AppError::db("product tag ids", e))
}

/// Replaces one product's tag set inside the caller's
/// transaction. The IDs were validated against the vocabulary
/// by the service before this runs.
pub async fn replace_product_tags(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    product_id: i64,
    tag_ids: &[i64],
) -> Result<(), AppError> {
    sqlx::query("DELETE FROM product_tags WHERE product_id = $1")
        .bind(product_id)
        .execute(&mut **tx)
        .await
        .map_err(|e| AppError::db("clear product tags", e))?;
    for tag_id in tag_ids {
        sqlx::query("INSERT INTO product_tags (product_id, tag_id) VALUES ($1, $2)")
            .bind(product_id)
            .bind(tag_id)
            .execute(&mut **tx)
            .await
            .map_err(|e| AppError::db("insert product tag", e))?;
    }
    Ok(())
}

/// The tags attached to one product, in vocabulary order — the
/// product detail page's tag row.
pub async fn tags_of_product(pool: &PgPool, product_id: i64) -> Result<Vec<Tag>, AppError> {
    sqlx::query_as::<_, Tag>(
        "SELECT t.id, t.slug, t.label, t.description FROM tags t \
         JOIN product_tags pt ON pt.tag_id = t.id \
         WHERE pt.product_id = $1 ORDER BY t.id",
    )
    .bind(product_id)
    .fetch_all(pool)
    .await
    .map_err(|e| AppError::db("tags of product", e))
}

/// Returns one tag by ID.
pub async fn find(pool: &PgPool, id: i64) -> Result<Option<Tag>, AppError> {
    sqlx::query_as::<_, Tag>(&format!("SELECT {TAG_COLUMNS} FROM tags WHERE id = $1"))
        .bind(id)
        .fetch_optional(pool)
        .await
        .map_err(|e| AppError::db("find tag", e))
}

/// Returns one tag by its slug — the storefront's tag filter
/// resolves the label it displays from the slug in the URL.
pub async fn find_by_slug(pool: &PgPool, slug: &str) -> Result<Option<Tag>, AppError> {
    sqlx::query_as::<_, Tag>(&format!("SELECT {TAG_COLUMNS} FROM tags WHERE slug = $1"))
        .bind(slug)
        .fetch_optional(pool)
        .await
        .map_err(|e| AppError::db("find tag by slug", e))
}

/// Whether a tag label is already used (case-insensitive),
/// ignoring the row being edited.
pub async fn label_taken(
    pool: &PgPool,
    label: &str,
    exclude_id: Option<i64>,
) -> Result<bool, AppError> {
    super::value_taken(pool, "tags", "label", "tag label check", label, exclude_id).await
}

/// Whether a tag slug is already used (case-insensitive),
/// ignoring the row being edited.
pub async fn slug_taken(
    pool: &PgPool,
    slug: &str,
    exclude_id: Option<i64>,
) -> Result<bool, AppError> {
    super::value_taken(pool, "tags", "slug", "tag slug check", slug, exclude_id).await
}

/// One admin list row: the tag plus how many products carry it.
pub struct TagListRow {
    /// The tag itself.
    pub tag: Tag,
    /// Products carrying the tag.
    pub product_count: i64,
}

/// [`TagListRow`] plus the filtered total, before splitting.
#[derive(sqlx::FromRow)]
struct TagRowWithCount {
    /// Primary key.
    id: i64,
    /// Stable ASCII identifier.
    slug: String,
    /// Japanese label.
    label: String,
    /// Operator-facing description.
    description: String,
    /// Products carrying the tag.
    product_count: i64,
    /// Filtered row total (window function).
    total: i64,
}

/// Returns one page of the admin tag list with its filtered
/// total. `query` partial-matches the label and the slug; `sort`
/// is one of the fixed keys the service allowlists (`id`,
/// `label`, `slug`, `products`) — the ORDER BY text comes only
/// from the match below.
pub async fn list_admin(
    pool: &PgPool,
    pattern: Option<&str>,
    sort: &str,
    limit: i64,
    offset: i64,
) -> Result<(Vec<TagListRow>, i64), AppError> {
    let order_by = match sort {
        "label" => "t.label ASC, t.id ASC",
        "slug" => "t.slug ASC, t.id ASC",
        "products" => "product_count DESC, t.id ASC",
        _ => "t.id ASC",
    };
    let sql = format!(
        "SELECT t.id, t.slug, t.label, t.description, \
            (SELECT COUNT(*) FROM product_tags pt WHERE pt.tag_id = t.id) AS product_count, \
            COUNT(*) OVER () AS total \
         FROM tags t \
         WHERE ($1::text IS NULL OR t.label ILIKE $1 OR t.slug ILIKE $1) \
         ORDER BY {order_by} \
         LIMIT $2 OFFSET $3"
    );
    let rows = sqlx::query_as::<_, TagRowWithCount>(&sql)
        .bind(pattern)
        .bind(limit)
        .bind(offset)
        .fetch_all(pool)
        .await
        .map_err(|e| AppError::db("list tags (admin)", e))?;
    if rows.is_empty() && offset > 0 {
        let total = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM tags t \
             WHERE ($1::text IS NULL OR t.label ILIKE $1 OR t.slug ILIKE $1)",
        )
        .bind(pattern)
        .fetch_one(pool)
        .await
        .map_err(|e| AppError::db("count tags (admin)", e))?;
        return Ok((Vec::new(), total));
    }
    let total = rows.first().map(|r| r.total).unwrap_or(0);
    Ok((
        rows.into_iter()
            .map(|r| TagListRow {
                tag: Tag {
                    id: r.id,
                    slug: r.slug,
                    label: r.label,
                    description: r.description,
                },
                product_count: r.product_count,
            })
            .collect(),
        total,
    ))
}

/// Creates one tag inside the caller's transaction.
pub async fn create(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    slug: &str,
    label: &str,
    description: &str,
) -> Result<i64, AppError> {
    sqlx::query_scalar::<_, i64>(
        "INSERT INTO tags (slug, label, description) VALUES ($1, $2, $3) RETURNING id",
    )
    .bind(slug)
    .bind(label)
    .bind(description)
    .fetch_one(&mut **tx)
    .await
    .map_err(|e| AppError::db("create tag", e))
}

/// The tag's current `updated_at`: the stamp an edit form
/// ticket is minted with at render time. (The `Tag` row
/// struct does not carry the stamp, so it is read on its
/// own, like the member profile's.)
pub async fn updated_at(
    pool: &PgPool,
    id: i64,
) -> Result<Option<chrono::DateTime<chrono::Utc>>, AppError> {
    let value = sqlx::query_scalar::<_, chrono::DateTime<chrono::Utc>>(
        "SELECT updated_at FROM tags WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?;
    Ok(value)
}

/// Returns the tag's current `updated_at` with the row
/// locked: the edit form's staleness guard compares its
/// form ticket's stamp against this value inside the
/// update transaction (the product guard's shape).
pub async fn updated_at_for_update(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    id: i64,
) -> Result<Option<chrono::DateTime<chrono::Utc>>, AppError> {
    let value = sqlx::query_scalar::<_, chrono::DateTime<chrono::Utc>>(
        "SELECT updated_at FROM tags WHERE id = $1 FOR UPDATE",
    )
    .bind(id)
    .fetch_optional(&mut **tx)
    .await?;
    Ok(value)
}

/// Updates one tag inside the caller's transaction.
pub async fn update(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    id: i64,
    slug: &str,
    label: &str,
    description: &str,
    admin_id: i64,
) -> Result<(), AppError> {
    sqlx::query(
        "UPDATE tags SET slug = $2, label = $3, description = $4, updated_by = $5 WHERE id = $1",
    )
    .bind(id)
    .bind(slug)
    .bind(label)
    .bind(description)
    .bind(admin_id)
    .execute(&mut **tx)
    .await
    .map_err(|e| AppError::db("update tag", e))?;
    Ok(())
}

/// Removes every product link of one tag inside the caller's
/// transaction — the first half of a tag deletion: a deleted tag
/// must never leave a dangling reference on a product.
pub async fn unlink_products(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    id: i64,
) -> Result<u64, AppError> {
    let res = sqlx::query("DELETE FROM product_tags WHERE tag_id = $1")
        .bind(id)
        .execute(&mut **tx)
        .await
        .map_err(|e| AppError::db("unlink tag products", e))?;
    Ok(res.rows_affected())
}

/// Deletes one tag inside the caller's transaction and returns
/// how many rows were removed, so the caller can audit only an
/// actual deletion.
pub async fn delete(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    id: i64,
) -> Result<u64, AppError> {
    let res = sqlx::query("DELETE FROM tags WHERE id = $1")
        .bind(id)
        .execute(&mut **tx)
        .await
        .map_err(|e| AppError::db("delete tag", e))?;
    Ok(res.rows_affected())
}
