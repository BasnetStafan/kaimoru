//! Product repository.
//!
//! Owns every storefront product query; rows are mapped into
//! [`Product`] by sqlx. The reads feed the top page sections,
//! the detail page, search and the favourites page.
//!
//! Storefront reads filter `status = 'public'`: drafts and
//! archived products are invisible to the storefront. The one
//! exception is `find_categories_all`, which spans every status
//! for the admin product list's filter. The admin side reads all
//! states through `product_repository` instead.

use std::collections::HashMap;

use sqlx::PgPool;

use crate::models::Product;

/// The stock expression every storefront product read uses: the
/// sum over the product variants (stock lives on variants only),
/// cast back to the INTEGER shape, plus whether the product has
/// more than one variant — a multi-variant product is never
/// added from a card, because a combination must not be chosen
/// silently for the customer.
const STOCK_AND_VARIANTS: &str = "(SELECT COALESCE(SUM(stock), 0)::int \
    FROM product_variants WHERE product_id = products.id AND is_enabled) AS stock, \
    (SELECT COUNT(*) FROM product_variants WHERE product_id = products.id AND is_enabled) > 1 AS has_variants";

/// The product's tax class code and name as select
/// expressions, joined from the classes table so cards and
/// the detail page can name the class without a second
/// round trip; the standard class stands in when a legacy
/// row names no class.
pub(crate) const TAX_CLASS_EXPR: &str = "COALESCE((SELECT code FROM tax_classes \
    WHERE id = products.tax_class_id), 'standard') AS tax_class_code, \
    COALESCE((SELECT name FROM tax_classes WHERE id = products.tax_class_id), '標準税率') AS tax_class_name";

/// The main image's stored alt text as a select expression:
/// the first image row's alt, empty when the product has no
/// image row at all, so a card read never fails on the join.
pub(crate) const MAIN_IMAGE_ALT: &str = "COALESCE((SELECT alt_text FROM product_images \
    WHERE product_id = products.id ORDER BY sort_order, id LIMIT 1), '') AS image_alt";

/// The publication-window predicate: a product is visible only
/// inside its optional window. Every storefront read pairs it
/// with `status = 'public'`.
const WINDOW: &str = "(publish_starts_at IS NULL OR publish_starts_at <= now()) \
    AND (publish_ends_at IS NULL OR publish_ends_at > now())";

/// Columns every top page section needs from the products table.
const PRODUCT_COLUMNS: &str = "id, name, price_yen, sale_price_yen, sale_starts_at, sale_ends_at, image_path, category, category_id, is_new, discount_percent, is_preorder, is_event, is_staff_pick, description, product_code, brand, manufacturer, country_of_origin, spec_text, features_text, description_sp, tax_class_id, track_stock, allow_backorder, low_stock_threshold, points_earn_percent, weight_grams, size_length_mm, size_width_mm, size_height_mm, cool_delivery, mail_delivery, free_shipping, undeliverable_regions, lead_time_days, slug, seo_title, meta_description";

/// Columns a product card needs: everything except the long
/// description, which cards never render — fetching it for every
/// card was pure waste.
const CARD_COLUMNS: &str = "id, name, price_yen, sale_price_yen, sale_starts_at, sale_ends_at, image_path, category, category_id, is_new, discount_percent, is_preorder, is_event, is_staff_pick, track_stock, allow_backorder, low_stock_threshold, points_earn_percent, tax_class_id";

/// One row of a card query: the card columns only, mapped into
/// [`Product`] with an empty description — the detail page loads
/// the full product separately.
#[derive(sqlx::FromRow)]
struct CardRow {
    id: i64,
    name: String,
    price_yen: i64,
    sale_price_yen: Option<i64>,
    sale_starts_at: Option<chrono::DateTime<chrono::Utc>>,
    sale_ends_at: Option<chrono::DateTime<chrono::Utc>>,
    stock: i32,
    has_variants: bool,
    image_path: String,
    image_alt: String,
    category: String,
    category_id: Option<i64>,
    is_new: bool,
    discount_percent: i32,
    is_preorder: bool,
    is_event: bool,
    is_staff_pick: bool,
    track_stock: bool,
    allow_backorder: bool,
    low_stock_threshold: Option<i32>,
    points_earn_percent: Option<i32>,
    tax_class_id: i64,
    tax_class_code: String,
    tax_class_name: String,
}

