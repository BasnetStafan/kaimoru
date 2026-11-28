//! Builds the agent API's catalog responses from the shared
//! product reads. The API exposes exactly what the storefront
//! shows — public products only, live review aggregates, stock
//! summed over variants — so an agent and a person looking at
//! the same product never see different facts.

use schemars::JsonSchema;
use serde::Serialize;
use sqlx::PgPool;

use crate::config::CONFIG;
use crate::error::AppError;
use crate::models::Product;
use crate::repositories::{
    category_repository, product_repository, review_repository, search_repository, tag_repository,
    variant_repository,
};
use crate::services::{search_service, shop_settings_service};

/// One tag on a product, as exposed to agents.
#[derive(Serialize, JsonSchema)]
pub struct ApiTag {
    /// Stable ASCII identifier (the agent's matching key).
    pub slug: String,
    /// Japanese label shown to people.
    pub label: String,
}

/// The product's rating as exposed to agents: the live reviews
/// aggregate. `average` is null when the product has no
/// reviews — the API reports the honest aggregate and never
/// invents one.
#[derive(Serialize, JsonSchema)]
pub struct ApiRating {
    /// Average rating rounded to one decimal; null at 0 reviews.
    pub average: Option<f64>,
    /// Number of reviews behind the average.
    pub count: i64,
}

/// The product's category as exposed to agents.
#[derive(Serialize, JsonSchema)]
pub struct ApiCategory {
    /// Category id; null when the product has none.
    pub id: Option<i64>,
    /// Category name (the product's snapshot of it).
    pub name: String,
}

/// One catalog product as exposed to agents.
#[derive(Serialize, JsonSchema)]
pub struct ApiProduct {
    /// Product id.
    pub id: i64,
    /// Product name.
    pub name: String,
    /// Storefront path of the product page.
    pub path: String,
    /// Product description, carried for in-process consumers
    /// (the shopping agent's keyword matching). Never
    /// serialised: the API's wire shape is unchanged — and the
    /// schema skips it too, so a generated output schema never
    /// requires a field the wire never carries.
    #[serde(skip_serializing)]
    #[schemars(skip)]
    pub description: String,
    /// Current selling price in integer yen, tax included,
    /// after any active sale — the price a buyer would pay.
    pub price_yen: i64,
    /// Stock summed over the product's enabled variants.
    pub stock: i32,
    /// Whether the product can be ordered beyond its stock.
    pub allow_backorder: bool,
    /// Whether the product can be bought right now, under the
    /// same rule the cart applies: untracked stock or backorder
    /// is always buyable; tracked stock needs a unit on hand.
    pub purchasable: bool,
    /// Live reviews aggregate for this product.
    pub rating: ApiRating,
    /// The product's category.
    pub category: ApiCategory,
    /// The product's tags from the controlled vocabulary.
    pub tags: Vec<ApiTag>,
    /// The product's purchasable combinations. Present on the
    /// detail response only — the list stays a summary — so an
    /// agent chooses a combination from the detail it already
    /// fetched before adding to a cart.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub variants: Option<Vec<ApiVariant>>,
}

/// One purchasable combination of a product, as exposed to
/// agents: the cart keys on the variant id.
#[derive(Serialize, JsonSchema)]
pub struct ApiVariant {
    /// Variant id (the cart line identity).
    pub id: i64,
    /// Combination label, e.g. `サイズ: M / カラー: ブラック`;
    /// empty for the default variant of a plain product.
    pub label: String,
    /// The combination's price in integer yen: its own price
    /// when it has one, else the product's effective price.
    pub price_yen: i64,
    /// Units in stock for this combination.
    pub stock: i32,
    /// Whether this combination can be bought right now,
    /// under the same rule as the product-level flag.
    pub purchasable: bool,
}

/// One page of the catalog.
#[derive(Serialize, JsonSchema)]
pub struct CatalogPage {
    /// The page's products, in id order.
    pub products: Vec<ApiProduct>,
    /// The 1-based page number this response serves.
    pub page: i64,
    /// The page size in effect.
    pub per_page: i64,
    /// Total number of public products.
    pub total: i64,
    /// Total number of pages at this page size.
    pub total_pages: i64,
}

