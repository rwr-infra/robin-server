use salvo::prelude::*;
use std::sync::Arc;
use tracing::{error, info, warn};

// Import from lib.rs
use rwrs_server::mcp;
use rwrs_server::routes::build_router;
use rwrs_server::upstream::RwrsService;
use rwrs_server::{
    Announcement, ApiCache, Config, MapsConfig, default_mcp_allowed_hosts, format_mcp_url,
    is_loopback_host, mcp_port_conflicts_with_main,
};

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt().init();

    let config = Config::new().unwrap();
    let listen_addr = format!("{}:{}", config.host, config.port);
    info!("listening at {}", listen_addr);

    // Create cache instance
    let cache = Arc::new(ApiCache::new(config.cache_duration_secs));

    // Load maps configuration
    info!(
        "Loading maps configuration from: {}",
        config.maps_config_path
    );
    let maps_config = match MapsConfig::load_from_file(&config.maps_config_path).await {
        Ok(maps) => Arc::new(maps),
        Err(e) => {
            error!(
                "Failed to load maps configuration: {}. Using empty configuration.",
                e
            );
            Arc::new(MapsConfig::new())
        }
    };

    // Load header announcement (optional). Read once at startup; updating the
    // announcement means editing the file and restarting the server.
    let announcement = match config.announcement_path {
        Some(ref path) => Arc::new(Announcement::load_from_file(path).await),
        None => Arc::new(Announcement::disabled()),
    };

    info!("Cache mechanism enabled:");
    info!(
        "  - Cache expiry time: {} seconds",
        config.cache_duration_secs
    );
    info!("  - Maps config file: {}", config.maps_config_path);
    if let Some(ref url) = config.android_repo_url {
        info!("  - Android repo URL: {}", url);
    }
    if let Some(ref url) = config.web_repo_url {
        info!("  - Web repo URL: {}", url);
    }
    match config.announcement_path {
        Some(ref path) => info!(
            "  - Announcement file: {} (enabled: {})",
            path, announcement.enabled
        ),
        None => info!("  - Announcement: not configured"),
    }

    // Create config for sharing
    let config = Arc::new(config);
    let data_service = Arc::new(RwrsService::new(cache));

    match config.mcp_port {
        Some(port) if mcp_port_conflicts_with_main(Some(port), &config.port) => {
            error!(
                "MCP_PORT {} conflicts with the main PORT {}; MCP listener disabled",
                port, config.port
            );
        }
        Some(port) => {
            // Resolve the MCP Host allowlist. Explicit configuration wins;
            // otherwise a loopback bind gets the rmcp default allowlist. A
            // non-loopback bind without MCP_ALLOWED_HOSTS disables Host
            // validation, which is unsafe for an unauthenticated endpoint, so
            // say so loudly.
            let allowed_hosts = match config.mcp_allowed_hosts.clone() {
                Some(hosts) => Some(hosts),
                None if is_loopback_host(&config.mcp_host) => {
                    Some(default_mcp_allowed_hosts(&config.mcp_host))
                }
                None => {
                    warn!(
                        "MCP_HOST '{}' is not loopback and MCP_ALLOWED_HOSTS is not set; \
                         disabling rmcp Host header validation. The MCP endpoint has no \
                         authentication: expose it only behind a protected reverse proxy.",
                        config.mcp_host
                    );
                    None
                }
            };

            info!(
                "MCP Streamable HTTP listening at {}",
                format_mcp_url(&config.mcp_host, port)
            );
            if let Some(hosts) = &allowed_hosts {
                info!("  - MCP allowed hosts: {}", hosts.join(", "));
            }

            let mcp_host = config.mcp_host.clone();
            let service = data_service.clone();
            let maps = maps_config.clone();
            tokio::spawn(async move {
                match mcp::serve_http(&mcp_host, port, service, maps, allowed_hosts).await {
                    Ok(()) => info!("MCP server stopped"),
                    Err(error) => error!("MCP server failed on {}:{}: {}", mcp_host, port, error),
                }
            });
        }
        None => info!("MCP server disabled (MCP_PORT=0)"),
    }

    let router = build_router(config, maps_config, announcement, data_service);

    let service = Service::new(router).hoop(Logger::new());
    let acceptor = TcpListener::new(listen_addr).bind().await;
    Server::new(acceptor).serve(service).await;
}
