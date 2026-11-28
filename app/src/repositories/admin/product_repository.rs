//! Admin-side product queries.
//!
//! The admin reads products in every publication state — the
//! storefront repository hides drafts and archives, this one
//! exists to manage them. Writes run inside the caller's
//! transaction so the service can append the audit row in the
//! same commit.

use sqlx::PgPool;

use crate::error::AppError;
use crate::models::{AdminProduct, AdminProductListRow};

/// The stock expression of the admin reads: the sum over the
/// product variants, cast to INTEGER. Stock lives on variants
/// only; the admin list and form show the total.
const STOCK_SUM: &str = "(SELECT COALESCE(SUM(stock), 0)::int FROM product_variants \
                          WHERE product_id = products.id AND is_enabled)";

/// [`STOCK_SUM`] for the list query's `p.` alias.
const STOCK_SUM_P: &str = "(SELECT COALESCE(SUM(stock), 0)::int FROM product_variants \
                            WHERE product_id = p.id AND is_enabled)";

/// Every column [`AdminProduct`] reads, named — never `SELECT *`:
/// the products table carries more than this (the retired
/// `rating` column among them), and this type's shape must
/// change only when this list does.
const PRODUCT_COLUMNS: &str = "id, name, description, price_yen, category, category_id, \
                               image_path, is_event, status, created_at, updated_at, \
                               product_code, brand, manufacturer, country_of_origin, \
                               spec_text, features_text, description_sp, search_keywords, \
                               tax_class_id, cost_yen, sale_price_yen, sale_starts_at, \
                               sale_ends_at, is_new, discount_percent, is_preorder, \
                               is_staff_pick, display_order, slug, seo_title, \
                               meta_description, track_stock, allow_backorder, \
                               low_stock_threshold, weight_grams, size_length_mm, \
                               size_width_mm, size_height_mm, cool_delivery, mail_delivery, \
                               free_shipping, undeliverable_regions, lead_time_days, \
                               publish_starts_at, publish_ends_at, points_earn_percent";

/// [`PRODUCT_COLUMNS`] with the list query's `p.` prefix.
const PRODUCT_COLUMNS_P: &str = "p.id, p.name, p.description, p.price_yen, \
                                 p.category, p.category_id, p.image_path, p.is_event, p.status, \
                                 p.created_at, p.updated_at, \
                                 p.product_code, p.brand, p.manufacturer, p.country_of_origin, \
                                 p.spec_text, p.features_text, p.description_sp, \
                                 p.search_keywords, p.tax_class_id, p.cost_yen, \
                                 p.sale_price_yen, p.sale_starts_at, p.sale_ends_at, \
                                 p.is_new, p.discount_percent, p.is_preorder, \
                                 p.is_staff_pick, p.display_order, p.slug, p.seo_title, \
                                 p.meta_description, p.track_stock, p.allow_backorder, \
                                 p.low_stock_threshold, p.weight_grams, p.size_length_mm, \
                                 p.size_width_mm, p.size_height_mm, p.cool_delivery, \
                                 p.mail_delivery, p.free_shipping, \
                                 p.undeliverable_regions, p.lead_time_days, \
                                 p.publish_starts_at, p.publish_ends_at, p.points_earn_percent";

/// The product LIST page's columns, with the `p.` prefix:
/// only what `templates/admin/pages/products/list.html` renders
/// (plus price and updated_at, which the controller formats
/// into the row's text fields). The long text columns —
/// description, spec, features, SEO — are deliberately not
/// in this list; see [`AdminProductListRow`].
const LIST_COLUMNS_P: &str = "p.id, p.name, p.price_yen, \
                              p.category, p.image_path, p.status, p.updated_at";

/// The filters of one admin product-list request, already validated
/// by the service: every value is bound as a parameter, and `sort`
/// selects one of the fixed ORDER BY constants in
/// [`list_products`] — user input never reaches the SQL text.
pub struct AdminProductQuery<'a> {
    /// Tab value: `all`, `public`, `draft` or `archived`
    /// (`all` excludes archived).
    pub tab: &'a str,
    /// Partial-match pattern for name + category, pre-wrapped
    /// in `%…%` by the service; `None` for no keyword.
    pub keyword: Option<String>,
    /// Exact category value; `None` for all categories.
    pub category: Option<String>,
    /// Tag ID filter; `None` for all tags.
    pub tag_id: Option<i64>,
    /// Sort key from the fixed allowlist.
    pub sort: &'a str,
    /// Page size and offset.
    pub limit: i64,
    /// Page size and offset.
    pub offset: i64,
}

