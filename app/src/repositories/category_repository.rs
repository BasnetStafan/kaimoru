//! Category repository: the managed category vocabulary.
//!
//! The admin side reads and writes the `categories` table; the
//! storefront reads only the navigation list. Products keep the
//! category name as a snapshot in `products.category`, so a
//! rename rewrites the snapshot here, inside the caller's
//! transaction, and the two never disagree.

use sqlx::PgPool;

use crate::error::AppError;
use crate::models::ProductCategory;

/// Every column [`ProductCategory`] reads, named once so the
/// queries below can never drift apart in shape.
const CATEGORY_COLUMNS: &str = "id, name, slug, description, parent_id, \
    display_order, is_active, created_at, updated_at";

/// [`CATEGORY_COLUMNS`] qualified with the list query's `c.`
/// alias (that query self-joins the table for the parent name,
/// so bare column names would be ambiguous there).
const CATEGORY_COLUMNS_C: &str = "c.id, c.name, c.slug, c.description, c.parent_id, \
    c.display_order, c.is_active, c.created_at, c.updated_at";

/// One admin list row: the category plus the two counts the
/// operator needs beside it — how many products link to it and
/// how many child categories it has. Both are also the delete
/// guard's evidence: a category with either is never deleted.
pub struct CategoryListRow {
    /// The category itself.
    pub category: ProductCategory,
    /// Products linked to the category, any status.
    pub product_count: i64,
    /// Categories whose parent this category is.
    pub child_count: i64,
    /// The parent category's name; `None` at the top level.
    pub parent_name: Option<String>,
}

/// [`CategoryListRow`] plus the filtered total, before splitting.
#[derive(sqlx::FromRow)]
struct CategoryRowWithCounts {
    /// Primary key.
    id: i64,
    /// Display name.
    name: String,
    /// Stable ASCII identifier.
    slug: String,
    /// Operator-facing description.
    description: String,
    /// Parent category ID; `None` at the top level.
    parent_id: Option<i64>,
    /// Ordering weight.
    display_order: i32,
    /// Whether the category is live on the storefront.
    is_active: bool,
    /// Registration instant.
    created_at: chrono::DateTime<chrono::Utc>,
    /// Last edit instant.
    updated_at: chrono::DateTime<chrono::Utc>,
    /// Products linked to the category.
    product_count: i64,
    /// Child categories.
    child_count: i64,
    /// The parent category's name.
    parent_name: Option<String>,
    /// Filtered row total (window function).
    total: i64,
}

/// Returns one page of the admin category list with its filtered
/// total. `query` partial-matches the name and the slug; `status`
/// is `active` or `inactive`; `sort` is one of the fixed keys the
/// service allowlists (`order`, `name`, `products`) — the ORDER
/// BY text itself comes only from the match below, never from
/// request input.
pub async fn list_admin(
    pool: &PgPool,
    pattern: Option<&str>,
    status: Option<&str>,
    sort: &str,
    limit: i64,
    offset: i64,
) -> Result<(Vec<CategoryListRow>, i64), AppError> {
    let order_by = match sort {
        "name" => "c.name ASC, c.id ASC",
        "products" => "product_count DESC, c.name ASC, c.id ASC",
        _ => "c.display_order ASC, c.name ASC, c.id ASC",
    };
    let sql = format!(
        "SELECT {CATEGORY_COLUMNS_C}, \
            (SELECT COUNT(*) FROM products p WHERE p.category_id = c.id) AS product_count, \
            (SELECT COUNT(*) FROM categories ch WHERE ch.parent_id = c.id) AS child_count, \
            par.name AS parent_name, \
            COUNT(*) OVER () AS total \
         FROM categories c \
         LEFT JOIN categories par ON par.id = c.parent_id \
         WHERE ($1::text IS NULL OR c.name ILIKE $1 OR c.slug ILIKE $1) \
           AND ($2::text IS NULL \
                OR ($2 = 'active' AND c.is_active) \
                OR ($2 = 'inactive' AND NOT c.is_active)) \
         ORDER BY {order_by} \
         LIMIT $3 OFFSET $4"
    );
    let rows = sqlx::query_as::<_, CategoryRowWithCounts>(&sql)
        .bind(pattern)
        .bind(status)
        .bind(limit)
        .bind(offset)
        .fetch_all(pool)
        .await
        .map_err(|e| AppError::db("list categories (admin)", e))?;
    if rows.is_empty() && offset > 0 {
        let total = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM categories c \
             WHERE ($1::text IS NULL OR c.name ILIKE $1 OR c.slug ILIKE $1) \
               AND ($2::text IS NULL \
                    OR ($2 = 'active' AND c.is_active) \
                    OR ($2 = 'inactive' AND NOT c.is_active))",
        )
        .bind(pattern)
        .bind(status)
        .fetch_one(pool)
        .await
        .map_err(|e| AppError::db("count categories (admin)", e))?;
        return Ok((Vec::new(), total));
    }
    let total = rows.first().map(|r| r.total).unwrap_or(0);
    Ok((
        rows.into_iter()
            .map(|r| CategoryListRow {
                category: ProductCategory {
                    id: r.id,
                    name: r.name,
                    slug: r.slug,
                    description: r.description,
                    parent_id: r.parent_id,
                    display_order: r.display_order,
                    is_active: r.is_active,
                    created_at: r.created_at,
                    updated_at: r.updated_at,
                },
                product_count: r.product_count,
                child_count: r.child_count,
                parent_name: r.parent_name,
            })
            .collect(),
        total,
    ))
}

