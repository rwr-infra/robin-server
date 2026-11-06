use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::env;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{Mutex, RwLock};
use tracing::{error, info};

pub mod domain;
pub mod mcp;
pub mod parsers;
pub mod routes;
pub mod upstream;

/// Default port of the MCP Streamable HTTP listener.
pub const DEFAULT_MCP_PORT: u16 = 5801;

/// Default interface of the MCP listener.
///
/// Deliberately independent of `HOST`: the unauthenticated MCP endpoint must
/// not become public just because the main API binds to `0.0.0.0`.
pub const DEFAULT_MCP_HOST: &str = "127.0.0.1";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MapsConfig {
    pub maps: Vec<MapEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MapEntry {
    pub name: String,
    pub path: String,
    pub image: String,
}

impl Default for MapsConfig {
    fn default() -> Self {
        Self::new()
    }
}

impl MapsConfig {
    pub fn new() -> Self {
        Self { maps: Vec::new() }
    }

    pub async fn load_from_file(file_path: &str) -> Result<Self, String> {
        let current_dir = match std::env::current_dir() {
            Ok(dir) => dir,
            Err(e) => {
                error!("Failed to get current directory: {}", e);
                return Err(format!("Failed to get current directory: {}", e));
            }
        };

        let full_path = if std::path::Path::new(file_path).is_absolute() {
            file_path.to_string()
        } else {
            current_dir.join(file_path).to_string_lossy().to_string()
        };

        info!(
            "Loading maps configuration from: {} (full path: {})",
            file_path, full_path
        );

        let content = match tokio::fs::read_to_string(&full_path).await {
            Ok(content) => content,
            Err(e) => {
                error!("Failed to read maps config file '{}': {}", full_path, e);
                return Err(format!(
                    "Failed to read maps config file '{}': {}",
                    full_path, e
                ));
            }
        };

        let config: MapsConfig = match serde_json::from_str(&content) {
            Ok(config) => config,
            Err(e) => {
                error!("Failed to parse maps config file '{}': {}", full_path, e);
                return Err(format!(
                    "Failed to parse maps config file '{}': {}",
                    full_path, e
                ));
            }
        };

        info!(
            "Successfully loaded {} map entries from config file",
            config.maps.len()
        );
        Ok(config)
    }

    pub fn get_maps(&self) -> Vec<MapEntry> {
        self.maps.clone()
    }
}

/// Header announcement returned to the frontend.
///
/// `html` is operator-controlled rich text loaded from a file at startup and
/// rendered verbatim by the frontend. `enabled` tells the frontend whether to
/// show anything at all.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Announcement {
    pub enabled: bool,
    pub html: String,
}

impl Default for Announcement {
    fn default() -> Self {
        Self::disabled()
    }
}

impl Announcement {
    /// An announcement that the frontend should not display.
    pub fn disabled() -> Self {
        Self {
            enabled: false,
            html: String::new(),
        }
    }

