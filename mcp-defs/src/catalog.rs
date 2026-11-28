//! The catalog and cart view types the machine surfaces
//! expose: the agent API's responses and the MCP tools'
//! results are built from these types, so a person, an
//! agent and an MCP client all read the same product and
//! cart facts.

use schemars::JsonSchema;
use serde::Serialize;

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

/// One cart line as exposed to agents.
#[derive(Serialize, JsonSchema)]
pub struct ApiCartLine {
    /// Variant id: the line's identity.
    pub variant_id: i64,
    /// Product id.
    pub product_id: i64,
    /// Product name.
    pub name: String,
    /// Combination label; empty for a plain product.
    pub variant_label: String,
    /// Units in the cart.
    pub quantity: i32,
    /// Unit price in integer yen, tax included.
    pub unit_price_yen: i64,
    /// Line total in integer yen.
    pub line_total_yen: i64,
    /// Whether the line still counts toward the totals (a
    /// product taken off sale after carting does not).
    pub purchasable: bool,
}

/// The caller's cart as exposed to agents. Totals follow the
/// cart page's own rule: no payment method is chosen yet, so
/// no cash-on-delivery fee is included, and a guest cart never
/// carries a coupon discount.
#[derive(Serialize, JsonSchema)]
pub struct ApiCart {
    /// The cart's lines, in cart order.
    pub lines: Vec<ApiCartLine>,
    /// Total units across all lines.
    pub item_count: i64,
    /// Sum of purchasable line totals, in yen.
    pub subtotal_yen: i64,
    /// Shipping fee in yen (free at the store's threshold).
    pub shipping_yen: i64,
    /// Grand total in yen: subtotal plus shipping.
    pub total_yen: i64,
}