/// Maps products to their API shape, resolving ratings and
/// tags for the whole set in one query each.
async fn assemble(pool: &PgPool, products: &[Product]) -> Result<Vec<ApiProduct>, AppError> {
    let ids: Vec<i64> = products.iter().map(|p| p.id).collect();
    let ratings = review_repository::aggregate_for_products(pool, &ids).await?;
    let tags = tag_repository::tags_for_products(pool, &ids).await?;
    Ok(products
        .iter()
        .map(|p| {
            let aggregate = ratings.get(&p.id);
            let rating = ApiRating {
                average: aggregate.and_then(|a| a.rating_avg),
                count: aggregate.map(|a| a.rating_count).unwrap_or(0),
            };
            let product_tags = tags
                .get(&p.id)
                .map(|ts| {
                    ts.iter()
                        .map(|t| ApiTag {
                            slug: t.slug.clone(),
                            label: t.label.clone(),
                        })
                        .collect()
                })
                .unwrap_or_default();
            ApiProduct {
                id: p.id,
                name: p.name.clone(),
                path: format!("/products/{}", p.id),
                description: p.description.clone(),
                price_yen: p.effective_price_yen(),
                stock: p.stock,
                allow_backorder: p.allow_backorder,
                purchasable: !p.track_stock || p.allow_backorder || p.stock > 0,
                rating,
                category: ApiCategory {
                    id: p.category_id,
                    name: p.category.clone(),
                },
                tags: product_tags,
                variants: None,
            }
        })
        .collect())
}

/// One page of the public catalog. Pages beyond the end answer
/// an empty product list with the true totals — a machine
/// client branches on `total`, not on an error.
pub async fn list_products(pool: &PgPool, page: i64) -> Result<CatalogPage, AppError> {
    let per_page = CONFIG.api_per_page;
    let page = page.max(1);
    let offset = (page - 1).saturating_mul(per_page);
    let (products, total) = product_repository::catalog_page(pool, per_page, offset).await?;
    let items = assemble(pool, &products).await?;
    Ok(CatalogPage {
        products: items,
        page,
        per_page,
        total,
        total_pages: crate::util::pagination::total_pages(total, per_page),
    })
}

/// One public product for the API; `None` when it does not
/// exist or is not public. The detail carries the product's
/// combinations so an agent can pick one for a cart add.
/// Every listed product's combinations with resolved
/// labels: four bulk queries for the
/// whole set - axes, options, links, variants - where
/// the per-product detail read used to run its own set.
/// Labels resolve from the axis/option vocabulary in axis
/// order, the same pairs the detail page's picker renders.
pub async fn variants_by_product(
    pool: &PgPool,
    products: &[Product],
) -> Result<std::collections::HashMap<i64, Vec<ApiVariant>>, AppError> {
    let ids: Vec<i64> = products.iter().map(|p| p.id).collect();
    let axes = variant_repository::axes_with_options_for_products(pool, &ids).await?;
    let mut option_by_id: std::collections::HashMap<i64, (String, i32, String)> =
        std::collections::HashMap::new();
    for axis in &axes {
        for option in &axis.options {
            option_by_id.insert(
                option.id,
                (
                    axis.axis.name.clone(),
                    axis.axis.sort_order,
                    option.value.clone(),
                ),
            );
        }
    }
    let mut pairs_by_variant: std::collections::HashMap<i64, Vec<(i32, String, String)>> =
        std::collections::HashMap::new();
    for (variant_id, option_id) in variant_repository::links_for_products(pool, &ids).await? {
        if let Some((axis_name, axis_order, value)) = option_by_id.get(&option_id) {
            pairs_by_variant.entry(variant_id).or_default().push((
                *axis_order,
                axis_name.clone(),
                value.clone(),
            ));
        }
    }
    let product_by_id: std::collections::HashMap<i64, &Product> =
        products.iter().map(|p| (p.id, p)).collect();
    let mut by_product: std::collections::HashMap<i64, Vec<ApiVariant>> =
        std::collections::HashMap::new();
    for variant in variant_repository::list_variants_for_products(pool, &ids).await? {
        let Some(product) = product_by_id.get(&variant.product_id) else {
            continue;
        };
        let mut pairs = pairs_by_variant.remove(&variant.id).unwrap_or_default();
        pairs.sort_by_key(|(order, _, _)| *order);
        let label_pairs: Vec<(String, String)> = pairs
            .into_iter()
            .map(|(_, axis, value)| (axis, value))
            .collect();
        by_product
            .entry(variant.product_id)
            .or_default()
            .push(ApiVariant {
                id: variant.id,
                label: crate::models::variant::label_for(&label_pairs),
                price_yen: variant
                    .price_yen
                    .unwrap_or_else(|| product.effective_price_yen()),
                stock: variant.stock,
                purchasable: variant.is_enabled
                    && (!product.track_stock || product.allow_backorder || variant.stock > 0),
            });
    }
    Ok(by_product)
}

pub async fn get_product(pool: &PgPool, id: i64) -> Result<Option<ApiProduct>, AppError> {
    let Some(product) = product_repository::catalog_product(pool, id).await? else {
        return Ok(None);
    };
    let mut items = assemble(pool, std::slice::from_ref(&product)).await?;
    let item = items.pop().expect("assemble maps every input");
    let mut by_product = variants_by_product(pool, std::slice::from_ref(&product)).await?;
    let variants = by_product.remove(&id).unwrap_or_default();
    Ok(Some(ApiProduct {
        variants: Some(variants),
        ..item
    }))
}

