#[cfg(test)]
mod tests {
    use crate::routes::build_router;
    use crate::upstream::{RwrsService, UpstreamClient};
    use crate::{Announcement, ApiCache, Config, MapEntry, MapsConfig};
    use salvo::http::StatusCode;
    use salvo::http::header::CONTENT_TYPE;
    use salvo::prelude::*;
    use salvo::test::{RequestBuilder, ResponseExt, TestClient};
    use std::sync::Arc;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{method, path, query_param, query_param_is_missing},
    };

    const SERVER_XML: &str = r#"<result>
  <server>
    <name>Alpha Server</name>
    <address>127.0.0.1</address>
    <port>1234</port>
    <map_id>desert_town</map_id>
    <map_name>Desert Town</map_name>
    <country>US</country>
    <current_players>10</current_players>
    <mode>Invasion</mode>
    <comment>friendly</comment>
    <player>Player1</player>
    <player>Player2</player>
    <max_players>32</max_players>
  </server>
  <server>
    <name>Beta Server</name>
    <address>127.0.0.2</address>
    <port>5678</port>
    <map_id>killhouse</map_id>
    <map_name>Killhouse</map_name>
    <country>UK</country>
    <current_players>0</current_players>
    <mode>Coop</mode>
    <comment>empty</comment>
    <player>Player3</player>
  </server>
  <server>
    <name>Gamma Server</name>
    <address>127.0.0.3</address>
    <port>9000</port>
    <map_id>desert_camp</map_id>
    <map_name>Desert Camp</map_name>
    <country>US</country>
    <current_players>3</current_players>
    <mode>Invasion</mode>
  </server>
  <server>
    <name>Delta Server</name>
    <address>127.0.0.4</address>
    <port>9100</port>
    <map_id>arctic_base</map_id>
    <map_name>Arctic Base</map_name>
    <country>DE</country>
    <current_players>1</current_players>
    <mode>Invasion</mode>
  </server>
  <server>
    <name>Epsilon Server</name>
    <address>127.0.0.5</address>
    <port>9200</port>
    <map_id>desert_outpost</map_id>
    <map_name>Desert Outpost</map_name>
    <country>US</country>
    <current_players>0</current_players>
    <mode>Invasion</mode>
  </server>
</result>"#;

    const PLAYER_HTML: &str = r#"<!DOCTYPE html>
<html>
<body>
  <table>
    <tr>
      <th>#</th><th>Username</th><th>Kills</th><th>Deaths</th>
    </tr>
    <tr>
      <td>1</td>
      <td>PlayerOne</td>
      <td>100</td>
      <td>50</td>
      <td>5000</td>
      <td>2.0</td>
      <td>10h 30m</td>
      <td>15</td>
      <td>20</td>
      <td>5</td>
      <td>30</td>
      <td>2</td>
      <td>10000m</td>
      <td>1000</td>
      <td>50</td>
      <td>100</td>
      <td>Captain</td>
      <td><img width="16" src="/rank/captain.png" /></td>
    </tr>
    <tr>
      <td>2</td>
      <td>PlayerTwo</td>
      <td>80</td>
      <td>40</td>
      <td>4000</td>
      <td>2.5</td>
      <td>8h 20m</td>
      <td>12</td>
      <td>15</td>
      <td>3</td>
      <td>25</td>
      <td>1</td>
      <td>8000m</td>
      <td>800</td>
      <td>40</td>
      <td>90</td>
      <td>Lieutenant</td>
      <td><img src="/rank/lieutenant.png" /></td>
    </tr>
    <tr>
      <td>3</td>
      <td>PlayerThree</td>
      <td>-</td>
    </tr>
  </table>
  <a href="?start=0">Previous</a>
  <a href="?start=40">Next</a>