/// Returns one page of the admin product list plus the total
/// count of rows matching the same filters. The FULL row:
/// the CSV export consumes every column, so it reads here.
pub async fn list_products(
    pool: &PgPool,
    q: &AdminProductQuery<'_>,
) -> Result<(Vec<AdminProduct>, i64), AppError> {
    list_rows(pool, q, PRODUCT_COLUMNS_P).await
}

/// The list page's own read: the same filters, ordering and
/// pagination as [`list_products`], but only the columns the
/// list renders (the `LIST_COLUMNS_P` list).
pub async fn list_products_page(
    pool: &PgPool,
    q: &AdminProductQuery<'_>,
) -> Result<(Vec<AdminProductListRow>, i64), AppError> {
    list_rows(pool, q, LIST_COLUMNS_P).await
}

/// One list row plus the pagination total, before splitting.
#[derive(sqlx::FromRow)]
struct RowWithTotal<T>
where
    T: for<'r> sqlx::FromRow<'r, sqlx::postgres::PgRow>,
{
    /// The row itself.
    #[sqlx(flatten)]
    row: T,
    /// The total rows matching the same filters.
    total: i64,
}

/// The shared body of the two product-list reads: identical
/// filters, ordering and pagination — only the SELECT list,
/// and with it the row shape, differs.
async fn list_rows<T>(
    pool: &PgPool,
    q: &AdminProductQuery<'_>,
    columns: &str,
) -> Result<(Vec<T>, i64), AppError>
where
    T: for<'r> sqlx::FromRow<'r, sqlx::postgres::PgRow> + Send + Unpin,
{
    // The ORDER BY is chosen from fixed constants only; the
    // service maps the request's sort value onto this list.
    let order_by = match q.sort {
        "old" => "p.created_at ASC, p.id ASC",
        "updated" => "p.updated_at DESC, p.id DESC",
        "name" => "p.name ASC, p.id ASC",
        "price_asc" => "p.price_yen ASC, p.id ASC",
        "price_desc" => "p.price_yen DESC, p.id ASC",
        "stock_asc" => "stock ASC, p.id ASC",
        "stock_desc" => "stock DESC, p.id ASC",
        _ => "p.created_at DESC, p.id DESC",
    };
    let status_cond = match q.tab {
        "public" => "p.status = 'public'",
        "draft" => "p.status = 'draft'",
        "archived" => "p.status = 'archived'",
        _ => "p.status <> 'archived'",
    };
    let sql = format!(
        "SELECT {columns}, {STOCK_SUM_P} AS stock, COUNT(*) OVER () AS total \
         FROM products p \
         WHERE {status_cond} \
           AND ($1::text IS NULL OR p.name ILIKE $1 OR p.category ILIKE $1) \
           AND ($2::text IS NULL OR p.category = $2) \
           AND ($3::bigint IS NULL OR EXISTS ( \
               SELECT 1 FROM product_tags pt WHERE pt.product_id = p.id AND pt.tag_id = $3)) \
         ORDER BY {order_by} \
         LIMIT $4 OFFSET $5"
    );
    let rows = sqlx::query_as::<_, RowWithTotal<T>>(&sql)
        .bind(&q.keyword)
        .bind(&q.category)
        .bind(q.tag_id)
        .bind(q.limit)
        .bind(q.offset)
        .fetch_all(pool)
        .await
        .map_err(|e| AppError::db("list admin products", e))?;
    // A page past the end returns no rows to carry the total;
    // only then is a separate count worth its round trip.
    if rows.is_empty() && q.offset > 0 {
        let count_sql = format!(
            "SELECT COUNT(*) FROM products p \
             WHERE {status_cond} \
               AND ($1::text IS NULL OR p.name ILIKE $1 OR p.category ILIKE $1) \
               AND ($2::text IS NULL OR p.category = $2) \
               AND ($3::bigint IS NULL OR EXISTS ( \
                   SELECT 1 FROM product_tags pt \
                   WHERE pt.product_id = p.id AND pt.tag_id = $3))"
        );
        let total = sqlx::query_scalar::<_, i64>(&count_sql)
            .bind(&q.keyword)
            .bind(&q.category)
            .bind(q.tag_id)
            .fetch_one(pool)
            .await
            .map_err(|e| AppError::db("count admin products", e))?;
        return Ok((Vec::new(), total));
    }
    let total = rows.first().map(|r| r.total).unwrap_or(0);
    Ok((rows.into_iter().map(|r| r.row).collect(), total))
}

