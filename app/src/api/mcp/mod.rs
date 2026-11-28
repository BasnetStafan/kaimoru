//! MCP (Model Context Protocol) server — the store's public
//! catalog for external AI clients (Claude, Cursor, and other
//! MCP hosts), mounted at `/mcp` when `mcp_enabled` is on.
//!
//! Phase 1 is read-only and anonymous by design: every tool
//! and resource serves exactly what the storefront and the
//! agent API already expose without a login — public
//! products, categories, store information and the published
//! pages. No member data, no orders, no writes. The handlers
//! call the same catalog service the agent API uses, so an MCP
//! client and a person browsing the store never see different
//! facts.
//!
//! Phase 2 adds cart tools behind bearer API keys (scope
//! `cart:write`). A cart is bound to the key that created it
//! and is checked on every call; checkout is never completed
//! here — `get_checkout_link` mints a single-use claim URL
//! and a person finishes in their own browser.

mod tools;

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant};

use rmcp::{
    ErrorData as McpError, RoleServer,
    handler::server::{
        ServerHandler,
        wrapper::{Json, Parameters},
    },
    model::*,
    service::RequestContext,
    tool, tool_router,
    transport::{
        StreamableHttpServerConfig, StreamableHttpService,
        streamable_http_server::session::never::NeverSessionManager,
    },
};

use self::tools::{
    AddCartItemParams, CartIdParams, GetProductParams, McpCart, McpCheckoutLink,
    RemoveCartItemParams, SearchProductsParams, UpdateCartItemParams, validate_search,
};
use crate::config::CONFIG;
use crate::services::api_cart_service::{self, AddError};
use crate::services::api_catalog_service::{ApiProduct, SearchPage, StoreInfo};
use crate::services::api_key_service::{self, ApiKeyIdentity};
use crate::services::cart_service::{self, CartOwner};
use crate::services::mcp_cart_service;
use sqlx::PgPool;

use crate::error::AppError;
use crate::repositories::content_page_repository;
use crate::services::api_catalog_service;

/// The store's MCP handler: a thin protocol shell over the
/// shared catalog service. One instance serves one request —
/// the transport is stateless.
#[derive(Clone)]
pub struct StoreMcpHandler {
    /// The shared database pool.
    pool: PgPool,
}

impl StoreMcpHandler {
    /// Builds the handler over the app's pool.
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Authenticates a cart-tool call: the bearer key from
    /// the HTTP request's `Authorization` header (the
    /// transport carries the request parts in the context
    /// extensions — Axum middleware extensions do not reach
    /// here, so the header is read from the parts directly),
    /// holding the `cart:write` scope, inside its rate
    /// budget. Every failure is the same error.
    async fn require_cart_key(
        &self,
        context: &RequestContext<RoleServer>,
    ) -> Result<ApiKeyIdentity, McpError> {
        let parts = context
            .extensions
            .get::<axum::http::request::Parts>()
            .ok_or_else(auth_error)?;
        let token = parts
            .headers
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .and_then(|header| header.strip_prefix("Bearer "))
            .ok_or_else(auth_error)?;
        let identity = api_key_service::authenticate(&self.pool, token)
            .await
            .map_err(|()| auth_error())?;
        if identity.scope != api_key_service::SCOPE_CART_WRITE {
            return Err(auth_error());
        }
        check_cart_rate_limit(identity.id)?;
        Ok(identity)
    }

    /// Resolves a cart handle to its owner for a call: the
    /// cart must exist AND belong to the caller's key. Both
    /// failures answer identically, so one key cannot probe
    /// for another key's carts.
    async fn owned_cart(
        &self,
        identity: &ApiKeyIdentity,
        cart_id: &str,
    ) -> Result<CartOwner, McpError> {
        let owner = mcp_cart_service::owner_key_id(&self.pool, cart_id)
            .await
            .map_err(service_error)?;
        match owner {
            Some(key_id) if key_id == identity.id => Ok(CartOwner::Session(cart_id.to_string())),
            _ => Err(McpError::invalid_params(
                format!("no cart with id {cart_id} for this API key"),
                None,
            )),
        }
    }

    /// Reads the caller's cart back after a write (or for a
    /// plain read), under its handle.
    async fn cart_view(&self, cart_id: &str, owner: &CartOwner) -> Result<Json<McpCart>, McpError> {
        let cart = api_cart_service::view_for_owner(&self.pool, owner)
            .await
            .map_err(service_error)?;
        Ok(Json(McpCart::new(cart_id.to_string(), cart)))
    }
}

