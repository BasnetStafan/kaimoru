//! MCP (Model Context Protocol) server — the store's public
//! catalog for external AI clients (Claude, Cursor, and other
//! MCP hosts), mounted at `/mcp` when `mcp_enabled` is on.
//!
//! Phase 1 is read-only and anonymous by design: every tool
//! and resource serves exactly what the storefront and the
//! agent API already expose without a login — public
//! products, categories, store information and the published
//! pages. No member data, no orders, no writes. The catalog
//! tools call the same catalog service the agent API uses, so
//! an MCP client and a person browsing the store never see
//! different product facts; the page resources read the
//! published content pages from the content-page repository.
//!
//! Phase 2 adds cart tools behind bearer API keys (scope
//! `cart:write`). A cart is bound to the key that created it
//! and is checked on every call; checkout is never completed
//! here — `get_checkout_link` mints a single-use claim URL
//! and a person finishes in their own browser.
//!
//! Phase 3 adds order drafts behind the `orders:write`
//! scope: a draft freezes a cart and its delivery address
//! behind a single-use approval URL, and the person who
//! opens it approves and pays in the browser — the client
//! never places the order and never confirms a payment.
//! `orders:read` (or the write scope) reads drafts back
//! and the status of the orders they placed.
//!
//! Phase 4 makes the server an OAuth 2.1 resource server:
//! alongside API keys it accepts access tokens issued by
//! the external authorization server (Auth0), validated
//! by the OAuth gate before the transport runs. A token's
//! subject is a principal of its own — its carts and
//! drafts belong to it exactly as a key's belong to the
//! key, and neither kind can reach the other's.

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
    AddCartItemParams, CartIdParams, CreateOrderDraftParams, DraftIdParams, GetProductParams,
    McpCart, McpCheckoutLink, McpDraftLine, McpOrderDraft, McpOrderStatus, OrderIdParams,
    RemoveCartItemParams, SearchProductsParams, UpdateCartItemParams, validate_draft,
    validate_search,
};
use crate::config::CONFIG;
use crate::middleware::mcp_oauth::{OAUTH_SCOPES_HEADER, OAUTH_SUBJECT_HEADER};
use crate::repositories::mcp_order_draft_repository;
use crate::services::api_cart_service::{self, AddError};
use crate::services::api_catalog_service::{ApiProduct, SearchPage, StoreInfo};
use crate::services::api_key_service;
use crate::services::cart_service::{self, CartOwner};
use crate::services::mcp_cart_service;
use crate::services::mcp_order_draft_service::{self, DraftError};
use crate::services::mcp_principal::McpPrincipal;
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

    /// Authenticates a credential-tool call and resolves
    /// the caller's principal: either an OAuth identity the
    /// gate stamped onto the request after validating the
    /// access token, or the API key in the request's
    /// `Authorization` header (the transport carries the
    /// request parts in the context extensions — Axum
    /// middleware extensions do not reach here, so the
    /// headers are read from the parts directly). The
    /// credential must hold one of the accepted scopes.
    /// Every failure — missing, malformed, unknown,
    /// revoked, wrong scope — is the same error carrying
    /// the caller's message, so the endpoint cannot be
    /// used as an oracle for probing credentials.
    async fn authenticate(
        &self,
        context: &RequestContext<RoleServer>,
        scopes: &[&str],
        message: &'static str,
    ) -> Result<McpPrincipal, McpError> {
        let fail = || McpError::new(ErrorCode(AUTH_ERROR_CODE), message, None);
        let parts = context
            .extensions
            .get::<axum::http::request::Parts>()
            .ok_or_else(fail)?;
        // The OAuth path: the internal identity headers
        // exist only when the gate validated a token for
        // this very request (it strips any the client sent
        // itself), so their presence is the proof, and the
        // scope list they carry is the token's own.
        if let Some(subject) = parts
            .headers
            .get(OAUTH_SUBJECT_HEADER)
            .and_then(|value| value.to_str().ok())
            .filter(|subject| !subject.is_empty())
        {
            let granted: Vec<String> = parts
                .headers
                .get(OAUTH_SCOPES_HEADER)
                .and_then(|value| value.to_str().ok())
                .unwrap_or("")
                .split_whitespace()
                .map(ToString::to_string)
                .collect();
            if !scopes
                .iter()
                .any(|scope| granted.iter().any(|g| g == scope))
            {
                return Err(fail());
            }
            return Ok(McpPrincipal::OAuth(subject.to_string()));
        }
        let token = parts
            .headers
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .and_then(|header| header.strip_prefix("Bearer "))
            .ok_or_else(fail)?;
        let identity = api_key_service::authenticate(&self.pool, token)
            .await
            .map_err(|()| fail())?;
        if !scopes.contains(&identity.scope.as_str()) {
            return Err(fail());
        }
        Ok(McpPrincipal::ApiKey(identity.id))
    }

    /// Authenticates a cart-tool call, inside the
    /// principal's cart-call rate budget. An `orders:write`
    /// credential is accepted alongside `cart:write`: keys
    /// are single-scope, and the orders credential must be
    /// able to build the cart its draft binds without a
    /// second credential.
    async fn require_cart_key(
        &self,
        context: &RequestContext<RoleServer>,
    ) -> Result<McpPrincipal, McpError> {
        let principal = self
            .authenticate(
                context,
                &[
                    api_key_service::SCOPE_CART_WRITE,
                    api_key_service::SCOPE_ORDERS_WRITE,
                ],
                AUTH_ERROR_MESSAGE,
            )
            .await?;
        check_cart_rate_limit(&principal.principal_key())?;
        Ok(principal)
    }

    /// Authenticates a draft-creation call: the
    /// `orders:write` scope only. The draft rate budget is
    /// charged by the handler, after the parameters
    /// validate, so a malformed call costs no budget.
    async fn require_orders_write_key(
        &self,
        context: &RequestContext<RoleServer>,
    ) -> Result<McpPrincipal, McpError> {
        self.authenticate(
            context,
            &[api_key_service::SCOPE_ORDERS_WRITE],
            ORDERS_WRITE_AUTH_MESSAGE,
        )
        .await
    }

    /// Authenticates an order-read call (`get_order_draft`,
    /// `get_order_status`): the `orders:read` scope, or the
    /// write scope that created the draft, inside the
    /// principal's call rate budget.
    async fn require_orders_read_key(
        &self,
        context: &RequestContext<RoleServer>,
    ) -> Result<McpPrincipal, McpError> {
        let principal = self
            .authenticate(
                context,
                &[
                    api_key_service::SCOPE_ORDERS_READ,
                    api_key_service::SCOPE_ORDERS_WRITE,
                ],
                ORDERS_READ_AUTH_MESSAGE,
            )
            .await?;
        check_cart_rate_limit(&principal.principal_key())?;
        Ok(principal)
    }

    /// Resolves a cart handle to its owner for a call: the
    /// cart must exist AND belong to the caller's
    /// principal. Both failures answer identically, so one
    /// principal cannot probe for another's carts.
    async fn owned_cart(
        &self,
        principal: &McpPrincipal,
        cart_id: &str,
    ) -> Result<CartOwner, McpError> {
        let owner = mcp_cart_service::owner_principal(&self.pool, cart_id)
            .await
            .map_err(service_error)?;
        match owner {
            Some(principal_key) if principal_key == principal.principal_key() => {
                Ok(CartOwner::Session(cart_id.to_string()))
            }
            _ => Err(McpError::invalid_params(
                format!("no cart with id {cart_id} for this API key"),
                None,
            )),
        }
    }

    /// Voids the cart's pending order drafts after a
    /// successful mutation: the draft's binding is stale
    /// from this moment, eagerly, so it reads `void` —
    /// the pay-time fingerprint re-check remains the hard
    /// guarantee behind this.
    async fn void_drafts(&self, cart_id: &str) -> Result<(), McpError> {
        mcp_order_draft_repository::void_pending_for_cart(&self.pool, cart_id)
            .await
            .map_err(|e| service_error(AppError::Db(e)))?;
        Ok(())
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
            .with_json_response(true)
            .with_allowed_hosts(allowed_hosts()),
    )
}