/// Returns every category in display order — the product form's
/// select, the category form's parent choices, and the product
/// validation vocabulary.
pub async fn list_all(pool: &PgPool) -> Result<Vec<ProductCategory>, AppError> {
    sqlx::query_as::<_, ProductCategory>(&format!(
        "SELECT {CATEGORY_COLUMNS} FROM categories ORDER BY display_order ASC, name ASC, id ASC"
    ))
    .fetch_all(pool)
    .await
    .map_err(|e| AppError::db("list categories", e))
}

/// Returns the category's current `updated_at` with the row
/// locked: the edit form's staleness guard compares its form
/// ticket's stamp against this value inside the update
/// transaction (the product guard's shape).
pub async fn updated_at_for_update(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    id: i64,
) -> Result<Option<chrono::DateTime<chrono::Utc>>, AppError> {
    let value = sqlx::query_scalar::<_, chrono::DateTime<chrono::Utc>>(
        "SELECT updated_at FROM categories WHERE id = $1 FOR UPDATE",
    )
    .bind(id)
    .fetch_optional(&mut **tx)
    .await?;
    Ok(value)
}

/// Returns one category by ID.
pub async fn find(pool: &PgPool, id: i64) -> Result<Option<ProductCategory>, AppError> {
    sqlx::query_as::<_, ProductCategory>(&format!(
        "SELECT {CATEGORY_COLUMNS} FROM categories WHERE id = $1"
    ))
    .bind(id)
    .fetch_optional(pool)
    .await
    .map_err(|e| AppError::db("find category", e))
}

/// Returns one category by its exact display name.
pub async fn find_by_name(pool: &PgPool, name: &str) -> Result<Option<ProductCategory>, AppError> {
    sqlx::query_as::<_, ProductCategory>(&format!(
        "SELECT {CATEGORY_COLUMNS} FROM categories WHERE name = $1"
    ))
    .bind(name)
    .fetch_optional(pool)
    .await
    .map_err(|e| AppError::db("find category by name", e))
}

/// Whether a category name is already used (case-insensitive),
/// ignoring the row being edited.
pub async fn name_taken(
    pool: &PgPool,
    name: &str,
    exclude_id: Option<i64>,
) -> Result<bool, AppError> {
    super::value_taken(
        pool,
        "categories",
        "name",
        "category name check",
        name,
        exclude_id,
    )
    .await
}

/// Whether a category slug is already used (case-insensitive),
/// ignoring the row being edited.
pub async fn slug_taken(
    pool: &PgPool,
    slug: &str,
    exclude_id: Option<i64>,
) -> Result<bool, AppError> {
    super::value_taken(
        pool,
        "categories",
        "slug",
        "category slug check",
        slug,
        exclude_id,
    )
    .await
}

/// Returns the IDs of every descendant of a category (children,
/// grandchildren, …) — the parent picker must never offer one:
/// choosing it would close a cycle in the hierarchy.
pub async fn descendant_ids(pool: &PgPool, id: i64) -> Result<Vec<i64>, AppError> {
    sqlx::query_scalar::<_, i64>(
        "WITH RECURSIVE tree AS ( \
             SELECT id FROM categories WHERE parent_id = $1 \
             UNION ALL \
             SELECT c.id FROM categories c JOIN tree t ON c.parent_id = t.id \
         ) SELECT id FROM tree",
    )
    .bind(id)
    .fetch_all(pool)
    .await
    .map_err(|e| AppError::db("category descendants", e))
}