/// Maps a cart add refusal to the protocol answer, in the
/// same prose shapes the catalog tools use: a refusal about
/// the caller's input is `invalid_params`; a store failure
/// stays the generic internal error.
fn add_error(error: AddError, product_id: i64, variant_id: Option<i64>) -> McpError {
    match error {
        AddError::ProductNotFound => {
            McpError::invalid_params(format!("no public product with id {product_id}"), None)
        }
        AddError::VariantRequired => McpError::invalid_params(
            format!("product {product_id} has combinations; variant_id is required"),
            None,
        ),
        AddError::VariantNotFound => McpError::invalid_params(
            format!(
                "variant {} does not belong to product {product_id}",
                variant_id.unwrap_or_default()
            ),
            None,
        ),
        AddError::SoldOut => {
            McpError::invalid_params("the requested combination is sold out", None)
        }
        AddError::InvalidQuantity => {
            McpError::invalid_params("quantity must be between 1 and 999999", None)
        }
        AddError::Internal(app_error) => service_error(app_error),
    }
}

/// Builds the streamable-HTTP MCP service for the router:
/// sessionless (both the current stateless mode and the
/// 2025-era compatibility mode), JSON responses.
pub fn service(pool: PgPool) -> StreamableHttpService<StoreMcpHandler, NeverSessionManager> {
    StreamableHttpService::new(
        move || Ok(StoreMcpHandler::new(pool.clone())),
        Arc::new(NeverSessionManager::default()),
        StreamableHttpServerConfig::default()
            .with_legacy_session_mode(false)
            .with_json_response(true),
    )
}

/// The one answer a store-side failure gives a client.
const SERVICE_ERROR_MESSAGE: &str = "the store could not answer just now";

/// Builds the generic internal error.
fn internal_error() -> McpError {
    McpError::internal_error(SERVICE_ERROR_MESSAGE, None)
}

/// Maps a service failure to a generic protocol error. The
/// service layer has already logged the detail (the AppError
/// contract); internal detail never reaches a client.
fn service_error(_error: AppError) -> McpError {
    internal_error()
}

/// The one answer every cart-tool authentication failure
/// gives — missing, malformed, unknown, revoked and
/// wrong-scope keys are indistinguishable, so the endpoint
/// cannot be used as an oracle for probing keys.
const AUTH_ERROR_MESSAGE: &str = "a valid API key with the cart:write scope is required";

/// The rate-limit answer for a key over its cart budget.
const RATE_LIMIT_MESSAGE: &str = "rate limit exceeded for this API key; retry shortly";

/// Cart-tool calls allowed per key inside one window.
const CART_CALLS_PER_WINDOW: u32 = 120;

/// The rate-limit window for cart tools.
const CART_WINDOW: Duration = Duration::from_secs(60);

/// Error code for cart-tool authentication failures (a
/// server-defined code in the JSON-RPC reserved range).
const AUTH_ERROR_CODE: i32 = -32001;

/// Error code for cart-tool rate limiting.
const RATE_LIMIT_ERROR_CODE: i32 = -32000;

/// Builds the shared authentication failure.
fn auth_error() -> McpError {
    McpError::new(ErrorCode(AUTH_ERROR_CODE), AUTH_ERROR_MESSAGE, None)
}

/// Per-key cart-call windows: key ID → (window start, calls
/// in the window). In-memory by design — a restart resets
/// the windows, which only ever makes the limit stricter
/// from the caller's side after enough new calls.
static CART_RATE_LIMIT: LazyLock<Mutex<HashMap<i64, (Instant, u32)>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Counts one cart-tool call against the key's window and
/// refuses once the budget is spent. The lock is never held
/// across an await; a poisoned lock recovers its contents
/// rather than panicking (the counts are advisory state).
fn check_cart_rate_limit(key_id: i64) -> Result<(), McpError> {
    let now = Instant::now();
    let mut windows = CART_RATE_LIMIT
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let entry = windows.entry(key_id).or_insert((now, 0));
    if now.duration_since(entry.0) >= CART_WINDOW {
        *entry = (now, 0);
    }
    entry.1 += 1;
    if entry.1 > CART_CALLS_PER_WINDOW {
        return Err(McpError::new(
            ErrorCode(RATE_LIMIT_ERROR_CODE),
            RATE_LIMIT_MESSAGE,
            None,
        ));
    }
    Ok(())
}