/// Per-tab counts for the product list's tab labels:
/// (all-excluding-archived, public, draft, archived).
pub async fn tab_counts(pool: &PgPool) -> Result<(i64, i64, i64, i64), AppError> {
    let row = sqlx::query_as::<_, (i64, i64, i64, i64)>(
        "SELECT \
           COUNT(*) FILTER (WHERE status <> 'archived'), \
           COUNT(*) FILTER (WHERE status = 'public'), \
           COUNT(*) FILTER (WHERE status = 'draft'), \
           COUNT(*) FILTER (WHERE status = 'archived') \
         FROM products",
    )
    .fetch_one(pool)
    .await
    .map_err(|e| AppError::db("admin product tab counts", e))?;
    Ok(row)
}

/// Finds one product in any publication state, for the admin edit
/// screen. Admin-only: the storefront's `find_by_id` hides
/// non-public products, and this one must never serve it.
pub async fn find_product(pool: &PgPool, id: i64) -> Result<Option<AdminProduct>, AppError> {
    sqlx::query_as::<_, AdminProduct>(&format!(
        "SELECT {PRODUCT_COLUMNS}, {STOCK_SUM} AS stock FROM products WHERE id = $1"
    ))
    .bind(id)
    .fetch_optional(pool)
    .await
    .map_err(|e| AppError::db("find admin product", e))
}

/// The validated field set of a create/update, shared by both.
pub struct ProductFields<'a> {
    /// Product name.
    pub name: &'a str,
    /// Description.
    pub description: &'a str,
    /// Tax-inclusive price in integer yen.
    pub price_yen: i64,
    /// Category name snapshot (synced from the linked category).
    pub category: &'a str,
    /// The managed category's ID; `None` only when a caller has
    /// no category to link (the product form always links one).
    pub category_id: Option<i64>,
    /// Served image path (`/static/images/…`).
    pub image_path: &'a str,
    /// Event product flag.
    pub is_event: bool,
    /// Publication status (`public` or `draft` from the form).
    pub status: &'a str,
    /// Operator-facing product code; unique when set.
    pub product_code: Option<&'a str>,
    /// Brand name.
    pub brand: &'a str,
    /// Manufacturer name.
    pub manufacturer: &'a str,
    /// Country of origin.
    pub country_of_origin: &'a str,
    /// Specification text.
    pub spec_text: &'a str,
    /// Selling-points text.
    pub features_text: &'a str,
    /// Smartphone description override.
    pub description_sp: &'a str,
    /// Extra search keywords.
    pub search_keywords: &'a str,
    /// Tax class ID.
    pub tax_class_id: i64,
    /// Cost in yen (admin-only).
    pub cost_yen: Option<i64>,
    /// Sale price in yen.
    pub sale_price_yen: Option<i64>,
    /// Sale window start.
    pub sale_starts_at: Option<chrono::DateTime<chrono::Utc>>,
    /// Sale window end.
    pub sale_ends_at: Option<chrono::DateTime<chrono::Utc>>,
    /// 新着 badge flag.
    pub is_new: bool,
    /// Label-only promotion percentage.
    pub discount_percent: i32,
    /// 予約 flag.
    pub is_preorder: bool,
    /// おすすめ flag.
    pub is_staff_pick: bool,
    /// Default-sort position.
    pub display_order: i32,
    /// URL slug; unique when set.
    pub slug: Option<&'a str>,
    /// Title override.
    pub seo_title: &'a str,
    /// Meta description.
    pub meta_description: &'a str,
    /// Stock tracking on/off.
    pub track_stock: bool,
    /// Oversell allowed.
    pub allow_backorder: bool,
    /// Low-stock threshold override.
    pub low_stock_threshold: Option<i32>,
    /// Points earn-rate override.
    pub points_earn_percent: Option<i32>,
    /// Weight in grams.
    pub weight_grams: Option<i32>,
    /// Parcel length in millimetres.
    pub size_length_mm: Option<i32>,
    /// Parcel width in millimetres.
    pub size_width_mm: Option<i32>,
    /// Parcel height in millimetres.
    pub size_height_mm: Option<i32>,
    /// Chilled delivery.
    pub cool_delivery: bool,
    /// Mail delivery.
    pub mail_delivery: bool,
    /// Always free shipping.
    pub free_shipping: bool,
    /// Comma-separated undeliverable prefectures.
    pub undeliverable_regions: &'a str,
    /// Days until dispatch.
    pub lead_time_days: Option<i32>,
    /// Publication window start.
    pub publish_starts_at: Option<chrono::DateTime<chrono::Utc>>,
    /// Publication window end.
    pub publish_ends_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// Inserts one product inside the caller's transaction and
