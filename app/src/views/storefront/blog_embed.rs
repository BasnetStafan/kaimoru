//! Blog embed views (CMS blog embeds, CMS_SPEC §8).
//!
//! The cards a post's shortcodes expand into: one product
//! card, and the category showcase that grids several of
//! them. They are rendered by the blog service per request
//! and spliced into the post body — server-side, so the
//! markup (and the add-to-cart form) is in the page HTML
//! itself, exactly like a catalog card. The card mirrors
//! the catalog card's price line (sale strike-through,
//! tax-inclusive price, tax-class note) and stock badges;
//! it deliberately carries no rating or favourite toggle —
//! an embed is a purchase card, not a listing row.

use askama::Template;

use crate::models::Product;

/// What an embed card's action area offers, decided from
/// the product alone — the catalog card's purchasability
/// rules, factored pure so they are unit-testable:
///
/// - a tracked, non-backorderable product at zero stock is
///   sold out: no purchase path at all;
/// - a multi-variant product must never have a combination
///   chosen for the customer, so the card links to the
///   detail page instead of adding;
/// - anything else adds directly, quantity 1, through the
///   cart endpoint (the server resolves a single-combination
///   product's variant, as it does for catalog cards).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum EmbedAction {
    /// The add-to-cart form (adds quantity 1).
    Add,
    /// A link to the product page （詳細を見る）.
    Detail,
    /// A disabled sold-out button.
    SoldOut,
}

impl EmbedAction {
    /// The action for one product, in the catalog card's
    /// own precedence: sold out first, then variants, then
    /// the direct add.
    pub fn for_product(product: &Product) -> Self {
        if product.track_stock && !product.allow_backorder && product.stock == 0 {
            EmbedAction::SoldOut
        } else if product.has_variants {
            EmbedAction::Detail
        } else {
            EmbedAction::Add
        }
    }
}

/// One product card inside a post body: the product plus
/// its decided action. Renders
/// parts/blog_embed_product.html, which the category
/// showcase template also includes per card.
pub struct BlogEmbedCard {
    /// The product on the card.
    pub product: Product,
    /// What the card's action area offers.
    pub action: EmbedAction,
}

impl BlogEmbedCard {
    /// Builds the card for one product, deciding its action.
    pub fn new(product: Product) -> Self {
        let action = EmbedAction::for_product(&product);
        BlogEmbedCard { product, action }
    }

    /// Whether the action area is the add-to-cart form.
    pub fn action_is_add(&self) -> bool {
        self.action == EmbedAction::Add
    }

    /// Whether the action area is the detail-page link.
    pub fn action_is_detail(&self) -> bool {
        self.action == EmbedAction::Detail
    }

    /// Whether the action area is the disabled sold-out
    /// button.
    pub fn action_is_sold_out(&self) -> bool {
        self.action == EmbedAction::SoldOut
    }
}

/// Renders parts/blog_embed_product.html on its own: one
/// product embed's expansion.
#[derive(Template)]
#[template(path = "storefront/parts/blog_embed_product.html")]
pub struct BlogEmbedProductView {
    /// The card the template renders (named `card`, the
    /// catalog templates' variable for the same markup).
    pub card: BlogEmbedCard,
}

/// Renders parts/blog_embed_category.html: one category
/// embed's expansion — the category name linking to its
/// storefront category page, a grid of its product cards,
/// and the see-all link to the same page.
#[derive(Template)]
#[template(path = "storefront/parts/blog_embed_category.html")]
pub struct BlogEmbedCategoryView {
    /// The category's display name; the template builds
    /// the category-page URL from it, like the category
    /// index does.
    pub name: String,
    /// The showcase's cards (the category's public
    /// products, capped by the service).
    pub cards: Vec<BlogEmbedCard>,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A bare product with the embed-relevant fields set by
    /// each test (the product service tests' constructor
    /// shape).
    fn product() -> Product {
        Product {
            id: 1,
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
            track_stock: true,
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
    fn simple_in_stock_product_adds_directly() {
        let mut p = product();
        p.stock = 3;
        assert_eq!(EmbedAction::for_product(&p), EmbedAction::Add);
    }

    #[test]
    fn variant_product_links_to_detail() {
        let mut p = product();
        p.stock = 3;
        p.has_variants = true;
        assert_eq!(EmbedAction::for_product(&p), EmbedAction::Detail);
    }

    #[test]
    fn sold_out_beats_variants() {
        let mut p = product();
        p.stock = 0;
        p.has_variants = true;
        assert_eq!(EmbedAction::for_product(&p), EmbedAction::SoldOut);
    }

    #[test]
    fn backorderable_zero_stock_still_adds() {
        let mut p = product();
        p.stock = 0;
        p.allow_backorder = true;
        assert_eq!(EmbedAction::for_product(&p), EmbedAction::Add);
    }

    #[test]
    fn untracked_zero_stock_still_adds() {
        let mut p = product();
        p.stock = 0;
        p.track_stock = false;
        assert_eq!(EmbedAction::for_product(&p), EmbedAction::Add);
    }
}
