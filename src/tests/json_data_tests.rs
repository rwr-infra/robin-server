#[cfg(test)]
mod tests {
    use crate::ApiCache;
    use crate::domain::{
        DataError, PLAYER_SORT_FIELDS, PlayerDatabase, PlayerItem, PlayerListResponse, PlayerQuery,
        ServerListResponse, ServerQuery, UpstreamError,
    };
    use crate::parsers::{parse_player_list, parse_server_list};
    use crate::upstream::{RwrsService, UpstreamClient};
    use std::sync::Arc;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{method, path, query_param, query_param_is_missing},
    };

    const FLAT_SERVER_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<result>
  <server>
    <name>Test Server 1</name>
    <address>127.0.0.1</address>
    <port>1234</port>
    <map_id>map1</map_id>
    <map_name>Test Map 1</map_name>
    <bots>5</bots>
    <country>US</country>
    <current_players>10</current_players>
    <timeStamp>1234567890</timeStamp>
    <version>1</version>
    <dedicated>1</dedicated>
    <mod>0</mod>
    <player>Player1</player>
    <player>Player2</player>
    <comment>Test server comment</comment>
    <url>http://example.com</url>
    <max_players>32</max_players>
    <mode>Test Mode</mode>
    <realm>test</realm>
  </server>
  <server>
    <name>Test Server 2</name>
    <address>127.0.0.2</address>
    <port>5678</port>
    <map_id>map2</map_id>
    <map_name>Test Map 2</map_name>
    <bots>0</bots>
    <country>UK</country>
    <current_players>5</current_players>
    <timestamp>1234567891</timestamp>
    <version>1</version>
    <dedicated>TRUE</dedicated>
    <mod>False</mod>
    <player>Player3</player>
    <comment>Another test server</comment>
    <url>http://example2.com</url>
    <max_players>16</max_players>
    <mode>Another Mode</mode>
    <realm>test</realm>
  </server>
</result>"#;

    const NESTED_SERVER_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<result>
  <server_list>
    <server>
      <name>Nested Server</name>
      <address>10.0.0.9</address>
      <port>7777</port>
      <map_id>map9</map_id>
      <player>Solo</player>
    </server>
  </server_list>
</result>"#;

    const FILTER_XML: &str = r#"<result>
  <server>
    <name>Alpha Server</name>
    <address>127.0.0.1</address>
    <port>1234</port>
    <map_id>map1</map_id>
    <map_name>Desert Town</map_name>
    <country>US</country>
    <current_players>10</current_players>
    <mode>Test Mode</mode>
    <comment>friendly</comment>
    <player>Player1</player>
    <player>Player2</player>
  </server>
  <server>
    <name>Beta Server</name>
    <address>127.0.0.2</address>
    <port>5678</port>
    <map_id>map2</map_id>
    <map_name>Killhouse</map_name>
    <country>UK</country>
    <current_players>0</current_players>
    <mode>Another Mode</mode>
    <comment>empty</comment>
    <player>Player3</player>
  </server>
  <server>
    <name>Gamma</name>
    <address>127.0.0.3</address>
    <port>9000</port>
    <map_id>map3</map_id>
    <map_name>Desert Camp</map_name>
    <country>US</country>
    <current_players>3</current_players>
    <mode>Test Mode</mode>
    <comment>third</comment>
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
      <td><a href="/profile?name=PlayerTwo">PlayerTwo</a></td>
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
  <a href="?start=40">Next</a>
