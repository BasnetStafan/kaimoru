//! The MCP tools' parameter types and their boundary
//! validation. The tool handlers themselves live in the
//! parent module, next to the `ServerHandler` impl the
//! `#[tool_handler]` macro completes. Validation lives here,
//! at the protocol boundary — a bad parameter is a protocol
//! error naming the parameter, never a silent fallback.

use rmcp::{ErrorData as McpError, schemars::JsonSchema};
use serde::{Deserialize, Serialize};

use crate::services::api_cart_service::{ApiCart, ApiCartLine};

/// Parameters for `search_products`.
#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct SearchProductsParams {
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
pub(crate) struct GetProductParams {
    /// The product id, from a search or list result.
    pub id: i64,
}

/// Parameters for the cart tools that address one cart.
#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct CartIdParams {
    /// The cart handle, from `create_cart`.
    pub cart_id: String,
}

/// Parameters for `add_cart_item`.
#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct AddCartItemParams {
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
pub(crate) struct UpdateCartItemParams {
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
pub(crate) struct RemoveCartItemParams {
    /// The cart handle, from `create_cart`.
    pub cart_id: String,
    /// The line's variant id.
    pub variant_id: i64,
}

/// One MCP cart as the cart tools return it: the API cart's
/// lines and totals, plus the handle they belong to. Totals
/// come from the store's one totals implementation — the
/// same numbers the cart page shows.
#[derive(Serialize, JsonSchema)]
pub(crate) struct McpCart {
    /// The cart handle.
    pub cart_id: String,
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

impl McpCart {
    /// Wraps an API cart view under its handle.
    pub(crate) fn new(cart_id: String, cart: ApiCart) -> Self {
        Self {
            cart_id,
            lines: cart.lines,
            item_count: cart.item_count,
            subtotal_yen: cart.subtotal_yen,
            shipping_yen: cart.shipping_yen,
            total_yen: cart.total_yen,
        }
    }
}

/// The browser handoff the `get_checkout_link` tool returns:
/// a person opens the URL and the cart becomes theirs.
#[derive(Serialize, JsonSchema)]
pub(crate) struct McpCheckoutLink {
    /// The cart handle the link hands off.
    pub cart_id: String,
    /// The single-use claim URL to open in a browser.
    pub checkout_url: String,
    /// When the link stops working, as an RFC 3339 timestamp.
    pub expires_at: String,
}

/// The sort keys the search tool accepts — the storefront
/// search's allowlist, mirrored for the protocol boundary so
/// an unknown key is refused instead of silently re-sorted.
const SORT_KEYS: [&str; 5] = ["recommended", "newest", "price_asc", "price_desc", "rating"];

/// The validated search inputs the service call runs with.
pub(crate) struct ValidSearch {
    /// 1-based page, at least 1.
    pub(crate) page: i64,
    /// The validated sort key.
    pub(crate) sort: String,
}

/// Validates the search parameters at the boundary.
pub(crate) fn validate_search(params: &SearchProductsParams) -> Result<ValidSearch, McpError> {
    let page = params.page.unwrap_or(1);
    if page < 1 {
        return Err(McpError::invalid_params("page must be 1 or greater", None));
    }
    for (name, value) in [
        ("min_price_yen", params.min_price_yen),
        ("max_price_yen", params.max_price_yen),
    ] {
        if let Some(value) = value
            && value < 0
        {
            return Err(McpError::invalid_params(
                format!("{name} must not be negative"),
                None,
            ));
        }
    }
    if let (Some(min), Some(max)) = (params.min_price_yen, params.max_price_yen)
        && min > max
    {
        return Err(McpError::invalid_params(
            "min_price_yen must not exceed max_price_yen",
            None,
        ));
    }
    let sort = params
        .sort
        .clone()
        .unwrap_or_else(|| "recommended".to_string());
    if !SORT_KEYS.contains(&sort.as_str()) {
        return Err(McpError::invalid_params(
            format!("sort must be one of: {}", SORT_KEYS.join(", ")),
            None,
        ));
    }
    Ok(ValidSearch { page, sort })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds params with only the fields a case sets.
    fn params() -> SearchProductsParams {
        SearchProductsParams {
            query: None,
            category: None,
            min_price_yen: None,
            max_price_yen: None,
            sort: None,
            in_stock_only: None,
            page: None,
        }
    }

    #[test]
    fn defaults_are_valid() {
        let valid = validate_search(&params()).expect("defaults validate");
        assert_eq!(valid.page, 1);
        assert_eq!(valid.sort, "recommended");
    }

    #[test]
    fn page_below_one_is_refused() {
        let mut p = params();
        p.page = Some(0);
        assert!(validate_search(&p).is_err());
    }

    #[test]
    fn negative_price_is_refused() {
        let mut p = params();
        p.min_price_yen = Some(-1);
        assert!(validate_search(&p).is_err());
    }

    #[test]
    fn inverted_price_range_is_refused() {
        let mut p = params();
        p.min_price_yen = Some(5000);
        p.max_price_yen = Some(100);
        assert!(validate_search(&p).is_err());
    }

    #[test]
    fn unknown_sort_is_refused() {
        let mut p = params();
        p.sort = Some("cheapest".to_string());
        assert!(validate_search(&p).is_err());
    }
}
