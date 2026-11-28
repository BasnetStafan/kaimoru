//! Product service.
//!
//! The business layer between controllers and the product
//! repository. Deliberately thin: this layer owns the section
//! limits below and the category list's in-memory cache
//! (invalidated on category writes), so the pass-through shape
//! is intentional.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use sqlx::PgPool;

use crate::config::CONFIG;
use crate::error::AppError;
use crate::models::variant::{AxisWithOptions, ProductVariant};
use crate::models::{HomePin, Product, Tag};
use crate::repositories::{
    category_repository, product_image_repository, product_repository, tag_repository,
    variant_repository,
};

/// How many products one top page section row shows: the
/// operator's `home_section_product_count` setting — one
/// value for every top page product section.
fn home_section_product_count() -> i64 {
    CONFIG.home_section_product_count
}

/// How many same-category products the detail page row shows.
const RELATED_LIMIT: i64 = 4;

/// Returns products for the new arrivals section, newest first.
pub async fn new_arrivals(pool: &PgPool) -> Result<Vec<Product>, AppError> {
    Ok(product_repository::find_new_arrivals(pool, home_section_product_count()).await?)
}

/// Returns products for the ranking section, best sellers first.
pub async fn ranking(pool: &PgPool) -> Result<Vec<Product>, AppError> {
    Ok(product_repository::find_ranking(pool, home_section_product_count()).await?)
}

/// Returns products for the event pickup row.
pub async fn event(pool: &PgPool) -> Result<Vec<Product>, AppError> {
    Ok(product_repository::find_event(pool, home_section_product_count()).await?)
}

/// Returns the display limit of a pinnable top page
/// section: how many cards its row shows. [`pin_cap`]
/// derives the rows' pin caps from these sizes. `None` for
/// sections that take no pins.
pub fn section_limit(key: &str) -> Option<i64> {
    match key {
        "new_arrivals" | "ranking" | "event" | "featured" => Some(home_section_product_count()),
        _ => None,
    }
}

/// Returns how many products an operator may pin into a
/// top page section. For the rows the cap is the row's own
/// display size — the shared `home_section_product_count`
/// setting: pinning more could never show, so the editor
/// stops there. The featured section is uncapped (0): its
/// pins are the /special page's list, which shows them
/// all, and the top page row simply leads with the display
/// limit's worth. `None` for sections that take no pins.
pub fn pin_cap(key: &str) -> Option<i64> {
    match key {
        "new_arrivals" | "ranking" | "event" => section_limit(key),
        "featured" => Some(0),
        _ => None,
    }
}

/// Returns how many products stand behind one top page
/// section's row, for the see-more rule: the row links to
/// its full listing only when the total exceeds the
/// display count. Zero for sections with no listing.
/// The totals behind every top page section's see-more
/// rule, loaded with one grouped query
/// instead of one count per section. Sections with no
/// listing are absent from the map.
pub async fn section_totals(
    pool: &PgPool,
) -> Result<std::collections::HashMap<String, i64>, AppError> {
    let (public, event, staff) = product_repository::section_counts(pool).await?;
    Ok(std::collections::HashMap::from([
        ("new_arrivals".to_string(), public),
        ("ranking".to_string(), public),
        ("event".to_string(), event),
        ("staff_picks".to_string(), staff),
    ]))
}

pub async fn section_total(pool: &PgPool, key: &str) -> Result<i64, AppError> {
    Ok(match key {
        "new_arrivals" | "ranking" => product_repository::count_public(pool).await?,
        "event" => product_repository::count_event(pool).await?,
        "staff_picks" => product_repository::count_staff_picks(pool).await?,
        _ => 0,
    })
}

/// The section listing pages' page size: the search
/// page's config size — the store's one paging size.
pub fn section_page_size() -> i64 {
    CONFIG.search_per_page.max(1)
}

/// Returns one page of a section's full listing, with
/// the section's total. The listing follows the general
/// rule — the section's live pins lead in pin order, the
/// non-pinned items follow in the section's own order —
/// and pages by the search page's config size, the
/// store's one paging size.
pub async fn section_page(
    pool: &PgPool,
    key: &str,
    page: i64,
    pins: &[crate::models::HomePin],
) -> Result<(Vec<Product>, i64), AppError> {
    let limit = section_page_size();
    let offset = (page.max(1) - 1) * limit;
    let total = section_total(pool, key).await?;

    let live_pins = pinned_products(pool, pins, key).await.unwrap_or_default();
    let pin_ids: Vec<i64> = live_pins.iter().map(|product| product.id).collect();

    let start = offset as usize;
    let mut items: Vec<Product> = live_pins
        .into_iter()
        .skip(start)
        .take(limit as usize)
        .collect();
    let room = limit as usize - items.len();
    if room > 0 {
        let auto_offset = start.saturating_sub(pin_ids.len()) as i64;
        let auto_items = match key {
            "new_arrivals" => {
                product_repository::find_new_arrivals_page(pool, room as i64, auto_offset, &pin_ids)
                    .await?
            }
            "ranking" => {
                product_repository::find_ranking_page(pool, room as i64, auto_offset, &pin_ids)
                    .await?
            }
            "event" => {
                product_repository::find_event_page(pool, room as i64, auto_offset, &pin_ids)
                    .await?
            }
            "staff_picks" => {
                product_repository::find_staff_picks_page(pool, room as i64, auto_offset, &pin_ids)
                    .await?
            }
            _ => Vec::new(),
        };
        items.extend(auto_items);
    }
    Ok((items, total))
}