</body>
</html>"#;

    const SERVER_PAGE_PARAMS: [(&str, &str); 3] = [("start", "0"), ("size", "100"), ("names", "1")];

    fn test_config() -> Config {
        Config {
            port: "0".to_string(),
            host: "127.0.0.1".to_string(),
            cache_duration_secs: 60,
            maps_config_path: "maps.json".to_string(),
            android_repo_url: None,
            web_repo_url: None,
            announcement_path: None,
            mcp_host: "127.0.0.1".to_string(),
            mcp_port: None,
            mcp_allowed_hosts: None,
        }
    }

    /// Router wired to `mock_server` with an isolated cache and service.
    fn test_router(
        mock_server: &MockServer,
        maps: Vec<MapEntry>,
        announcement: Announcement,
    ) -> Arc<Router> {
        let client = UpstreamClient::with_urls(
            Arc::new(ApiCache::new(60)),
            format!("{}/server_list.php", mock_server.uri()),
            format!("{}/view_players.php", mock_server.uri()),
        );
        Arc::new(build_router(
            Arc::new(test_config()),
            Arc::new(MapsConfig { maps }),
            Arc::new(announcement),
            Arc::new(RwrsService::with_client(client)),
        ))
    }

    fn default_router(mock_server: &MockServer) -> Arc<Router> {
        test_router(mock_server, Vec::new(), Announcement::disabled())
    }

    fn request(router_path: &str) -> RequestBuilder {
        TestClient::get(format!("http://127.0.0.1:5800{router_path}"))
    }

    fn assert_json_content_type(res: &Response) {
        let content_type = res
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default();
        assert!(
            content_type.starts_with("application/json"),
            "unexpected content type: {content_type}"
        );
    }

    /// Error bodies are exactly `{"error": "..."}`: one key, no internals.
    fn error_message(body: &serde_json::Value) -> &str {
        let object = body.as_object().expect("error body must be a JSON object");
        assert_eq!(
            object.len(),
            1,
            "error body must contain only the 'error' key: {body}"
        );
        object
            .get("error")
            .and_then(|value| value.as_str())
            .expect("error body must contain an 'error' string")
    }

    async fn mount_server_list(mock_server: &MockServer, body: &str, times: u64) {
        let mut mock = Mock::given(method("GET")).and(path("/server_list.php"));
        for (key, value) in SERVER_PAGE_PARAMS {
            mock = mock.and(query_param(key, value));
        }
        mock.respond_with(ResponseTemplate::new(200).set_body_string(body))
            .expect(times)
            .mount(mock_server)
            .await;
    }

    #[tokio::test]
    async fn servers_json_uses_default_pagination() {
        let mock_server = MockServer::start().await;
        mount_server_list(&mock_server, SERVER_XML, 1).await;
        let router = default_router(&mock_server);

        // Unknown query parameters (e.g. a cache buster or another endpoint's
        // parameter names) are ignored rather than rejected.
        let mut res = request("/api/servers.json?db=pacific&sort=kills&_t=123")
            .send(router)
            .await;

        assert_eq!(res.status_code, Some(StatusCode::OK));
        assert_json_content_type(&res);
        let body: serde_json::Value = res.take_json().await.unwrap();
        assert_eq!(body["total"], 5);
        assert_eq!(body["offset"], 0);
        assert_eq!(body["limit"], 20);
        let servers = body["servers"].as_array().expect("servers array");
        assert_eq!(servers.len(), 5);

        let first = &servers[0];
        assert_eq!(first["id"], "127.0.0.1:1234");
        assert_eq!(first["name"], "Alpha Server");
        assert_eq!(first["ipAddress"], "127.0.0.1");
        assert_eq!(first["mapName"], "Desert Town");
        assert_eq!(first["currentPlayers"], 10);
        assert_eq!(first["mode"], "Invasion");
        assert_eq!(
            first["playerList"],
            serde_json::json!(["Player1", "Player2"])
        );
        // JSON field names are camelCase, never the Rust snake_case ones.
        assert!(first.get("map_id").is_none());
        assert!(first.get("current_players").is_none());
    }

    #[tokio::test]
    async fn servers_json_filters_then_paginates() {
        let mock_server = MockServer::start().await;
        mount_server_list(&mock_server, SERVER_XML, 1).await;
        let router = default_router(&mock_server);

        let mut res = request(
            "/api/servers.json?search=desert&online_only=true&map=desert\
             &mode=invasion&country=us&offset=1&limit=1",
        )
        .send(router)
        .await;

        assert_eq!(res.status_code, Some(StatusCode::OK));
        assert_json_content_type(&res);
        let body: serde_json::Value = res.take_json().await.unwrap();
        // `search=desert` matches Alpha, Gamma and Epsilon; `online_only`
        // drops the empty Epsilon; the remaining two pass map/mode/country.
        assert_eq!(body["total"], 2);
        assert_eq!(body["offset"], 1);
        assert_eq!(body["limit"], 1);
        let servers = body["servers"].as_array().expect("servers array");
        assert_eq!(servers.len(), 1);
        assert_eq!(servers[0]["id"], "127.0.0.3:9000");
        assert_eq!(servers[0]["mapName"], "Desert Camp");
        assert_eq!(servers[0]["currentPlayers"], 3);
    }

    #[tokio::test]
    async fn players_json_forwards_query_and_returns_camel_case_fields() {
        let mock_server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/view_players.php"))
            .and(query_param("db", "pacific"))
            .and(query_param("start", "20"))
            .and(query_param("size", "5"))
            .and(query_param("sort", "kills"))
            .and(query_param_is_missing("search"))
            .respond_with(ResponseTemplate::new(200).set_body_string(PLAYER_HTML))
            .expect(1)
            .mount(&mock_server)
            .await;
        let router = default_router(&mock_server);

        let mut res = request("/api/players.json?db=pacific&sort=kills&offset=20&limit=5")
            .send(router)
            .await;

        assert_eq!(res.status_code, Some(StatusCode::OK));
        assert_json_content_type(&res);
        let body: serde_json::Value = res.take_json().await.unwrap();
        assert_eq!(body["db"], "pacific");
        assert_eq!(body["offset"], 20);
        assert_eq!(body["limit"], 5);
        assert_eq!(body["hasNext"], true);
        assert_eq!(body["hasPrevious"], true);

        let players = body["players"].as_array().expect("players array");
        assert_eq!(players.len(), 3);
        let first = &players[0];
        assert_eq!(first["id"], "pacific:PlayerOne");
        assert_eq!(first["username"], "PlayerOne");
        assert_eq!(first["db"], "pacific");
        assert_eq!(first["rowNumber"], 1);
        assert_eq!(first["rankProgression"], 100);
        assert_eq!(first["kills"], 100);
        assert_eq!(first["kd"].as_f64(), Some(2.0));
        assert_eq!(first["timePlayed"], "10h 30m");
        assert_eq!(first["rankName"], "Captain");
        assert_eq!(first["rankIcon"], "/rank/captain.png");
        assert!(first.get("row_number").is_none());

        // Missing values stay explicit nulls instead of 0 / "".
        let sparse = &players[2];
        assert_eq!(sparse["username"], "PlayerThree");
        assert!(sparse["kills"].is_null());
        assert!(sparse["kd"].is_null());
        assert!(sparse["rankName"].is_null());
        assert!(sparse["rankIcon"].is_null());
    }

    #[tokio::test]
    async fn invalid_queries_return_400_without_calling_upstream() {
        let mock_server = MockServer::start().await;
        let router = default_router(&mock_server);

        let cases = [
            "/api/players.json?db=not_a_db",
            "/api/players.json?sort=not_a_sort",
            "/api/servers.json?limit=0",
            "/api/servers.json?limit=101",
            "/api/servers.json?offset=-1",
            "/api/players.json?limit=0",
            "/api/players.json?limit=101",
            "/api/servers.json?online_only=maybe",
            "/api/servers.json?online_only=1",
            "/api/servers.json?online_only=TRUE",
        ];

        for case in cases {
            let mut res = request(case).send(router.clone()).await;
            assert_eq!(
                res.status_code,
                Some(StatusCode::BAD_REQUEST),
                "case {case} should be rejected"
            );
            assert_json_content_type(&res);
            let body: serde_json::Value = res.take_json().await.unwrap();
            let message = error_message(&body);
            assert!(!message.is_empty(), "case {case} returned an empty error");
        }

        let received = mock_server.received_requests().await.unwrap();
        assert!(
            received.is_empty(),
            "invalid queries must not reach the upstream, got {} request(s)",
            received.len()
        );
    }

    #[tokio::test]
    async fn upstream_failure_returns_502_json_error() {
        let mock_server = MockServer::start().await;
        for upstream_path in ["/server_list.php", "/view_players.php"] {
            Mock::given(method("GET"))
                .and(path(upstream_path))
                .respond_with(ResponseTemplate::new(500).set_body_string("upstream exploded"))
                .expect(1)
                .mount(&mock_server)
                .await;
        }
        let router = default_router(&mock_server);

        for endpoint in ["/api/servers.json", "/api/players.json"] {
            let mut res = request(endpoint).send(router.clone()).await;
            assert_eq!(
                res.status_code,
                Some(StatusCode::BAD_GATEWAY),
                "{endpoint} should report the upstream failure"
            );
            assert_json_content_type(&res);
            let body: serde_json::Value = res.take_json().await.unwrap();
            let message = error_message(&body);
            assert!(
                message.contains("500"),
                "{endpoint} error should mention the upstream status, got: {message}"
            );
        }
    }

    #[tokio::test]
    async fn metadata_routes_still_work_after_router_extraction() {
        let mock_server = MockServer::start().await;
        let maps = vec![MapEntry {
            name: "Test Map".to_string(),
            path: "media/packages/vanilla/maps/test".to_string(),
            image: "test.png".to_string(),
        }];
        let announcement = Announcement {
            enabled: true,
            html: "<b>maintenance</b>".to_string(),
        };
        let router = test_router(&mock_server, maps, announcement);

        let mut ping = request("/ping").send(router.clone()).await;
        assert_eq!(ping.status_code, Some(StatusCode::OK));
        assert_eq!(ping.take_string().await.unwrap(), "pong");

        let mut maps_res = request("/api/maps").send(router.clone()).await;
        assert_eq!(maps_res.status_code, Some(StatusCode::OK));
        assert_json_content_type(&maps_res);
        let maps_body: serde_json::Value = maps_res.take_json().await.unwrap();
        assert_eq!(maps_body[0]["name"], "Test Map");

        let mut announcement_res = request("/api/announcement").send(router.clone()).await;
        assert_eq!(announcement_res.status_code, Some(StatusCode::OK));
        assert_json_content_type(&announcement_res);
        let announcement_body: serde_json::Value = announcement_res.take_json().await.unwrap();
        assert_eq!(announcement_body["enabled"], true);
        assert_eq!(announcement_body["html"], "<b>maintenance</b>");

        // No repository URLs configured: must answer without any network call.
        let mut version_res = request("/api/version").send(router).await;
        assert_eq!(version_res.status_code, Some(StatusCode::OK));
        assert_json_content_type(&version_res);
        let version_body: serde_json::Value = version_res.take_json().await.unwrap();
        assert!(version_body["android"]["version"].is_null());
        assert!(version_body["web"]["version"].is_null());

        let received = mock_server.received_requests().await.unwrap();
        assert!(
            received.is_empty(),
            "metadata endpoints must not call the upstream, got {} request(s)",
            received.len()
        );
    }

    #[tokio::test]
    async fn passthrough_server_list_forwards_query_and_body() {
        let mock_server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/server_list.php"))
            .and(query_param("foo", "bar"))
            .and(query_param("baz", "1"))
            .respond_with(ResponseTemplate::new(200).set_body_string(SERVER_XML))
            .expect(1)
            .mount(&mock_server)
            .await;
        let router = default_router(&mock_server);

        let mut res = request("/api/server_list?foo=bar&baz=1")
            .send(router.clone())
            .await;
        assert_eq!(res.status_code, Some(StatusCode::OK));
        assert_eq!(
            res.take_string().await.unwrap(),
            SERVER_XML,
            "passthrough body must match the upstream payload byte for byte"
        );

        // `goal()` keeps the passthrough method-agnostic, so POST is proxied
        // through the same handler (and served from the shared cache).
        let mut post = TestClient::post("http://127.0.0.1:5800/api/server_list?foo=bar&baz=1")
            .send(router.clone())
            .await;
        assert_eq!(post.status_code, Some(StatusCode::OK));
        assert_eq!(post.take_string().await.unwrap(), SERVER_XML);

        // Non-2xx upstream status and body are passed through unchanged.
        Mock::given(method("GET"))
            .and(path("/server_list.php"))
            .and(query_param("foo", "bar"))
            .and(query_param("baz", "3"))
            .respond_with(ResponseTemplate::new(503).set_body_string("overloaded"))
            .expect(1)
            .mount(&mock_server)
            .await;
        let mut unavailable = request("/api/server_list?foo=bar&baz=3").send(router).await;
        assert_eq!(
            unavailable.status_code,
            Some(StatusCode::SERVICE_UNAVAILABLE)
        );
        assert_eq!(unavailable.take_string().await.unwrap(), "overloaded");

        let received = mock_server.received_requests().await.unwrap();
        let original = received
            .iter()
            .find(|received| received.url.query() == Some("foo=bar&baz=1"))
            .expect("upstream must receive the original query string");
        assert_eq!(original.method, wiremock::http::Method::GET);
    }

    #[tokio::test]
    async fn passthrough_player_list_forwards_query_and_body() {
        let mock_server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/view_players.php"))
            .and(query_param("db", "pacific"))
            .respond_with(ResponseTemplate::new(200).set_body_string(PLAYER_HTML))
            .expect(1)
            .mount(&mock_server)
            .await;
        let router = default_router(&mock_server);

        let mut res = request("/api/player_list?db=pacific").send(router).await;
        assert_eq!(res.status_code, Some(StatusCode::OK));
        assert_eq!(res.take_string().await.unwrap(), PLAYER_HTML);
    }

    #[tokio::test]
    async fn post_servers_json_is_not_served_and_does_not_reach_upstream() {
        let mock_server = MockServer::start().await;
        let router = default_router(&mock_server);

        let res = TestClient::post("http://127.0.0.1:5800/api/servers.json")
            .send(router)
            .await;

        assert!(
            res.status_code.is_some_and(|status| !status.is_success()),
            "POST must not be served by the GET-only JSON route, got {:?}",
            res.status_code
        );
        let received = mock_server.received_requests().await.unwrap();
        assert!(
            received.is_empty(),
            "rejected POST must not reach the upstream, got {} request(s)",
            received.len()
        );
    }
}
