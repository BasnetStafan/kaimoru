//! Product search repository.
//!
//! Page rows and total match count come back in one query via
//! `COUNT(*) OVER ()`; a page past the end falls back to a
//! separate count query. Each search runs
//! in a short transaction so `SET LOCAL statement_timeout` (500ms)
//! bounds the query itself: a pathological search fails fast
//! instead of holding a pooled connection.

use sqlx::PgPool;

use crate::models::Product;

/// Sort orders the search page may request.
/// The ORDER BY text is chosen from this fixed list in Rust;
/// request input only ever selects a variant, never SQL text.
#[derive(Clone, Copy, Debug)]
pub enum SearchSort {
    /// Best sellers first (the store default).
    Recommended,
    /// Newest products first.
    Newest,
    /// Cheapest first.
    PriceAsc,
    /// Most expensive first.
    PriceDesc,
    /// Highest rated first.
    Rating,
}

impl SearchSort {
    /// The fixed ORDER BY fragment for this sort.
    fn order_by(&self) -> &'static str {
        match self {
            Self::Recommended => "products.display_order ASC, products.created_at DESC",
            Self::Newest => "created_at DESC",
            Self::PriceAsc => "price_yen ASC, products.id ASC",
            Self::PriceDesc => "price_yen DESC, products.id ASC",
            Self::Rating => {
                "r.avg_rating DESC NULLS LAST, COALESCE(s.sold, 0) DESC, products.id ASC"
            }
        }
    }
}

/// The outcome of one search: the page of products and the total
/// number of matches across all pages.
pub struct SearchOutcome {
    /// The products on the requested page.
    pub products: Vec<Product>,
    /// Total matches for the same conditions.
    pub total: i64,
}

/// One row of the search query: the product columns plus the
/// window-counted total.
#[derive(sqlx::FromRow)]
struct SearchRow {
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
    total_count: i64,
}