/// Returns products for the staff picks section: the
/// operator-flagged おすすめ products, newest first.
pub async fn staff_picks(pool: &PgPool) -> Result<Vec<Product>, AppError> {
    Ok(product_repository::find_staff_picks(pool, home_section_product_count()).await?)
}

/// Returns one section's pinned products in pin order,
/// behind the storefront gate: a pinned product that is
/// not public (or outside its publish window) is skipped,
/// never rendered.
pub async fn pinned_products(
    pool: &PgPool,
    pins: &[HomePin],
    key: &str,
) -> Result<Vec<Product>, AppError> {
    let ids: Vec<i64> = pins
        .iter()
        .filter(|pin| pin.section_key == key)
        .map(|pin| pin.product_id)
        .collect();
    // The repository returns the cards in the ids' order
    // behind the storefront gate, so the result is already
    // the pin order with unrenderable pins skipped.
    Ok(product_repository::find_cards_by_ids(pool, &ids).await?)
}

/// Merges one section's row: the pinned products first, in
/// pin order, then the automatic selection filling the
/// remaining slots up to the section limit, each product
/// at most once.
pub fn merge_section(pinned: Vec<Product>, auto: Vec<Product>, limit: i64) -> Vec<Product> {
    let mut merged = pinned;
    for product in auto {
        if merged.len() >= limit as usize {
            break;
        }
        if !merged.iter().any(|shown| shown.id == product.id) {
            merged.push(product);
        }
    }
    merged.truncate(limit as usize);
    merged
}

/// How long the category navigation may be served from
/// memory before the database is asked again.
const CATEGORIES_TTL: Duration = Duration::from_secs(60);

/// The cached navigation names and when they were read. The
/// admin category service invalidates on its own writes; the
/// TTL bounds the one edge no write hook can see — a product
/// edit giving a category its first public product, or taking
/// its last one away.
static CATEGORIES_CACHE: Mutex<Option<(Instant, Vec<String>)>> = Mutex::new(None);

/// Returns category names for the category navigation: the
/// managed categories that are active and hold public products.
/// How many categories the top page's category nav
/// shows: the operator's `home_category_nav_count`
/// setting. The nav links to the category index when
/// more categories exist than it shows.
pub fn category_nav_count() -> i64 {
    CONFIG.home_category_nav_count
}

pub async fn categories(pool: &PgPool) -> Result<Vec<String>, AppError> {
    if let Ok(cache) = CATEGORIES_CACHE.lock()
        && let Some((read_at, names)) = cache.as_ref()
        && read_at.elapsed() < CATEGORIES_TTL
    {
        return Ok(names.clone());
    }
    let names = category_repository::nav_categories(pool).await?;
    if let Ok(mut cache) = CATEGORIES_CACHE.lock() {
        *cache = Some((Instant::now(), names.clone()));
    }
    Ok(names)
}

/// Drops the cached navigation names, so the next
/// [`categories`] call re-reads them. The admin category
/// service calls this after a create, update or delete
/// commits.
pub fn invalidate_categories_cache() {
    if let Ok(mut cache) = CATEGORIES_CACHE.lock() {
        *cache = None;
    }
}

/// Whether a category with this exact name exists in any state.
/// The search page adds the requested category to its filter
/// list even when inactive, so a bookmarked category URL still
/// shows its own filter as the selected one.
pub async fn category_exists(pool: &PgPool, name: &str) -> Result<bool, AppError> {
    Ok(category_repository::find_by_name(pool, name)
        .await?
        .is_some())
}

/// Whether the linked category is active. A missing or unlinked
/// category is not active: the detail page then prints the
/// category name without linking it, so the storefront never
/// navigates into a retired category.
pub async fn category_is_active(pool: &PgPool, category_id: Option<i64>) -> bool {
    match category_id {
        Some(id) => category_repository::is_active(pool, id)
            .await
            .ok()
            .flatten()
            .unwrap_or(false),
        None => false,
    }
}

/// Returns the tags attached to one product, for the detail
/// page's tag row.
pub async fn product_tags(pool: &PgPool, product_id: i64) -> Result<Vec<Tag>, AppError> {
    tag_repository::tags_of_product(pool, product_id).await
}

/// Returns a product's gallery image paths in display order
/// (main first), for the detail page. Empty only when the
/// product has no image rows — public products always carry
/// at least the main one (the admin publish gate), and the
/// detail controller falls back to the synced main-image
/// accessor for the rest.
pub async fn product_images(pool: &PgPool, product_id: i64) -> Result<Vec<String>, AppError> {
    product_image_repository::list_for_product(pool, product_id).await
}