/// The `Host` values the MCP transport accepts: the SDK's
/// loopback defaults (local clients and SSH-tunnelled ones)
/// plus the store's public host when `PUBLIC_BASE_URL` names
/// one — a public deployment stays unreachable until its
/// own host is on this list (the transport's DNS-rebinding
/// guard). Fails closed: an unset or unparseable public base
/// URL leaves the loopback-only list.
fn allowed_hosts() -> Vec<String> {
    let mut hosts = vec![
        "localhost".to_string(),
        "127.0.0.1".to_string(),
        "::1".to_string(),
    ];
    if let Some(base) = crate::config::CONFIG.public_base_url() {
        match public_host(&base) {
            Some(host) => {
                // A client may spell the scheme's default port
                // out in its `Host` header; accept both forms.
                let with_port = if host.contains(':') {
                    None
                } else if base.starts_with("https://") {
                    Some(format!("{host}:443"))
                } else {
                    Some(format!("{host}:80"))
                };
                if !hosts.contains(&host) {
                    hosts.push(host);
                }
                if let Some(with_port) = with_port
                    && !hosts.contains(&with_port)
                {
                    hosts.push(with_port);
                }
            }
            None => {
                tracing::warn!(
                    "PUBLIC_BASE_URL has no readable host; \
                     the MCP host allowlist stays loopback-only"
                );
            }
        }
    }
    hosts
}

