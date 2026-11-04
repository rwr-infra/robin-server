use salvo::prelude::*;
use std::sync::Arc;
use tracing::{error, info};

// Import from lib.rs
use rwrs_server::routes::build_router;
use rwrs_server::upstream::RwrsService;
use rwrs_server::{Announcement, ApiCache, Config, MapsConfig};

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

    let router = build_router(config, maps_config, announcement, data_service);

    let service = Service::new(router).hoop(Logger::new());
    let acceptor = TcpListener::new(listen_addr).bind().await;
    Server::new(acceptor).serve(service).await;
}