/// returns its ID.
pub async fn create_product(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    f: &ProductFields<'_>,
) -> Result<i64, AppError> {
    sqlx::query_scalar::<_, i64>(
        "INSERT INTO products \
             (name, description, price_yen, category, category_id, image_path, is_event, status, \
             product_code, brand, manufacturer, country_of_origin, spec_text, \
             features_text, description_sp, search_keywords, tax_class_id, cost_yen, \
             sale_price_yen, sale_starts_at, sale_ends_at, is_new, discount_percent, \
             is_preorder, is_staff_pick, display_order, slug, seo_title, \
             meta_description, track_stock, allow_backorder, low_stock_threshold, \
             weight_grams, size_length_mm, size_width_mm, size_height_mm, \
             cool_delivery, mail_delivery, free_shipping, undeliverable_regions, \
             lead_time_days, publish_starts_at, publish_ends_at, points_earn_percent) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, \
             $16, $17, $18, $19, $20, $21, $22, $23, $24, $25, $26, $27, $28, $29, \
             $30, $31, $32, $33, $34, $35, $36, $37, $38, $39, $40, $41, $42, $43, $44) \
         RETURNING id",
    )
    .bind(f.name)
    .bind(f.description)
    .bind(f.price_yen)
    .bind(f.category)
    .bind(f.category_id)
    .bind(f.image_path)
    .bind(f.is_event)
    .bind(f.status)
    .bind(f.product_code)
    .bind(f.brand)
    .bind(f.manufacturer)
    .bind(f.country_of_origin)
    .bind(f.spec_text)
    .bind(f.features_text)
    .bind(f.description_sp)
    .bind(f.search_keywords)
    .bind(f.tax_class_id)
    .bind(f.cost_yen)
    .bind(f.sale_price_yen)
    .bind(f.sale_starts_at)
    .bind(f.sale_ends_at)
    .bind(f.is_new)
    .bind(f.discount_percent)
    .bind(f.is_preorder)
    .bind(f.is_staff_pick)
    .bind(f.display_order)
    .bind(f.slug)
    .bind(f.seo_title)
    .bind(f.meta_description)
    .bind(f.track_stock)
    .bind(f.allow_backorder)
    .bind(f.low_stock_threshold)
    .bind(f.weight_grams)
    .bind(f.size_length_mm)
    .bind(f.size_width_mm)
    .bind(f.size_height_mm)
    .bind(f.cool_delivery)
    .bind(f.mail_delivery)
    .bind(f.free_shipping)
    .bind(f.undeliverable_regions)
    .bind(f.lead_time_days)
    .bind(f.publish_starts_at)
    .bind(f.publish_ends_at)
    .bind(f.points_earn_percent)
    .fetch_one(&mut **tx)
    .await
    .map_err(|e| AppError::db("create product", e))
}