/// Extracts the authority (`host` or `host:port`) from a
/// base URL, dropping any scheme, userinfo and path.
/// Returns `None` when no authority is present.
fn public_host(base: &str) -> Option<String> {
    let after_scheme = base.split("://").nth(1)?;
    let authority = after_scheme.split('/').next()?;
    let authority = authority.rsplit('@').next()?;
    if authority.is_empty() {
        None
    } else {
        Some(authority.to_string())
    }
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
pub(crate) const AUTH_ERROR_MESSAGE: &str = "a valid API key with the cart:write scope is required";

/// The rate-limit answer for a key over its cart budget.
const RATE_LIMIT_MESSAGE: &str = "rate limit exceeded for this API key; retry shortly";

/// Cart-tool calls allowed per key inside one window.
const CART_CALLS_PER_WINDOW: u32 = 120;

/// The rate-limit window for cart tools.
const CART_WINDOW: Duration = Duration::from_secs(60);

/// Error code for cart-tool authentication failures (a
/// server-defined code in the JSON-RPC reserved range).
pub(crate) const AUTH_ERROR_CODE: i32 = -32001;

/// Error code for cart-tool rate limiting.
const RATE_LIMIT_ERROR_CODE: i32 = -32000;

/// The one answer every draft-creation authentication
/// failure gives, on the same no-oracle terms as the cart
/// tools' answer.
const ORDERS_WRITE_AUTH_MESSAGE: &str = "a valid API key with the orders:write scope is required";

/// The one answer every order-read authentication failure
/// gives.
const ORDERS_READ_AUTH_MESSAGE: &str = "a valid API key with the orders:read scope is required";

/// Per-principal cart-call windows: principal key →
/// (window start, calls in the window). In-memory by
/// design — a restart resets the windows, which only ever
/// makes the limit stricter from the caller's side after
/// enough new calls.
static CART_RATE_LIMIT: LazyLock<Mutex<HashMap<String, (Instant, u32)>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Counts one cart-tool call against the principal's
/// window and refuses once the budget is spent. The lock
/// is never held across an await; a poisoned lock recovers
/// its contents rather than panicking (the counts are
/// advisory state). Order reads charge the same budget:
/// it is the principal's general call window, the cart
/// tools' name aside.
fn check_cart_rate_limit(principal_key: &str) -> Result<(), McpError> {
    let now = Instant::now();
    let mut windows = CART_RATE_LIMIT
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let entry = windows.entry(principal_key.to_string()).or_insert((now, 0));
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

/// Draft creations allowed per key inside one window: a
/// draft freezes a cart for a person's decision, so ten
/// an hour is generous for a real client and tight for
/// abuse.
const DRAFT_CALLS_PER_WINDOW: usize = 10;

/// The rate-limit window for draft creation.
const DRAFT_WINDOW: Duration = Duration::from_secs(3600);

/// Recent draft-creation instants per principal key.
/// In-memory like the cart windows: a restart forgets
/// them, which only ever grants a fresh budget.
static DRAFT_RATE_LIMIT: LazyLock<Mutex<HashMap<String, Vec<Instant>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Counts one draft creation against the principal's
/// hourly window and refuses once the budget is spent.
/// Sliding window of instants, the approval limiter's
/// pattern; the lock is never held across an await.
fn check_draft_rate_limit(principal_key: &str) -> Result<(), McpError> {
    let now = Instant::now();
    let mut attempts = DRAFT_RATE_LIMIT
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let times = attempts.entry(principal_key.to_string()).or_default();
    times.retain(|t| now.duration_since(*t) < DRAFT_WINDOW);
    if times.len() >= DRAFT_CALLS_PER_WINDOW {
        return Err(McpError::new(
            ErrorCode(RATE_LIMIT_ERROR_CODE),
            RATE_LIMIT_MESSAGE,
            None,
        ));
    }
    times.push(now);
    Ok(())
}

/// The public base a browser URL is built on: the store's
/// configured public base when one exists, otherwise the
/// address this very request arrived on (the development
/// shape: a client tunnelled to loopback).
fn public_base(context: &RequestContext<RoleServer>) -> Result<String, McpError> {
    match CONFIG.public_base_url() {
        Some(base) => Ok(base.trim_end_matches('/').to_string()),
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
            Ok(format!("http://{host}"))
        }
    }
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
        .with_server_info(Implementation::new(
            mcp_defs::SERVER_NAME,
            env!("CARGO_PKG_VERSION"),
        ))
        .with_instructions(mcp_defs::SERVER_INSTRUCTIONS)
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
        description = mcp_defs::SEARCH_PRODUCTS_DESCRIPTION,
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
        description = mcp_defs::GET_PRODUCT_DESCRIPTION,
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
        description = mcp_defs::LIST_CATEGORIES_DESCRIPTION,
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
        description = mcp_defs::GET_STORE_INFO_DESCRIPTION,
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
        description = mcp_defs::CREATE_CART_DESCRIPTION,
        output_schema = rmcp::handler::server::common::schema_for_output::<McpCart>()
    )]
    async fn create_cart(
        &self,
        context: RequestContext<RoleServer>,
    ) -> Result<Json<McpCart>, McpError> {
        let principal = self.require_cart_key(&context).await?;
        let cart_id = mcp_cart_service::create_cart(&self.pool, &principal)
            .await
            .map_err(service_error)?;
        let owner = CartOwner::Session(cart_id.clone());
        self.cart_view(&cart_id, &owner).await
    }

    /// Reads one of the caller's carts.
    #[tool(
        description = mcp_defs::GET_CART_DESCRIPTION,
        output_schema = rmcp::handler::server::common::schema_for_output::<McpCart>()
    )]
    async fn get_cart(
        &self,
        context: RequestContext<RoleServer>,
        Parameters(params): Parameters<CartIdParams>,
    ) -> Result<Json<McpCart>, McpError> {
        let principal = self.require_cart_key(&context).await?;
        let owner = self.owned_cart(&principal, &params.cart_id).await?;
        self.cart_view(&params.cart_id, &owner).await
    }

    /// Adds units of one product to the caller's cart.
    #[tool(
        description = mcp_defs::ADD_CART_ITEM_DESCRIPTION,
        output_schema = rmcp::handler::server::common::schema_for_output::<McpCart>()
    )]
    async fn add_cart_item(
        &self,
        context: RequestContext<RoleServer>,
        Parameters(params): Parameters<AddCartItemParams>,
    ) -> Result<Json<McpCart>, McpError> {
        let principal = self.require_cart_key(&context).await?;
        let owner = self.owned_cart(&principal, &params.cart_id).await?;
        api_cart_service::add_item_for_owner(
            &self.pool,
            &owner,
            params.product_id,
            params.variant_id,
            params.quantity,
        )
        .await
        .map_err(|error| add_error(error, params.product_id, params.variant_id))?;
        self.void_drafts(&params.cart_id).await?;
        self.cart_view(&params.cart_id, &owner).await
    }

    /// Sets one cart line's quantity.
    #[tool(
        description = mcp_defs::UPDATE_CART_ITEM_DESCRIPTION,
        output_schema = rmcp::handler::server::common::schema_for_output::<McpCart>()
    )]
    async fn update_cart_item(
        &self,
        context: RequestContext<RoleServer>,
        Parameters(params): Parameters<UpdateCartItemParams>,
    ) -> Result<Json<McpCart>, McpError> {
        let principal = self.require_cart_key(&context).await?;
        let owner = self.owned_cart(&principal, &params.cart_id).await?;
        if params.quantity < 1 {
            return Err(McpError::invalid_params(
                "quantity must be at least 1; use remove_cart_item to remove a line",
                None,
            ));
        }
        cart_service::update_quantity(&self.pool, &owner, params.variant_id, params.quantity)
            .await
            .map_err(service_error)?;
        self.void_drafts(&params.cart_id).await?;
        self.cart_view(&params.cart_id, &owner).await
    }

    /// Removes one cart line.
    #[tool(
        description = mcp_defs::REMOVE_CART_ITEM_DESCRIPTION,
        output_schema = rmcp::handler::server::common::schema_for_output::<McpCart>()
    )]
    async fn remove_cart_item(
        &self,
        context: RequestContext<RoleServer>,
        Parameters(params): Parameters<RemoveCartItemParams>,
    ) -> Result<Json<McpCart>, McpError> {
        let principal = self.require_cart_key(&context).await?;
        let owner = self.owned_cart(&principal, &params.cart_id).await?;
        cart_service::remove_line(&self.pool, &owner, params.variant_id)
            .await
            .map_err(service_error)?;
        self.void_drafts(&params.cart_id).await?;
        self.cart_view(&params.cart_id, &owner).await
    }

    /// Mints the browser checkout handoff for a cart.
    #[tool(
        description = mcp_defs::GET_CHECKOUT_LINK_DESCRIPTION,
        output_schema = rmcp::handler::server::common::schema_for_output::<McpCheckoutLink>()
    )]
    async fn get_checkout_link(
        &self,
        context: RequestContext<RoleServer>,
        Parameters(params): Parameters<CartIdParams>,
    ) -> Result<Json<McpCheckoutLink>, McpError> {
        let principal = self.require_cart_key(&context).await?;
        let owner = self.owned_cart(&principal, &params.cart_id).await?;
        let cart = api_cart_service::view_for_owner(&self.pool, &owner)
            .await
            .map_err(service_error)?;
        if cart.lines.is_empty() {
            return Err(McpError::invalid_params("the cart is empty", None));
        }
        let (token, expires_at) = mcp_cart_service::mint_claim(&self.pool, &params.cart_id)
            .await
            .map_err(service_error)?;
        let base = public_base(&context)?;
        Ok(Json(McpCheckoutLink {
            cart_id: params.cart_id,
            checkout_url: format!("{base}/cart/claim/{token}"),
            expires_at: expires_at.to_rfc3339(),
        }))
    }

    /// Freezes a cart into an order draft for human approval.
    #[tool(
        description = mcp_defs::CREATE_ORDER_DRAFT_DESCRIPTION,
        output_schema = rmcp::handler::server::common::schema_for_output::<McpOrderDraft>()
    )]
    async fn create_order_draft(
        &self,
        context: RequestContext<RoleServer>,
        Parameters(params): Parameters<CreateOrderDraftParams>,
    ) -> Result<Json<McpOrderDraft>, McpError> {
        let principal = self.require_orders_write_key(&context).await?;
        let valid = validate_draft(&params)?;
        check_draft_rate_limit(&principal.principal_key())?;
        let (row, token) = mcp_order_draft_service::create_draft(
            &self.pool,
            &principal,
            &params.cart_id,
            &valid.idempotency_key,
            &valid.address,
        )
        .await
        .map_err(|error| match error {
            DraftError::CartNotOwned => McpError::invalid_params(
                format!("no cart with id {} for this API key", params.cart_id),
                None,
            ),
            DraftError::EmptyCart => {
                McpError::invalid_params("the cart has no purchasable lines", None)
            }
            DraftError::Internal(app_error) => service_error(app_error),
        })?;
        // The approval URL exists only on the creating
        // call: the token is never stored, so a replayed
        // create returns the draft without it.
        let approval_url = match token {
            Some(token) => {
                let base = public_base(&context)?;
                Some(format!("{base}/orders/draft/{token}"))
            }
            None => None,
        };
        draft_answer(&row, approval_url)
    }

    /// Reads one of the caller's order drafts.
    #[tool(
        description = mcp_defs::GET_ORDER_DRAFT_DESCRIPTION,
        output_schema = rmcp::handler::server::common::schema_for_output::<McpOrderDraft>()
    )]
    async fn get_order_draft(
        &self,
        context: RequestContext<RoleServer>,
        Parameters(params): Parameters<DraftIdParams>,
    ) -> Result<Json<McpOrderDraft>, McpError> {
        let principal = self.require_orders_read_key(&context).await?;
        if params.draft_id < 1 {
            return Err(McpError::invalid_params(
                "draft_id must be a positive draft id",
                None,
            ));
        }
        let row = mcp_order_draft_service::get_draft(
            &self.pool,
            &principal.principal_key(),
            params.draft_id,
        )
        .await
        .map_err(service_error)?;
        match row {
            Some(row) => draft_answer(&row, None),
            None => Err(McpError::invalid_params(
                format!("no draft with id {} for this API key", params.draft_id),
                None,
            )),
        }
    }

    /// Reads the status of an order a draft placed.
    #[tool(
        description = mcp_defs::GET_ORDER_STATUS_DESCRIPTION,
        output_schema = rmcp::handler::server::common::schema_for_output::<McpOrderStatus>()
    )]
    async fn get_order_status(
        &self,
        context: RequestContext<RoleServer>,
        Parameters(params): Parameters<OrderIdParams>,
    ) -> Result<Json<McpOrderStatus>, McpError> {
        let principal = self.require_orders_read_key(&context).await?;
        if params.order_id < 1 {
            return Err(McpError::invalid_params(
                "order_id must be a positive order id",
                None,
            ));
        }
        let status = mcp_order_draft_service::order_status(
            &self.pool,
            &principal.principal_key(),
            params.order_id,
        )
        .await
        .map_err(service_error)?;
        match status {
            Some(status) => Ok(Json(McpOrderStatus {
                order_id: status.order_id,
                status: status.status,
                total_yen: status.total_yen,
                created_at: status.created_at.to_rfc3339(),
                delivered_at: status.delivered_at.map(|at| at.to_rfc3339()),
                carrier: status.carrier,
                tracking_number: status.tracking_number,
            })),
            None => Err(McpError::invalid_params(
                format!("no order with id {} for this API key", params.order_id),
                None,
            )),
        }
    }
}

