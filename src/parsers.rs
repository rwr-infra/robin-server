//! Parsers for the untrusted upstream formats:
//! - server list: XML (`get_server_list.php`)
//! - player leaderboard: HTML table (`view_players.php`)
//!
//! Both parsers tolerate missing fields and malformed rows instead of panicking;
//! the upstream is an old PHP page that changes shape without notice.

use crate::domain::{PlayerDatabase, PlayerItem, ServerItem, UpstreamError};
use roxmltree::{Document, Node};
use scraper::{ElementRef, Html, Selector};
use std::fmt;

/// A server list document could not be parsed at all.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    message: String,
}

impl ParseError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    /// Human readable description of the parse failure.
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ParseError {}

impl From<ParseError> for UpstreamError {
    fn from(error: ParseError) -> Self {
        Self::Parse(error.message)
    }
}

/// Player rows plus the pagination state advertised by the upstream page.
#[derive(Debug, Clone, PartialEq)]
pub struct PlayerListResult {
    pub players: Vec<PlayerItem>,
    pub has_next: bool,
    pub has_previous: bool,
}

/// Parse the upstream server list XML into normalized items.
///
/// Accepts both documented shapes, `<result><server>` and
/// `<result><server_list><server>`, with a single or repeated `<server>` and
/// `<player>` element. Missing fields fall back to empty strings / zeroes.
pub fn parse_server_list(xml: &str) -> Result<Vec<ServerItem>, ParseError> {
    let document =
        Document::parse(xml).map_err(|error| ParseError::new(format!("invalid XML: {error}")))?;

    let root = document.root_element();
    let container = if root.has_tag_name("server_list") {
        root
    } else {
        root.children()
            .find(|node| node.has_tag_name("server_list"))
            .unwrap_or(root)
    };

    let mut server_nodes: Vec<Node<'_, '_>> = container
        .children()
        .filter(|node| node.has_tag_name("server"))
        .collect();

    if server_nodes.is_empty() {
        // Unwrap unexpected wrappers rather than silently returning nothing.
        server_nodes = document
            .descendants()
            .filter(|node| node.has_tag_name("server"))
            .collect();
    }

    Ok(server_nodes.into_iter().map(server_from_node).collect())
}

fn server_from_node(node: Node<'_, '_>) -> ServerItem {
    let ip_address = child_text(node, "address").unwrap_or_default();
    let port = child_text(node, "port")
        .and_then(|value| value.parse::<u16>().ok())
        .unwrap_or(0);

    let time_stamp = child_text(node, "timeStamp")
        .or_else(|| child_text(node, "timestamp"))
        .and_then(|value| value.parse::<i64>().ok())
        .unwrap_or(0);

    let player_list = node
        .children()
        .filter(|child| child.has_tag_name("player"))
        .filter_map(|child| child.text())
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_string)
        .collect();

    ServerItem {
        id: format!("{ip_address}:{port}"),
        name: child_text(node, "name").unwrap_or_default(),
        ip_address,
        port,
        map_id: child_text(node, "map_id").unwrap_or_default(),
        map_name: child_text(node, "map_name").unwrap_or_default(),
        bots: child_int(node, "bots"),
        country: child_text(node, "country").unwrap_or_default(),
        current_players: child_int(node, "current_players"),
        time_stamp,
        version: child_text(node, "version").unwrap_or_default(),
        dedicated: child_bool(node, "dedicated"),
        mod_flag: child_bool(node, "mod"),
        player_list,
        comment: child_text(node, "comment").unwrap_or_default(),
        url: child_text(node, "url").unwrap_or_default(),
        max_players: child_int(node, "max_players"),
        mode: child_text(node, "mode").unwrap_or_default(),
        realm: non_empty(child_text(node, "realm")),
    }
}

fn child_text(node: Node<'_, '_>, name: &str) -> Option<String> {
    node.children()
        .find(|child| child.has_tag_name(name))
        .and_then(|child| child.text())
        .map(str::trim)
        .map(str::to_string)
}

