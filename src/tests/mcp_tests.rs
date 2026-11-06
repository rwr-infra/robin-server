#[cfg(test)]
mod tests {
    use crate::mcp::build_mcp_router;
    use crate::upstream::{RwrsService, UpstreamClient};
    use crate::{
        ApiCache, DEFAULT_MCP_PORT, MapEntry, MapsConfig, default_mcp_allowed_hosts,
        format_mcp_url, is_loopback_host, mcp_port_conflicts_with_main, parse_allowed_hosts,
        parse_mcp_port,
    };
    use serde_json::{Value, json};
    use std::sync::Arc;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{method, path, query_param, query_param_is_missing},
    };

    const PROTOCOL_VERSION: &str = "2025-11-25";

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
  </table>
  <a href="?start=0">Previous</a>
  <a href="?start=5">Next</a>
</body>
</html>"#;

    #[test]
    fn parse_mcp_port_handles_default_zero_and_invalid_values() {
        assert_eq!(parse_mcp_port(None), Some(DEFAULT_MCP_PORT));
        assert_eq!(parse_mcp_port(Some("")), Some(DEFAULT_MCP_PORT));
        assert_eq!(parse_mcp_port(Some("0")), None);
        assert_eq!(parse_mcp_port(Some("8080")), Some(8080));
        assert_eq!(parse_mcp_port(Some("65535")), Some(65535));
        assert_eq!(parse_mcp_port(Some("not-a-port")), Some(DEFAULT_MCP_PORT));
        assert_eq!(parse_mcp_port(Some("70000")), Some(DEFAULT_MCP_PORT));
    }

    #[test]
    fn mcp_port_conflicts_with_main_only_on_matching_numeric_ports() {
        assert!(mcp_port_conflicts_with_main(Some(5800), "5800"));
        assert!(mcp_port_conflicts_with_main(Some(5800), " 5800 "));
        assert!(!mcp_port_conflicts_with_main(Some(5801), "5800"));
        assert!(!mcp_port_conflicts_with_main(Some(5800), "not-a-port"));
        assert!(!mcp_port_conflicts_with_main(Some(5800), ""));
        assert!(!mcp_port_conflicts_with_main(None, "5800"));
    }

    #[test]
    fn parse_allowed_hosts_splits_and_drops_empty_entries() {
        assert_eq!(parse_allowed_hosts(None), None);
        assert_eq!(parse_allowed_hosts(Some("")), None);
        assert_eq!(parse_allowed_hosts(Some(" , ,")), None);
        assert_eq!(
            parse_allowed_hosts(Some("localhost, 127.0.0.1 ,,::1,")),
            Some(vec![
                "localhost".to_string(),
                "127.0.0.1".to_string(),
                "::1".to_string(),
            ])
        );
    }

    #[test]
    fn is_loopback_host_accepts_names_and_addresses() {
        assert!(is_loopback_host("localhost"));
        assert!(is_loopback_host("LOCALHOST"));
        assert!(is_loopback_host("127.0.0.1"));
        assert!(is_loopback_host("127.0.0.2"));
        assert!(is_loopback_host("::1"));
        assert!(is_loopback_host("[::1]"));
        assert!(!is_loopback_host("0.0.0.0"));
        assert!(!is_loopback_host("::"));
        assert!(!is_loopback_host("example.com"));
        assert!(!is_loopback_host(""));
    }

    #[test]
    fn default_mcp_allowed_hosts_covers_extra_loopback_bind_addresses() {
        let extended = default_mcp_allowed_hosts("127.0.0.2");
        assert!(extended.contains(&"127.0.0.2".to_string()));
        assert!(extended.contains(&"localhost".to_string()));
        assert!(extended.contains(&"127.0.0.1".to_string()));

        let ipv6 = default_mcp_allowed_hosts("::1");
        assert_eq!(ipv6.iter().filter(|host| *host == "::1").count(), 1);

        let bracketed = default_mcp_allowed_hosts("[::1]");
        assert_eq!(bracketed.iter().filter(|host| *host == "::1").count(), 1);
        assert!(!bracketed.iter().any(|host| host == "[::1]"));

        let localhost = default_mcp_allowed_hosts("localhost");
        assert_eq!(
            localhost.iter().filter(|host| *host == "localhost").count(),
            1
        );
    }

    #[test]
    fn format_mcp_url_brackets_ipv6_literals() {
        assert_eq!(format_mcp_url("[::1]", 5801), "http://[::1]:5801/mcp");
        assert_eq!(format_mcp_url("::1", 5801), "http://[::1]:5801/mcp");
        assert_eq!(
            format_mcp_url("127.0.0.1", 5801),
            "http://127.0.0.1:5801/mcp"
        );
        assert_eq!(
            format_mcp_url("localhost", 5801),
            "http://localhost:5801/mcp"
        );
    }

    /// MCP listener bound to an ephemeral loopback port, aborted on drop.
    struct McpTestServer {
        handle: tokio::task::JoinHandle<()>,
        url: String,
    }

    impl McpTestServer {
        async fn start(mock_server: &MockServer, maps: Vec<MapEntry>) -> Self {
            Self::start_with_allowed_hosts(
                mock_server,
                maps,
                Some(vec![
                    "localhost".to_string(),
                    "127.0.0.1".to_string(),
                    "::1".to_string(),
                ]),
            )
            .await
        }

        async fn start_with_allowed_hosts(
            mock_server: &MockServer,
            maps: Vec<MapEntry>,
            allowed_hosts: Option<Vec<String>>,
        ) -> Self {
            let client = UpstreamClient::with_urls(
                Arc::new(ApiCache::new(60)),
                format!("{}/server_list.php", mock_server.uri()),
                format!("{}/view_players.php", mock_server.uri()),
            );
            let service = Arc::new(RwrsService::with_client(client));
            let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
                .await
                .expect("bind MCP test listener");
            let address = listener.local_addr().expect("listener address");
            let router = build_mcp_router(service, Arc::new(MapsConfig { maps }), allowed_hosts);
            let handle = tokio::spawn(async move {
                let _ = axum::serve(listener, router).await;
            });

            Self {
                handle,
                url: format!("http://{address}/mcp"),
            }
        }

        fn client() -> reqwest::Client {
            reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(10))
                .build()
                .expect("build reqwest test client")
        }

        async fn post(&self, client: &reqwest::Client, body: &Value) -> reqwest::Response {
            client
                .post(&self.url)
                .header("Accept", "application/json, text/event-stream")
                .header("Content-Type", "application/json")
                .header("MCP-Protocol-Version", PROTOCOL_VERSION)
                .json(body)
                .send()
                .await
                .expect("send MCP request")
        }

        async fn get(&self, client: &reqwest::Client) -> reqwest::Response {
            client
                .get(&self.url)
                .header("Accept", "application/json, text/event-stream")
                .header("MCP-Protocol-Version", PROTOCOL_VERSION)
                .send()
                .await
                .expect("send MCP GET request")
        }

        async fn rpc(&self, client: &reqwest::Client, body: Value) -> Value {
            let response = self.post(client, &body).await;
            assert_eq!(response.status(), 200, "unexpected HTTP status for {body}");
            parse_rpc_body(response).await
        }

        async fn initialize(&self, client: &reqwest::Client) -> Value {
            self.rpc(client, initialize_body()).await
        }

        async fn call_tool(
            &self,
            client: &reqwest::Client,
            id: u64,
            name: &str,
            arguments: Value,
        ) -> Value {
            let body = json!({
                "jsonrpc": "2.0",
                "id": id,
                "method": "tools/call",
                "params": {"name": name, "arguments": arguments}
            });
            self.rpc(client, body).await
        }
    }

    impl Drop for McpTestServer {
        fn drop(&mut self) {
            self.handle.abort();
        }
    }

    fn initialize_body() -> Value {
        json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": PROTOCOL_VERSION,
                "capabilities": {},
                "clientInfo": {"name": "rwrs-test", "version": "0.1.0"}
            }
        })
    }

    /// Read a JSON-RPC response, unwrapping the last SSE event when needed.
    async fn parse_rpc_body(response: reqwest::Response) -> Value {
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_string();
        let text = response.text().await.expect("read MCP response body");

        if content_type.contains("text/event-stream") {
            text.lines()
                .filter_map(|line| line.strip_prefix("data:"))
                .filter_map(|data| serde_json::from_str::<Value>(data.trim()).ok())
                .last()
                .unwrap_or_else(|| panic!("no JSON payload in SSE response: {text}"))
        } else {
            serde_json::from_str(&text)
                .unwrap_or_else(|error| panic!("invalid JSON response ({error}): {text}"))
        }
    }

    async fn mount_server_list(mock_server: &MockServer, body: &str) {
        Mock::given(method("GET"))
            .and(path("/server_list.php"))
            .and(query_param("start", "0"))
            .and(query_param("size", "100"))
            .and(query_param("names", "1"))
            .respond_with(ResponseTemplate::new(200).set_body_string(body))
            .mount(mock_server)
            .await;
    }

    fn maps_config() -> Vec<MapEntry> {
        vec![MapEntry {
            name: "Desert Town".to_string(),
            path: "desert_town".to_string(),
            image: "/maps/desert_town.png".to_string(),
        }]
    }

    fn tool_names(result: &Value) -> Vec<String> {
        let mut names: Vec<String> = result["result"]["tools"]
            .as_array()
            .expect("tools array")
            .iter()
            .map(|tool| tool["name"].as_str().expect("tool name").to_string())
            .collect();
        names.sort();
        names
    }

    #[tokio::test]
    async fn initialize_then_tools_list_then_tool_calls_return_structured_json() {
        let mock_server = MockServer::start().await;
        mount_server_list(&mock_server, SERVER_XML).await;
        let server = McpTestServer::start(&mock_server, maps_config()).await;
        let client = McpTestServer::client();

        let initialize = server.initialize(&client).await;
        assert_eq!(
            initialize["result"]["protocolVersion"], PROTOCOL_VERSION,
            "initialize result: {initialize}"
        );
        assert!(
            initialize["result"]["capabilities"]["tools"].is_object(),
            "initialize result: {initialize}"
        );
        assert_eq!(initialize["result"]["serverInfo"]["name"], "rwrs-server");

        let notification = json!({"jsonrpc": "2.0", "method": "notifications/initialized"});
        let response = server.post(&client, &notification).await;
        assert_eq!(response.status(), 202);

        let tools = server
            .rpc(
                &client,
                json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {}}),
            )
            .await;
        assert_eq!(
            tool_names(&tools),
            vec!["get_server", "list_maps", "list_servers", "search_players"]
        );

        let maps = server.call_tool(&client, 3, "list_maps", json!({})).await;
        assert_eq!(maps["result"]["isError"], false, "list_maps: {maps}");
        assert_eq!(
            maps["result"]["structuredContent"]["maps"][0]["path"],
            Value::String("desert_town".to_string())
        );

        let servers = server
            .call_tool(&client, 4, "list_servers", json!({"limit": 1}))
            .await;
        assert_eq!(servers["result"]["isError"], false, "servers: {servers}");
        assert_eq!(servers["result"]["structuredContent"]["total"], 2);
        assert_eq!(
            servers["result"]["structuredContent"]["servers"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            servers["result"]["structuredContent"]["servers"][0]["id"],
            "127.0.0.1:1234"
        );
        assert_eq!(
            servers["result"]["structuredContent"]["servers"][0]["mapId"],
            "desert_town"
        );

        let server_lookup = server
            .call_tool(
                &client,
                5,
                "get_server",
                json!({"address": "127.0.0.1", "port": 1234}),
            )
            .await;
        assert_eq!(
            server_lookup["result"]["isError"], false,
            "get_server: {server_lookup}"
        );
        let found = &server_lookup["result"]["structuredContent"];
        assert_eq!(found["id"], "127.0.0.1:1234");
        assert_eq!(found["playerList"], json!(["Player1", "Player2"]));
    }

    #[tokio::test]
    async fn invalid_player_arguments_are_json_rpc_invalid_params_errors() {
        let mock_server = MockServer::start().await;
        let server = McpTestServer::start(&mock_server, Vec::new()).await;
        let client = McpTestServer::client();
        server.initialize(&client).await;

        let response = server
            .call_tool(&client, 2, "search_players", json!({"db": "bogus"}))
            .await;

        assert!(
            response.get("result").is_none(),
            "expected a JSON-RPC error, got {response}"
        );
        let error = &response["error"];
        assert_eq!(error["code"], -32602, "error: {error}");
        assert!(
            error["message"].as_str().unwrap_or_default().contains("db"),
            "error: {error}"
        );

        let response = server
            .call_tool(&client, 3, "search_players", json!({"limit": 0}))
            .await;
        assert!(
            response.get("result").is_none(),
            "expected a JSON-RPC error, got {response}"
        );
        assert_eq!(response["error"]["code"], -32602, "error: {response}");
        assert!(
            response["error"]["message"]
                .as_str()
                .unwrap_or_default()
                .contains("limit"),
            "error: {response}"
        );
    }

    #[tokio::test]
    async fn upstream_failure_is_a_tool_error_with_structured_message() {
        let mock_server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/server_list.php"))
            .and(query_param("start", "0"))
            .respond_with(ResponseTemplate::new(500).set_body_string("boom"))
            .mount(&mock_server)
            .await;
        let server = McpTestServer::start(&mock_server, Vec::new()).await;
        let client = McpTestServer::client();
        server.initialize(&client).await;

        let response = server
            .call_tool(&client, 2, "list_servers", json!({}))
            .await;

        assert_eq!(response["result"]["isError"], true, "response: {response}");
        let message = response["result"]["structuredContent"]["error"]
            .as_str()
            .expect("structured error message");
        assert!(message.contains("500"), "error message: {message}");
    }

    #[tokio::test]
    async fn get_server_reports_not_found_as_a_tool_error() {
        let mock_server = MockServer::start().await;
        mount_server_list(&mock_server, SERVER_XML).await;
        let server = McpTestServer::start(&mock_server, Vec::new()).await;
        let client = McpTestServer::client();
        server.initialize(&client).await;

        let response = server
            .call_tool(
                &client,
                2,
                "get_server",
                json!({"address": "10.0.0.9", "port": 9999}),
            )
            .await;

        assert_eq!(response["result"]["isError"], true, "response: {response}");
        let structured = &response["result"]["structuredContent"];
        assert_eq!(structured["error"], "server not found");
        assert_eq!(structured["address"], "10.0.0.9");
        assert_eq!(structured["port"], 9999);
    }

    #[tokio::test]
    async fn disabled_host_validation_accepts_default_host_header() {
        let mock_server = MockServer::start().await;
        let server = McpTestServer::start_with_allowed_hosts(&mock_server, Vec::new(), None).await;
        let client = McpTestServer::client();

        // reqwest sends `Host: 127.0.0.1:<port>`, which must pass when the
        // allowlist is disabled.
        let initialize = server.initialize(&client).await;
        assert_eq!(
            initialize["result"]["protocolVersion"], PROTOCOL_VERSION,
            "initialize result: {initialize}"
        );
    }

    #[tokio::test]
    async fn disallowed_host_header_is_rejected_with_403() {
        let mock_server = MockServer::start().await;
        let server = McpTestServer::start_with_allowed_hosts(
            &mock_server,
            Vec::new(),
            Some(vec!["allowed.example".to_string()]),
        )
        .await;
        let client = McpTestServer::client();

        let response = server.post(&client, &initialize_body()).await;

        assert_eq!(
            response.status(),
            reqwest::StatusCode::FORBIDDEN,
            "Host 127.0.0.1 must not match allowed.example"
        );
    }

    #[tokio::test]
    async fn search_players_returns_structured_page_and_forwards_query() {
        let mock_server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/view_players.php"))
            .and(query_param("db", "pacific"))
            .and(query_param("start", "0"))
            .and(query_param("size", "5"))
            .and(query_param("sort", "kills"))
            .and(query_param_is_missing("search"))
            .respond_with(ResponseTemplate::new(200).set_body_string(PLAYER_HTML))
            .expect(1)
            .mount(&mock_server)
            .await;
        let server = McpTestServer::start(&mock_server, Vec::new()).await;
        let client = McpTestServer::client();
        server.initialize(&client).await;

        let response = server
            .call_tool(
                &client,
                2,
                "search_players",
                json!({"db": "pacific", "sort": "kills", "limit": 5}),
            )
            .await;

        assert_eq!(response["result"]["isError"], false, "response: {response}");
        let structured = &response["result"]["structuredContent"];
        assert_eq!(structured["db"], "pacific");
        assert_eq!(structured["offset"], 0);
        assert_eq!(structured["limit"], 5);
        assert_eq!(structured["hasNext"], true);
        assert_eq!(structured["hasPrevious"], true);
        let players = structured["players"].as_array().expect("players array");
        assert_eq!(players.len(), 2);
        assert_eq!(players[0]["id"], "pacific:PlayerOne");
        assert_eq!(players[0]["kills"], 100);
        assert_eq!(players[1]["id"], "pacific:PlayerTwo");

        let requests = mock_server
            .received_requests()
            .await
            .expect("received requests");
        assert_eq!(requests.len(), 1);
        let query = requests[0].url.query().unwrap_or_default();
        for expected in ["db=pacific", "start=0", "size=5", "sort=kills"] {
            assert!(
                query.contains(expected),
                "query '{query}' missing {expected}"
            );
        }
        assert!(!query.contains("search="), "query: {query}");
    }

    #[tokio::test]
    async fn tools_list_works_without_initialize_in_stateless_mode() {
        let mock_server = MockServer::start().await;
        let server = McpTestServer::start(&mock_server, Vec::new()).await;
        let client = McpTestServer::client();

        // Stateless transport: no initialize/session handshake first.
        let tools = server
            .rpc(
                &client,
                json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list", "params": {}}),
            )
            .await;

        assert_eq!(
            tool_names(&tools),
            vec!["get_server", "list_maps", "list_servers", "search_players"]
        );
    }

    #[tokio::test]
    async fn get_on_mcp_endpoint_is_method_not_allowed() {
        let mock_server = MockServer::start().await;
        let server = McpTestServer::start(&mock_server, Vec::new()).await;
        let client = McpTestServer::client();

        let response = server.get(&client).await;

        assert_eq!(response.status(), reqwest::StatusCode::METHOD_NOT_ALLOWED);
        let allow = response
            .headers()
            .get(reqwest::header::ALLOW)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default();
        assert!(allow.contains("POST"), "Allow: {allow}");
    }

    #[tokio::test]
    async fn list_maps_with_empty_configuration_returns_empty_array() {
        let mock_server = MockServer::start().await;
        let server = McpTestServer::start(&mock_server, Vec::new()).await;
        let client = McpTestServer::client();

        let response = server.call_tool(&client, 1, "list_maps", json!({})).await;

        assert_eq!(response["result"]["structuredContent"], json!({"maps": []}));
        assert_eq!(response["result"]["isError"], false);
    }
}