/// Shapes a stored draft into its tool answer, from the
/// snapshot it froze. A snapshot that no longer parses is
/// a store failure, never a partial answer.
fn draft_answer(
    row: &mcp_order_draft_repository::McpOrderDraftRow,
    approval_url: Option<String>,
) -> Result<Json<McpOrderDraft>, McpError> {
    let snapshot = mcp_order_draft_service::snapshot_of(row).ok_or_else(internal_error)?;
    Ok(Json(McpOrderDraft {
        draft_id: row.id,
        status: row.status.clone(),
        order_id: row.order_id,
        approval_url,
        expires_at: row.expires_at.to_rfc3339(),
        lines: snapshot.lines.iter().map(McpDraftLine::from).collect(),
        subtotal_yen: snapshot.subtotal_yen,
        shipping_yen: snapshot.shipping_yen,
        total_yen: snapshot.total_yen,
    }))
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

#[cfg(test)]
mod tests {
    use super::{StoreMcpHandler, public_host};

    #[test]
    fn public_host_extracts_the_authority() {
        assert_eq!(
            public_host("https://shop.example.jp"),
            Some("shop.example.jp".to_string())
        );
        assert_eq!(
            public_host("https://shop.example.jp/some/path"),
            Some("shop.example.jp".to_string())
        );
        assert_eq!(
            public_host("http://127.0.0.1:3000"),
            Some("127.0.0.1:3000".to_string())
        );
        assert_eq!(
            public_host("https://user@shop.example.jp"),
            Some("shop.example.jp".to_string())
        );
    }

    #[test]
    fn public_host_rejects_urls_without_an_authority() {
        assert_eq!(public_host("shop.example.jp"), None);
        assert_eq!(public_host("https://"), None);
        assert_eq!(public_host("https:///path-only"), None);
        assert_eq!(public_host(""), None);
    }

    /// The stdio adapter serves its listing from the
    /// shared definitions in `mcp-defs`; this router builds
    /// its own from the same types and description
    /// constants. The two listings must agree exactly — a
    /// schema or description change on either side fails
    /// here until both are updated together.
    #[test]
    fn tool_definitions_match_the_shared_definitions() {
        let mut http = StoreMcpHandler::tool_router().list_all();
        let mut shared = mcp_defs::tools();
        http.sort_by_key(|tool| tool.name.to_string());
        shared.sort_by_key(|tool| tool.name.to_string());
        assert_eq!(
            serde_json::to_value(&http).expect("router tools serialise"),
            serde_json::to_value(&shared).expect("shared tools serialise"),
        );
    }
}
