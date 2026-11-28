//! The Kaimoru MCP stdio adapter: the store's MCP server
//! carried over stdio, for the clients and registries
//! that only speak that transport (Glama's server checker
//! introspects over stdio; so do local MCP hosts).
//!
//! The adapter is a genuine second transport for the same
//! server, not a store of its own: it holds the shared
//! tool definitions from `mcp-defs`, answers `initialize`
//! and `tools/list` locally — with an empty environment,
//! no database and no network — and proxies every tool
//! call, as JSON-RPC, to an upstream streamable-HTTP MCP
//! endpoint (the live store by default). Only an actual
//! tool call touches the network. The store's resources
//! stay HTTP-only; the adapter serves the tools.

mod proxy;

use rmcp::model::{
    CallToolRequestParams, CallToolResponse, Implementation, InitializeResult, ListToolsResult,
    PaginatedRequestParams, ServerCapabilities,
};
use rmcp::service::{RequestContext, ServerInitializeError};
use rmcp::{ErrorData as McpError, RoleServer, ServerHandler, ServiceExt};

use proxy::UpstreamClient;

/// The environment variable that overrides the upstream
/// MCP endpoint.
pub const ENDPOINT_ENV: &str = "KAIMORU_MCP_ENDPOINT";

/// The environment variable carrying the API key the
/// adapter forwards to the upstream as a bearer token.
/// The cart and draft tools refuse calls without one
/// upstream; the catalog tools do not need it.
pub const API_KEY_ENV: &str = "KAIMORU_MCP_API_KEY";

/// The upstream endpoint when none is configured: the
/// live store's MCP endpoint.
pub const DEFAULT_ENDPOINT: &str = "https://ik1-104-58605.vs.sakura.ne.jp/mcp";

/// The adapter's runtime configuration, read from the
/// environment. Both values are optional: with an empty
/// environment the adapter proxies anonymously to the
/// live store. No `Debug` derive: the key is a credential
/// and must not reach a log line through a format call.
pub struct Config {
    /// The upstream streamable-HTTP MCP endpoint.
    pub endpoint: String,
    /// The API key to forward, when one is configured.
    pub api_key: Option<String>,
}

impl Config {
    /// Reads the configuration from the process
    /// environment; empty values count as unset.
    pub fn from_env() -> Self {
        Self::from_values(
            std::env::var(ENDPOINT_ENV).ok(),
            std::env::var(API_KEY_ENV).ok(),
        )
    }

    /// Builds a configuration from raw values, applying
    /// the defaults. Split from [`Config::from_env`] so
    /// the defaulting is testable without touching the
    /// process environment.
    fn from_values(endpoint: Option<String>, api_key: Option<String>) -> Self {
        Self {
            endpoint: endpoint
                .filter(|value| !value.trim().is_empty())
                .unwrap_or_else(|| DEFAULT_ENDPOINT.to_string()),
            api_key: api_key.filter(|value| !value.trim().is_empty()),
        }
    }
}

/// The stdio MCP handler: serves the shared tool
/// definitions and proxies every call upstream. One
/// instance serves the whole session — the transport is
/// one process per client.
#[derive(Clone)]
pub struct ProxyHandler {
    /// The upstream JSON-RPC client.
    upstream: UpstreamClient,
}

impl ProxyHandler {
    /// Builds the handler over an upstream client for the
    /// configured endpoint.
    pub fn new(config: &Config) -> Self {
        Self {
            upstream: UpstreamClient::new(config.endpoint.clone(), config.api_key.clone()),
        }
    }
}

impl ServerHandler for ProxyHandler {
    fn get_info(&self) -> InitializeResult {
        InitializeResult::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new(
                mcp_defs::SERVER_NAME,
                env!("CARGO_PKG_VERSION"),
            ))
            .with_instructions(mcp_defs::SERVER_INSTRUCTIONS)
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, McpError> {
        Ok(ListToolsResult::with_all_items(mcp_defs::tools()))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, McpError> {
        let result = self
            .upstream
            .call_tool(&request.name, request.arguments)
            .await?;
        Ok(CallToolResponse::Complete(result))
    }
}

/// Why a serving session ended in failure: the stdio
/// transport distinguishes a failed initialize handshake
/// from a failed session task, and both stay distinct
/// here so the binary's one error line can name the stage
/// that failed.
#[derive(Debug)]
pub enum RunError {
    /// The stdio service failed its initialize handshake.
    /// Boxed: the handshake error is large and this path
    /// is cold (once, at startup).
    Initialize(Box<ServerInitializeError>),
    /// The session task itself failed.
    Session(tokio::task::JoinError),
}

impl std::fmt::Display for RunError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Initialize(error) => write!(f, "initialize failed: {error}"),
            Self::Session(error) => write!(f, "session failed: {error}"),
        }
    }
}

impl std::error::Error for RunError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Initialize(error) => Some(error.as_ref()),
            Self::Session(error) => Some(error),
        }
    }
}

impl From<ServerInitializeError> for RunError {
    fn from(error: ServerInitializeError) -> Self {
        Self::Initialize(Box::new(error))
    }
}

impl From<tokio::task::JoinError> for RunError {
    fn from(error: tokio::task::JoinError) -> Self {
        Self::Session(error)
    }
}

/// Serves the adapter on the process's stdio until the
/// client closes the stream.
///
/// # Errors
/// Returns [`RunError::Initialize`] when the stdio
/// service fails its handshake and [`RunError::Session`]
/// when the session task fails; a clean client disconnect
/// is not an error.
pub async fn run(config: Config) -> Result<(), RunError> {
    let service = ProxyHandler::new(&config)
        .serve(rmcp::transport::stdio())
        .await?;
    service.waiting().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_environment_defaults_to_the_live_store_anonymously() {
        let config = Config::from_values(None, None);
        assert_eq!(config.endpoint, DEFAULT_ENDPOINT);
        assert_eq!(config.api_key, None);
    }

    #[test]
    fn configured_values_win_and_blank_values_count_as_unset() {
        let config = Config::from_values(
            Some("http://127.0.0.1:3000/mcp".to_string()),
            Some("kmcp_key".to_string()),
        );
        assert_eq!(config.endpoint, "http://127.0.0.1:3000/mcp");
        assert_eq!(config.api_key.as_deref(), Some("kmcp_key"));
        let blank = Config::from_values(Some("  ".to_string()), Some(String::new()));
        assert_eq!(blank.endpoint, DEFAULT_ENDPOINT);
        assert_eq!(blank.api_key, None);
    }
}