/// The writable fields of one category, grouped into one
/// struct (the `ProductFields` pattern) because the create and
/// update parameter lists outgrew clippy's argument limit.
pub struct CategoryFields<'a> {
    /// Display name (doubles as the products' name snapshot).
    pub name: &'a str,
    /// Stable ASCII identifier.
    pub slug: &'a str,
    /// Free-text description.
    pub description: &'a str,
    /// Parent category ID, when nested.
    pub parent_id: Option<i64>,
    /// Ordering weight.
    pub display_order: i32,
    /// Whether the category is live.
    pub is_active: bool,
}

/// Creates one category inside the caller's transaction.
pub async fn create(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    fields: &CategoryFields<'_>,
) -> Result<i64, AppError> {
    sqlx::query_scalar::<_, i64>(
        "INSERT INTO categories (name, slug, description, parent_id, display_order, is_active) \
         VALUES ($1, $2, $3, $4, $5, $6) RETURNING id",
    )
    .bind(fields.name)
    .bind(fields.slug)
    .bind(fields.description)
    .bind(fields.parent_id)
    .bind(fields.display_order)
    .bind(fields.is_active)
    .fetch_one(&mut **tx)
    .await
    .map_err(|e| AppError::db("create category", e))
}

/// Updates one category inside the caller's transaction,
/// stamping `updated_at` in the statement.
pub async fn update(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    id: i64,
    fields: &CategoryFields<'_>,
    admin_id: i64,
) -> Result<(), AppError> {
    sqlx::query(
        "UPDATE categories SET name = $2, slug = $3, description = $4, \
                parent_id = $5, display_order = $6, is_active = $7, updated_at = now(), updated_by = $8 \
         WHERE id = $1",
    )
    .bind(id)
    .bind(fields.name)
    .bind(fields.slug)
    .bind(fields.description)
    .bind(fields.parent_id)
    .bind(fields.display_order)
    .bind(fields.is_active)
    .bind(admin_id)
    .execute(&mut **tx)
    .await
    .map_err(|e| AppError::db("update category", e))?;
    Ok(())
}

/// Rewrites the category name snapshot on every product linked
/// to the category — the second half of a rename, in the same
/// transaction, so storefront search by the old name never
/// outlives the rename.
pub async fn sync_product_snapshots(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    category_id: i64,
    new_name: &str,
) -> Result<(), AppError> {
    sqlx::query("UPDATE products SET category = $2 WHERE category_id = $1")
        .bind(category_id)
        .bind(new_name)
        .execute(&mut **tx)
        .await
        .map_err(|e| AppError::db("sync product category snapshots", e))?;
    Ok(())
}

/// How many products link to a category, read inside the
/// caller's transaction (the delete guard).
pub async fn product_count(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    id: i64,
) -> Result<i64, AppError> {
    sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM products WHERE category_id = $1")
        .bind(id)
        .fetch_one(&mut **tx)
        .await
        .map_err(|e| AppError::db("category product count", e))
}

/// How many categories name this one as their parent, read
/// inside the caller's transaction (the delete guard).
pub async fn child_count(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    id: i64,
) -> Result<i64, AppError> {
    sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM categories WHERE parent_id = $1")
        .bind(id)
        .fetch_one(&mut **tx)
        .await
        .map_err(|e| AppError::db("category child count", e))
}

/// Deletes one category inside the caller's transaction and
/// returns how many rows were removed, so the caller can audit
/// only an actual deletion.
pub async fn delete(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    id: i64,
) -> Result<u64, AppError> {
    let res = sqlx::query("DELETE FROM categories WHERE id = $1")
        .bind(id)
        .execute(&mut **tx)
        .await
        .map_err(|e| AppError::db("delete category", e))?;
    Ok(res.rows_affected())
}

/// Category names for the storefront navigation: active
/// categories that hold at least one public product, in display
/// order. A category with nothing on sale is not navigation, and
/// an inactive category is never offered — the two rules the top
/// page's category row is built on.
pub async fn nav_categories(pool: &PgPool) -> Result<Vec<String>, sqlx::Error> {
    sqlx::query_scalar::<_, String>(
        "SELECT c.name FROM categories c \
         WHERE c.is_active \
           AND EXISTS (SELECT 1 FROM products p \
                       WHERE p.category_id = c.id AND p.status = 'public') \
         ORDER BY c.display_order ASC, c.name ASC, c.id ASC",
    )
    .fetch_all(pool)
    .await
}

/// Whether one category is active; `None` when it does not exist.
pub async fn is_active(pool: &PgPool, id: i64) -> Result<Option<bool>, AppError> {
    sqlx::query_scalar::<_, bool>("SELECT is_active FROM categories WHERE id = $1")
        .bind(id)
        .fetch_optional(pool)
        .await
        .map_err(|e| AppError::db("category active check", e))
}