/// Updates one product's editable fields inside the caller's
/// transaction, stamping `updated_at`. The status is not set here
/// beyond what the form carries; archive transitions use
/// [`set_product_status`] so they audit distinctly.
pub async fn update_product(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    id: i64,
    f: &ProductFields<'_>,
    admin_id: i64,
) -> Result<(), AppError> {
    sqlx::query(
        "UPDATE products SET name = $2, description = $3, price_yen = $4, \
             category = $5, category_id = $6, image_path = $7, is_event = $8, \
             status = $9, product_code = $10, brand = $11, manufacturer = $12, \
             country_of_origin = $13, spec_text = $14, features_text = $15, \
             description_sp = $16, search_keywords = $17, tax_class_id = $18, \
             cost_yen = $19, sale_price_yen = $20, sale_starts_at = $21, \
             sale_ends_at = $22, is_new = $23, discount_percent = $24, \
             is_preorder = $25, is_staff_pick = $26, display_order = $27, \
             slug = $28, seo_title = $29, meta_description = $30, \
             track_stock = $31, allow_backorder = $32, low_stock_threshold = $33, \
             weight_grams = $34, size_length_mm = $35, size_width_mm = $36, \
             size_height_mm = $37, cool_delivery = $38, mail_delivery = $39, \
             free_shipping = $40, undeliverable_regions = $41, \
             lead_time_days = $42, publish_starts_at = $43, \
             publish_ends_at = $44, points_earn_percent = $46, updated_at = now(), updated_by = $45 \
         WHERE id = $1",
    )
    .bind(id)
    .bind(f.name)
    .bind(f.description)
    .bind(f.price_yen)
    .bind(f.category)
    .bind(f.category_id)
    .bind(f.image_path)
    .bind(f.is_event)
    .bind(f.status)
    .bind(f.product_code)
    .bind(f.brand)
    .bind(f.manufacturer)
    .bind(f.country_of_origin)
    .bind(f.spec_text)
    .bind(f.features_text)
    .bind(f.description_sp)
    .bind(f.search_keywords)
    .bind(f.tax_class_id)
    .bind(f.cost_yen)
    .bind(f.sale_price_yen)
    .bind(f.sale_starts_at)
    .bind(f.sale_ends_at)
    .bind(f.is_new)
    .bind(f.discount_percent)
    .bind(f.is_preorder)
    .bind(f.is_staff_pick)
    .bind(f.display_order)
    .bind(f.slug)
    .bind(f.seo_title)
    .bind(f.meta_description)
    .bind(f.track_stock)
    .bind(f.allow_backorder)
    .bind(f.low_stock_threshold)
    .bind(f.weight_grams)
    .bind(f.size_length_mm)
    .bind(f.size_width_mm)
    .bind(f.size_height_mm)
    .bind(f.cool_delivery)
    .bind(f.mail_delivery)
    .bind(f.free_shipping)
    .bind(f.undeliverable_regions)
    .bind(f.lead_time_days)
    .bind(f.publish_starts_at)
    .bind(f.publish_ends_at)
    .bind(admin_id)
    .bind(f.points_earn_percent)
    .execute(&mut **tx)
    .await
    .map_err(|e| AppError::db("update product", e))?;
    Ok(())
}

/// Returns the current statuses of many products in one query,
/// read inside the caller's transaction — the bulk bar's skip
/// check (a product already in the target state is not an action
/// and earns no audit row). Returns (id, status, has_image)
/// triples; a product that does not exist simply has no triple.
/// `has_image` reads the synced main-image accessor —
/// `image_path` is `''` exactly when a product has no images —
/// so the publish gate needs no second query.
pub async fn statuses_of(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    ids: &[i64],
) -> Result<Vec<(i64, String, bool)>, AppError> {
    sqlx::query_as::<_, (i64, String, bool)>(
        "SELECT id, status, image_path <> '' FROM products WHERE id = ANY($1)",
    )
    .bind(ids)
    .fetch_all(&mut **tx)
    .await
    .map_err(|e| AppError::db("bulk status read", e))
}

/// Moves many products to a new publication status in one
/// statement inside the caller's transaction — the bulk path of
/// [`set_product_status`].
pub async fn set_product_status_many(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    ids: &[i64],
    status: &str,
    admin_id: i64,
) -> Result<(), AppError> {
    sqlx::query(
        "UPDATE products SET status = $2, updated_at = now(), updated_by = $3 WHERE id = ANY($1)",
    )
    .bind(ids)
    .bind(status)
    .bind(admin_id)
    .execute(&mut **tx)
    .await
    .map_err(|e| AppError::db("bulk set product status", e))?;
    Ok(())
}

/// Moves one product to a new publication status inside the
/// caller's transaction (archive / unarchive / bulk changes).
pub async fn set_product_status(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    id: i64,
    status: &str,
    admin_id: i64,
) -> Result<(), AppError> {
    sqlx::query(
        "UPDATE products SET status = $2, updated_at = now(), updated_by = $3 WHERE id = $1",
    )
    .bind(id)
    .bind(status)
    .bind(admin_id)
    .execute(&mut **tx)
    .await
    .map_err(|e| AppError::db("set product status", e))?;
    Ok(())
}