impl From<CardRow> for Product {
    fn from(row: CardRow) -> Self {
        Self {
            id: row.id,
            name: row.name,
            price_yen: row.price_yen,
            sale_price_yen: row.sale_price_yen,
            sale_starts_at: row.sale_starts_at,
            sale_ends_at: row.sale_ends_at,
            stock: row.stock,
            has_variants: row.has_variants,
            image_path: row.image_path,
            image_alt: row.image_alt,
            category: row.category,
            category_id: row.category_id,
            is_new: row.is_new,
            discount_percent: row.discount_percent,
            is_preorder: row.is_preorder,
            is_event: row.is_event,
            is_staff_pick: row.is_staff_pick,
            description: String::new(),
            product_code: None,
            brand: String::new(),
            manufacturer: String::new(),
            country_of_origin: String::new(),
            spec_text: String::new(),
            features_text: String::new(),
            description_sp: String::new(),
            tax_class_id: row.tax_class_id,
            tax_class_code: row.tax_class_code.clone(),
            tax_class_name: row.tax_class_name.clone(),
            track_stock: row.track_stock,
            allow_backorder: row.allow_backorder,
            low_stock_threshold: row.low_stock_threshold,
            points_earn_percent: row.points_earn_percent,
            weight_grams: None,
            size_length_mm: None,
            size_width_mm: None,
            size_height_mm: None,
            cool_delivery: false,
            mail_delivery: false,
            free_shipping: false,
            undeliverable_regions: String::new(),
            lead_time_days: None,
            slug: None,
            seo_title: String::new(),
            meta_description: String::new(),
        }
    }
}

/// One page of the public catalog for the agent API, with the
/// total number of public products. Visibility matches every
/// storefront read: `status = 'public'` inside the publication
/// window. Ordered by id so successive pages are stable.
pub async fn catalog_page(
    pool: &PgPool,
    limit: i64,
    offset: i64,
) -> Result<(Vec<Product>, i64), sqlx::Error> {
    let rows: Vec<CardRow> = sqlx::query_as(&format!(
        "SELECT {CARD_COLUMNS}, {STOCK_AND_VARIANTS}, {MAIN_IMAGE_ALT}, {TAX_CLASS_EXPR} \
         FROM products WHERE status = 'public' AND {WINDOW} \
         ORDER BY products.id ASC LIMIT $1 OFFSET $2"
    ))
    .bind(limit)
    .bind(offset)
    .fetch_all(pool)
    .await?;
    let total: (i64,) = sqlx::query_as(&format!(
        "SELECT COUNT(*) FROM products WHERE status = 'public' AND {WINDOW}"
    ))
    .fetch_one(pool)
    .await?;
    Ok((rows.into_iter().map(Product::from).collect(), total.0))
}

/// One public product for the agent API, by id. `None` when
/// the product does not exist or is not public — the API never
/// distinguishes the two, so drafts cannot be probed through
/// it.
pub async fn catalog_product(pool: &PgPool, id: i64) -> Result<Option<Product>, sqlx::Error> {
    let row: Option<CardRow> = sqlx::query_as(&format!(
        "SELECT {CARD_COLUMNS}, {STOCK_AND_VARIANTS}, {MAIN_IMAGE_ALT}, {TAX_CLASS_EXPR} \
         FROM products WHERE products.id = $1 AND status = 'public' AND {WINDOW}"
    ))
    .bind(id)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(Product::from))
}

/// The bulk twin of [`catalog_product`]: every public
/// product among `ids`, in one query.
pub async fn catalog_products(pool: &PgPool, ids: &[i64]) -> Result<Vec<Product>, sqlx::Error> {
    let rows: Vec<CardRow> = sqlx::query_as(&format!(
        "SELECT {CARD_COLUMNS}, {STOCK_AND_VARIANTS}, {MAIN_IMAGE_ALT}, {TAX_CLASS_EXPR} \
         FROM products WHERE products.id = ANY($1) AND status = 'public' AND {WINDOW}"
    ))
    .bind(ids)
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(Product::from).collect())
}