/// Returns a product's gallery images with alt texts, in
/// display order (main first) — the detail page renders the alt
/// text, falling back to the product name when a row's is empty.
pub async fn product_images_detailed(
    pool: &PgPool,
    product_id: i64,
) -> Result<Vec<(String, String)>, AppError> {
    product_image_repository::list_detailed(pool, product_id).await
}

/// Returns one product by its URL slug, with the same
/// visibility rules as the ID lookup (public and inside its
/// publication window).
pub async fn detail_by_slug(pool: &PgPool, slug: &str) -> Result<Option<Product>, AppError> {
    Ok(product_repository::find_by_slug(pool, slug).await?)
}

/// Returns one product by ID, full detail columns (the detail
/// page and the review form both render from this).
pub async fn detail(pool: &PgPool, product_id: i64) -> Result<Option<Product>, AppError> {
    Ok(product_repository::find_by_id(pool, product_id).await?)
}

/// Returns a product's variation axes with their options, in
/// display order (the detail-page picker).
pub async fn variant_axes(
    pool: &PgPool,
    product_id: i64,
) -> Result<Vec<AxisWithOptions>, AppError> {
    Ok(variant_repository::axes_with_options(pool, product_id).await?)
}

/// Returns a product's combinations in cartesian (grid) order —
/// the order the picker walks for its initial selection. The
/// combinations are regenerated from the axes and matched to
/// the stored rows by signature.
pub async fn variants(pool: &PgPool, product_id: i64) -> Result<Vec<ProductVariant>, AppError> {
    let axes = variant_repository::axes_with_options(pool, product_id).await?;
    variants_for_axes(pool, product_id, &axes).await
}

/// Returns a product's combinations in cartesian (grid) order
/// for axes the caller already loaded, so a page that renders
/// the axes too (the detail picker) pays one axes load instead
/// of the two [`variants`] would run.
pub async fn variants_for_axes(
    pool: &PgPool,
    product_id: i64,
    axes: &[AxisWithOptions],
) -> Result<Vec<ProductVariant>, AppError> {
    let stored = variant_repository::list_variants(pool, product_id).await?;
    let per_axis: Vec<Vec<i64>> = axes
        .iter()
        .map(|a| a.options.iter().map(|o| o.id).collect())
        .collect();
    let mut ordered: Vec<ProductVariant> = Vec::with_capacity(stored.len());
    for combo in crate::models::variant::cartesian(&per_axis) {
        let signature = crate::models::variant::signature_for(&combo);
        if let Some(variant) = stored.iter().find(|v| v.option_signature == signature) {
            ordered.push(variant.clone());
        }
    }
    Ok(ordered)
}

/// Returns same-category products for the detail page row.
///
/// The row is a page section: on a database failure it is omitted
/// with a log line, never fatal to the product page it decorates.
pub async fn related(pool: &PgPool, product: &Product) -> Vec<Product> {
    match product_repository::find_related(pool, &product.category, product.id, RELATED_LIMIT).await
    {
        Ok(products) => products,
        Err(e) => {
            tracing::error!(product_id = product.id, "related products failed: {e}");
            Vec::new()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a bare product with the given id for the
    /// merge tests: only the id is read by the merge.
    fn product(id: i64) -> Product {
        Product {
            id,
            name: String::new(),
            price_yen: 0,
            sale_price_yen: None,
            sale_starts_at: None,
            sale_ends_at: None,
            stock: 0,
            has_variants: false,
            image_path: String::new(),
            image_alt: String::new(),
            category: String::new(),
            category_id: None,
            is_new: false,
            discount_percent: 0,
            is_preorder: false,
            is_event: false,
            is_staff_pick: false,
            description: String::new(),
            product_code: None,
            brand: String::new(),
            manufacturer: String::new(),
            country_of_origin: String::new(),
            spec_text: String::new(),
            features_text: String::new(),
            description_sp: String::new(),
            tax_class_id: 0,
            tax_class_code: String::new(),
            tax_class_name: String::new(),
            track_stock: false,
            allow_backorder: false,
            low_stock_threshold: None,
            points_earn_percent: None,
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

    #[test]
    fn merge_puts_pins_first_and_fills_the_rest() {
        let merged = merge_section(
            vec![product(3), product(1)],
            vec![product(1), product(2), product(4), product(5)],
            4,
        );
        let ids: Vec<i64> = merged.iter().map(|p| p.id).collect();
        assert_eq!(ids, vec![3, 1, 2, 4]);
    }

    #[test]
    fn merge_caps_at_the_limit() {
        let merged = merge_section(vec![product(1), product(2)], vec![product(3)], 2);
        assert_eq!(merged.len(), 2);
    }

    #[test]
    fn merge_with_no_pins_is_the_automatic_row() {
        let merged = merge_section(vec![], vec![product(7), product(8)], 5);
        let ids: Vec<i64> = merged.iter().map(|p| p.id).collect();
        assert_eq!(ids, vec![7, 8]);
    }
}
