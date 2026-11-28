//! The Kaimoru MCP tool definitions, shared by the two
//! transports that serve them: the app's streamable-HTTP
//! server and the stdio adapter (`mcp-stdio`). Tool names,
//! descriptions and the parameter and result types live
//! here exactly once, so a client sees the same surface
//! whichever transport it connects to. The app carries a
//! parity test that fails if the HTTP router's listing
//! ever drifts from the listing built here.

pub mod catalog;
pub mod outputs;
pub mod params;
pub mod tools;

pub use catalog::{
    ApiCart, ApiCartLine, ApiCategory, ApiCategoryEntry, ApiProduct, ApiRating, ApiTag, ApiVariant,
    CategoryList, SearchPage, StoreInfo,
};
pub use outputs::{McpCart, McpCheckoutLink, McpDraftLine, McpOrderDraft, McpOrderStatus};
pub use params::{
    AddCartItemParams, CartIdParams, CreateOrderDraftParams, DraftAddressParams, DraftIdParams,
    GetProductParams, OrderIdParams, RemoveCartItemParams, SearchProductsParams,
    UpdateCartItemParams,
};
pub use tools::{
    ADD_CART_ITEM_DESCRIPTION, ADD_CART_ITEM_NAME, CREATE_CART_DESCRIPTION, CREATE_CART_NAME,
    CREATE_ORDER_DRAFT_DESCRIPTION, CREATE_ORDER_DRAFT_NAME, GET_CART_DESCRIPTION, GET_CART_NAME,
    GET_CHECKOUT_LINK_DESCRIPTION, GET_CHECKOUT_LINK_NAME, GET_ORDER_DRAFT_DESCRIPTION,
    GET_ORDER_DRAFT_NAME, GET_ORDER_STATUS_DESCRIPTION, GET_ORDER_STATUS_NAME,
    GET_PRODUCT_DESCRIPTION, GET_PRODUCT_NAME, GET_STORE_INFO_DESCRIPTION, GET_STORE_INFO_NAME,
    LIST_CATEGORIES_DESCRIPTION, LIST_CATEGORIES_NAME, REMOVE_CART_ITEM_DESCRIPTION,
    REMOVE_CART_ITEM_NAME, SEARCH_PRODUCTS_DESCRIPTION, SEARCH_PRODUCTS_NAME,
    UPDATE_CART_ITEM_DESCRIPTION, UPDATE_CART_ITEM_NAME, tools,
};

/// The server's protocol identity (`serverInfo.name`),
/// identical on both transports.
pub const SERVER_NAME: &str = "kaimoru";

/// The instructions the server introduces itself with on
/// `initialize`, identical on both transports.
pub const SERVER_INSTRUCTIONS: &str = "Kaimoru is a Japanese e-commerce store. This server exposes its public catalog: search and read products, list categories, and read store information and published pages. Prices are integer Japanese yen, tax included. With an API key (cart:write scope) it also builds carts; checkout finishes in the customer's own browser via the link from get_checkout_link, or — with an orders:write key — as an order draft from create_order_draft that the customer approves and pays for in the browser. An MCP client never places an order itself and never takes payment details: the person approving in the browser places every order.";