impl From<SearchRow> for Product {
    fn from(row: SearchRow) -> Self {
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
            // Search results never render the description; the
            // detail page loads the full product separately.
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

/// Searches the catalog.
///
/// `patterns` are already-escaped `%term%` ILIKE patterns from the
/// search service; every term must match the name or the category
/// (multi-term AND). The WHERE clause is composed only of fixed
/// fragments and placeholder numbers — every user value reaches the
/// database as a bound parameter.
/// The inputs of one search, grouped into one struct because
/// the parameter list outgrew clippy's argument limit when the
/// tag filter joined the category filter.
pub struct SearchArgs<'a> {
    /// Already-escaped `%term%` ILIKE patterns; every term must
    /// match the name or the category (multi-term AND).
    pub patterns: &'a [String],
    /// Category name filter (exact snapshot match).
    pub category: Option<&'a str>,
    /// Tag slug filter (the product must carry the tag).
    pub tag_slug: Option<&'a str>,
    /// Whether to keep only products with stock.
    pub in_stock_only: bool,
    /// Lowest effective price to keep, in yen (inclusive).
    pub min_price: Option<i64>,
    /// Highest effective price to keep, in yen (inclusive).
    pub max_price: Option<i64>,
    /// Result ordering.
    pub sort: SearchSort,
    /// Page size.
    pub limit: i64,
    /// Page offset.
    pub offset: i64,
}

pub async fn search(pool: &PgPool, args: SearchArgs<'_>) -> Result<SearchOutcome, sqlx::Error> {
    let SearchArgs {
        patterns,
        category,
        tag_slug,
        in_stock_only,
        min_price,
        max_price,
        sort,
        limit,
        offset,
    } = args;
    // The storefront only ever searches public products: a fixed
    // fragment, always present.
    let mut conditions: Vec<String> = vec![
        "status = 'public'".to_string(),
        "(publish_starts_at IS NULL OR publish_starts_at <= now())".to_string(),
        "(publish_ends_at IS NULL OR publish_ends_at > now())".to_string(),
    ];
    let mut binds: Vec<String> = Vec::new();
    for pattern in patterns {
        binds.push(pattern.clone());
        conditions.push(format!(
            "(name ILIKE ${0} OR category ILIKE ${0} OR search_keywords ILIKE ${0} OR brand ILIKE ${0})",
            binds.len()
        ));
    }
    if let Some(category) = category {
        binds.push(category.to_string());
        conditions.push(format!("category = ${}", binds.len()));
    }
    if let Some(slug) = tag_slug {
        binds.push(slug.to_string());
        conditions.push(format!(
            "EXISTS (SELECT 1 FROM product_tags pt \
             JOIN tags t ON t.id = pt.tag_id \
             WHERE pt.product_id = products.id AND t.slug = ${})",
            binds.len()
        ));
    }
    if in_stock_only {
        // Stock lives on variants now: in stock means the
        // product variant sums are positive.
        conditions.push(
            "(NOT track_stock OR allow_backorder OR (SELECT COALESCE(SUM(stock), 0) FROM product_variants \
              WHERE product_id = products.id AND is_enabled) > 0)"
                .to_string(),
        );
    }
    // The price range reads the EFFECTIVE price (the sale
    // price while a sale is live) — the number the shopper
    // sees on the card. The values are validated integers
    // from the search service; they still travel as bound
    // parameters, cast in SQL because this query's bind list
    // is text-typed for the pattern filters.
    let effective_price = super::effective_price_for("products");
    if let Some(min) = min_price {
        binds.push(min.to_string());
        conditions.push(format!("({effective_price}) >= ${}::bigint", binds.len()));
    }
    if let Some(max) = max_price {
        binds.push(max.to_string());
        conditions.push(format!("({effective_price}) <= ${}::bigint", binds.len()));
    }
    let where_clause = format!("WHERE {}", conditions.join(" AND "));
    // The card reads' main-image alt expression, shared so search
    // cards carry the same alt text as the section cards.
    let main_image_alt = super::product_repository::MAIN_IMAGE_ALT;
    let limit_pos = binds.len() + 1;
    let offset_pos = binds.len() + 2;
    let sql = format!(
        "SELECT id, name, price_yen, sale_price_yen, sale_starts_at, sale_ends_at, \
         (SELECT COALESCE(SUM(stock), 0)::int FROM product_variants \
            WHERE product_id = products.id AND is_enabled) AS stock, \
         (SELECT COUNT(*) FROM product_variants \
            WHERE product_id = products.id AND is_enabled) > 1 AS has_variants, \
         image_path, category, category_id, is_new, \
         discount_percent, is_preorder, is_event, is_staff_pick, track_stock, allow_backorder, \
         low_stock_threshold, points_earn_percent, tax_class_id, \
         COALESCE((SELECT code FROM tax_classes WHERE id = products.tax_class_id), 'standard') AS tax_class_code, \
         COALESCE((SELECT name FROM tax_classes WHERE id = products.tax_class_id), '標準税率') AS tax_class_name, \
         {main_image_alt}, COUNT(*) OVER () AS total_count \
         FROM products LEFT JOIN (SELECT product_id, SUM(quantity) AS sold FROM order_items GROUP BY product_id) s ON s.product_id = products.id LEFT JOIN (SELECT product_id, AVG(rating) AS avg_rating FROM reviews WHERE hidden_at IS NULL GROUP BY product_id) r ON r.product_id = products.id {where_clause} ORDER BY {} LIMIT ${limit_pos} OFFSET ${offset_pos}",
        sort.order_by(),
    );

    // The statement timeout is local to this transaction, so it
    // bounds search queries only and never leaks to other work.
    let mut tx = pool.begin().await?;
    sqlx::query("SET LOCAL statement_timeout = '500ms'")
        .execute(&mut *tx)
        .await?;
    let mut query = sqlx::query_as::<_, SearchRow>(&sql);
    for bind in &binds {
        query = query.bind(bind);
    }
    let rows = query.bind(limit).bind(offset).fetch_all(&mut *tx).await?;
    // A page past the end returns no rows to carry the total; only
    // then is a separate count worth its round trip. It runs inside
    // this transaction under the same timeout; the page query's
    // joins are one-to-one, so counting products gives the same total.
    let total = if let Some(first) = rows.first() {
        first.total_count
    } else if offset == 0 {
        0
    } else {
        let count_sql = format!("SELECT COUNT(*) FROM products {where_clause}");
        let mut count_query = sqlx::query_scalar::<_, i64>(&count_sql);
        for bind in &binds {
            count_query = count_query.bind(bind);
        }
        count_query.fetch_one(&mut *tx).await?
    };
    tx.commit().await?;
    Ok(SearchOutcome {
        products: rows.into_iter().map(Product::from).collect(),
        total,
    })
}

/// Suggests product names for the search box (htmx fragment).
/// Names only, best sellers first, under the same statement timeout.
pub async fn suggest(pool: &PgPool, pattern: &str, limit: i64) -> Result<Vec<String>, sqlx::Error> {
    let mut tx = pool.begin().await?;
    sqlx::query("SET LOCAL statement_timeout = '500ms'")
        .execute(&mut *tx)
        .await?;
    let names = sqlx::query_scalar(
        "SELECT name FROM products LEFT JOIN (SELECT product_id, SUM(quantity) AS sold FROM order_items GROUP BY product_id) s ON s.product_id = products.id WHERE status = 'public' AND (publish_starts_at IS NULL OR publish_starts_at <= now()) AND (publish_ends_at IS NULL OR publish_ends_at > now()) AND name ILIKE $1 ORDER BY COALESCE(s.sold, 0) DESC LIMIT $2",
    )
    .bind(pattern)
    .bind(limit)
    .fetch_all(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(names)
}