</body>
</html>"#;

    fn servers_response(servers: &[(String, u16)]) -> String {
        let mut xml = String::from("<result>");
        for (address, port) in servers {
            xml.push_str(&format!(
                "<server><name>Server {address}</name><address>{address}</address>\
                 <port>{port}</port><current_players>1</current_players></server>"
            ));
        }
        xml.push_str("</result>");
        xml
    }

    async fn mock_endpoint(
        mock_server: &MockServer,
        url_path: &str,
        params: &[(&str, &str)],
        body: String,
        times: u64,
    ) {
        let mut mock = Mock::given(method("GET")).and(path(url_path));
        for (key, value) in params {
            mock = mock.and(query_param(*key, *value));
        }
        mock.respond_with(ResponseTemplate::new(200).set_body_string(body))
            .expect(times)
            .mount(mock_server)
            .await;
    }

    fn client_for(mock_server: &MockServer) -> UpstreamClient {
        UpstreamClient::with_urls(
            Arc::new(ApiCache::new(60)),
            format!("{}/server_list.php", mock_server.uri()),
            format!("{}/view_players.php", mock_server.uri()),
        )
    }

    // --- Server list XML -------------------------------------------------

    #[test]
    fn parses_flat_result_structure() {
        let servers = parse_server_list(FLAT_SERVER_XML).expect("valid XML");

        assert_eq!(servers.len(), 2);
        assert_eq!(servers[0].id, "127.0.0.1:1234");
        assert_eq!(servers[0].name, "Test Server 1");
        assert_eq!(servers[0].ip_address, "127.0.0.1");
        assert_eq!(servers[0].port, 1234);
        assert_eq!(servers[0].map_id, "map1");
        assert_eq!(servers[0].map_name, "Test Map 1");
        assert_eq!(servers[0].bots, 5);
        assert_eq!(servers[0].country, "US");
        assert_eq!(servers[0].current_players, 10);
        assert_eq!(servers[0].time_stamp, 1_234_567_890);
        assert_eq!(servers[0].version, "1");
        assert!(servers[0].dedicated);
        assert!(!servers[0].mod_flag);
        assert_eq!(servers[0].player_list, vec!["Player1", "Player2"]);
        assert_eq!(servers[0].comment, "Test server comment");
        assert_eq!(servers[0].url, "http://example.com");
        assert_eq!(servers[0].max_players, 32);
        assert_eq!(servers[0].mode, "Test Mode");
        assert_eq!(servers[0].realm.as_deref(), Some("test"));

        // Lowercase `timestamp` and textual booleans are accepted too.
        assert_eq!(servers[1].time_stamp, 1_234_567_891);
        assert!(servers[1].dedicated);
        assert!(!servers[1].mod_flag);
    }

    #[test]
    fn serializes_with_frontend_camel_case_keys() {
        let servers = parse_server_list(FLAT_SERVER_XML).expect("valid XML");
        let value = serde_json::to_value(&servers[0]).expect("serializes");

        for key in [
            "id",
            "name",
            "ipAddress",
            "port",
            "mapId",
            "mapName",
            "bots",
            "country",
            "currentPlayers",
            "timeStamp",
            "version",
            "dedicated",
            "mod",
            "playerList",
            "comment",
            "url",
            "maxPlayers",
            "mode",
            "realm",
        ] {
            assert!(value.get(key).is_some(), "missing key {key}");
        }
        assert!(value.get("modFlag").is_none());
    }

    #[test]
    fn parses_nested_server_list_structure() {
        let servers = parse_server_list(NESTED_SERVER_XML).expect("valid XML");

        assert_eq!(servers.len(), 1);
        assert_eq!(servers[0].id, "10.0.0.9:7777");
        assert_eq!(servers[0].player_list, vec!["Solo"]);
    }

    #[test]
    fn parses_server_list_root_and_unexpected_wrappers() {
        let root = parse_server_list(
            "<server_list><server><name>Rooted</name><address>1.2.3.4</address>\
             <port>42</port></server></server_list>",
        )
        .unwrap();
        assert_eq!(root.len(), 1);
        assert_eq!(root[0].name, "Rooted");

        let wrapped = parse_server_list(
            "<result><data><server><name>Wrapped</name><address>5.6.7.8</address>\
             <port>7</port></server></data></result>",
        )
        .unwrap();
        assert_eq!(wrapped.len(), 1);
        assert_eq!(wrapped[0].name, "Wrapped");
    }

    #[test]
    fn empty_server_list_parses_to_empty_vec() {
        assert!(parse_server_list("<result></result>").unwrap().is_empty());
        assert!(
            parse_server_list("<result><server_list></server_list></result>")
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn missing_server_fields_fall_back_to_defaults() {
        let servers =
            parse_server_list("<result><server><name>Only</name></server></result>").unwrap();

        assert_eq!(servers.len(), 1);
        let server = &servers[0];
        assert_eq!(server.id, ":0");
        assert_eq!(server.name, "Only");
        assert_eq!(server.ip_address, "");
        assert_eq!(server.port, 0);
        assert_eq!(server.map_id, "");
        assert_eq!(server.map_name, "");
        assert_eq!(server.bots, 0);
        assert_eq!(server.country, "");
        assert_eq!(server.current_players, 0);
        assert_eq!(server.time_stamp, 0);
        assert_eq!(server.version, "");
        assert!(!server.dedicated);
        assert!(!server.mod_flag);
        assert!(server.player_list.is_empty());
        assert_eq!(server.comment, "");
        assert_eq!(server.url, "");
        assert_eq!(server.max_players, 0);
        assert_eq!(server.mode, "");
        assert_eq!(server.realm, None);
    }

    #[test]
    fn malformed_xml_returns_parse_error() {
        assert!(parse_server_list("<result><server>").is_err());
        assert!(parse_server_list("not xml at all").is_err());
    }

    #[test]
    fn empty_xml_body_is_a_parse_error() {
        assert!(parse_server_list("").is_err());
        assert!(parse_server_list("   \n").is_err());
    }

    #[test]
    fn timestamp_prefers_camel_case_when_both_exist() {
        let servers = parse_server_list(
            "<result><server><address>1.1.1.1</address><port>1</port>\
             <timeStamp>111</timeStamp><timestamp>222</timestamp></server></result>",
        )
        .unwrap();

        assert_eq!(servers[0].time_stamp, 111);
    }

    #[test]
    fn non_numeric_server_fields_fall_back_to_zero() {
        let servers = parse_server_list(
            "<result><server><address>1.1.1.1</address><port>nope</port>\
             <bots>abc</bots><current_players>x</current_players>\
             <max_players>12.5</max_players></server></result>",
        )
        .unwrap();

        assert_eq!(servers[0].port, 0);
        assert_eq!(servers[0].bots, 0);
        assert_eq!(servers[0].current_players, 0);
        assert_eq!(servers[0].max_players, 0);
        assert_eq!(servers[0].id, "1.1.1.1:0");
    }

    // --- Player list HTML ------------------------------------------------

    #[test]
    fn parses_player_rows_and_skips_header() {
        let result = parse_player_list(PLAYER_HTML, PlayerDatabase::Invasion);

        assert_eq!(result.players.len(), 2);
        assert_eq!(
            result.players[0],
            PlayerItem {
                id: "invasion:PlayerOne".to_string(),
                username: "PlayerOne".to_string(),
                db: PlayerDatabase::Invasion,
                row_number: 1,
                rank_progression: Some(100),
                kills: Some(100),
                deaths: Some(50),
                score: Some(5000),
                teamkills: Some(2),
                longest_kill_streak: Some(15),
                targets_destroyed: Some(20),
                vehicles_destroyed: Some(5),
                soldiers_healed: Some(30),
                shots_fired: Some(1000),
                throwables_thrown: Some(50),
                kd: Some(2.0),
                time_played: Some("10h 30m".to_string()),
                distance_moved: Some("10000m".to_string()),
                rank_name: Some("Captain".to_string()),
                rank_icon: Some("/rank/captain.png".to_string()),
            }
        );
        // Username inside an <a> element is still extracted.
        assert_eq!(result.players[1].username, "PlayerTwo");
        assert_eq!(result.players[1].id, "invasion:PlayerTwo");
        assert_eq!(result.players[1].kd, Some(2.5));
        assert_eq!(
            result.players[1].rank_icon.as_deref(),
            Some("/rank/lieutenant.png")
        );
    }

    #[test]
    fn serializes_players_with_frontend_camel_case_keys() {
        let result = parse_player_list(PLAYER_HTML, PlayerDatabase::Pacific);
        let value = serde_json::to_value(&result.players[0]).expect("serializes");

        for key in [
            "id",
            "username",
            "db",
            "rowNumber",
            "rankProgression",
            "kills",
            "deaths",
            "score",
            "teamkills",
            "longestKillStreak",
            "targetsDestroyed",
            "vehiclesDestroyed",
            "soldiersHealed",
            "shotsFired",
            "throwablesThrown",
            "kd",
            "timePlayed",
            "distanceMoved",
            "rankName",
            "rankIcon",
        ] {
            assert!(value.get(key).is_some(), "missing key {key}");
        }
        assert_eq!(value["db"], "pacific");
        assert_eq!(value["id"], "pacific:PlayerOne");
    }

    #[test]
    fn detects_pagination_links() {
        let result = parse_player_list(PLAYER_HTML, PlayerDatabase::Invasion);
        assert!(result.has_next);
        assert!(result.has_previous);

        let no_links = parse_player_list(
            "<html><body><table><tr><td>1</td><td>X</td></tr></table></body></html>",
            PlayerDatabase::Invasion,
        );
        assert!(!no_links.has_next);
        assert!(!no_links.has_previous);
    }

    #[test]
    fn empty_or_missing_table_returns_no_players() {
        let header_only = parse_player_list(
            "<html><body><table><tr><th>#</th><th>Username</th></tr></table></body></html>",
            PlayerDatabase::Invasion,
        );
        assert!(header_only.players.is_empty());

        let no_table = parse_player_list(
            "<html><body><div>No table here</div></body></html>",
            PlayerDatabase::Invasion,
        );
        assert!(no_table.players.is_empty());
        assert!(!no_table.has_next);
        assert!(!no_table.has_previous);
    }

    #[test]
    fn malformed_html_never_panics() {
        for html in ["", "<<<>>>", "<table><tr><td>", "<table><tr><td>1</td>"] {
            let result = parse_player_list(html, PlayerDatabase::Invasion);
            assert!(result.players.is_empty(), "html {html:?}");
        }
    }

    #[test]
    fn rows_with_fewer_than_two_cells_are_skipped() {
        let html = r#"<html><body><table>
            <tr><td>1</td></tr>
            <tr></tr>
            <tr><td>2</td><td>RealUser</td><td>7</td></tr>
        </table></body></html>"#;

        let result = parse_player_list(html, PlayerDatabase::Invasion);
        assert_eq!(result.players.len(), 1);
        assert_eq!(result.players[0].username, "RealUser");
        assert_eq!(result.players[0].kills, Some(7));
    }

    #[test]
    fn dash_and_empty_cells_become_none() {
        let html = r#"<html><body><table>
            <tr><th>#</th><th>Username</th></tr>
            <tr>
              <td>1</td><td>TestUser</td>
              <td></td><td>-</td><td></td><td>-</td><td></td>
              <td></td><td></td><td></td><td></td><td></td><td></td>
              <td></td><td></td><td></td><td></td><td></td>
            </tr>
        </table></body></html>"#;

        let result = parse_player_list(html, PlayerDatabase::Invasion);
        assert_eq!(result.players.len(), 1);
        let player = &result.players[0];
        assert_eq!(player.username, "TestUser");
        assert_eq!(player.kills, None);
        assert_eq!(player.deaths, None);
        assert_eq!(player.score, None);
        assert_eq!(player.kd, None);
        assert_eq!(player.time_played, None);
        assert_eq!(player.rank_name, None);
        assert_eq!(player.rank_icon, None);
    }

    #[test]
    fn short_rows_do_not_panic_and_decode_entities() {
        let html = r#"<html><body><table>
            <tr><td>3</td><td>A&amp;B</td><td>10</td></tr>
        </table></body></html>"#;

        let result = parse_player_list(html, PlayerDatabase::PreresetInvasion);
        assert_eq!(result.players.len(), 1);
        assert_eq!(result.players[0].username, "A&B");
        assert_eq!(result.players[0].row_number, 3);
        assert_eq!(result.players[0].kills, Some(10));
        assert_eq!(result.players[0].score, None);
        assert_eq!(result.players[0].rank_icon, None);
    }

    // --- Query validation ------------------------------------------------

    #[test]
    fn server_query_applies_defaults() {
        let mut query = ServerQuery::default();
        query.normalize().expect("defaults are valid");

        assert_eq!(query.limit, Some(20));
        assert_eq!(query.offset, Some(0));
        assert_eq!(query.limit(), 20);
        assert_eq!(query.offset(), 0);
    }

    #[test]
    fn server_query_rejects_out_of_range_limit() {
        for limit in [0, -1, 101] {
            let mut query = ServerQuery {
                limit: Some(limit),
                ..ServerQuery::default()
            };
            match query.normalize() {
                Err(DataError::InvalidQuery(message)) => {
                    assert!(message.contains("limit"), "unexpected message: {message}");
                }
                other => panic!("expected invalid query for limit {limit}, got {other:?}"),
            }
        }

        for limit in [1, 100] {
            let mut query = ServerQuery {
                limit: Some(limit),
                ..ServerQuery::default()
            };
            assert!(
                query.normalize().is_ok(),
                "limit {limit} should be accepted"
            );
        }
    }

    #[test]
    fn server_query_rejects_negative_offset_but_allows_large_ones() {
        let mut query = ServerQuery {
            offset: Some(-1),
            ..ServerQuery::default()
        };
        match query.normalize() {
            Err(DataError::InvalidQuery(message)) => {
                assert!(message.contains("offset"), "unexpected message: {message}");
            }
            other => panic!("expected invalid query, got {other:?}"),
        }

        let mut query = ServerQuery {
            offset: Some(1_000_000),
            ..ServerQuery::default()
        };
        query.normalize().expect("large offsets are allowed");
        assert_eq!(query.offset(), 1_000_000);
    }

    #[test]
    fn server_query_deserializes_before_validating() {
        let mut parsed: ServerQuery =
            serde_json::from_str(r#"{"offset":-5,"online_only":true}"#).expect("deserializes");
        assert_eq!(parsed.offset, Some(-5));
        assert!(matches!(
            parsed.normalize(),
            Err(DataError::InvalidQuery(_))
        ));

        // Non-numeric pagination values fail at deserialization, before normalize.
        assert!(serde_json::from_str::<ServerQuery>(r#"{"offset":"abc"}"#).is_err());
        assert!(serde_json::from_str::<ServerQuery>(r#"{"limit":"abc"}"#).is_err());
    }

    #[test]
    fn empty_filters_are_treated_as_absent() {
        let mut query = ServerQuery {
            search: Some("   ".to_string()),
            map: Some(String::new()),
            mode: Some(String::new()),
            country: Some(String::new()),
            ..ServerQuery::default()
        };
        query.normalize().expect("empty filters are valid");

        assert_eq!(query.search, None);
        assert_eq!(query.map, None);
        assert_eq!(query.mode, None);
        assert_eq!(query.country, None);
    }

    #[test]
    fn player_query_applies_defaults() {
        let mut query = PlayerQuery::default();
        query.normalize().expect("defaults are valid");

        assert_eq!(query.db.as_deref(), Some("invasion"));
        assert_eq!(query.database(), PlayerDatabase::Invasion);
        assert_eq!(query.sort.as_deref(), Some("rank_progression"));
        assert_eq!(query.limit, Some(20));
        assert_eq!(query.offset, Some(0));
    }

    #[test]
    fn player_query_accepts_every_frontend_sort_field() {
        // Hard-coded copy of the frontend contract on purpose: using the
        // implementation constant as its own expectation would not catch drift.
        const EXPECTED_SORT_FIELDS: [&str; 16] = [
            "rank_progression",
            "username",
            "kills",
            "deaths",
            "kd",
            "score",
            "time_played",
            "teamkills",
            "longest_kill_streak",
            "targets_destroyed",
            "vehicles_destroyed",
            "soldiers_healed",
            "distance_moved",
            "shots_fired",
            "throwables_thrown",
            "sid",
        ];
        assert_eq!(PLAYER_SORT_FIELDS, EXPECTED_SORT_FIELDS);

        for sort in EXPECTED_SORT_FIELDS {
            let mut query = PlayerQuery {
                sort: Some(sort.to_string()),
                ..PlayerQuery::default()
            };
            assert!(query.normalize().is_ok(), "sort {sort} should be accepted");
            assert_eq!(query.sort.as_deref(), Some(sort));
        }
    }

    #[test]
    fn player_query_rejects_unknown_sort() {
        for sort in ["map_name", "Kills", "RANK_PROGRESSION"] {
            let mut query = PlayerQuery {
                sort: Some(sort.to_string()),
                ..PlayerQuery::default()
            };

            match query.normalize() {
                Err(DataError::InvalidQuery(message)) => {
                    assert!(message.contains("sort"), "unexpected message: {message}");
                }
                other => panic!("expected invalid query for sort {sort}, got {other:?}"),
            }
        }
    }

    #[test]
    fn player_query_validates_db_strictly() {
        for db in ["bogus", "Pacific", "INVASION"] {
            let mut query = PlayerQuery {
                db: Some(db.to_string()),
                ..PlayerQuery::default()
            };
            assert!(
                matches!(query.normalize(), Err(DataError::InvalidQuery(_))),
                "db {db} should be rejected"
            );
        }

        let mut query = PlayerQuery {
            db: Some("pacific".to_string()),
            ..PlayerQuery::default()
        };
        query.normalize().expect("valid db");
        assert_eq!(query.database(), PlayerDatabase::Pacific);

        // An empty value is treated as absent and falls back to the default.
        let mut query = PlayerQuery {
            db: Some("  ".to_string()),
            ..PlayerQuery::default()
        };
        query.normalize().expect("empty db falls back");
        assert_eq!(query.database(), PlayerDatabase::Invasion);
    }

    #[test]
    fn player_database_round_trips_to_upstream_values() {
        assert_eq!(
            "prereset_invasion"
                .parse::<PlayerDatabase>()
                .expect("known db"),
            PlayerDatabase::PreresetInvasion
        );
        assert_eq!(
            serde_json::to_value(PlayerDatabase::PreresetInvasion).unwrap(),
            "prereset_invasion"
        );
        assert_eq!(
            serde_json::to_value(PlayerDatabase::Pacific).unwrap(),
            "pacific"
        );
    }

    #[test]
    fn player_query_deserializes_from_json() {
        let parsed: PlayerQuery =
            serde_json::from_str(r#"{"db":"pacific","sort":"kills","limit":5}"#)
                .expect("valid query");
        assert_eq!(parsed.db.as_deref(), Some("pacific"));
        assert_eq!(parsed.sort.as_deref(), Some("kills"));
        assert_eq!(parsed.limit, Some(5));
        assert_eq!(parsed.offset, None);

        let mut unknown = PlayerQuery {
            db: Some("bogus".to_string()),
            ..PlayerQuery::default()
        };
        assert!(matches!(
            unknown.normalize(),
            Err(DataError::InvalidQuery(_))
        ));
    }

    #[test]
    fn server_query_rejects_invalid_bool_on_deserialize() {
        assert!(serde_json::from_str::<ServerQuery>(r#"{"online_only":"yes"}"#).is_err());

        let parsed: ServerQuery =
            serde_json::from_str(r#"{"online_only":true,"map":"desert"}"#).expect("valid query");
        assert_eq!(parsed.online_only, Some(true));
        assert_eq!(parsed.map.as_deref(), Some("desert"));

        // The camelCase alias is accepted so MCP/JSON callers do not silently
        // lose the filter; the REST query-string contract remains snake_case.
        let aliased: ServerQuery =
            serde_json::from_str(r#"{"onlineOnly":true}"#).expect("alias is valid");
        assert_eq!(aliased.online_only, Some(true));
    }

    #[test]
    fn deserialization_errors_map_to_invalid_query() {
        let error = serde_json::from_str::<ServerQuery>(r#"{"limit":"abc"}"#)
            .expect_err("non-numeric limit must fail deserialization");
        assert!(matches!(
            DataError::from_query_error(error),
            DataError::InvalidQuery(_)
        ));
    }

    // --- Upstream HTTP ---------------------------------------------------

    #[tokio::test]
    async fn fetch_servers_sends_expected_query_params() {
        let mock_server = MockServer::start().await;
        let body = servers_response(&[("10.0.0.1".to_string(), 1111)]);
        mock_endpoint(
            &mock_server,
            "/server_list.php",
            &[("start", "0"), ("size", "100"), ("names", "1")],
            body,
            1,
        )
        .await;

        let servers = client_for(&mock_server)
            .fetch_servers()
            .await
            .expect("fetch succeeds");

        assert_eq!(servers.len(), 1);
        assert_eq!(servers[0].id, "10.0.0.1:1111");
    }

    #[tokio::test]
    async fn fetch_servers_paginates_and_deduplicates() {
        let mock_server = MockServer::start().await;

        let first_page: Vec<(String, u16)> = (0..100)
            .map(|index| (format!("10.0.0.{}", index % 250), 1000 + index))
            .collect();
        mock_endpoint(
            &mock_server,
            "/server_list.php",
            &[("start", "0")],
            servers_response(&first_page),
            1,
        )
        .await;

        // Page two repeats the first id and adds two new ones; it is short, so
        // pagination stops here.
        let second_page = vec![
            ("10.0.0.0".to_string(), 1000),
            ("10.0.1.1".to_string(), 2000),
            ("10.0.1.2".to_string(), 3000),
        ];
        mock_endpoint(
            &mock_server,
            "/server_list.php",
            &[("start", "100")],
            servers_response(&second_page),
            1,
        )
        .await;

        let servers = client_for(&mock_server)
            .fetch_servers()
            .await
            .expect("fetch succeeds");

        assert_eq!(servers.len(), 102, "duplicate address:port must be dropped");
        let mut ids: Vec<&str> = servers.iter().map(|server| server.id.as_str()).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), 102);
    }

    #[tokio::test]
    async fn fetch_servers_keeps_the_first_duplicate() {
        let mock_server = MockServer::start().await;
        let body = r#"<result>
            <server><name>First</name><address>10.0.0.1</address><port>1000</port></server>
            <server><name>Second</name><address>10.0.0.1</address><port>1000</port></server>
        </result>"#;
        mock_endpoint(
            &mock_server,
            "/server_list.php",
            &[("start", "0")],
            body.to_string(),
            1,
        )
        .await;

        let servers = client_for(&mock_server)
            .fetch_servers()
            .await
            .expect("fetch succeeds");

        assert_eq!(servers.len(), 1);
        assert_eq!(servers[0].name, "First");
    }

    #[tokio::test]
    async fn fetch_servers_stops_after_max_pages() {
        let mock_server = MockServer::start().await;
        let page: Vec<(String, u16)> = (0..100)
            .map(|index| (format!("10.9.0.{}", index % 200), 3000 + index))
            .collect();

        // Always return a full page: without the page cap this would loop
        // forever; `expect(10)` proves the cap is enforced.
        Mock::given(method("GET"))
            .and(path("/server_list.php"))
            .respond_with(ResponseTemplate::new(200).set_body_string(servers_response(&page)))
            .expect(10)
            .mount(&mock_server)
            .await;

        let servers = client_for(&mock_server)
            .fetch_servers()
            .await
            .expect("fetch succeeds");

        assert_eq!(servers.len(), 100, "identical pages are deduplicated");
    }

    #[tokio::test]
    async fn fetch_servers_maps_invalid_url_to_request_error() {
        let client =
            UpstreamClient::with_urls(Arc::new(ApiCache::new(60)), "not a url", "not a url");

        let error = client.fetch_servers().await.expect_err("invalid URL");
        assert!(matches!(error, UpstreamError::Request(_)));
    }

    #[tokio::test]
    async fn fetch_servers_maps_non_success_status_to_upstream_error() {
        let mock_server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/server_list.php"))
            .respond_with(ResponseTemplate::new(503))
            .mount(&mock_server)
            .await;

        let error = client_for(&mock_server)
            .fetch_servers()
            .await
            .expect_err("503 must fail");

        assert!(matches!(error, UpstreamError::Status { status: 503, .. }));
    }

    #[tokio::test]
    async fn fetch_servers_maps_malformed_xml_to_upstream_error() {
        let mock_server = MockServer::start().await;
        mock_endpoint(
            &mock_server,
            "/server_list.php",
            &[("start", "0")],
            "<result><server>".to_string(),
            1,
        )
        .await;

        let error = client_for(&mock_server)
            .fetch_servers()
            .await
            .expect_err("malformed XML must fail");

        assert!(matches!(error, UpstreamError::Parse(_)));
    }

    #[tokio::test]
    async fn fetch_servers_reuses_cache() {
        let mock_server = MockServer::start().await;
        let body = servers_response(&[("10.0.0.1".to_string(), 1111)]);
        mock_endpoint(&mock_server, "/server_list.php", &[("start", "0")], body, 1).await;

        let client = client_for(&mock_server);
        let first = client.fetch_servers().await.expect("first fetch");
        let second = client.fetch_servers().await.expect("cached fetch");

        assert_eq!(first, second);
    }

    #[tokio::test]
    async fn fetch_players_sends_query_params_and_parses_pagination() {
        let mock_server = MockServer::start().await;
        mock_endpoint(
            &mock_server,
            "/view_players.php",
            &[
                ("db", "pacific"),
                ("start", "2"),
                ("size", "5"),
                ("sort", "kills"),
                ("search", "Bob"),
            ],
            PLAYER_HTML.to_string(),
            1,
        )
        .await;

        let mut query = PlayerQuery {
            search: Some("Bob".to_string()),
            db: Some("pacific".to_string()),
            sort: Some("kills".to_string()),
            offset: Some(2),
            limit: Some(5),
        };
        query.normalize().expect("valid query");

        let result = client_for(&mock_server)
            .fetch_players(&query)
            .await
            .expect("fetch succeeds");

        assert_eq!(result.players.len(), 2);
        assert_eq!(result.players[0].db, PlayerDatabase::Pacific);
        assert!(result.has_next);
        assert!(result.has_previous);
    }

    #[tokio::test]
    async fn fetch_players_omits_empty_search() {
        let mock_server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/view_players.php"))
            .and(query_param("db", "invasion"))
            .and(query_param("start", "0"))
            .and(query_param("size", "20"))
            .and(query_param("sort", "rank_progression"))
            .and(query_param_is_missing("search"))
            .and(query_param_is_missing("_t"))
            .respond_with(ResponseTemplate::new(200).set_body_string(PLAYER_HTML))
            .expect(1)
            .mount(&mock_server)
            .await;

        let mut query = PlayerQuery::default();
        query.normalize().expect("valid query");

        let result = client_for(&mock_server)
            .fetch_players(&query)
            .await
            .expect("fetch succeeds");

        assert_eq!(result.players.len(), 2);
    }

    // --- Service facade --------------------------------------------------

    #[tokio::test]
    async fn service_filters_servers_before_paginating() {
        let mock_server = MockServer::start().await;
        mock_endpoint(
            &mock_server,
            "/server_list.php",
            &[("start", "0")],
            FILTER_XML.to_string(),
            1,
        )
        .await;
        let service = RwrsService::with_client(client_for(&mock_server));

        let all = service
            .list_servers(&ServerQuery::default())
            .await
            .expect("list succeeds");
        assert_eq!(all.total, 3);
        assert_eq!(all.servers.len(), 3);
        assert_eq!(all.offset, 0);
        assert_eq!(all.limit, 20);

        let map_page = service
            .list_servers(&ServerQuery {
                map: Some("desert".to_string()),
                offset: Some(1),
                limit: Some(1),
                ..ServerQuery::default()
            })
            .await
            .expect("list succeeds");
        assert_eq!(map_page.total, 2);
        assert_eq!(map_page.servers.len(), 1);
        assert_eq!(map_page.servers[0].id, "127.0.0.3:9000");

        let online = service
            .list_servers(&ServerQuery {
                online_only: Some(true),
                ..ServerQuery::default()
            })
            .await
            .expect("list succeeds");
        assert_eq!(online.total, 2);

        let mode = service
            .list_servers(&ServerQuery {
                mode: Some("TEST mode".to_string()),
                ..ServerQuery::default()
            })
            .await
            .expect("list succeeds");
        assert_eq!(mode.total, 2);

        let country = service
            .list_servers(&ServerQuery {
                country: Some("uk".to_string()),
                ..ServerQuery::default()
            })
            .await
            .expect("list succeeds");
        assert_eq!(country.total, 1);
        assert_eq!(country.servers[0].id, "127.0.0.2:5678");
    }

    #[tokio::test]
    async fn service_keeps_upstream_order_and_allows_large_offsets() {
        let mock_server = MockServer::start().await;
        mock_endpoint(
            &mock_server,
            "/server_list.php",
            &[("start", "0")],
            FILTER_XML.to_string(),
            1,
        )
        .await;
        let service = RwrsService::with_client(client_for(&mock_server));

        let all = service
            .list_servers(&ServerQuery::default())
            .await
            .expect("list succeeds");
        let ids: Vec<&str> = all
            .servers
            .iter()
            .map(|server| server.id.as_str())
            .collect();
        assert_eq!(ids, ["127.0.0.1:1234", "127.0.0.2:5678", "127.0.0.3:9000"]);

        let beyond = service
            .list_servers(&ServerQuery {
                offset: Some(1_000_000),
                ..ServerQuery::default()
            })
            .await
            .expect("list succeeds");
        assert_eq!(beyond.total, 3);
        assert!(beyond.servers.is_empty());

        // Filters combine with AND.
        let combined = service
            .list_servers(&ServerQuery {
                online_only: Some(true),
                country: Some("us".to_string()),
                mode: Some("test".to_string()),
                ..ServerQuery::default()
            })
            .await
            .expect("list succeeds");
        assert_eq!(combined.total, 2);
    }

    #[tokio::test]
    async fn service_search_matches_frontend_fields() {
        let mock_server = MockServer::start().await;
        mock_endpoint(
            &mock_server,
            "/server_list.php",
            &[("start", "0")],
            FILTER_XML.to_string(),
            1,
        )
        .await;
        let service = RwrsService::with_client(client_for(&mock_server));

        for (search, expected_id) in [
            ("player2", "127.0.0.1:1234"),
            ("ALPHA", "127.0.0.1:1234"),
            ("9000", "127.0.0.3:9000"),
            ("killhouse", "127.0.0.2:5678"),
            ("friendly", "127.0.0.1:1234"),
            ("127.0.0.2", "127.0.0.2:5678"),
        ] {
            let response = service
                .list_servers(&ServerQuery {
                    search: Some(search.to_string()),
                    ..ServerQuery::default()
                })
                .await
                .expect("list succeeds");
            assert_eq!(response.total, 1, "search {search}");
            assert_eq!(response.servers[0].id, expected_id, "search {search}");
        }
    }

    #[tokio::test]
    async fn service_get_server_looks_up_address_and_port() {
        let mock_server = MockServer::start().await;
        mock_endpoint(
            &mock_server,
            "/server_list.php",
            &[("start", "0")],
            FILTER_XML.to_string(),
            1,
        )
        .await;
        let service = RwrsService::with_client(client_for(&mock_server));

        let found = service
            .get_server("127.0.0.2", 5678)
            .await
            .expect("lookup succeeds");
        assert_eq!(
            found.map(|server| server.name),
            Some("Beta Server".to_string())
        );

        let missing = service
            .get_server("127.0.0.2", 9999)
            .await
            .expect("lookup succeeds");
        assert!(missing.is_none());
    }

    #[tokio::test]
    async fn service_search_players_builds_response() {
        let mock_server = MockServer::start().await;
        mock_endpoint(
            &mock_server,
            "/view_players.php",
            &[
                ("db", "pacific"),
                ("start", "0"),
                ("size", "2"),
                ("sort", "kills"),
            ],
            PLAYER_HTML.to_string(),
            1,
        )
        .await;
        let service = RwrsService::with_client(client_for(&mock_server));

        let response: PlayerListResponse = service
            .search_players(&PlayerQuery {
                db: Some("pacific".to_string()),
                sort: Some("kills".to_string()),
                limit: Some(2),
                ..PlayerQuery::default()
            })
            .await
            .expect("search succeeds");

        assert_eq!(response.db, PlayerDatabase::Pacific);
        assert_eq!(response.offset, 0);
        assert_eq!(response.limit, 2);
        assert!(response.has_next);
        assert!(response.has_previous);
        assert_eq!(response.players.len(), 2);
        assert_eq!(response.players[0].id, "pacific:PlayerOne");
    }

    #[tokio::test]
    async fn service_rejects_invalid_query_without_calling_upstream() {
        let mock_server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_string("unused"))
            .expect(0)
            .mount(&mock_server)
            .await;
        let service = RwrsService::with_client(client_for(&mock_server));

        let error = service
            .search_players(&PlayerQuery {
                limit: Some(0),
                ..PlayerQuery::default()
            })
            .await
            .expect_err("limit 0 must fail");
        assert!(matches!(error, DataError::InvalidQuery(_)));

        let error = service
            .search_players(&PlayerQuery {
                sort: Some("not_a_field".to_string()),
                ..PlayerQuery::default()
            })
            .await
            .expect_err("unknown sort must fail");
        assert!(matches!(error, DataError::InvalidQuery(_)));

        let error = service
            .search_players(&PlayerQuery {
                db: Some("Pacific".to_string()),
                ..PlayerQuery::default()
            })
            .await
            .expect_err("mixed-case db must fail");
        assert!(matches!(error, DataError::InvalidQuery(_)));

        let error = service
            .search_players(&PlayerQuery {
                offset: Some(-1),
                ..PlayerQuery::default()
            })
            .await
            .expect_err("negative offset must fail");
        assert!(matches!(error, DataError::InvalidQuery(_)));

        let error = service
            .list_servers(&ServerQuery {
                limit: Some(101),
                ..ServerQuery::default()
            })
            .await
            .expect_err("limit 101 must fail");
        assert!(matches!(error, DataError::InvalidQuery(_)));

        let error = service
            .list_servers(&ServerQuery {
                offset: Some(-3),
                ..ServerQuery::default()
            })
            .await
            .expect_err("negative offset must fail");
        assert!(matches!(error, DataError::InvalidQuery(_)));
    }

    #[tokio::test]
    async fn service_surfaces_upstream_failure() {
        let mock_server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/server_list.php"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&mock_server)
            .await;
        let service = RwrsService::with_client(client_for(&mock_server));

        let error = service
            .list_servers(&ServerQuery::default())
            .await
            .expect_err("500 must fail");
        assert!(matches!(
            error,
            DataError::Upstream(UpstreamError::Status { status: 500, .. })
        ));
    }

    #[test]
    fn server_list_response_serializes_with_pagination_keys() {
        let response = ServerListResponse {
            total: 0,
            offset: 5,
            limit: 20,
            servers: Vec::new(),
        };
        let value = serde_json::to_value(&response).expect("serializes");
        for key in ["total", "offset", "limit", "servers"] {
            assert!(value.get(key).is_some(), "missing key {key}");
        }
    }

    #[test]
    fn player_list_response_serializes_with_pagination_keys() {
        let response = PlayerListResponse {
            db: PlayerDatabase::PreresetInvasion,
            offset: 10,
            limit: 20,
            has_next: true,
            has_previous: false,
            players: Vec::new(),
        };
        let value = serde_json::to_value(&response).expect("serializes");

        assert_eq!(value["db"], "prereset_invasion");
        assert_eq!(value["hasNext"], true);
        assert_eq!(value["hasPrevious"], false);
        assert!(value.get("has_next").is_none());
        assert!(value.get("has_previous").is_none());
    }
}