    /// Load announcement HTML from a file.
    ///
    /// Missing file, empty/whitespace-only content, or a read error all degrade
    /// to a disabled announcement instead of failing, mirroring how map config
    /// loading tolerates errors.
    pub async fn load_from_file(file_path: &str) -> Self {
        let full_path = if std::path::Path::new(file_path).is_absolute() {
            file_path.to_string()
        } else {
            match std::env::current_dir() {
                Ok(dir) => dir.join(file_path).to_string_lossy().to_string(),
                Err(e) => {
                    error!("Failed to get current directory: {}", e);
                    file_path.to_string()
                }
            }
        };

        info!(
            "Loading announcement from: {} (full path: {})",
            file_path, full_path
        );

        match tokio::fs::read_to_string(&full_path).await {
            Ok(content) if !content.trim().is_empty() => {
                info!("Loaded announcement ({} bytes)", content.len());
                Self {
                    enabled: true,
                    html: content,
                }
            }
            Ok(_) => {
                info!("Announcement file '{}' is empty, disabling", full_path);
                Self::disabled()
            }
            Err(e) => {
                error!(
                    "Failed to read announcement file '{}': {}. Announcement disabled.",
                    full_path, e
                );
                Self::disabled()
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VersionInfo {
    pub android: RepoVersion,
    pub web: RepoVersion,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoVersion {
    pub version: Option<String>,
    pub url: Option<String>,
}

#[derive(Debug, Clone)]
pub struct CachedResponse {
    data: String,
    timestamp: Instant,
    status_code: u16,
}

impl CachedResponse {
    fn new(data: String, status_code: u16) -> Self {
        Self {
            data,
            timestamp: Instant::now(),
            status_code,
        }
    }

    fn is_expired(&self, duration: Duration) -> bool {
        self.timestamp.elapsed() > duration
    }
}

pub struct Config {
    pub port: String,
    pub host: String,
    pub cache_duration_secs: u64,
    pub maps_config_path: String,
    pub android_repo_url: Option<String>,
    pub web_repo_url: Option<String>,
    pub announcement_path: Option<String>,
    /// Interface the MCP listener binds to; defaults to [`DEFAULT_MCP_HOST`]
    /// and never inherits `HOST`.
    pub mcp_host: String,
    /// MCP listener port; `None` disables the MCP transport entirely.
    pub mcp_port: Option<u16>,
    /// Explicit `Host` header allowlist for the MCP transport.
    pub mcp_allowed_hosts: Option<Vec<String>>,
}

/// Parse the `MCP_PORT` environment value.
///
/// Unset or empty means the default port 5801, `0` disables MCP, an invalid
/// value falls back to 5801 and logs a warning.
pub fn parse_mcp_port(raw: Option<&str>) -> Option<u16> {
    let Some(raw) = raw.map(str::trim).filter(|value| !value.is_empty()) else {
        return Some(DEFAULT_MCP_PORT);
    };
    match raw.parse::<u16>() {
        Ok(0) => None,
        Ok(port) => Some(port),
        Err(_) => {
            tracing::warn!(
                "invalid MCP_PORT '{}', expected 0..=65535; using {}",
                raw,
                DEFAULT_MCP_PORT
            );
            Some(DEFAULT_MCP_PORT)
        }
    }
}

/// Whether the MCP listener and the main server would bind the same port.
///
/// `main_port` is the raw `PORT` value; anything that is not a valid port
/// number cannot conflict.
pub fn mcp_port_conflicts_with_main(mcp_port: Option<u16>, main_port: &str) -> bool {
    match (mcp_port, main_port.trim().parse::<u16>()) {
        (Some(mcp_port), Ok(main_port)) => mcp_port == main_port,
        _ => false,
    }
}

/// Parse a comma-separated `MCP_ALLOWED_HOSTS` value.
///
/// Empty entries and surrounding whitespace are dropped; an unset or empty
/// result means no explicit allowlist.
pub fn parse_allowed_hosts(raw: Option<&str>) -> Option<Vec<String>> {
    let hosts: Vec<String> = raw
        .into_iter()
        .flat_map(|value| value.split(','))
        .map(str::trim)
        .filter(|host| !host.is_empty())
        .map(str::to_string)
        .collect();
    (!hosts.is_empty()).then_some(hosts)
}

/// Strip one pair of surrounding `[...]` from an IPv6 literal.
fn strip_brackets(host: &str) -> &str {
    host.strip_prefix('[')
        .and_then(|value| value.strip_suffix(']'))
        .unwrap_or(host)
}

/// Default `Host` allowlist for a loopback MCP bind address.
///
/// The rmcp defaults cover the usual loopback names; a bind address outside
/// that set (for example `127.0.0.2`) is appended so its own requests are not
/// rejected.
pub fn default_mcp_allowed_hosts(host: &str) -> Vec<String> {
    let mut hosts = vec![
        "localhost".to_string(),
        "127.0.0.1".to_string(),
        "::1".to_string(),
    ];
    let host = strip_brackets(host.trim());
    if !host.is_empty()
        && !hosts
            .iter()
            .any(|allowed| allowed.eq_ignore_ascii_case(host))
    {
        hosts.push(host.to_string());
    }
    hosts
}

/// Build the human-facing MCP endpoint URL, bracketing IPv6 literals.
pub fn format_mcp_url(host: &str, port: u16) -> String {
    let host = host.trim();
    if host.contains(':') && !(host.starts_with('[') && host.ends_with(']')) {
        format!("http://[{host}]:{port}/mcp")
    } else {
        format!("http://{host}:{port}/mcp")
    }
}

/// Whether `host` is a loopback name or address.
pub fn is_loopback_host(host: &str) -> bool {
    let host = host.trim();
    if host.eq_ignore_ascii_case("localhost") {
        return true;
    }
    let host = strip_brackets(host);
    host.parse::<std::net::IpAddr>()
        .map(|address| address.is_loopback())
        .unwrap_or(false)
}

impl Config {
    pub fn new() -> Result<Config, &'static str> {
        let host = env::var("HOST").unwrap_or_else(|_| "127.0.0.1".to_string());
        let port = env::var("PORT").unwrap_or_else(|_| "5800".to_string());

        // Cache expiry time in seconds, default 3
        let cache_duration_secs = env::var("CACHE_DURATION_SECS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(3);

        // Maps config file path, default "maps.json"
        let maps_config_path = env::var("MAPS_CONFIG").unwrap_or_else(|_| "maps.json".to_string());

        // Repository URLs for version info
        let android_repo_url = env::var("ANDROID_REPO_URL").ok();
        let web_repo_url = env::var("WEB_REPO_URL").ok();

        // Optional path to the header announcement HTML file. Unset = no announcement.
        let announcement_path = env::var("ANNOUNCEMENT_PATH").ok();

        // MCP listener: separate port so the Salvo API and the rmcp/axum
        // transport can run side by side in the same process. The bind address
        // never inherits HOST: the MCP endpoint has no authentication, so a
        // HOST=0.0.0.0 deployment must opt in with an explicit MCP_HOST.
        let mcp_host = env::var("MCP_HOST")
            .ok()
            .map(|value| strip_brackets(value.trim()).to_string())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| DEFAULT_MCP_HOST.to_string());
        let mcp_port = parse_mcp_port(env::var("MCP_PORT").ok().as_deref());
        let mcp_allowed_hosts = parse_allowed_hosts(env::var("MCP_ALLOWED_HOSTS").ok().as_deref());

        Ok(Config {
            port: port.to_string(),
            host: host.to_string(),
            cache_duration_secs,
            maps_config_path,
            android_repo_url,
            web_repo_url,
            announcement_path,
            mcp_host,
            mcp_port,
            mcp_allowed_hosts,
        })
    }
}

pub struct ApiCache {
    cache: Arc<RwLock<HashMap<String, CachedResponse>>>,
    inflight: Mutex<HashMap<String, Arc<Mutex<()>>>>,
    cache_expiry_duration: Duration,
    client: reqwest::Client,
}

impl ApiCache {
    pub fn new(cache_expiry_secs: u64) -> Self {
        // Build the HTTP client once and reuse it across all requests.
        // Creating a Client per request accumulates connection pools / TLS
        // state and is the primary source of memory growth.
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .expect("failed to build reqwest client");

        Self {
            cache: Arc::new(RwLock::new(HashMap::new())),
            inflight: Mutex::new(HashMap::new()),
            cache_expiry_duration: Duration::from_secs(cache_expiry_secs),
            client,
        }
    }

    /// Return a fresh cache entry, or `None` when the URL is missing/expired.
    async fn cached_response(&self, url: &str) -> Option<(String, u16)> {
        let cache_guard = self.cache.read().await;
        let cached = cache_guard.get(url)?;
        if cached.is_expired(self.cache_expiry_duration) {
            return None;
        }

        info!(
            "Cache hit for {}, age: {:?}",
            url,
            cached.timestamp.elapsed()
        );
        Some((cached.data.clone(), cached.status_code))
    }

    /// Fetch `url` and cache the response.
    async fn fetch_and_cache(&self, url: &str) -> Result<(String, u16), String> {
        info!("Making request to external API: {}", url);
        match self.client.get(url).send().await {
            Ok(response) => {
                let status = response.status().as_u16();
                info!("Received response with status: {}", status);
                match response.text().await {
                    Ok(text) => {
                        info!("Successfully fetched {} bytes from API", text.len());
                        // Cache the response with URL as key
                        self.update_cache(url.to_string(), text.clone(), status)
                            .await;
                        Ok((text, status))
                    }
                    Err(e) => {
                        error!("Failed to read response body: {}", e);
                        Err(format!("Failed to read response body: {}", e))
                    }
                }
            }
            Err(e) => {
                error!("Request failed: {}", e);
                Err(format!("Request failed: {}", e))
            }
        }
    }

    pub async fn get_cached_response(&self, url: &str) -> Result<(String, u16), String> {
        if let Some(response) = self.cached_response(url).await {
            return Ok(response);
        }

        info!(
            "No fresh cache data available for {}, fetching from API",
            url
        );

        // Single-flight: concurrent misses for the same URL share one
        // per-URL lock, so only the first caller talks to the upstream.
        let lock = {
            let mut inflight = self.inflight.lock().await;
            inflight
                .entry(url.to_string())
                .or_insert_with(|| Arc::new(Mutex::new(())))
                .clone()
        };

        let _guard = lock.lock().await;

        // A previous holder may have populated the cache while we waited.
        if let Some(response) = self.cached_response(url).await {
            return Ok(response);
        }

        info!("Fetching {} after an in-flight miss", url);
        let result = self.fetch_and_cache(url).await;

        // Drop the map entry after the attempt. Callers already waiting on
        // this Arc still serialize on it; later callers either hit the cache
        // or start a fresh request.
        self.inflight.lock().await.remove(url);
        result
    }

    async fn update_cache(&self, url: String, data: String, status_code: u16) {
        let cached_response = CachedResponse::new(data, status_code);
        let mut cache_guard = self.cache.write().await;
        // Drop stale entries so distinct query strings can't grow the map
        // unbounded over time.
        cache_guard.retain(|_, cached| !cached.is_expired(self.cache_expiry_duration));
        cache_guard.insert(url, cached_response);
    }
}

pub async fn get_latest_tag(repo_url: &str) -> Option<(String, String)> {
    // Extract owner and repo from GitHub URL
    let url_parts: Vec<&str> = repo_url.trim_end_matches('/').split('/').collect();
    if url_parts.len() < 5 || url_parts[2] != "github.com" {
        return None;
    }

    let owner = url_parts[3];
    let repo = url_parts[4];

    // Use GitHub API to get the latest release
    let api_url = format!(
        "https://api.github.com/repos/{}/{}/releases/latest",
        owner, repo
    );

    // Reuse a single client across all version lookups instead of rebuilding
    // one per request.
    static VERSION_CLIENT: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
    let client = VERSION_CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .user_agent("rwrs-server")
            .timeout(Duration::from_secs(10))
            .build()
            .expect("failed to build reqwest client")
    });

    match client.get(&api_url).send().await {
        Ok(response) => {
            if response.status().is_success()
                && let Ok(release_info) = response.json::<serde_json::Value>().await
            {
                let tag_name = release_info.get("tag_name")?.as_str()?.to_string();
                // Return URL to the release page instead of zipball
                let release_url = format!(
                    "https://github.com/{}/{}/releases/tag/{}",
                    owner, repo, tag_name
                );
                return Some((tag_name, release_url));
            }
        }
        Err(e) => {
            error!("Failed to fetch release info from {}: {}", api_url, e);
        }
    }

    None
}

#[cfg(test)]
mod tests;
