//! Normalized data-transfer objects shared by the JSON REST API and MCP tools.
//!
//! Field names follow the frontend models (`robin-web/src/lib/models`) so the
//! browser can consume responses without an adapter layer.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

/// Default page size for list queries.
pub const DEFAULT_LIMIT: usize = 20;
/// Maximum accepted page size.
pub const MAX_LIMIT: usize = 100;

/// Sort fields accepted by the upstream player list endpoint.
///
/// Mirrors `PLAYER_SORT_FIELDS` from `robin-web/src/lib/models/player.model.ts`.
/// An unknown value makes upstream answer with an empty table, so every sort
/// value that reaches the API must be validated against this list.
pub const PLAYER_SORT_FIELDS: [&str; 16] = [
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

/// Default player sort field.
pub const DEFAULT_PLAYER_SORT: &str = "rank_progression";

/// One server from the upstream server list, normalized for JSON output.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerItem {
    pub id: String,
    pub name: String,
    pub ip_address: String,
    pub port: u16,
    pub map_id: String,
    pub map_name: String,
    pub bots: i64,
    pub country: String,
    pub current_players: i64,
    pub time_stamp: i64,
    pub version: String,
    pub dedicated: bool,
    #[serde(rename = "mod")]
    pub mod_flag: bool,
    pub player_list: Vec<String>,
    pub comment: String,
    pub url: String,
    pub max_players: i64,
    pub mode: String,
    pub realm: Option<String>,
}

/// Upstream player database (leaderboard) selection.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlayerDatabase {
    #[default]
    Invasion,
    Pacific,
    PreresetInvasion,
}

impl PlayerDatabase {
    /// Upstream `db` query value and the prefix of a player id.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Invasion => "invasion",
            Self::Pacific => "pacific",
            Self::PreresetInvasion => "prereset_invasion",
        }
    }
}

impl fmt::Display for PlayerDatabase {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for PlayerDatabase {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "invasion" => Ok(Self::Invasion),
            "pacific" => Ok(Self::Pacific),
            "prereset_invasion" => Ok(Self::PreresetInvasion),
            other => Err(format!(
                "unsupported db '{other}' (expected one of: invasion, pacific, prereset_invasion)"
            )),
        }
    }
}

/// One player row from the upstream leaderboard, normalized for JSON output.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayerItem {
    pub id: String,
    pub username: String,
    pub db: PlayerDatabase,
    pub row_number: i64,
    pub rank_progression: Option<i64>,
    pub kills: Option<i64>,
    pub deaths: Option<i64>,
    pub score: Option<i64>,
    pub teamkills: Option<i64>,
    pub longest_kill_streak: Option<i64>,
    pub targets_destroyed: Option<i64>,
    pub vehicles_destroyed: Option<i64>,
    pub soldiers_healed: Option<i64>,
    pub shots_fired: Option<i64>,
    pub throwables_thrown: Option<i64>,
    pub kd: Option<f64>,
    pub time_played: Option<String>,
    pub distance_moved: Option<String>,
    pub rank_name: Option<String>,
    pub rank_icon: Option<String>,
}

/// Raw server list query parameters, as received from the HTTP layer.
///
/// Call [`ServerQuery::normalize`] before using the values; it fills defaults
/// and rejects out-of-range pagination. `offset` deliberately stays signed so
/// negative values produce a query error instead of wrapping.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ServerQuery {
    pub search: Option<String>,
    #[serde(alias = "onlineOnly")]
    pub online_only: Option<bool>,
    pub map: Option<String>,
    pub mode: Option<String>,
    pub country: Option<String>,
    pub offset: Option<i64>,
    pub limit: Option<i64>,
}

impl ServerQuery {
    /// Fill defaults and validate the page size and offset.
    ///
    /// After a successful call, `offset` and `limit` are always `Some` and
    /// non-negative.
    pub fn normalize(&mut self) -> Result<(), DataError> {
        let limit = validate_limit(self.limit)?;
        let offset = validate_offset(self.offset)?;
        self.limit = Some(limit as i64);
        self.offset = Some(offset as i64);
        self.search = trim_filter(self.search.take());
        self.map = trim_filter(self.map.take());
        self.mode = trim_filter(self.mode.take());
        self.country = trim_filter(self.country.take());
        Ok(())
    }

    /// Page size after [`ServerQuery::normalize`], defaulting to [`DEFAULT_LIMIT`].
    pub fn limit(&self) -> usize {
        self.limit.unwrap_or(DEFAULT_LIMIT as i64).max(0) as usize
    }

    /// Offset after [`ServerQuery::normalize`], defaulting to zero.
    pub fn offset(&self) -> usize {
        self.offset.unwrap_or(0).max(0) as usize
    }
}

/// Raw player list query parameters, as received from the HTTP layer.
///
/// Call [`PlayerQuery::normalize`] before using the values; it fills defaults
/// and validates the database and sort field. `db` stays a string so unknown
/// or mixed-case values can be reported as query errors.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PlayerQuery {
    pub search: Option<String>,
    pub db: Option<String>,
    pub sort: Option<String>,
    pub offset: Option<i64>,
    pub limit: Option<i64>,
}