/// One page of catalog search results, in the search's own
/// ordering (the list endpoint's id order does not apply here).
#[derive(Serialize, JsonSchema)]
pub struct SearchPage {
    /// The page's products, in result order.
    pub products: Vec<ApiProduct>,
    /// The 1-based page number this response serves.
    pub page: i64,
    /// The page size in effect.
    pub per_page: i64,
    /// Total matches for the same conditions.
    pub total: i64,
    /// Total number of pages at this page size.
    pub total_pages: i64,
}

/// The catalog search inputs for machine clients, bundled so
/// the call site names every knob it sets.
pub struct CatalogSearch<'a> {
    /// Keyword text; empty browses the whole catalog.
    pub query: &'a str,
    /// Category name filter (the snapshot match the storefront
    /// filter uses); `None` searches every category.
    pub category: Option<&'a str>,
    /// Lowest effective price to keep, in yen (inclusive).
    pub min_price_yen: Option<i64>,
    /// Highest effective price to keep, in yen (inclusive).
    pub max_price_yen: Option<i64>,
    /// Sort key: one of the allowlisted keys, already
    /// validated by the caller.
    pub sort: &'a str,
    /// Whether to keep only products with stock.
    pub in_stock_only: bool,
    /// 1-based page number.
    pub page: i64,
}

/// Searches the public catalog with the storefront search's own
/// machinery — the same normalisation, term splitting and public
/// predicate — so an MCP client and a person typing in the
/// storefront search box get the same products for the same
/// words.
pub async fn search_products(
    pool: &PgPool,
    search: CatalogSearch<'_>,
) -> Result<SearchPage, AppError> {
    let per_page = CONFIG.api_per_page;
    let page = search.page.max(1);
    let patterns = search_service::normalize_patterns(search.query);
    let (sort, _) = search_service::parse_sort(Some(search.sort));
    let outcome = search_repository::search(
        pool,
        search_repository::SearchArgs {
            patterns: &patterns,
            category: search.category,
            tag_slug: None,
            in_stock_only: search.in_stock_only,
            min_price: search.min_price_yen,
            max_price: search.max_price_yen,
            sort,
            limit: per_page,
            offset: (page - 1).saturating_mul(per_page),
        },
    )
    .await?;
    let total = outcome.total;
    let items = assemble(pool, &outcome.products).await?;
    Ok(SearchPage {
        products: items,
        page,
        per_page,
        total,
        total_pages: crate::util::pagination::total_pages(total, per_page),
    })
}

/// One product category as exposed to machine clients.
#[derive(Serialize, JsonSchema)]
pub struct ApiCategoryEntry {
    /// Category id.
    pub id: i64,
    /// Display name — the value search filters match on.
    pub name: String,
    /// Stable ASCII identifier.
    pub slug: String,
    /// Parent category id; null at the top level.
    pub parent_id: Option<i64>,
}

/// The category list as one object: machine surfaces that
/// attach a schema to their results (the MCP tools) need an
/// object at the root, not a bare array.
#[derive(Serialize, JsonSchema)]
pub struct CategoryList {
    /// Every category, in display order.
    pub categories: Vec<ApiCategoryEntry>,
}

/// Every product category, in display order.
pub async fn list_categories(pool: &PgPool) -> Result<CategoryList, AppError> {
    let categories = category_repository::list_all(pool).await?;
    Ok(CategoryList {
        categories: categories
            .into_iter()
            .map(|c| ApiCategoryEntry {
                id: c.id,
                name: c.name,
                slug: c.slug,
                parent_id: c.parent_id,
            })
            .collect(),
    })
}

/// The store's identity and shopping basics, from the same
/// shop settings the storefront renders.
#[derive(Serialize, JsonSchema)]
pub struct StoreInfo {
    /// Shop name.
    pub shop_name: String,
    /// Short shop tagline.
    pub tagline: String,
    /// Shop description.
    pub description: String,
    /// Public contact email; empty when the shop shows none.
    pub contact_email: String,
    /// Public contact page URL; empty when the shop shows none.
    pub contact_url: String,
    /// Flat shipping fee in integer yen, tax included.
    pub shipping_flat_yen: i64,
    /// Product subtotal at or above which shipping is free,
    /// in integer yen.
    pub free_shipping_threshold_yen: i64,
    /// Currency every price on this surface uses.
    pub currency: String,
}

/// The store info machine clients introduce the shop with.
pub async fn store_info(pool: &PgPool) -> Result<StoreInfo, AppError> {
    let settings = shop_settings_service::load(pool).await;
    Ok(StoreInfo {
        shop_name: settings.shop_name,
        tagline: settings.tagline,
        description: settings.shop_description,
        contact_email: settings.contact_email,
        contact_url: settings.contact_url,
        shipping_flat_yen: settings.shipping_flat_yen,
        free_shipping_threshold_yen: settings.free_shipping_threshold_yen,
        currency: "JPY".to_string(),
    })
}