/// The published system pages, in the order a person meets
/// them in the store footer.
const SYSTEM_PAGE_SLUGS: [&str; 5] = ["guide", "faq", "terms", "privacy", "legal"];

#[rmcp::tool_handler]
impl ServerHandler for StoreMcpHandler {
    fn get_info(&self) -> InitializeResult {
        InitializeResult::new(
            ServerCapabilities::builder()
                .enable_tools()
                .enable_resources()
                .build(),
        )
        .with_server_info(Implementation::new("kaimoru", env!("CARGO_PKG_VERSION")))
        .with_instructions(
            "Kaimoru is a Japanese e-commerce store. This server exposes its public catalog: search and read products, list categories, and read store information and published pages. Prices are integer Japanese yen, tax included. With an API key (cart:write scope) it also builds carts; checkout always finishes in the customer's own browser via the link from get_checkout_link. The server never places orders and never takes payment details.",
        )
    }

    async fn list_resources(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListResourcesResult, McpError> {
        let mut resources = vec![
            Resource::new("store://store/info", "Store information")
                .with_mime_type("application/json"),
        ];
        for slug in SYSTEM_PAGE_SLUGS {
            if let Some(page) = content_page_repository::find_published_by_slug(&self.pool, slug)
                .await
                .map_err(service_error)?
            {
                resources.push(
                    Resource::new(format!("store://pages/{}", page.slug), page.title)
                        .with_mime_type("text/markdown"),
                );
            }
        }
        for (slug, _updated_at) in content_page_repository::list_published_custom(&self.pool)
            .await
            .map_err(service_error)?
        {
            // The published list carries slugs only; the title
            // comes from the page itself. A page that stopped
            // being published between the two reads is skipped.
            if let Some(page) = content_page_repository::find_published_by_slug(&self.pool, &slug)
                .await
                .map_err(service_error)?
            {
                resources.push(
                    Resource::new(format!("store://pages/{}", page.slug), page.title)
                        .with_mime_type("text/markdown"),
                );
            }
        }
        Ok(ListResourcesResult::with_all_items(resources))
    }

    async fn list_resource_templates(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListResourceTemplatesResult, McpError> {
        Ok(ListResourceTemplatesResult::with_all_items(vec![
            ResourceTemplate::new("store://products/{id}", "Product")
                .with_mime_type("application/json"),
        ]))
    }

    async fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResponse, McpError> {
        let uri = request.uri.as_str();
        if uri == "store://store/info" {
            let info = api_catalog_service::store_info(&self.pool)
                .await
                .map_err(service_error)?;
            return Ok(json_resource(uri, &info));
        }
        if let Some(id_raw) = uri.strip_prefix("store://products/") {
            let id: i64 = id_raw.parse().map_err(|_| {
                McpError::resource_not_found(format!("no such resource: {uri}"), None)
            })?;
            let product = api_catalog_service::get_product(&self.pool, id)
                .await
                .map_err(service_error)?;
            return match product {
                Some(product) => Ok(json_resource(uri, &product)),
                None => Err(McpError::resource_not_found(
                    format!("no such resource: {uri}"),
                    None,
                )),
            };
        }
        if let Some(slug) = uri.strip_prefix("store://pages/") {
            let page = content_page_repository::find_published_by_slug(&self.pool, slug)
                .await
                .map_err(service_error)?;
            return match page {
                Some(page) => {
                    Ok(
                        ReadResourceResult::new(vec![ResourceContents::TextResourceContents {
                            uri: uri.to_string(),
                            mime_type: Some("text/markdown".to_string()),
                            text: page.body_md,
                            meta: None,
                        }])
                        .into(),
                    )
                }
                None => Err(McpError::resource_not_found(
                    format!("no such resource: {uri}"),
                    None,
                )),
            };
        }
        Err(McpError::resource_not_found(
            format!("no such resource: {uri}"),
            None,
        ))
    }
}

#[tool_router]
impl StoreMcpHandler {
    /// Searches the public catalog.
    #[tool(
        description = "Search the store's public catalog by keyword, with optional category, price-range and stock filters and several sort orders. Returns one page of products with prices in integer yen (tax included), stock and ratings — exactly what the storefront shows.",
        output_schema = rmcp::handler::server::common::schema_for_output::<SearchPage>()
    )]
    async fn search_products(
        &self,
        Parameters(params): Parameters<SearchProductsParams>,
    ) -> Result<Json<SearchPage>, McpError> {
        let valid = validate_search(&params)?;
        let query = params.query.as_deref().unwrap_or("");
        let category = params.category.as_deref().filter(|c| !c.is_empty());
        let page = api_catalog_service::search_products(
            &self.pool,
            api_catalog_service::CatalogSearch {
                query,
                category,
                min_price_yen: params.min_price_yen,
                max_price_yen: params.max_price_yen,
                sort: &valid.sort,
                in_stock_only: params.in_stock_only.unwrap_or(false),
                page: valid.page,
            },
        )
        .await
        .map_err(service_error)?;
        Ok(Json(page))
    }