/// Every public product in any of the named categories, as
/// card data, in one query: the blog embed expansion's
/// showcase load. Categories are matched by the name
/// snapshot — the same predicate the storefront category
/// page (the search filter) uses, so an embed previews
/// exactly the set its "see all" link lands on. Ordered by
/// id, the catalog order; the caller caps each category's
/// share.
pub async fn catalog_products_for_categories(
    pool: &PgPool,
    names: &[String],
) -> Result<Vec<Product>, sqlx::Error> {
    if names.is_empty() {
        return Ok(Vec::new());
    }
    let rows: Vec<CardRow> = sqlx::query_as(&format!(
        "SELECT {CARD_COLUMNS}, {STOCK_AND_VARIANTS}, {MAIN_IMAGE_ALT}, {TAX_CLASS_EXPR} \
         FROM products WHERE category = ANY($1) AND status = 'public' AND {WINDOW} \
         ORDER BY products.id ASC"
    ))
    .bind(names)
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(Product::from).collect())
}

/// The long descriptions of several products at once, keyed
/// by product id — the shopping agent's candidate assembly
/// resolves the whole pool's descriptions in one query,
/// because the card reads it builds on never fetch them
/// (cards do not render the text). Products with no row
/// are absent from the map.
pub async fn descriptions_for_products(
    pool: &PgPool,
    product_ids: &[i64],
) -> Result<HashMap<i64, String>, sqlx::Error> {
    if product_ids.is_empty() {
        return Ok(HashMap::new());
    }
    let rows: Vec<(i64, String)> =
        sqlx::query_as("SELECT id, description FROM products WHERE id = ANY($1)")
            .bind(product_ids)
            .fetch_all(pool)
            .await?;
    Ok(rows.into_iter().collect())
}

/// Returns the public products among `ids` as card data, in
/// the order the ids were given (the favourites page passes
/// them newest mark first). Unknown or no-longer-public ids
/// are skipped: their card would link to a page that does not
/// exist.
pub async fn find_cards_by_ids(pool: &PgPool, ids: &[i64]) -> Result<Vec<Product>, sqlx::Error> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let rows = sqlx::query_as::<_, CardRow>(&format!(
        "SELECT {CARD_COLUMNS}, {STOCK_AND_VARIANTS}, {MAIN_IMAGE_ALT}, {TAX_CLASS_EXPR} \
         FROM products WHERE id = ANY($1) AND status = 'public' AND {WINDOW}"
    ))
    .bind(ids)
    .fetch_all(pool)
    .await?;
    let mut by_id: std::collections::HashMap<i64, Product> = rows
        .into_iter()
        .map(|row| {
            let product = Product::from(row);
            (product.id, product)
        })
        .collect();
    Ok(ids.iter().filter_map(|id| by_id.remove(id)).collect())
}

/// Runs a card query and maps its rows into products.
async fn fetch_cards(pool: &PgPool, sql: String, limit: i64) -> Result<Vec<Product>, sqlx::Error> {
    let rows = sqlx::query_as::<_, CardRow>(&sql)
        .bind(limit)
        .fetch_all(pool)
        .await?;
    Ok(rows.into_iter().map(Product::from).collect())
}

/// New arrivals section: the most recently added products first.
pub async fn find_new_arrivals(pool: &PgPool, limit: i64) -> Result<Vec<Product>, sqlx::Error> {
    fetch_cards(
        pool,
        format!("SELECT {CARD_COLUMNS}, {STOCK_AND_VARIANTS}, {MAIN_IMAGE_ALT}, {TAX_CLASS_EXPR} FROM products WHERE status = 'public' AND {WINDOW} ORDER BY created_at DESC LIMIT $1"),
        limit,
    )
    .await
}

