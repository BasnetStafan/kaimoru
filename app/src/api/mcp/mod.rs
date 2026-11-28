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

mod tools;

use std::sync::Arc;

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

use self::tools::{GetProductParams, SearchProductsParams, validate_search};
use crate::services::api_catalog_service::{ApiProduct, SearchPage, StoreInfo};
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

/// Maps a service failure to a generic protocol error. The
/// service layer has already logged the detail (the AppError
/// contract); internal detail never reaches a client.
fn service_error(_error: AppError) -> McpError {
    McpError::internal_error("the store could not answer just now", None)
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
            "Kaimoru is a Japanese e-commerce store. This server exposes its public catalog: search and read products, list categories, and read store information and published pages. Prices are integer Japanese yen, tax included. The server is read-only: it cannot place orders or change anything.",
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