    /// Reads one product in full.
    #[tool(
        description = "Get one public product by id, including its purchasable variants. Use an id from search_products results.",
        output_schema = rmcp::handler::server::common::schema_for_output::<ApiProduct>()
    )]
    async fn get_product(
        &self,
        Parameters(params): Parameters<GetProductParams>,
    ) -> Result<Json<ApiProduct>, McpError> {
        if params.id < 1 {
            return Err(McpError::invalid_params(
                "id must be a positive product id",
                None,
            ));
        }
        let product = api_catalog_service::get_product(&self.pool, params.id)
            .await
            .map_err(service_error)?;
        match product {
            Some(product) => Ok(Json(product)),
            None => Err(McpError::invalid_params(
                format!("no public product with id {}", params.id),
                None,
            )),
        }
    }

    /// Lists the catalog's categories.
    #[tool(
        description = "List the store's product categories (id, name, slug, parent) so searches can be filtered by category name.",
        output_schema = rmcp::handler::server::common::schema_for_output::<api_catalog_service::CategoryList>()
    )]
    async fn list_categories(&self) -> Result<Json<api_catalog_service::CategoryList>, McpError> {
        let categories = api_catalog_service::list_categories(&self.pool)
            .await
            .map_err(service_error)?;
        Ok(Json(categories))
    }

    /// Reads the store's identity and shopping basics.
    #[tool(
        description = "Get the store's identity and shopping basics: name, tagline, description, contact, the flat shipping fee and the free-shipping threshold in yen.",
        output_schema = rmcp::handler::server::common::schema_for_output::<StoreInfo>()
    )]
    async fn get_store_info(&self) -> Result<Json<StoreInfo>, McpError> {
        let info = api_catalog_service::store_info(&self.pool)
            .await
            .map_err(service_error)?;
        Ok(Json(info))
    }

    /// Creates a cart bound to the caller's API key.
    #[tool(
        description = "Create an empty cart and get its cart_id handle. Requires an API key with the cart:write scope. The cart belongs to that key: only the same key can read or change it.",
        output_schema = rmcp::handler::server::common::schema_for_output::<McpCart>()
    )]
    async fn create_cart(
        &self,
        context: RequestContext<RoleServer>,
    ) -> Result<Json<McpCart>, McpError> {
        let identity = self.require_cart_key(&context).await?;
        let cart_id = mcp_cart_service::create_cart(&self.pool, identity.id)
            .await
            .map_err(service_error)?;
        let owner = CartOwner::Session(cart_id.clone());
        self.cart_view(&cart_id, &owner).await
    }

    /// Reads one of the caller's carts.
    #[tool(
        description = "Get one of your carts by cart_id: its lines and totals in integer yen, computed exactly as the store's cart page computes them.",
        output_schema = rmcp::handler::server::common::schema_for_output::<McpCart>()
    )]
    async fn get_cart(
        &self,
        context: RequestContext<RoleServer>,
        Parameters(params): Parameters<CartIdParams>,
    ) -> Result<Json<McpCart>, McpError> {
        let identity = self.require_cart_key(&context).await?;
        let owner = self.owned_cart(&identity, &params.cart_id).await?;
        self.cart_view(&params.cart_id, &owner).await
    }

    /// Adds units of one product to the caller's cart.
    #[tool(
        description = "Add units of a product to one of your carts. Pass variant_id when the product has combinations (see get_product). Stock is re-validated on every call; the answer is the cart read back after the add.",
        output_schema = rmcp::handler::server::common::schema_for_output::<McpCart>()
    )]
    async fn add_cart_item(
        &self,
        context: RequestContext<RoleServer>,
        Parameters(params): Parameters<AddCartItemParams>,
    ) -> Result<Json<McpCart>, McpError> {
        let identity = self.require_cart_key(&context).await?;
        let owner = self.owned_cart(&identity, &params.cart_id).await?;
        api_cart_service::add_item_for_owner(
            &self.pool,
            &owner,
            params.product_id,
            params.variant_id,
            params.quantity,
        )
        .await
        .map_err(|error| add_error(error, params.product_id, params.variant_id))?;
        self.cart_view(&params.cart_id, &owner).await
    }

    /// Sets one cart line's quantity.
    #[tool(
        description = "Set the quantity of one line in one of your carts, addressed by variant_id. The cart service caps the quantity at the stock on hand. Use remove_cart_item to remove a line.",
        output_schema = rmcp::handler::server::common::schema_for_output::<McpCart>()
    )]
    async fn update_cart_item(
        &self,
        context: RequestContext<RoleServer>,
        Parameters(params): Parameters<UpdateCartItemParams>,
    ) -> Result<Json<McpCart>, McpError> {
        let identity = self.require_cart_key(&context).await?;
        let owner = self.owned_cart(&identity, &params.cart_id).await?;
        if params.quantity < 1 {
            return Err(McpError::invalid_params(
                "quantity must be at least 1; use remove_cart_item to remove a line",
                None,
            ));
        }
        cart_service::update_quantity(&self.pool, &owner, params.variant_id, params.quantity)
            .await
            .map_err(service_error)?;
        self.cart_view(&params.cart_id, &owner).await
    }

    /// Removes one cart line.
    #[tool(
        description = "Remove one line from one of your carts, addressed by variant_id.",
        output_schema = rmcp::handler::server::common::schema_for_output::<McpCart>()
    )]
    async fn remove_cart_item(
        &self,
        context: RequestContext<RoleServer>,
        Parameters(params): Parameters<RemoveCartItemParams>,
    ) -> Result<Json<McpCart>, McpError> {
        let identity = self.require_cart_key(&context).await?;
        let owner = self.owned_cart(&identity, &params.cart_id).await?;
        cart_service::remove_line(&self.pool, &owner, params.variant_id)
            .await
            .map_err(service_error)?;
        self.cart_view(&params.cart_id, &owner).await
    }

    /// Mints the browser checkout handoff for a cart.
    #[tool(
        description = "Get a single-use checkout link for one of your carts. A person opens it in a browser within 30 minutes; the cart becomes theirs and they complete checkout and payment on the store site. This tool never places an order or takes payment details.",
        output_schema = rmcp::handler::server::common::schema_for_output::<McpCheckoutLink>()
    )]
    async fn get_checkout_link(
        &self,
        context: RequestContext<RoleServer>,
        Parameters(params): Parameters<CartIdParams>,
    ) -> Result<Json<McpCheckoutLink>, McpError> {
        let identity = self.require_cart_key(&context).await?;
        let owner = self.owned_cart(&identity, &params.cart_id).await?;
        let cart = api_cart_service::view_for_owner(&self.pool, &owner)
            .await
            .map_err(service_error)?;
        if cart.lines.is_empty() {
            return Err(McpError::invalid_params("the cart is empty", None));
        }
        let (token, expires_at) = mcp_cart_service::mint_claim(&self.pool, &params.cart_id)
            .await
            .map_err(service_error)?;
        // The link's base is the store's configured public
        // base when one exists; otherwise the address this
        // very request arrived on (the development shape:
        // a client tunnelled to loopback).
        let base = match CONFIG.public_base_url() {
            Some(base) => base.trim_end_matches('/').to_string(),
            None => {
                let parts = context
                    .extensions
                    .get::<axum::http::request::Parts>()
                    .ok_or_else(internal_error)?;
                let host = parts
                    .headers
                    .get(axum::http::header::HOST)
                    .and_then(|value| value.to_str().ok())
                    .map(str::to_string)
                    .or_else(|| parts.uri.authority().map(|a| a.to_string()))
                    .ok_or_else(internal_error)?;
                format!("http://{host}")
            }
        };
        Ok(Json(McpCheckoutLink {
            cart_id: params.cart_id,
            checkout_url: format!("{base}/cart/claim/{token}"),
            expires_at: expires_at.to_rfc3339(),
        }))
    }
}

/// Serialises a value as a JSON resource body.
fn json_resource<T: serde::Serialize>(uri: &str, value: &T) -> ReadResourceResponse {
    ReadResourceResult::new(vec![ResourceContents::TextResourceContents {
        uri: uri.to_string(),
        mime_type: Some("application/json".to_string()),
        text: serde_json::to_string(value).unwrap_or_default(),
        meta: None,
    }])
    .into()
}
