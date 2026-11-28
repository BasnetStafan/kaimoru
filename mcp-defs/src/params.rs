//! The MCP tools' parameter types: the input contract of
//! every tool, shared by both transports. Boundary
//! validation of these values lives in the app
//! (`app::api::mcp::tools`), next to the store rules it
//! mirrors; this crate carries the shapes only.

use schemars::JsonSchema;
use serde::Deserialize;

/// Parameters for `search_products`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct SearchProductsParams {
    /// Keyword text matched against product names and
    /// categories (multi-term AND). Empty or omitted browses
    /// the whole catalog.
    pub query: Option<String>,
    /// Category name, exactly as `list_categories` reports it.
    pub category: Option<String>,
    /// Lowest price to keep, in integer yen (inclusive).
    pub min_price_yen: Option<i64>,
    /// Highest price to keep, in integer yen (inclusive).
    pub max_price_yen: Option<i64>,
    /// One of: recommended, newest, price_asc, price_desc,
    /// rating. Defaults to recommended.
    pub sort: Option<String>,
    /// Keep only products with stock on hand.
    pub in_stock_only: Option<bool>,
    /// 1-based page number. Defaults to 1.
    pub page: Option<i64>,
}

/// Parameters for `get_product`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetProductParams {
    /// The product id, from a search or list result.
    pub id: i64,
}

/// Parameters for the cart tools that address one cart.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct CartIdParams {
    /// The cart handle, from `create_cart`.
    pub cart_id: String,
}

/// Parameters for `add_cart_item`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct AddCartItemParams {
    /// The cart handle, from `create_cart`.
    pub cart_id: String,
    /// The product id, from a search or list result.
    pub product_id: i64,
    /// The combination to add; required when the product
    /// has combinations, omitted for a plain product.
    pub variant_id: Option<i64>,
    /// Units to add (1 to 999999).
    pub quantity: i32,
}

/// Parameters for `update_cart_item`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct UpdateCartItemParams {
    /// The cart handle, from `create_cart`.
    pub cart_id: String,
    /// The line's variant id.
    pub variant_id: i64,
    /// The line's new quantity (at least 1; stock-capped
    /// by the cart service).
    pub quantity: i32,
}

/// Parameters for `remove_cart_item`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct RemoveCartItemParams {
    /// The cart handle, from `create_cart`.
    pub cart_id: String,
    /// The line's variant id.
    pub variant_id: i64,
}

/// Parameters for `create_order_draft`: the cart to
/// freeze, the idempotency key that makes the create safe
/// to retry, and the delivery address the draft orders
/// to — field for field the guest checkout's delivery
/// step, because the draft is placed as a guest order.
// No Debug derive: the address is personal data, and a
// parameter struct is one log line away from leaking it.
#[derive(Deserialize, JsonSchema)]
pub struct CreateOrderDraftParams {
    /// The cart handle, from `create_cart`.
    pub cart_id: String,
    /// A caller-chosen key: repeating a create with the
    /// same key returns the same draft instead of drafting
    /// twice. At most 128 characters.
    pub idempotency_key: String,
    /// The delivery address the order will ship to.
    pub address: DraftAddressParams,
}

/// The delivery address inside [`CreateOrderDraftParams`].
#[derive(Deserialize, JsonSchema)]
pub struct DraftAddressParams {
    /// Contact email the order confirms to.
    pub email: String,
    /// Recipient family name.
    pub family_name: String,
    /// Recipient given name.
    pub given_name: String,
    /// Postal code (hyphens optional).
    pub postal_code: String,
    /// Prefecture.
    pub prefecture: String,
    /// City / ward.
    pub city: String,
    /// Street address.
    pub address_line: String,
    /// Building, when given.
    pub building: Option<String>,
    /// Phone number (hyphens optional).
    pub phone: String,
}

/// Parameters for `get_order_draft`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct DraftIdParams {
    /// The draft id, from `create_order_draft`.
    pub draft_id: i64,
}

/// Parameters for `get_order_status`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct OrderIdParams {
    /// The order id, from `get_order_draft` once its
    /// draft is consumed.
    pub order_id: i64,
}
