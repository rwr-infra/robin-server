//! Fetches and normalizes data from the Running with Rifles upstream pages.
//!
//! All HTTP traffic goes through the shared [`ApiCache`], so repeated queries
//! within the cache TTL never hit the upstream twice.

use crate::ApiCache;
use crate::domain::{
    DEFAULT_PLAYER_SORT, DataError, PlayerListResponse, PlayerQuery, ServerItem,
    ServerListResponse, ServerQuery, UpstreamError,
};
use crate::parsers::{PlayerListResult, parse_player_list, parse_server_list};
use reqwest::Url;
use std::collections::HashSet;
use std::sync::Arc;

/// Upstream server list endpoint.
pub const SERVER_LIST_URL: &str =
    "http://rwr.runningwithrifles.com/rwr_server_list/get_server_list.php";
/// Upstream player leaderboard endpoint.
pub const PLAYER_LIST_URL: &str = "http://rwr.runningwithrifles.com/rwr_stats/view_players.php";
/// Servers requested per upstream page.
pub const SERVER_PAGE_SIZE: usize = 100;
/// Hard cap on upstream server pages (10 x 100 servers).
pub const MAX_SERVER_PAGES: usize = 10;

/// Thin HTTP client for the two upstream endpoints.
#[derive(Clone)]
pub struct UpstreamClient {
    cache: Arc<ApiCache>,
    server_list_url: String,
    player_list_url: String,
}

impl UpstreamClient {
    /// Client for the public upstream endpoints.
    pub fn new(cache: Arc<ApiCache>) -> Self {
        Self::with_urls(cache, SERVER_LIST_URL, PLAYER_LIST_URL)
    }

    /// Client with explicit base URLs, used by tests.
    pub fn with_urls(
        cache: Arc<ApiCache>,
        server_list_url: impl Into<String>,
        player_list_url: impl Into<String>,
    ) -> Self {
        Self {
            cache,
            server_list_url: server_list_url.into(),
            player_list_url: player_list_url.into(),
        }
    }

    /// Base URL of the upstream server list endpoint.
    pub fn server_list_url(&self) -> &str {
        &self.server_list_url
    }

    /// Base URL of the upstream player leaderboard endpoint.
    pub fn player_list_url(&self) -> &str {
        &self.player_list_url
    }

    /// Shared response cache used for every upstream request.
    pub fn cache(&self) -> &Arc<ApiCache> {
        &self.cache
    }

    /// Fetch every server, following `start=0,100,...` pages.
    ///
    /// Stops on the first short page or after [`MAX_SERVER_PAGES`], and drops
    /// duplicate `address:port` entries.
    pub async fn fetch_servers(&self) -> Result<Vec<ServerItem>, UpstreamError> {
        let mut servers: Vec<ServerItem> = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();

        for page in 0..MAX_SERVER_PAGES {
            let url = self.server_page_url(page * SERVER_PAGE_SIZE)?;
            let (body, _status) = self.fetch(&url).await?;
            let page_servers = parse_server_list(&body)?;
            let fetched = page_servers.len();

            for server in page_servers {
                if seen.insert(server.id.clone()) {
                    servers.push(server);
                }
            }

            if fetched < SERVER_PAGE_SIZE {
                break;
            }
        }

        Ok(servers)
    }

    /// Fetch one leaderboard page.
    ///
    /// `query` must have been normalized with [`PlayerQuery::normalize`] so the
    /// upstream receives a known `db` and `sort`.
    pub async fn fetch_players(
        &self,
        query: &PlayerQuery,
    ) -> Result<PlayerListResult, UpstreamError> {
        let url = self.player_page_url(query)?;
        let (body, _status) = self.fetch(&url).await?;
        Ok(parse_player_list(&body, query.database()))
    }

    async fn fetch(&self, url: &Url) -> Result<(String, u16), UpstreamError> {
        match self.cache.get_cached_response(url.as_str()).await {
            Ok((body, status)) if (200..300).contains(&status) => Ok((body, status)),
            Ok((_body, status)) => Err(UpstreamError::Status {
                url: url.to_string(),
                status,
            }),
            Err(message) => Err(UpstreamError::Request(message)),
        }
    }

    fn server_page_url(&self, start: usize) -> Result<Url, UpstreamError> {
        let mut url = Url::parse(&self.server_list_url).map_err(|error| {
            UpstreamError::Request(format!(
                "invalid server list URL '{}': {error}",
                self.server_list_url
            ))
        })?;
        url.query_pairs_mut()
            .append_pair("start", &start.to_string())
            .append_pair("size", &SERVER_PAGE_SIZE.to_string())
            .append_pair("names", "1");
        Ok(url)
    }