fn child_int(node: Node<'_, '_>, name: &str) -> i64 {
    child_text(node, name)
        .and_then(|value| value.parse::<i64>().ok())
        .unwrap_or(0)
}

fn child_bool(node: Node<'_, '_>, name: &str) -> bool {
    match child_text(node, name).as_deref() {
        Some("1") => true,
        Some(value) => value.eq_ignore_ascii_case("true"),
        None => false,
    }
}

fn non_empty(value: Option<String>) -> Option<String> {
    value.filter(|value| !value.is_empty())
}

/// Parse the upstream leaderboard HTML table into normalized player rows.
///
/// Header rows (`<th>`), rows without cells and missing tables all degrade to
/// an empty player list. Pagination links are detected by their `NEXT` /
/// `PREVIOUS` text.
pub fn parse_player_list(html: &str, db: PlayerDatabase) -> PlayerListResult {
    let document = Html::parse_document(html);
    let row_selector = Selector::parse("table tr").expect("static selector");
    let img_selector = Selector::parse("img").expect("static selector");

    let mut players = Vec::new();
    for row in document.select(&row_selector) {
        let mut cells = Vec::new();
        let mut has_header_cell = false;
        for child in row.child_elements() {
            match child.value().name() {
                "td" => cells.push(child),
                "th" => has_header_cell = true,
                _ => {}
            }
        }

        if has_header_cell || cells.len() < 2 {
            continue;
        }

        players.push(player_from_cells(&cells, db, &img_selector));
    }

    let (has_next, has_previous) = pagination_links(&document);

    PlayerListResult {
        players,
        has_next,
        has_previous,
    }
}

fn player_from_cells(
    cells: &[ElementRef<'_>],
    db: PlayerDatabase,
    img_selector: &Selector,
) -> PlayerItem {
    let cell_text = |index: usize| -> String {
        cells
            .get(index)
            .map(|cell| cell.text().collect::<String>())
            .unwrap_or_default()
            .trim()
            .to_string()
    };
    let cell_option = |index: usize| -> Option<String> {
        let text = cell_text(index);
        if text.is_empty() { None } else { Some(text) }
    };
    let cell_int = |index: usize| -> Option<i64> { parse_i64(&cell_text(index)) };

    let username = cell_text(1);
    let rank_icon = cells
        .get(17)
        .and_then(|cell| cell.select(img_selector).next())
        .and_then(|img| img.value().attr("src"))
        .map(str::to_string);

    PlayerItem {
        id: format!("{}:{}", db.as_str(), username),
        username,
        db,
        row_number: cell_int(0).unwrap_or(0),
        rank_progression: cell_int(15),
        kills: cell_int(2),
        deaths: cell_int(3),
        score: cell_int(4),
        teamkills: cell_int(11),
        longest_kill_streak: cell_int(7),
        targets_destroyed: cell_int(8),
        vehicles_destroyed: cell_int(9),
        soldiers_healed: cell_int(10),
        shots_fired: cell_int(13),
        throwables_thrown: cell_int(14),
        kd: parse_f64(&cell_text(5)),
        time_played: cell_option(6),
        distance_moved: cell_option(12),
        rank_name: cell_option(16),
        rank_icon,
    }
}

fn parse_i64(text: &str) -> Option<i64> {
    let text = text.trim();
    if text.is_empty() || text == "-" {
        return None;
    }
    text.parse().ok()
}

fn parse_f64(text: &str) -> Option<f64> {
    let text = text.trim();
    if text.is_empty() || text == "-" {
        return None;
    }
    text.parse().ok()
}

fn pagination_links(document: &Html) -> (bool, bool) {
    let link_selector = Selector::parse("a").expect("static selector");
    let mut has_next = false;
    let mut has_previous = false;

    for link in document.select(&link_selector) {
        let text = link.text().collect::<String>();
        let text = text.trim();
        if text.eq_ignore_ascii_case("next") {
            has_next = true;
        } else if text.eq_ignore_ascii_case("previous") {
            has_previous = true;
        }
    }

    (has_next, has_previous)
}