/// Returns the ranking section: best sellers first, the review
/// average breaking ties (the static `products.rating` is retired,
/// so the aggregate comes from the reviews table).
///
/// Sales totals come from one aggregated subquery joined once,
/// not a per-row correlated SUM.
pub async fn find_ranking(pool: &PgPool, limit: i64) -> Result<Vec<Product>, sqlx::Error> {
    fetch_cards(
        pool,
        format!(
            "SELECT {CARD_COLUMNS}, {STOCK_AND_VARIANTS}, {MAIN_IMAGE_ALT}, {TAX_CLASS_EXPR} FROM products \
             LEFT JOIN (SELECT product_id, SUM(quantity) AS sold FROM order_items GROUP BY product_id) s \
               ON s.product_id = products.id \
             LEFT JOIN (SELECT product_id, AVG(rating) AS avg_rating FROM reviews WHERE hidden_at IS NULL GROUP BY product_id) r \
               ON r.product_id = products.id \
             WHERE products.status = 'public' AND {WINDOW} \
             ORDER BY COALESCE(s.sold, 0) DESC, r.avg_rating DESC NULLS LAST, products.id ASC LIMIT $1"
        ),
        limit,
    )
    .await
}

/// Event pickup section: products flagged for venue pickup at events.
pub async fn find_event(pool: &PgPool, limit: i64) -> Result<Vec<Product>, sqlx::Error> {
    fetch_cards(
        pool,
        format!(
            "SELECT {CARD_COLUMNS}, {STOCK_AND_VARIANTS}, {MAIN_IMAGE_ALT}, {TAX_CLASS_EXPR} FROM products WHERE is_event AND status = 'public' AND {WINDOW} ORDER BY created_at DESC LIMIT $1"
        ),
        limit,
    )
    .await
}

/// Returns (id, is_event) for the given product ids that
/// exist: the pin validation's fact base — an id missing
/// from the result does not exist, and the event section
/// may only pin event products.
pub async fn pin_facts(pool: &PgPool, ids: &[i64]) -> Result<Vec<(i64, bool)>, sqlx::Error> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    sqlx::query_as::<_, (i64, bool)>("SELECT id, is_event FROM products WHERE id = ANY($1)")
        .bind(ids)
        .fetch_all(pool)
        .await
}

/// Staff picks section: the products the operator flagged
/// as recommended (the product form's おすすめ), newest
/// first.
pub async fn find_staff_picks(pool: &PgPool, limit: i64) -> Result<Vec<Product>, sqlx::Error> {
    fetch_cards(
        pool,
        format!(
            "SELECT {CARD_COLUMNS}, {STOCK_AND_VARIANTS}, {MAIN_IMAGE_ALT}, {TAX_CLASS_EXPR} \
             FROM products WHERE is_staff_pick AND status = 'public' AND {WINDOW} \
             ORDER BY created_at DESC LIMIT $1"
        ),
        limit,
    )
    .await
}

/// Counts the products behind the storefront gate, for
/// the see-more rule (a section links to its full list
/// only when it holds more than its row shows).
async fn count_where(pool: &PgPool, condition: &str) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar(&format!(
        "SELECT count(*) FROM products WHERE status = 'public' AND {WINDOW} AND {condition}"
    ))
    .fetch_one(pool)
    .await
}

/// Counts every public product inside its publish window.
pub async fn count_public(pool: &PgPool) -> Result<i64, sqlx::Error> {
    count_where(pool, "TRUE").await
}

/// Ids and update stamps of every public product, for the
/// sitemap: the catalog's own public predicate (status plus
/// the publish window), but only the two columns a sitemap
/// entry needs.
pub async fn sitemap_products(
    pool: &PgPool,
) -> Result<Vec<(i64, chrono::DateTime<chrono::Utc>)>, sqlx::Error> {
    sqlx::query_as(&format!(
        "SELECT id, updated_at FROM products \
         WHERE status = 'public' AND {WINDOW} ORDER BY id ASC"
    ))
    .fetch_all(pool)
    .await
}

/// Counts the event pickup section's products.
pub async fn count_event(pool: &PgPool) -> Result<i64, sqlx::Error> {
    count_where(pool, "is_event").await
}

