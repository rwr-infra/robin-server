# RWRS Server

Backend server for [rwrs-another-page](https://github.com/Kreedzt/rwrs-another-page).

[![Quality Gate Status](https://sonarcloud.io/api/project_badges/measure?project=Kreedzt_rwrs-server&metric=alert_status)](https://sonarcloud.io/summary/new_code?id=Kreedzt_rwrs-server)
[![Coverage](https://sonarcloud.io/api/project_badges/measure?project=Kreedzt_rwrs-server&metric=coverage)](https://sonarcloud.io/summary/new_code?id=Kreedzt_rwrs-server)
[![codecov](https://codecov.io/gh/Kreedzt/rwrs-server/branch/master/graph/badge.svg?token=MWGXZH7GO9)](https://codecov.io/gh/Kreedzt/rwrs-server)
![CI](https://github.com/Kreedzt/rwrs-server/actions/workflows/test.yml/badge.svg?branch=master)
[![Docker Image Size](https://badgen.net/docker/size/zhaozisong0/rwrs-server?icon=docker&label=image%20size)](https://hub.docker.com/r/zhaozisong0/rwrs-server/)

## Introduction

This project serves as the backend for [rwrs-another-page](https://github.com/Kreedzt/rwrs-another-page), developed using Rust and the Salvo framework. Its primary function is to proxy API requests for the Running with Rifles game server list, provide static file serving, and serve game map configuration data.

## Development

1. Install Rust development environment

```bash
# Install rustup
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
# Or download the installer for Windows
```

2. Clone the repository

```bash
git clone https://github.com/zhaozisong0/rwrs-server.git
cd rwrs-server
```

3. Local development

```bash
# Run the development server
cargo run
```

By default, the server will start on `127.0.0.1:5800`. You can modify this using environment variables:

```bash
# Custom IP and port
HOST=0.0.0.0 PORT=8080 cargo run

# Custom cache and rate limiting settings
CACHE_DURATION_SECS=60 RATE_LIMIT_SECS=10 cargo run
```

## Environment Variables

| Variable | Default | Description |
|----------|----------|-------------|
| `HOST` | `127.0.0.1` | Server bind address |
| `PORT` | `5800` | Server port |
| `MAPS_CONFIG` | `maps.json` | Maps configuration file path |
| `CACHE_DURATION_SECS` | `3` | Cache expiry time in seconds |
| `RATE_LIMIT_SECS` | `3` | Rate limit interval in seconds |
| `ANDROID_REPO_URL` | (empty) | GitHub repository URL for Android app releases |
| `WEB_REPO_URL` | (empty) | GitHub repository URL for Web app releases |
| `ANNOUNCEMENT_PATH` | (empty) | Path to an HTML file whose content is served as the header announcement |
| `MCP_HOST` | `127.0.0.1` | Bind address of the MCP Streamable HTTP listener; independent of `HOST`. IPv6 may be written `::1` or `[::1]`; brackets are normalized away |
| `MCP_PORT` | `5801` | MCP listener port; `0` disables the MCP listener entirely; invalid values fall back to `5801` with a warning |
| `MCP_ALLOWED_HOSTS` | (empty) | Comma-separated `Host` header allowlist for MCP; unset on a loopback bind means `localhost,127.0.0.1,::1` plus the bind address itself, unset on a non-loopback bind disables `Host` validation |

## API Endpoints

### GET /api/ping

Health check endpoint that returns "pong".

### GET /api/version

Returns the latest release information for Android and Web applications configured through environment variables. The endpoint fetches the latest release information from GitHub repositories.

#### Response Format

```json
{
  "android": {
    "version": "v2.0.0",
    "url": "https://github.com/owner/android-repo/releases/tag/v2.0.0"
  },
  "web": {
    "version": "v1.5.0",
    "url": "https://github.com/owner/web-repo/releases/tag/v1.5.0"
  }
}
```

If a repository URL is not configured or the repository has no releases, the corresponding fields will be `null`:

```json
{
  "android": {
    "version": null,
    "url": null
  },
  "web": {
    "version": null,
    "url": null
  }
}
```

#### Configuration Example

```bash
# Set repository URLs when running the server
ANDROID_REPO_URL=https://github.com/your-org/android-app \
WEB_REPO_URL=https://github.com/your-org/web-app \
cargo run

# Or in Docker
docker run -e ANDROID_REPO_URL=https://github.com/your-org/android-app \
           -e WEB_REPO_URL=https://github.com/your-org/web-app \
           -p 8080:5800 \
           rwrs-server
```

**Note**: Only GitHub repository URLs are supported. The API will return null for non-GitHub URLs.

### GET /api/maps

Returns game map configuration data. See the Maps Configuration section below for details.

### GET /api/announcement

Returns the header announcement to display above the page header. The HTML is loaded from the file at `ANNOUNCEMENT_PATH` once at server startup and rendered verbatim by the frontend.

#### Response Format

```json
{
  "enabled": true,
  "html": "<div class=\"notice\">Server maintenance at 20:00 UTC</div>"
}
```

When `ANNOUNCEMENT_PATH` is unset, the file is missing, or its content is empty, the announcement is disabled and the frontend should show nothing:

```json
{
  "enabled": false,
  "html": ""
}
```

**Note**: The HTML is operator-controlled trusted content served as-is (no sanitization). Only put HTML you control in the announcement file. Updating the announcement requires editing the file and restarting the server.

#### Configuration Example

```bash
# Point at an announcement HTML file
ANNOUNCEMENT_PATH=announcement.html cargo run

# Or in Docker (mount the file in)
docker run -e ANNOUNCEMENT_PATH=/config/announcement.html \
           -v $(pwd)/announcement.html:/config/announcement.html \
           -p 8080:5800 \
           rwrs-server
```

### GET /api/server_list

Proxies requests to the Running with Rifles game server list API. Supports query parameters for filtering.

### GET /api/player_list

Proxies requests to the Running with Rifles player statistics API. Returns HTML content with player rankings and statistics. Supports query parameters for filtering and sorting.

#### Query Parameters (Optional)

You can pass query parameters to filter and sort the player data:
- `sort`: Field to sort by (e.g., kills, deaths, score)
- `order`: Sort order (asc, desc)
- Other parameters supported by the original API

Example:
```bash
# Get players sorted by kills in descending order
curl "http://localhost:5800/api/player_list?sort=kills&order=desc"

# Get all players
curl http://localhost:5800/api/player_list
```

### GET /api/servers.json

Returns the server list normalized to JSON. Filtering happens over the complete server list (the upstream is followed for up to 10 pages of 100 servers, de-duplicated by `address:port`) and pagination is applied afterwards. All field names are camelCase.

#### Query Parameters

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| `search` | string | (empty) | Case-insensitive match against name, IP address, port, country, mode, map id/name, comment and player names |
| `online_only` | boolean | `false` | Only servers with at least one current player |
| `map` | string | (empty) | Case-insensitive substring match on map id or map name |
| `mode` | string | (empty) | Case-insensitive substring match on game mode |
| `country` | string | (empty) | Case-insensitive substring match on country code |
| `offset` | integer | `0` | Number of matching servers to skip; must be `>= 0` |
| `limit` | integer | `20` | Page size; must be between `1` and `100` |

#### Response Format

```json
{
  "total": 2,
  "offset": 0,
  "limit": 2,
  "servers": [
    {
      "id": "127.0.0.1:1234",
      "name": "Alpha Server",
      "ipAddress": "127.0.0.1",
      "port": 1234,
      "mapId": "desert_town",
      "mapName": "Desert Town",
      "bots": 0,
      "country": "US",
      "currentPlayers": 10,
      "timeStamp": 1234567890,
      "version": "1",
      "dedicated": true,
      "mod": false,
      "playerList": ["Player1", "Player2"],
      "comment": "",
      "url": "",
      "maxPlayers": 32,
      "mode": "Invasion",
      "realm": null
    }
  ]
}
```

`total` counts every server matching the filters before pagination; `servers` only holds the requested page. `id` is `address:port`, which stays stable across upstream reordering.

Example:

```bash
curl "http://localhost:5800/api/servers.json?search=desert&online_only=true&limit=10"
```

### GET /api/players.json

Returns one page of the player leaderboard normalized to JSON. The upstream table is parsed into the same fields the web frontend uses, and all field names are camelCase.

#### Query Parameters

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| `search` | string | (empty) | Player name filter, delegated to the upstream leaderboard |
| `db` | string | `invasion` | Leaderboard: `invasion`, `pacific` or `prereset_invasion` |
| `sort` | string | `rank_progression` | One of `rank_progression`, `username`, `kills`, `deaths`, `kd`, `score`, `time_played`, `teamkills`, `longest_kill_streak`, `targets_destroyed`, `vehicles_destroyed`, `soldiers_healed`, `distance_moved`, `shots_fired`, `throwables_thrown`, `sid` |
| `offset` | integer | `0` | Number of rows to skip; must be `>= 0` |
| `limit` | integer | `20` | Page size; must be between `1` and `100` |

#### Response Format

```json
{
  "db": "pacific",
  "offset": 0,
  "limit": 2,
  "hasNext": true,
  "hasPrevious": false,
  "players": [
    {
      "id": "pacific:PlayerOne",
      "username": "PlayerOne",
      "db": "pacific",
      "rowNumber": 1,
      "rankProgression": 100,
      "kills": 1000,
      "deaths": 500,
      "score": 5000,
      "teamkills": 2,
      "longestKillStreak": 15,
      "targetsDestroyed": 20,
      "vehiclesDestroyed": 5,
      "soldiersHealed": 30,
      "shotsFired": 1000,
      "throwablesThrown": 50,
      "kd": 2.0,
      "timePlayed": "10h 30m",
      "distanceMoved": "10000m",
      "rankName": "Captain",
      "rankIcon": "/rank/captain.png"
    }
  ]
}
```

Numbers missing from the upstream table are `null` rather than `0`. `hasNext` / `hasPrevious` mirror the pagination links of the upstream page. `id` is `db:username`.

**Search window behavior**: when `search` is supplied, the upstream `view_players.php` ignores the `start` parameter and always answers from the beginning of its own result set. The returned `offset` / `limit` therefore do not slice the search window; use `hasNext` / `hasPrevious` to walk the pages the upstream exposes.

Example:

```bash
curl "http://localhost:5800/api/players.json?db=pacific&sort=kills&limit=10"
```

#### Errors

Both JSON endpoints answer with a JSON body on failure:

- `400 Bad Request` — invalid query parameter (unknown `db`/`sort`, `limit` outside `1`–`100`, negative `offset`, non-boolean `online_only`, ...): `{"error": "..."}`. The upstream is not contacted.
- `502 Bad Gateway` — the upstream request failed or returned data that could not be parsed: `{"error": "..."}`.

The existing `/api/server_list` and `/api/player_list` endpoints are untouched raw passthroughs; they keep returning the upstream HTML/XML with the original query string.

## MCP Server

The process also serves a [Model Context Protocol](https://modelcontextprotocol.io) server over **Streamable HTTP** on a separate listener. It exposes the same normalized data as the JSON endpoints through four read-only tools.

- Endpoint: `http://<MCP_HOST>:<MCP_PORT>/mcp` (default `http://127.0.0.1:5801/mcp`).
- Transport: Streamable HTTP, **stateless** (`legacy_session_mode` off, plain JSON responses). Every `POST /mcp` is independent; no `Mcp-Session-Id` bookkeeping is required.
- Tool results carry their payload in `structuredContent`, and failures set `isError: true` with a `structuredContent.error` message. Invalid arguments (unknown `db`/`sort`, `limit` out of range, ...) are JSON-RPC `-32602` errors instead.
- The server is read-only: no writes, no arbitrary URL proxying.

### Tools

| Tool | Parameters | Result |
|------|------------|--------|
| `list_servers` | `search`, `online_only`, `map`, `mode`, `country`, `offset`, `limit` — same semantics as `/api/servers.json` | `{total, offset, limit, servers}` |
| `get_server` | `address` (string), `port` (integer) | One server object, or `isError` with `{error: "server not found"}` |
| `search_players` | `search`, `db`, `sort`, `offset`, `limit` — same semantics as `/api/players.json` | `{db, offset, limit, hasNext, hasPrevious, players}` |
| `list_maps` | none | `{maps: [{name, path, image}]}` |

Like the REST player endpoint, `search_players` delegates `search` to the upstream leaderboard, whose search mode ignores `start`; use `hasNext`/`hasPrevious` to walk those pages.

### Connecting

Claude Code:

```bash
claude mcp add --transport http rwrs http://127.0.0.1:5801/mcp
```

Any MCP client configured for Streamable HTTP uses the same URL; there is no authentication.

Raw `curl` walkthrough (initialize, then list tools, then call one):

```bash
# 1. initialize
curl -sS -X POST http://127.0.0.1:5801/mcp \
  -H 'Content-Type: application/json' \
  -H 'Accept: application/json, text/event-stream' \
  -H 'MCP-Protocol-Version: 2025-11-25' \
  -d '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"curl","version":"0.1.0"}}}'

# 2. list tools
curl -sS -X POST http://127.0.0.1:5801/mcp \
  -H 'Content-Type: application/json' \
  -H 'Accept: application/json, text/event-stream' \
  -H 'MCP-Protocol-Version: 2025-11-25' \
  -d '{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}'

# 3. call a tool
curl -sS -X POST http://127.0.0.1:5801/mcp \
  -H 'Content-Type: application/json' \
  -H 'Accept: application/json, text/event-stream' \
  -H 'MCP-Protocol-Version: 2025-11-25' \
  -d '{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"list_servers","arguments":{"limit":2}}}'
```

### Configuration and security

- `MCP_HOST` defaults to `127.0.0.1` and never inherits `HOST`: with `HOST=0.0.0.0` the MCP listener stays on loopback unless `MCP_HOST` is set explicitly. IPv6 addresses may be written `::1` or `[::1]`; brackets are normalized to `::1`. `MCP_PORT` defaults to `5801`.
- `MCP_PORT` must differ from `PORT`; if both name the same port the MCP listener is disabled with an error log and the main server keeps running.
- `MCP_PORT=0` disables the MCP listener; the main JSON/static server keeps running normally.
- No authentication, OAuth or rate limiting is implemented. Keep the listener on loopback (the default) and put it behind a reverse proxy that authenticates and filters requests before exposing it.
- MCP validates the incoming `Host` header to block DNS-rebinding. When `MCP_ALLOWED_HOSTS` is unset, a loopback `MCP_HOST` gets `localhost,127.0.0.1,::1`; a non-loopback `MCP_HOST` disables that validation and logs a warning. Set `MCP_ALLOWED_HOSTS=example.com,example.com:5801` to allow public hostnames explicitly.

Docker:

```bash
docker run -d -p 80:80 -p 5801:5801 --name rwrs-server \
  -e HOST=0.0.0.0 \
  -e PORT=80 \
  -e MCP_HOST=0.0.0.0 \
  -e MCP_ALLOWED_HOSTS=rwrs.example.com \
  zhaozisong0/rwrs-server:latest
```

`MCP_ALLOWED_HOSTS` must contain the hostname clients put in the URL (here `rwrs.example.com`); anything else is rejected with HTTP 403 before reaching a tool. Publishing port 5801 exposes an unauthenticated endpoint, so terminate authentication/TLS in a reverse proxy or restrict access at the network level. To keep MCP off entirely, add `-e MCP_PORT=0`.

## Maps Configuration

Maps are configured through a JSON file specified by the `MAPS_CONFIG` environment variable (default: `maps.json`). The configuration is exposed via the `/api/maps` endpoint.

### Custom Configuration File Path

You can specify a custom location for the maps configuration file:

```bash
# Use a custom configuration file
MAPS_CONFIG=/etc/rwrs/custom-maps.json cargo run

# In Docker
docker run -e MAPS_CONFIG=/config/maps.json -v $(pwd)/maps.json:/config/maps.json rwrs-server
```

### Maps Configuration File Format

Create a `maps.json` file in the same directory as the executable (or specify a custom path):

```json
{
  "maps": [
    {
      "name": "map9",
      "path": "media/packages/vanilla.desert/maps/map9",
      "image": "md5_1.png"
    },
    {
      "name": "map10",
      "path": "media/packages/vanilla.desert/maps/map10",
      "image": "md5_2.png"
    },
    {
      "name": "map5",
      "path": "media/packages/vanilla.jungle/maps/map5",
      "image": "md5_3.png"
    }
  ]
}
```

#### Map Fields

- **`name`**: Human-readable map name (e.g., "map9")
- **`path`**: Full game path for the map (used as unique identifier)
- **`image`**: Image filename or CDN URL for the map preview

### API Endpoint

**GET** `/api/maps`

Returns a JSON response with all configured maps:

```json
{
  "maps": [
    {
      "name": "map9",
      "path": "media/packages/vanilla.desert/maps/map9",
      "image": "md5_1.png"
    }
  ]
}
```

### Configuration Examples

#### Basic Local Images
```json
{
  "maps": [
    {
      "name": "desert_outpost",
      "path": "media/packages/vanilla.desert/maps/map9",
      "image": "desert_outpost.png"
    }
  ]
}
```

#### CDN Images
```json
{
  "maps": [
    {
      "name": "desert_outpost",
      "path": "media/packages/vanilla.desert/maps/map9",
      "image": "https://cdn.example.com/maps/desert_outpost.png"
    }
  ]
}
```

## Building

### Local Build

```bash
# Build optimized version
cargo build --release
```

After building, the executable will be located at `target/release/rwrs-server`.

### Docker Build

```bash
# Build Docker image
docker build -t rwrs-server .
```

## Deployment

### Direct Deployment

1. Build the server:
   ```bash
   cargo build --release
   ```

2. Upload the built executable (`target/release/rwrs-server`) to your server

3. Create a `static` directory in the same location as the executable:
   ```bash
   mkdir static
   ```

4. Copy the rwrs-another-page frontend build files (from the `dist` directory) to the `static` directory:
   ```bash
   # Assuming you have the rwrs-another-page frontend build in ./dist
   cp -r /path/to/rwrs-another-page/dist/* ./static/
   ```

5. Copy or create the `maps.json` configuration file in the same directory:
   ```bash
   # Copy the example maps.json
   cp maps.json ./deploy/maps.json
   # Or create a custom one based on the examples above
   ```

6. Set environment variables and run:
   ```bash
   HOST=0.0.0.0 PORT=80 ./target/release/rwrs-server
   ```

### Docker Deployment

There are two ways to deploy using Docker:

#### Option 1: Separate Containers with Network

This approach runs the backend (rwrs-server) and frontend (rwrs-another-page) in separate containers.

1. Create a Docker network:
   ```bash
   docker network create rwrs-network
   ```

2. Start the rwrs-server container:
   ```bash
   docker pull zhaozisong0/rwrs-server:latest
   docker run -d --name rwrs-server --network rwrs-network \
     -e HOST=0.0.0.0 \
     -e PORT=80 \
     -e ANDROID_REPO_URL=https://github.com/your-org/android-app \
     -e WEB_REPO_URL=https://github.com/your-org/web-app \
     zhaozisong0/rwrs-server:latest
   ```

3. Start the rwrs-another-page container:
   ```bash
   docker pull zhaozisong0/rwrs-another-page:latest
   docker run -d --name rwrs-another-page --network rwrs-network -p 80:80 zhaozisong0/rwrs-another-page:latest
   ```

4. Configure a reverse proxy (like Nginx) to route:
   - `/api/*` requests to the rwrs-server container
   - `/*` requests to the rwrs-another-page container

#### Option 2: Single Container

You can also deploy by serving the frontend directly from the rwrs-server's static directory:

1. Pull and run the rwrs-server container:
   ```bash
   docker pull zhaozisong0/rwrs-server:latest
   docker run -d -p 80:80 --name rwrs-server \
     -e HOST=0.0.0.0 \
     -e PORT=80 \
     -e ANDROID_REPO_URL=https://github.com/your-org/android-app \
     -e WEB_REPO_URL=https://github.com/your-org/web-app \
     zhaozisong0/rwrs-server:latest
   ```

2. Copy the frontend build to the container's static directory:
   ```bash
   # Assuming you have the rwrs-another-page frontend build in ./dist
   docker cp ./dist/. rwrs-server:/static/
   ```

   Or mount the directory when starting the container:
   ```bash
   docker run -d -p 80:80 --name rwrs-server \
     -e HOST=0.0.0.0 \
     -e PORT=80 \
     -e ANDROID_REPO_URL=https://github.com/your-org/android-app \
     -e WEB_REPO_URL=https://github.com/your-org/web-app \
     -v $(pwd)/dist:/static \
     -v $(pwd)/maps.json:/maps.json \
     zhaozisong0/rwrs-server:latest
   ```

#### Using Custom Maps Configuration Path

If you want to use a custom location for the maps configuration:

```bash
docker run -d -p 80:80 --name rwrs-server \
  -e HOST=0.0.0.0 \
  -e PORT=80 \
  -e MAPS_CONFIG=/config/custom-maps.json \
  -e ANDROID_REPO_URL=https://github.com/your-org/android-app \
  -e WEB_REPO_URL=https://github.com/your-org/web-app \
  -v $(pwd)/dist:/static \
  -v $(pwd)/config/custom-maps.json:/config/custom-maps.json \
  zhaozisong0/rwrs-server:latest
```

#### Using Docker Compose

Alternatively, use docker-compose:

```yaml
version: '3'
services:
  rwrs-server:
    image: zhaozisong0/rwrs-server:latest
    ports:
      - "80:80"
    environment:
      - HOST=0.0.0.0
      - PORT=80
      - MAPS_CONFIG=/config/maps.json
      - ANDROID_REPO_URL=https://github.com/your-org/android-app
      - WEB_REPO_URL=https://github.com/your-org/web-app
    restart: always
    volumes:
      # Mount frontend build to static directory
      - ./dist:/static
      # Mount maps configuration
      - ./maps.json:/config/maps.json
```

#### With Custom Maps Configuration

```yaml
version: '3'
services:
  rwrs-server:
    image: zhaozisong0/rwrs-server:latest
    ports:
      - "8080:5800"
    environment:
      - HOST=0.0.0.0
      - PORT=5800
      - MAPS_CONFIG=/app/config/custom-maps.json
      - CACHE_DURATION_SECS=10
      - ANDROID_REPO_URL=https://github.com/your-org/android-app
      - WEB_REPO_URL=https://github.com/your-org/web-app
    restart: always
    volumes:
      - ./dist:/static
      - ./config/custom-maps.json:/app/config/custom-maps.json
```

## Testing

### Running Tests

```bash
# Run all tests
cargo test

# Run tests with output
cargo test -- --nocapture

# Run specific test module
cargo test api_cache_tests

# Run integration tests only
cargo test --test '*'

# Run tests with coverage (requires cargo-llvm-cov)
cargo install cargo-llvm-cov
cargo llvm-cov --all-features
```

### Test Coverage

The project uses `cargo-llvm-cov` for code coverage reporting. Coverage reports are automatically generated on GitHub Actions and uploaded to Codecov.

To generate coverage locally:

```bash
# Install cargo-llvm-cov
cargo install cargo-llvm-cov

# Generate coverage report
cargo llvm-cov --all-features --html

# View the HTML report
open target/llvm-cov/html/index.html
```

### Test Structure

- **Unit Tests**: Located in `tests/` directory
  - `api_cache_tests.rs` - Tests for ApiCache functionality
  - `maps_config_tests.rs` - Tests for MapsConfig loading and parsing
  - `api_endpoints_tests.rs` - Integration tests for API endpoints
  - `test_data/` - Test configuration and data files

### Continuous Integration

GitHub Actions automatically runs:
- Unit tests across multiple Rust versions (stable, beta, nightly)
- Code coverage reporting (Codecov)
- SonarQube quality analysis
- Clippy linting
- Code formatting checks
- Security audits

### SonarQube Quality Analysis

The project integrates with SonarQube/SonarCloud for comprehensive code quality analysis:

**What it analyzes:**
- Code coverage integration
- Code complexity and maintainability
- Security vulnerabilities
- Code smells and technical debt
- Duplicated code detection

**Configuration:**
- Analysis runs after all tests pass
- Only for main branch and pull requests
- Quality Gate enforcement can block merges if quality standards are not met

**Setup Requirements:**
1. Set `SONAR_TOKEN` in GitHub repository secrets
2. Update `sonar-project.properties` with your organization details
3. Configure SonarCloud/SonarQube project settings

## License

- [MIT](./LICENSE).
