# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

RWRS Server is a Rust backend server for the [rwrs-another-page](https://github.com/Kreedzt/rwrs-another-page) frontend. It proxies the Running with Rifles upstream data, serves normalized JSON over HTTP (`/api/servers.json`, `/api/players.json`) and MCP (`/mcp`), and serves static files.

## Architecture

This is a Rust web application using the Salvo framework:

- **Core server**: `src/main.rs` wires startup; `src/routes.rs` owns the Salvo router/handlers; `src/mcp.rs` owns the MCP listener; `src/lib.rs` owns config, cache, maps, announcement and MCP helpers.
- **Data layer**: `src/domain.rs` (DTOs + query validation), `src/parsers.rs` (upstream XML/HTML parsing), `src/upstream.rs` (`UpstreamClient` / `RwrsService`, shared `ApiCache`).
- **JSON API**: `/api/servers.json` and `/api/players.json`. Responses use camelCase field names; query parameters use snake_case. Invalid input returns `400 {"error": ...}`; upstream failure returns `502`.
- **MCP**: stateless Streamable HTTP on a separate axum listener at `/mcp`. Read-only tools: `list_servers`, `get_server`, `search_players`, `list_maps`. Success uses `structuredContent`; invalid parameters use JSON-RPC `-32602`; upstream failures use `isError: true`.
- **Proxy functionality**: `/api/server_list` and `/api/player_list` remain raw XML/HTML passthroughs to the upstream endpoints.
- **Static serving**: Serves files from a `static/` directory (defaults to `index.html` for unknown paths).
- **Announcement**: `/api/announcement` returns operator-controlled header HTML loaded from the file at `ANNOUNCEMENT_PATH` (read once at startup; disabled when unset/missing/empty).
- **Configuration**: `HOST`/`PORT`; `MCP_HOST` (default `127.0.0.1`, independent of `HOST`), `MCP_PORT` (default `5801`; `0` disables; must differ from `PORT`), `MCP_ALLOWED_HOSTS`.

## Development Commands

### Local Development
```bash
# Run development server
cargo run

# Custom host/port
HOST=0.0.0.0 PORT=8080 cargo run

# Disable the MCP listener
MCP_PORT=0 cargo run
```

### Checks
```bash
cargo fmt --all -- --check
cargo clippy --all-features -- -D warnings
cargo test --all-features
```

### Build
```bash
# Debug build
cargo build

# Release build
cargo build --release
```

### Docker
```bash
# Build Docker image
docker build -t rwrs-server .

# Run container
docker run -d -p 80:80 --name rwrs-server -e HOST=0.0.0.0 -e PORT=80 rwrs-server
```

## Project Structure

- `src/main.rs` - Startup and wiring
- `src/lib.rs` - Config, cache, maps, announcement, MCP helpers
- `src/routes.rs` - Salvo router and HTTP handlers
- `src/mcp.rs` - rmcp tools and axum listener
- `src/domain.rs` - DTOs and query validation
- `src/parsers.rs` - Upstream XML/HTML parsers
- `src/upstream.rs` - Upstream client and `RwrsService`
- `src/tests/` - Unit and integration tests
- `static/` - Directory for static files (created during deployment)
- `Cargo.toml` - Rust project configuration
- `Dockerfile` - Multi-stage build for production
- `README.md` - Detailed deployment and development instructions

## Key Dependencies

- `salvo` (0.89.3) - Web framework
- `tokio` (1.48.0) - Async runtime
- `tracing` (0.1.43) - Logging and instrumentation
- `reqwest` (0.12) - Upstream HTTP client
- `scraper` (0.29) / `roxmltree` (0.21) - Player HTML / server XML parsing
- `rmcp` (3.8) / `axum` (0.8) - MCP Streamable HTTP listener

## Deployment Notes

The server expects a `static/` directory containing the frontend build files from rwrs-another-page. The Docker image uses a scratch base layer for minimal container size.

The MCP listener binds `127.0.0.1:5801` by default and has no authentication. To expose it, set `MCP_HOST=0.0.0.0` and `MCP_ALLOWED_HOSTS` explicitly, and put it behind an authenticating reverse proxy or network policy. The Docker image exposes both `5800` and `5801`.