/// Counts the staff picks section's products.
pub async fn count_staff_picks(pool: &PgPool) -> Result<i64, sqlx::Error> {
    count_where(pool, "is_staff_pick").await
}

/// All three distinct section totals in one grouped query
///: the new-arrivals and ranking sections
/// share the public total, so one scan under the same
/// public gate answers every see-more decision on the top
/// page. Returns (public, event, staff picks).
pub async fn section_counts(pool: &PgPool) -> Result<(i64, i64, i64), sqlx::Error> {
    sqlx::query_as::<_, (i64, i64, i64)>(&format!(
        "SELECT count(*), \
                count(*) FILTER (WHERE is_event), \
                count(*) FILTER (WHERE is_staff_pick) \
         FROM products WHERE status = 'public' AND {WINDOW}"
    ))
    .fetch_one(pool)
    .await
}

/// New arrivals, one page of the section listing:
/// the non-pinned items (the pins lead the listing,
/// merged in by the caller).
pub async fn find_new_arrivals_page(
    pool: &PgPool,
    limit: i64,
    offset: i64,
    exclude: &[i64],
) -> Result<Vec<Product>, sqlx::Error> {
    let rows = sqlx::query_as::<_, CardRow>(&format!(
        "SELECT {CARD_COLUMNS}, {STOCK_AND_VARIANTS}, {MAIN_IMAGE_ALT}, {TAX_CLASS_EXPR} \
         FROM products WHERE status = 'public' AND {WINDOW} AND NOT (id = ANY($3)) \
         ORDER BY created_at DESC LIMIT $1 OFFSET $2"
    ))
    .bind(limit)
    .bind(offset)
    .bind(exclude)
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(Product::from).collect())
}

/// Ranking, one page of the section listing: the row's
/// sales order over the non-pinned items, paged.
pub async fn find_ranking_page(
    pool: &PgPool,
    limit: i64,
    offset: i64,
    exclude: &[i64],
) -> Result<Vec<Product>, sqlx::Error> {
    let rows = sqlx::query_as::<_, CardRow>(&format!(
        "SELECT {CARD_COLUMNS}, {STOCK_AND_VARIANTS}, {MAIN_IMAGE_ALT}, {TAX_CLASS_EXPR} FROM products \
         LEFT JOIN (SELECT product_id, SUM(quantity) AS sold FROM order_items GROUP BY product_id) s \
           ON s.product_id = products.id \
         LEFT JOIN (SELECT product_id, AVG(rating) AS avg_rating FROM reviews WHERE hidden_at IS NULL GROUP BY product_id) r \
           ON r.product_id = products.id \
         WHERE products.status = 'public' AND {WINDOW} AND NOT (products.id = ANY($3)) \
         ORDER BY COALESCE(s.sold, 0) DESC, r.avg_rating DESC NULLS LAST, products.id ASC \
         LIMIT $1 OFFSET $2"
    ))
    .bind(limit)
    .bind(offset)
    .bind(exclude)
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(Product::from).collect())
}

/// Event pickup, one page of the section listing:
/// the non-pinned items.
pub async fn find_event_page(
    pool: &PgPool,
    limit: i64,
    offset: i64,
    exclude: &[i64],
) -> Result<Vec<Product>, sqlx::Error> {
    let rows = sqlx::query_as::<_, CardRow>(&format!(
        "SELECT {CARD_COLUMNS}, {STOCK_AND_VARIANTS}, {MAIN_IMAGE_ALT}, {TAX_CLASS_EXPR} \
         FROM products WHERE is_event AND status = 'public' AND {WINDOW} AND NOT (id = ANY($3)) \
         ORDER BY created_at DESC LIMIT $1 OFFSET $2"
    ))
    .bind(limit)
    .bind(offset)
    .bind(exclude)
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(Product::from).collect())
}

