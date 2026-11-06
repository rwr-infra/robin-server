//! Read-only MCP server exposing the normalized Running with Rifles data.
//!
//! The transport is MCP Streamable HTTP served by rmcp on its own axum
//! listener (the main API stays on Salvo). The server is stateless and prefers
//! plain JSON responses so `curl` and simple clients can use it; every tool
//! reuses [`RwrsService`], which in turn shares the [`crate::ApiCache`].

use crate::upstream::RwrsService;
use crate::{MapsConfig, domain::DataError, domain::PlayerQuery, domain::ServerQuery};
use rmcp::ErrorData;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::CallToolResult;
use rmcp::transport::streamable_http_server::{
    StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
};
use rmcp::{ServerHandler, schemars, tool, tool_handler, tool_router};
use serde::Serialize;
use serde_json::json;
use std::sync::Arc;

/// Parameters of the `list_servers` tool.
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ListServersParams {
    /// Substring matched case-insensitively against name, address, port,
    /// country, mode, map id/name, comment and connected player names.
    pub search: Option<String>,
    /// Only return servers with at least one connected player.
    pub online_only: Option<bool>,
    /// Substring matched against the map id or map name.
    pub map: Option<String>,
    /// Substring matched against the game mode.
    pub mode: Option<String>,
    /// Substring matched against the server country.
    pub country: Option<String>,
    /// Zero-based index of the first server to return (default 0).
    pub offset: Option<i64>,
    /// Page size between 1 and 100 (default 20).
    pub limit: Option<i64>,
}

impl From<ListServersParams> for ServerQuery {
    fn from(params: ListServersParams) -> Self {
        Self {
            search: params.search,
            online_only: params.online_only,
            map: params.map,
            mode: params.mode,
            country: params.country,
            offset: params.offset,
            limit: params.limit,
        }
    }
}

/// Parameters of the `get_server` tool.
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct GetServerParams {
    /// Server address as reported by the upstream, e.g. `127.0.0.1`.
    pub address: String,
    /// Game port of the server, e.g. `1234`.
    pub port: u16,
}

/// Parameters of the `search_players` tool.
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SearchPlayersParams {
    /// Substring matched against player names. When present the upstream
    /// anchors its page around the match, so `offset` is ignored by upstream.
    pub search: Option<String>,
    /// Leaderboard database: `invasion` (default), `pacific` or
    /// `prereset_invasion`.
    pub db: Option<String>,
    /// Sort field, e.g. `rank_progression` (default), `kills`, `score`.
    pub sort: Option<String>,
    /// Zero-based index of the first row to return (default 0).
    pub offset: Option<i64>,
    /// Page size between 1 and 100 (default 20).
    pub limit: Option<i64>,
}

impl From<SearchPlayersParams> for PlayerQuery {
    fn from(params: SearchPlayersParams) -> Self {
        Self {
            search: params.search,
            db: params.db,
            sort: params.sort,
            offset: params.offset,
            limit: params.limit,
        }
    }
}

/// MCP handler backed by the shared upstream service and map configuration.
#[derive(Clone)]
struct RwrsMcp {
    service: Arc<RwrsService>,
    maps: Arc<MapsConfig>,
}

#[tool_router]
impl RwrsMcp {
    fn new(service: Arc<RwrsService>, maps: Arc<MapsConfig>) -> Self {
        Self { service, maps }
    }

    #[tool(
        description = "List live Running with Rifles servers with optional search, map, mode, country and online-only filters. Returns normalized servers plus pagination metadata."
    )]
    async fn list_servers(
        &self,
        Parameters(params): Parameters<ListServersParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let query = ServerQuery::from(params);
        match self.service.list_servers(&query).await {
            Ok(response) => structured(response),
            Err(error) => map_data_error(error),
        }
    }

    #[tool(
        description = "Look up one live server by its address and game port. Returns a structured error result when no server matches."
    )]
    async fn get_server(
        &self,
        Parameters(params): Parameters<GetServerParams>,
    ) -> Result<CallToolResult, ErrorData> {
        match self.service.get_server(&params.address, params.port).await {
            Ok(Some(server)) => structured(server),
            Ok(None) => Ok(CallToolResult::structured_error(json!({
                "error": "server not found",
                "address": params.address,
                "port": params.port,
            }))),
            Err(error) => map_data_error(error),
        }
    }

    #[tool(
        description = "Search a Running with Rifles player leaderboard (invasion, pacific or prereset_invasion). Returns normalized players plus hasNext/hasPrevious pagination flags."
    )]
    async fn search_players(
        &self,
        Parameters(params): Parameters<SearchPlayersParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let query = PlayerQuery::from(params);
        match self.service.search_players(&query).await {
            Ok(response) => structured(response),
            Err(error) => map_data_error(error),
        }
    }

    #[tool(
        description = "List the maps known to this deployment (name, path and preview image), as loaded from the server's maps configuration."
    )]
    async fn list_maps(&self) -> Result<CallToolResult, ErrorData> {
        structured(json!({ "maps": self.maps.get_maps() }))
    }
}

#[tool_handler(
    name = "rwrs-server",
    instructions = "Read-only Running with Rifles data: live servers and player leaderboards from the public upstream. Use list_servers/search_players for queries; player search anchors its window around a match."
)]
impl ServerHandler for RwrsMcp {}

/// Serialize a tool payload into a successful structured result.
fn structured(value: impl Serialize) -> Result<CallToolResult, ErrorData> {
    serde_json::to_value(value)
        .map(CallToolResult::structured)
        .map_err(|error| {
            ErrorData::internal_error(format!("failed to serialize tool result: {error}"), None)
        })
}

/// Translate data-layer failures into MCP results.
///
/// Bad client input is a JSON-RPC `invalid_params` error; upstream and parse
/// failures become a tool error result with a structured message so the model
/// can see what went wrong.
fn map_data_error(error: DataError) -> Result<CallToolResult, ErrorData> {
    match error {
        DataError::InvalidQuery(message) => Err(ErrorData::invalid_params(message, None)),
        DataError::Upstream(error) => Ok(CallToolResult::structured_error(
            json!({"error": error.to_string()}),
        )),
    }
}

/// Build the axum router serving the MCP endpoint at `/mcp`.
///
/// Sessions are disabled (each request is handled independently) and plain
/// JSON responses are preferred. `allowed_hosts` `None` disables rmcp's
/// `Host` header validation; callers are expected to only do that after
/// logging a warning.
pub fn build_mcp_router(
    service: Arc<RwrsService>,
    maps: Arc<MapsConfig>,
    allowed_hosts: Option<Vec<String>>,
) -> axum::Router {
    let config = StreamableHttpServerConfig::default()
        .with_legacy_session_mode(false)
        .with_json_response(true);
    let config = match allowed_hosts {
        Some(hosts) => config.with_allowed_hosts(hosts),
        None => config.disable_allowed_hosts(),
    };

    let service = StreamableHttpService::new(
        move || Ok(RwrsMcp::new(service.clone(), maps.clone())),
        LocalSessionManager::default().into(),
        config,
    );

    axum::Router::new().nest_service("/mcp", service)
}

/// Bind and serve the MCP transport on `host:port`.
pub async fn serve_http(
    host: &str,
    port: u16,
    service: Arc<RwrsService>,
    maps: Arc<MapsConfig>,
    allowed_hosts: Option<Vec<String>>,
) -> std::io::Result<()> {
    let listener = tokio::net::TcpListener::bind((host, port)).await?;
    axum::serve(listener, build_mcp_router(service, maps, allowed_hosts)).await
}
