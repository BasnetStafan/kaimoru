//! The tool listing both transports serve: the thirteen
//! tools' names and descriptions, and the listing built
//! from the shared parameter and result types with the
//! same schema helpers the HTTP router uses — identical
//! by construction, and pinned by the app's parity test.

use std::any::Any;
use std::sync::Arc;

use rmcp::handler::server::common::{schema_for_empty_input, schema_for_input, schema_for_output};
use rmcp::model::{JsonObject, Tool};
use schemars::JsonSchema;

use crate::catalog::{ApiProduct, CategoryList, SearchPage, StoreInfo};
use crate::outputs::{McpCart, McpCheckoutLink, McpOrderDraft, McpOrderStatus};
use crate::params::{
    AddCartItemParams, CartIdParams, CreateOrderDraftParams, DraftIdParams, GetProductParams,
    OrderIdParams, RemoveCartItemParams, SearchProductsParams, UpdateCartItemParams,
};

/// Name of the `search_products` tool.
pub const SEARCH_PRODUCTS_NAME: &str = "search_products";
/// Description of the `search_products` tool.
pub const SEARCH_PRODUCTS_DESCRIPTION: &str = "Search the store's public catalog by keyword, with optional category, price-range and stock filters and several sort orders. Returns one page of products with prices in integer yen (tax included), stock and ratings — exactly what the storefront shows.";

/// Name of the `get_product` tool.
pub const GET_PRODUCT_NAME: &str = "get_product";
/// Description of the `get_product` tool.
pub const GET_PRODUCT_DESCRIPTION: &str = "Get one public product by id, including its purchasable variants. Use an id from search_products results.";

/// Name of the `list_categories` tool.
pub const LIST_CATEGORIES_NAME: &str = "list_categories";
/// Description of the `list_categories` tool.
pub const LIST_CATEGORIES_DESCRIPTION: &str = "List the store's product categories (id, name, slug, parent) so searches can be filtered by category name.";

/// Name of the `get_store_info` tool.
pub const GET_STORE_INFO_NAME: &str = "get_store_info";
/// Description of the `get_store_info` tool.
pub const GET_STORE_INFO_DESCRIPTION: &str = "Get the store's identity and shopping basics: name, tagline, description, contact, the flat shipping fee and the free-shipping threshold in yen.";

/// Name of the `create_cart` tool.
pub const CREATE_CART_NAME: &str = "create_cart";
/// Description of the `create_cart` tool.
pub const CREATE_CART_DESCRIPTION: &str = "Create an empty cart and get its cart_id handle. Requires an API key with the cart:write scope. The cart belongs to that key: only the same key can read or change it.";

/// Name of the `get_cart` tool.
pub const GET_CART_NAME: &str = "get_cart";
/// Description of the `get_cart` tool.
pub const GET_CART_DESCRIPTION: &str = "Get one of your carts by cart_id: its lines and totals in integer yen, computed exactly as the store's cart page computes them.";

/// Name of the `add_cart_item` tool.
pub const ADD_CART_ITEM_NAME: &str = "add_cart_item";
/// Description of the `add_cart_item` tool.
pub const ADD_CART_ITEM_DESCRIPTION: &str = "Add units of a product to one of your carts. Pass variant_id when the product has combinations (see get_product). Stock is re-validated on every call; the answer is the cart read back after the add.";

/// Name of the `update_cart_item` tool.
pub const UPDATE_CART_ITEM_NAME: &str = "update_cart_item";
/// Description of the `update_cart_item` tool.
pub const UPDATE_CART_ITEM_DESCRIPTION: &str = "Set the quantity of one line in one of your carts, addressed by variant_id. The cart service caps the quantity at the stock on hand. Use remove_cart_item to remove a line.";

/// Name of the `remove_cart_item` tool.
pub const REMOVE_CART_ITEM_NAME: &str = "remove_cart_item";
/// Description of the `remove_cart_item` tool.
pub const REMOVE_CART_ITEM_DESCRIPTION: &str =
    "Remove one line from one of your carts, addressed by variant_id.";

/// Name of the `get_checkout_link` tool.
pub const GET_CHECKOUT_LINK_NAME: &str = "get_checkout_link";
/// Description of the `get_checkout_link` tool.
pub const GET_CHECKOUT_LINK_DESCRIPTION: &str = "Get a single-use checkout link for one of your carts. A person opens it in a browser within 30 minutes; the cart becomes theirs and they complete checkout and payment on the store site. This tool never places an order or takes payment details.";

/// Name of the `create_order_draft` tool.
pub const CREATE_ORDER_DRAFT_NAME: &str = "create_order_draft";
/// Description of the `create_order_draft` tool.
pub const CREATE_ORDER_DRAFT_DESCRIPTION: &str = "Freeze one of your carts, with a delivery address, into an order draft and get its approval URL. Requires an API key with the orders:write scope. A person opens the URL, checks the frozen lines, address and totals, picks a payment method and pays in the browser — this tool never places the order itself, and any later change to the cart voids the draft. Repeating the call with the same idempotency_key returns the same draft, but the approval URL is shown only on the first, creating call.";

/// Name of the `get_order_draft` tool.
pub const GET_ORDER_DRAFT_NAME: &str = "get_order_draft";
/// Description of the `get_order_draft` tool.
pub const GET_ORDER_DRAFT_DESCRIPTION: &str = "Read one of your order drafts by id: its status (pending, consumed, expired or void), the frozen lines and totals from its snapshot, and the placed order's id once the draft is consumed. Requires an orders:read or orders:write key.";