/// Staff picks, one page of the section listing:
/// the non-pinned items.
pub async fn find_staff_picks_page(
    pool: &PgPool,
    limit: i64,
    offset: i64,
    exclude: &[i64],
) -> Result<Vec<Product>, sqlx::Error> {
    let rows = sqlx::query_as::<_, CardRow>(&format!(
        "SELECT {CARD_COLUMNS}, {STOCK_AND_VARIANTS}, {MAIN_IMAGE_ALT}, {TAX_CLASS_EXPR} \
         FROM products WHERE is_staff_pick AND status = 'public' AND {WINDOW} AND NOT (id = ANY($3)) \
         ORDER BY created_at DESC LIMIT $1 OFFSET $2"
    ))
    .bind(limit)
    .bind(offset)
    .bind(exclude)
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(Product::from).collect())
}

/// Returns category names across all products, any status — the
/// admin product list's filter: an operator filtering drafts
/// must still see categories only drafts use. The storefront
/// navigation reads the managed categories table instead
/// (`category_repository::nav_categories`).
pub async fn find_categories_all(pool: &PgPool) -> Result<Vec<String>, sqlx::Error> {
    sqlx::query_scalar("SELECT DISTINCT category FROM products ORDER BY category")
        .fetch_all(pool)
        .await
}

/// One product by ID, for the detail page.
pub async fn find_by_id(pool: &PgPool, id: i64) -> Result<Option<Product>, sqlx::Error> {
    sqlx::query_as::<_, Product>(&format!(
        "SELECT {PRODUCT_COLUMNS}, {STOCK_AND_VARIANTS}, {MAIN_IMAGE_ALT}, {TAX_CLASS_EXPR} FROM products WHERE id = $1 AND status = 'public' AND {WINDOW}"
    ))
    .bind(id)
    .fetch_optional(pool)
    .await
}

/// Several public products by ID in one query:
/// the bulk sibling of [`find_by_id`], same public
/// gate, for callers assembling data over the whole pool.
pub async fn find_by_ids(pool: &PgPool, ids: &[i64]) -> Result<Vec<Product>, sqlx::Error> {
    sqlx::query_as::<_, Product>(&format!(
        "SELECT {PRODUCT_COLUMNS}, {STOCK_AND_VARIANTS}, {MAIN_IMAGE_ALT}, {TAX_CLASS_EXPR} FROM products WHERE id = ANY($1) AND status = 'public' AND {WINDOW}"
    ))
    .bind(ids)
    .fetch_all(pool)
    .await
}

/// One product by its URL slug, for the detail page's
/// slug route; the ID route stays canonical.
pub async fn find_by_slug(pool: &PgPool, slug: &str) -> Result<Option<Product>, sqlx::Error> {
    sqlx::query_as::<_, Product>(&format!(
        "SELECT {PRODUCT_COLUMNS}, {STOCK_AND_VARIANTS}, {MAIN_IMAGE_ALT}, {TAX_CLASS_EXPR} FROM products WHERE slug = $1 AND status = 'public' AND {WINDOW}"
    ))
    .bind(slug)
    .fetch_optional(pool)
    .await
}

/// Same-category products for the detail page row: best sellers
/// first, the product itself excluded. These render as cards,
/// so they take the card column list like the sections do.
pub async fn find_related(
    pool: &PgPool,
    category: &str,
    exclude_id: i64,
    limit: i64,
) -> Result<Vec<Product>, sqlx::Error> {
    let rows = sqlx::query_as::<_, CardRow>(&format!(
        "SELECT {CARD_COLUMNS}, {STOCK_AND_VARIANTS}, {MAIN_IMAGE_ALT}, {TAX_CLASS_EXPR} FROM products LEFT JOIN (SELECT product_id, SUM(quantity) AS sold FROM order_items GROUP BY product_id) s ON s.product_id = products.id LEFT JOIN (SELECT product_id, AVG(rating) AS avg_rating FROM reviews WHERE hidden_at IS NULL GROUP BY product_id) r ON r.product_id = products.id WHERE category = $1 AND id <> $2 AND status = 'public' AND {WINDOW} ORDER BY COALESCE(s.sold, 0) DESC, r.avg_rating DESC NULLS LAST, products.id ASC LIMIT $3"
    ))
    .bind(category)
    .bind(exclude_id)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(Product::from).collect())
}