/// Public products at or below the low-stock threshold, least
/// stock first — the dashboard's attention list. Drafts and
/// archives are excluded: they cannot sell out.
pub async fn low_stock_products(
    pool: &PgPool,
    threshold: i64,
) -> Result<Vec<AdminProduct>, AppError> {
    sqlx::query_as::<_, AdminProduct>(&format!(
        "SELECT {PRODUCT_COLUMNS}, {STOCK_SUM} AS stock FROM products \
         WHERE status = 'public' AND track_stock \
         AND {STOCK_SUM} <= COALESCE(low_stock_threshold, $1) \
         ORDER BY stock ASC, id ASC"
    ))
    .bind(threshold)
    .fetch_all(pool)
    .await
    .map_err(|e| AppError::db("low stock products", e))
}

/// Every product's ID and name, for filter selects (includes
/// archived products: moderation covers all of them).
pub async fn list_all_id_name(pool: &PgPool) -> Result<Vec<(i64, String)>, AppError> {
    sqlx::query_as("SELECT id, name FROM products ORDER BY id")
        .fetch_all(pool)
        .await
        .map_err(|e| AppError::db("list product names", e))
}

/// The identifiers other products already use — the uniqueness
/// universe for the form's product-code, slug, SKU and JAN
/// checks. Each set excludes the product being edited, so
/// keeping your own identifier never reads as a collision.
pub struct TakenIdentifiers {
    /// Product codes in use elsewhere.
    pub product_codes: Vec<String>,
    /// Slugs in use elsewhere.
    pub slugs: Vec<String>,
    /// Variant SKUs in use on other products.
    pub skus: Vec<String>,
    /// Variant JAN codes in use on other products.
    pub jan_codes: Vec<String>,
}

/// Loads the identifier sets of every product except
/// `exclude_product_id` (none excluded on create).
pub async fn taken_identifiers(
    pool: &PgPool,
    exclude_product_id: Option<i64>,
) -> Result<TakenIdentifiers, AppError> {
    let exclude = exclude_product_id.unwrap_or(-1);
    let product_codes = sqlx::query_scalar::<_, String>(
        "SELECT product_code FROM products WHERE product_code IS NOT NULL AND id <> $1",
    )
    .bind(exclude)
    .fetch_all(pool)
    .await
    .map_err(|e| AppError::db("load taken product codes", e))?;
    let slugs = sqlx::query_scalar::<_, String>(
        "SELECT slug FROM products WHERE slug IS NOT NULL AND id <> $1",
    )
    .bind(exclude)
    .fetch_all(pool)
    .await
    .map_err(|e| AppError::db("load taken slugs", e))?;
    let skus = sqlx::query_scalar::<_, String>(
        "SELECT v.sku FROM product_variants v \
         WHERE v.sku IS NOT NULL AND v.sku <> '' AND v.product_id <> $1",
    )
    .bind(exclude)
    .fetch_all(pool)
    .await
    .map_err(|e| AppError::db("load taken skus", e))?;
    let jan_codes = sqlx::query_scalar::<_, String>(
        "SELECT v.jan_code FROM product_variants v \
         WHERE v.jan_code IS NOT NULL AND v.jan_code <> '' AND v.product_id <> $1",
    )
    .bind(exclude)
    .fetch_all(pool)
    .await
    .map_err(|e| AppError::db("load taken jan codes", e))?;
    Ok(TakenIdentifiers {
        product_codes,
        slugs,
        skus,
        jan_codes,
    })
}

/// Returns the product's current `updated_at` with the row
/// locked, so no concurrent save can slip between the check
/// and the write: the product edit form's staleness guard
/// compares its expected version against this value inside
/// the update transaction.
pub async fn product_updated_at_for_update(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    id: i64,
) -> Result<Option<chrono::DateTime<chrono::Utc>>, AppError> {
    let value = sqlx::query_scalar::<_, chrono::DateTime<chrono::Utc>>(
        "SELECT updated_at FROM products WHERE id = $1 FOR UPDATE",
    )
    .bind(id)
    .fetch_optional(&mut **tx)
    .await?;
    Ok(value)
}
