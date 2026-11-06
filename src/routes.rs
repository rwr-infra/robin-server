//! Salvo router construction and HTTP handlers.
//!
//! Three groups of endpoints live here:
//! - metadata: `/ping`, `/api/version`, `/api/maps`, `/api/announcement`;
//! - passthrough of the upstream HTML/XML payloads: `/api/server_list`,
//!   `/api/player_list` (unchanged behavior);
//! - normalized JSON: `/api/servers.json`, `/api/players.json`, served by
//!   [`RwrsService`].

use crate::domain::{DataError, PlayerQuery, ServerQuery};
use crate::upstream::RwrsService;
use crate::{Announcement, Config, MapsConfig, RepoVersion, VersionInfo, get_latest_tag};
use salvo::affix_state;
use salvo::prelude::*;
use salvo::serve_static::StaticDir;
use std::sync::Arc;
use tracing::{debug, error};

#[handler]
async fn ping() -> &'static str {
    "pong"
}

#[handler]
async fn maps_handler(depot: &mut Depot, res: &mut Response) {
    let maps_config = depot.obtain::<Arc<MapsConfig>>().unwrap();
    let maps = maps_config.get_maps();

    res.render(Json(&maps));
}

#[handler]
async fn announcement_handler(depot: &mut Depot, res: &mut Response) {
    let announcement = depot.obtain::<Arc<Announcement>>().unwrap();
    res.render(Json(announcement.as_ref()));
}

#[handler]
async fn version_handler(depot: &mut Depot, res: &mut Response) {
    let config = depot.obtain::<Arc<Config>>().unwrap();

    let android_version = if let Some(android_repo_url) = &config.android_repo_url {
        match get_latest_tag(android_repo_url).await {
            Some((version, url)) => RepoVersion {
                version: Some(version),
                url: Some(url),
            },
            None => RepoVersion {
                version: None,
                url: None,
            },
        }
    } else {
        RepoVersion {
            version: None,
            url: None,
        }
    };

    let web_version = if let Some(web_repo_url) = &config.web_repo_url {
        match get_latest_tag(web_repo_url).await {
            Some((version, url)) => RepoVersion {
                version: Some(version),
                url: Some(url),
            },
            None => RepoVersion {
                version: None,
                url: None,
            },
        }
    } else {
        RepoVersion {
            version: None,
            url: None,
        }
    };

    let version_info = VersionInfo {
        android: android_version,
        web: web_version,
    };

    res.render(Json(&version_info));
}

#[handler]
async fn servers_handler(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    let service = depot.obtain::<Arc<RwrsService>>().unwrap();
    let upstream = service.upstream();

    // Get query string from original request
    let query_string = req.uri().query().unwrap_or("");
    let base_url = upstream.server_list_url();
    let url = if query_string.is_empty() {
        base_url.to_string()
    } else {
        format!("{}?{}", base_url, query_string)
    };

    match upstream.cache().get_cached_response(&url).await {
        Ok((data, status_code)) => {
            // Cache successful response
            res.status_code(StatusCode::from_u16(status_code).unwrap_or(StatusCode::OK));
            res.render(Text::Html(data));
        }
        Err(e) => {
            error!("Failed to get server list: {}", e);
            res.status_code(StatusCode::INTERNAL_SERVER_ERROR);
            res.render(Text::Plain(format!("Unable to fetch server list: {}", e)));
        }
    }
}

#[handler]
async fn players_handler(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    let service = depot.obtain::<Arc<RwrsService>>().unwrap();
    let upstream = service.upstream();

    // Get query string from original request
    let query_string = req.uri().query().unwrap_or("");
    let base_url = upstream.player_list_url();
    let url = if query_string.is_empty() {
        base_url.to_string()
    } else {
        format!("{}?{}", base_url, query_string)
    };

    match upstream.cache().get_cached_response(&url).await {
        Ok((data, status_code)) => {
            // Cache successful response
            res.status_code(StatusCode::from_u16(status_code).unwrap_or(StatusCode::OK));
            res.render(Text::Html(data));
        }
        Err(e) => {
            error!("Failed to get players data: {}", e);
            res.status_code(StatusCode::INTERNAL_SERVER_ERROR);
            res.render(Text::Plain(format!("Unable to fetch players data: {}", e)));
        }
    }
}