    fn player_page_url(&self, query: &PlayerQuery) -> Result<Url, UpstreamError> {
        let db = query.database();
        let offset = query.offset();
        let limit = query.limit();
        let sort = query.sort.as_deref().unwrap_or(DEFAULT_PLAYER_SORT);

        let mut url = Url::parse(&self.player_list_url).map_err(|error| {
            UpstreamError::Request(format!(
                "invalid player list URL '{}': {error}",
                self.player_list_url
            ))
        })?;
        {
            let mut pairs = url.query_pairs_mut();
            pairs
                .append_pair("db", db.as_str())
                .append_pair("start", &offset.to_string())
                .append_pair("size", &limit.to_string())
                .append_pair("sort", sort);
            if let Some(search) = query.search.as_deref().filter(|value| !value.is_empty()) {
                pairs.append_pair("search", search);
            }
        }
        Ok(url)
    }
}

/// Query facade used by the HTTP and MCP layers.
#[derive(Clone)]
pub struct RwrsService {
    client: UpstreamClient,
}

impl RwrsService {
    /// Service backed by the public upstream endpoints.
    pub fn new(cache: Arc<ApiCache>) -> Self {
        Self {
            client: UpstreamClient::new(cache),
        }
    }

    /// Service with an explicitly constructed client, used by tests.
    pub fn with_client(client: UpstreamClient) -> Self {
        Self { client }
    }

    /// Underlying upstream client; the passthrough handlers use it to build
    /// the original upstream URL and share the response cache.
    pub fn upstream(&self) -> &UpstreamClient {
        &self.client
    }

    /// Fetch all servers, filter, then paginate.
    pub async fn list_servers(&self, query: &ServerQuery) -> Result<ServerListResponse, DataError> {
        let mut query = query.clone();
        query.normalize()?;

        let limit = query.limit();
        let offset = query.offset();
        let filters = Filters::from_query(&query);

        let servers = self.client.fetch_servers().await?;
        let matched: Vec<ServerItem> = servers
            .into_iter()
            .filter(|server| filters.matches(server))
            .collect();
        let total = matched.len();

        let page = matched.into_iter().skip(offset).take(limit).collect();
        Ok(ServerListResponse {
            total,
            offset,
            limit,
            servers: page,
        })
    }

    /// Fetch all servers and look one up by address and port.
    pub async fn get_server(
        &self,
        address: &str,
        port: u16,
    ) -> Result<Option<ServerItem>, DataError> {
        let servers = self.client.fetch_servers().await?;
        Ok(servers
            .into_iter()
            .find(|server| server.port == port && server.ip_address.eq_ignore_ascii_case(address)))
    }

    /// Fetch one leaderboard page.
    pub async fn search_players(
        &self,
        query: &PlayerQuery,
    ) -> Result<PlayerListResponse, DataError> {
        let mut query = query.clone();
        query.normalize()?;

        let db = query.database();
        let offset = query.offset();
        let limit = query.limit();

        let result = self.client.fetch_players(&query).await?;
        Ok(PlayerListResponse {
            db,
            offset,
            limit,
            has_next: result.has_next,
            has_previous: result.has_previous,
            players: result.players,
        })
    }
}

/// Lowercased filter values, computed once per request.
struct Filters {
    search: Option<String>,
    map: Option<String>,
    mode: Option<String>,
    country: Option<String>,
    online_only: bool,
}

impl Filters {
    fn from_query(query: &ServerQuery) -> Self {
        Self {
            search: query.search.as_deref().map(str::to_lowercase),
            map: query.map.as_deref().map(str::to_lowercase),
            mode: query.mode.as_deref().map(str::to_lowercase),
            country: query.country.as_deref().map(str::to_lowercase),
            online_only: query.online_only == Some(true),
        }
    }

    fn matches(&self, server: &ServerItem) -> bool {
        if self.online_only && server.current_players <= 0 {
            return false;
        }

        if let Some(search) = &self.search
            && !self.matches_search(server, search)
        {
            return false;
        }

        if let Some(map) = &self.map
            && !contains_ignore_case(&server.map_id, map)
            && !contains_ignore_case(&server.map_name, map)
        {
            return false;
        }

        if let Some(mode) = &self.mode
            && !contains_ignore_case(&server.mode, mode)
        {
            return false;
        }

        if let Some(country) = &self.country
            && !contains_ignore_case(&server.country, country)
        {
            return false;
        }

        true
    }

    fn matches_search(&self, server: &ServerItem, search: &str) -> bool {
        let port = server.port.to_string();
        contains_ignore_case(&server.name, search)
            || contains_ignore_case(&server.ip_address, search)
            || port.contains(search)
            || contains_ignore_case(&server.country, search)
            || contains_ignore_case(&server.mode, search)
            || contains_ignore_case(&server.map_id, search)
            || contains_ignore_case(&server.map_name, search)
            || contains_ignore_case(&server.comment, search)
            || server
                .player_list
                .iter()
                .any(|player| contains_ignore_case(player, search))
    }
}

fn contains_ignore_case(haystack: &str, lowercase_needle: &str) -> bool {
    haystack.to_lowercase().contains(lowercase_needle)
}