/// Name of the `get_order_status` tool.
pub const GET_ORDER_STATUS_NAME: &str = "get_order_status";
/// Description of the `get_order_status` tool.
pub const GET_ORDER_STATUS_DESCRIPTION: &str = "Read the status of an order one of your drafts placed: fulfilment status, grand total, and the shipment facts (carrier, tracking number, delivery time) once they exist. Orders placed any other way are not visible here. Requires an orders:read or orders:write key.";

/// Builds the tool listing both transports serve, in the
/// HTTP router's declaration order. The schemas come from
/// the same rmcp helpers the router uses, generated from
/// the shared types in this crate.
pub fn tools() -> Vec<Tool> {
    vec![
        Tool::new(
            SEARCH_PRODUCTS_NAME,
            SEARCH_PRODUCTS_DESCRIPTION,
            input_schema::<SearchProductsParams>(),
        )
        .with_raw_output_schema(schema_for_output::<SearchPage>()),
        Tool::new(
            GET_PRODUCT_NAME,
            GET_PRODUCT_DESCRIPTION,
            input_schema::<GetProductParams>(),
        )
        .with_raw_output_schema(schema_for_output::<ApiProduct>()),
        Tool::new(
            LIST_CATEGORIES_NAME,
            LIST_CATEGORIES_DESCRIPTION,
            schema_for_empty_input(),
        )
        .with_raw_output_schema(schema_for_output::<CategoryList>()),
        Tool::new(
            GET_STORE_INFO_NAME,
            GET_STORE_INFO_DESCRIPTION,
            schema_for_empty_input(),
        )
        .with_raw_output_schema(schema_for_output::<StoreInfo>()),
        Tool::new(
            CREATE_CART_NAME,
            CREATE_CART_DESCRIPTION,
            schema_for_empty_input(),
        )
        .with_raw_output_schema(schema_for_output::<McpCart>()),
        Tool::new(
            GET_CART_NAME,
            GET_CART_DESCRIPTION,
            input_schema::<CartIdParams>(),
        )
        .with_raw_output_schema(schema_for_output::<McpCart>()),
        Tool::new(
            ADD_CART_ITEM_NAME,
            ADD_CART_ITEM_DESCRIPTION,
            input_schema::<AddCartItemParams>(),
        )
        .with_raw_output_schema(schema_for_output::<McpCart>()),
        Tool::new(
            UPDATE_CART_ITEM_NAME,
            UPDATE_CART_ITEM_DESCRIPTION,
            input_schema::<UpdateCartItemParams>(),
        )
        .with_raw_output_schema(schema_for_output::<McpCart>()),
        Tool::new(
            REMOVE_CART_ITEM_NAME,
            REMOVE_CART_ITEM_DESCRIPTION,
            input_schema::<RemoveCartItemParams>(),
        )
        .with_raw_output_schema(schema_for_output::<McpCart>()),
        Tool::new(
            GET_CHECKOUT_LINK_NAME,
            GET_CHECKOUT_LINK_DESCRIPTION,
            input_schema::<CartIdParams>(),
        )
        .with_raw_output_schema(schema_for_output::<McpCheckoutLink>()),
        Tool::new(
            CREATE_ORDER_DRAFT_NAME,
            CREATE_ORDER_DRAFT_DESCRIPTION,
            input_schema::<CreateOrderDraftParams>(),
        )
        .with_raw_output_schema(schema_for_output::<McpOrderDraft>()),
        Tool::new(
            GET_ORDER_DRAFT_NAME,
            GET_ORDER_DRAFT_DESCRIPTION,
            input_schema::<DraftIdParams>(),
        )
        .with_raw_output_schema(schema_for_output::<McpOrderDraft>()),
        Tool::new(
            GET_ORDER_STATUS_NAME,
            GET_ORDER_STATUS_DESCRIPTION,
            input_schema::<OrderIdParams>(),
        )
        .with_raw_output_schema(schema_for_output::<McpOrderStatus>()),
    ]
}

/// Generates a tool's input schema from its parameter
/// type, through the same helper the HTTP router uses.
///
/// # Panics
/// Panics when the parameter type's schema root is not an
/// object. Every parameter type in this crate is a plain
/// struct, whose schema root always is one — the panic is
/// the invariant check, not an expected path.
fn input_schema<T: JsonSchema + Any>() -> Arc<JsonObject> {
    schema_for_input::<T>().expect("tool parameter schemas have an object root")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn listing_has_the_thirteen_tools_in_router_order() {
        let listing = tools();
        let names: Vec<&str> = listing.iter().map(|tool| tool.name.as_ref()).collect();
        assert_eq!(
            names,
            [
                "search_products",
                "get_product",
                "list_categories",
                "get_store_info",
                "create_cart",
                "get_cart",
                "add_cart_item",
                "update_cart_item",
                "remove_cart_item",
                "get_checkout_link",
                "create_order_draft",
                "get_order_draft",
                "get_order_status",
            ]
        );
    }

    #[test]
    fn every_tool_carries_a_description_and_schemas() {
        for tool in tools() {
            assert!(
                tool.description
                    .as_deref()
                    .is_some_and(|text| !text.is_empty()),
                "{} has no description",
                tool.name
            );
            assert!(
                tool.input_schema.get("type").and_then(|t| t.as_str()) == Some("object"),
                "{} input schema is not an object",
                tool.name
            );
            assert!(
                tool.output_schema.is_some(),
                "{} has no output schema",
                tool.name
            );
        }
    }
}