impl PlayerQuery {
    /// Fill defaults and validate the page size, offset, database and sort field.
    ///
    /// After a successful call, `db`, `sort`, `offset` and `limit` are `Some`.
    pub fn normalize(&mut self) -> Result<(), DataError> {
        let limit = validate_limit(self.limit)?;
        let offset = validate_offset(self.offset)?;
        self.limit = Some(limit as i64);
        self.offset = Some(offset as i64);
        self.db = Some(validate_db(self.db.take())?);
        self.sort = Some(validate_sort(self.sort.take())?);
        self.search = trim_filter(self.search.take());
        Ok(())
    }

    /// Page size after [`PlayerQuery::normalize`], defaulting to [`DEFAULT_LIMIT`].
    pub fn limit(&self) -> usize {
        self.limit.unwrap_or(DEFAULT_LIMIT as i64).max(0) as usize
    }

    /// Offset after [`PlayerQuery::normalize`], defaulting to zero.
    pub fn offset(&self) -> usize {
        self.offset.unwrap_or(0).max(0) as usize
    }

    /// Leaderboard after [`PlayerQuery::normalize`], defaulting to invasion.
    pub fn database(&self) -> PlayerDatabase {
        self.db
            .as_deref()
            .and_then(|value| value.parse().ok())
            .unwrap_or_default()
    }
}

/// Paginated server list.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerListResponse {
    pub total: usize,
    pub offset: usize,
    pub limit: usize,
    pub servers: Vec<ServerItem>,
}

/// Paginated player leaderboard.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayerListResponse {
    pub db: PlayerDatabase,
    pub offset: usize,
    pub limit: usize,
    pub has_next: bool,
    pub has_previous: bool,
    pub players: Vec<PlayerItem>,
}

/// Failure while answering a data query.
#[derive(Debug)]
pub enum DataError {
    /// The caller supplied an invalid query parameter; upstream was not contacted.
    InvalidQuery(String),
    /// The upstream endpoint failed or returned unusable data.
    Upstream(UpstreamError),
}

impl fmt::Display for DataError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidQuery(message) => write!(f, "invalid query: {message}"),
            Self::Upstream(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for DataError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::InvalidQuery(_) => None,
            Self::Upstream(error) => Some(error),
        }
    }
}

impl DataError {
    /// Convert a query deserialization failure into an invalid-query error.
    ///
    /// Non-numeric pagination values are rejected while parsing the HTTP query
    /// string, before [`ServerQuery::normalize`]/[`PlayerQuery::normalize`] can
    /// see them; the REST layer uses this to keep the contract that bad input is
    /// a query error rather than an upstream error.
    pub fn from_query_error(error: impl fmt::Display) -> Self {
        Self::InvalidQuery(error.to_string())
    }
}

impl From<UpstreamError> for DataError {
    fn from(error: UpstreamError) -> Self {
        Self::Upstream(error)
    }
}

/// Failure while talking to a Running with Rifles upstream endpoint.
#[derive(Debug)]
pub enum UpstreamError {
    /// Connection failure or invalid URL.
    Request(String),
    /// Non-2xx response.
    Status { url: String, status: u16 },
    /// The response body could not be parsed.
    Parse(String),
}

impl fmt::Display for UpstreamError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Request(message) => write!(f, "upstream request failed: {message}"),
            Self::Status { url, status } => {
                write!(f, "upstream returned HTTP {status} for {url}")
            }
            Self::Parse(message) => {
                write!(f, "failed to parse upstream response: {message}")
            }
        }
    }
}

impl std::error::Error for UpstreamError {}

fn validate_limit(limit: Option<i64>) -> Result<usize, DataError> {
    let limit = limit.unwrap_or(DEFAULT_LIMIT as i64);
    if (1..=MAX_LIMIT as i64).contains(&limit) {
        Ok(limit as usize)
    } else {
        Err(DataError::InvalidQuery(format!(
            "limit must be between 1 and {MAX_LIMIT}, got {limit}"
        )))
    }
}

fn validate_offset(offset: Option<i64>) -> Result<usize, DataError> {
    let offset = offset.unwrap_or(0);
    if offset < 0 {
        Err(DataError::InvalidQuery(format!(
            "offset must be zero or greater, got {offset}"
        )))
    } else {
        Ok(offset as usize)
    }
}

fn validate_db(db: Option<String>) -> Result<String, DataError> {
    let db = trim_filter(db).unwrap_or_else(|| PlayerDatabase::default().as_str().to_string());
    match db.parse::<PlayerDatabase>() {
        Ok(database) => Ok(database.as_str().to_string()),
        Err(message) => Err(DataError::InvalidQuery(message)),
    }
}

fn validate_sort(sort: Option<String>) -> Result<String, DataError> {
    let sort = trim_filter(sort).unwrap_or_else(|| DEFAULT_PLAYER_SORT.to_string());
    if PLAYER_SORT_FIELDS.contains(&sort.as_str()) {
        Ok(sort)
    } else {
        Err(DataError::InvalidQuery(format!(
            "unsupported sort '{sort}' (expected one of: {})",
            PLAYER_SORT_FIELDS.join(", ")
        )))
    }
}

fn trim_filter(filter: Option<String>) -> Option<String> {
    filter
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}