#[handler]
async fn servers_json_handler(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    let service = depot.obtain::<Arc<RwrsService>>().unwrap();

    // A missing query string deserializes to the defaults; a malformed one
    // (bad bool, non-numeric limit...) is a client error and must not reach
    // the upstream.
    let query = match req.parse_queries::<ServerQuery>() {
        Ok(query) => query,
        Err(error) => {
            render_data_error(res, DataError::from_query_error(error));
            return;
        }
    };

    match service.list_servers(&query).await {
        Ok(body) => {
            res.render(Json(&body));
        }
        Err(error) => render_data_error(res, error),
    }
}

#[handler]
async fn players_json_handler(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    let service = depot.obtain::<Arc<RwrsService>>().unwrap();

    let query = match req.parse_queries::<PlayerQuery>() {
        Ok(query) => query,
        Err(error) => {
            render_data_error(res, DataError::from_query_error(error));
            return;
        }
    };

    match service.search_players(&query).await {
        Ok(body) => {
            res.render(Json(&body));
        }
        Err(error) => render_data_error(res, error),
    }
}

/// Map a data-layer failure onto a JSON HTTP response.
///
/// Invalid client input becomes `400`, anything that went wrong while talking
/// to (or parsing) the upstream becomes `502`. Only a human-readable message is
/// exposed; internal error sources and backtraces stay in the logs.
fn render_data_error(res: &mut Response, error: DataError) {
    let status = match &error {
        DataError::InvalidQuery(_) => StatusCode::BAD_REQUEST,
        DataError::Upstream(_) => StatusCode::BAD_GATEWAY,
    };
    if let DataError::Upstream(_) = &error {
        error!("Data request failed: {error}");
    } else {
        debug!("Rejected data request: {error}");
    }

    let message = match error {
        DataError::InvalidQuery(message) => message,
        DataError::Upstream(error) => error.to_string(),
    };
    res.status_code(status);
    res.render(Json(serde_json::json!({ "error": message })));
}

/// Build the complete application router.
///
/// The caller is expected to wrap the returned router in a [`Service`] with the
/// desired middlewares (e.g. `Logger`).
pub fn build_router(
    config: Arc<Config>,
    maps: Arc<MapsConfig>,
    announcement: Arc<Announcement>,
    service: Arc<RwrsService>,
) -> Router {
    Router::new()
        .hoop(RequestId::new())
        .push(Router::new().path("/ping").get(ping))
        .push(
            Router::new()
                .path("/api/version")
                .hoop(affix_state::inject(config))
                .get(version_handler),
        )
        .push(
            Router::new()
                .path("/api/maps")
                .hoop(affix_state::inject(maps))
                .get(maps_handler),
        )
        .push(
            Router::new()
                .path("/api/announcement")
                .hoop(affix_state::inject(announcement))
                .get(announcement_handler),
        )
        .push(
            Router::new()
                .path("/api/server_list")
                .hoop(affix_state::inject(service.clone()))
                .goal(servers_handler),
        )
        .push(
            Router::new()
                .path("/api/player_list")
                .hoop(affix_state::inject(service.clone()))
                .goal(players_handler),
        )
        .push(
            Router::new()
                .path("/api/servers.json")
                .hoop(affix_state::inject(service.clone()))
                .get(servers_json_handler),
        )
        .push(
            Router::new()
                .path("/api/players.json")
                .hoop(affix_state::inject(service))
                .get(players_json_handler),
        )
        .push(Router::with_path("{**path}").get(StaticDir::new(["static"]).defaults("index.html")))
}
