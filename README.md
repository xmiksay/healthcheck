# Healthcheck Service Monitor

A comprehensive Rust-based service monitoring application with real-time web dashboard, Telegram notifications, and CLI tools for testing and alerting.

## Features

### Monitoring Capabilities
- **HTTP/HTTPS Monitoring**: Check endpoint availability with expected status codes
- **TCP Connectivity**: Monitor database servers, Redis, SSH, and other TCP services
- **SSL Certificate Expiration**: Track certificate expiration dates with configurable thresholds
- **Configurable Intervals**: Separate check intervals for healthy vs. failing services
- **Service-Level Overrides**: Override global settings per service

### Web Dashboard
- **Real-Time Monitoring**: Auto-refreshing service status display (5-second intervals)
- **Visual Configuration Editor**: User-friendly form-based config editing
- **Raw Configuration Editor**: Direct JSON editing for advanced users
- **Responsive Design**: Works on desktop and mobile devices
- **Service Statistics**: Track uptime, total checks, success/failure counts
- **Alphabetically Sorted**: Services automatically sorted by name

### Telegram Notifications
- **Smart Alerting**: Notify after N consecutive failures (configurable)
- **Periodic Re-notifications**: Re-alert every N failures to ensure awareness
- **Recovery Notifications**: Sent only when an alert for the outage was actually delivered (short blips stay silent; a failed alert send is retried)
- **Root-Cause Reasons**: Alerts carry the full error chain (DNS failure, connection refused, timeout, TLS error)
- **HTML Formatting**: Rich message formatting with emojis (🚨 alerts, ✅ recovery)

### CLI Tools
- **Service Testing**: Test individual services by ID without starting the server
- **Telegram Messaging**: Send success/error messages via Telegram from scripts
- **Configuration Sharing**: Uses same config file as server

## Architecture

### Binaries
- **`healthcheck`**: Main server with web dashboard and monitoring
- **`healthcheck_cli`**: Command-line tool for testing and notifications

### Backend Components (Rust)
- **[src/lib.rs](src/lib.rs)**: Shared library exports
- **[src/checks.rs](src/checks.rs)**: Health check implementations (http / certificate / tcpPing) and `State`
- **[src/config.rs](src/config.rs)**: YAML configuration model and the per-service monitor loop
- **[src/notify.rs](src/notify.rs)**: Per-service runtime state and notification policy (`apply_check`)
- **[src/state/](src/state/mod.rs)**: `AppState` — shared config, live state, monitor tasks and the serialized mutation API (`mutate.rs`)
- **[src/validate.rs](src/validate.rs)**: Config / API input validation
- **[src/error.rs](src/error.rs)**: `AppError` returned by the `AppState` API
- **[src/web/](src/web/mod.rs)**: REST API (Axum): `public.rs`, `admin.rs`, bearer auth in `auth.rs`, embedded static files
- **[src/mcp.rs](src/mcp.rs)**: MCP server at `/mcp` (tools + `healthcheck://status` resource) over the same `AppState` API
- **[src/telegram.rs](src/telegram.rs)**: Telegram Bot API integration (reqwest-based)
- **[src/bin/healthcheck.rs](src/bin/healthcheck.rs)**: Server entry point
- **[src/bin/healthcheck_cli.rs](src/bin/healthcheck_cli.rs)**: CLI entry point

### Frontend (AngularJS)
- **[frontend/index.html](frontend/index.html)**: Dashboard UI with visual/raw config editors
- **[frontend/index.js](frontend/index.js)**: AngularJS controller with API integration
- **[frontend/index.css](frontend/index.css)**: Responsive styling

## Installation

### Prerequisites
- Rust 1.70+ ([install from rustup.rs](https://rustup.rs))
- Telegram bot token (get from [@BotFather](https://t.me/botfather))
- Telegram chat ID — two ways to get it:
  1. Send a message to your bot, then open `https://api.telegram.org/bot{YOUR_TOKEN}/getUpdates` in a browser — look for `"chat":{"id":...}` in the response
  2. Use [@userinfobot](https://t.me/userinfobot) for your personal chat ID

### Build
```bash
git clone <repository-url>
cd healthcheck
cargo build --release
```

Binaries will be in `target/release/`:
- `healthcheck` (server)
- `healthcheck_cli` (CLI tool)

### Cross-compilation for Turris (armv7)

Turris Omnia / Turris MOX routers run on ARMv7. Use [`cross`](https://github.com/cross-rs/cross) (requires Docker):

```bash
cargo install cross
make cross   # = cross build --release --target armv7-unknown-linux-musleabihf
```

The binary will be at `target/armv7-unknown-linux-musleabihf/release/healthcheck`. Copy it to the router:

```bash
scp target/armv7-unknown-linux-musleabihf/release/healthcheck root@turris:/usr/local/bin/
scp target/armv7-unknown-linux-musleabihf/release/healthcheck_cli root@turris:/usr/local/bin/
```

### Turris init script (procd)

`deploy/healthcheck.init` runs the service under procd with stdout/stderr forwarded to the
system log and automatic respawn. Install it once (and again whenever it changes):

```bash
scp deploy/healthcheck.init root@turris:/etc/init.d/healthcheck
ssh root@turris 'chmod +x /etc/init.d/healthcheck && /etc/init.d/healthcheck enable && /etc/init.d/healthcheck restart'
ssh root@turris 'logread -e healthcheck'   # check failures are logged with their reason
```

### Make targets

`make build | release | run | test | test-unit | test-integration | lint | fmt | check | clean | cross`
(`check` = fmt + clippy + tests; `CARGO_BUILD_JOBS` defaults to 4).

## Configuration

### Quick Start

1. Copy example configuration:
```bash
cp healthcheck.yaml.example healthcheck.yaml
```

2. Edit `healthcheck.yaml` with your settings:
```yaml
telegram_token: "YOUR_BOT_TOKEN"
telegram_chat_id: YOUR_CHAT_ID
check_interval_success: 60000  # 60 seconds
check_interval_fail: 10000     # 10 seconds
notify_failures: 3
rereport: 10
web_port: 8080

services:
  550e8400-e29b-41d4-a716-446655440001:
    enabled: true
    name: "My Website"
    description: "Main website health"
    check: !http
      url: "https://example.com"
      expected_status: 200
```

### Configuration Structure

#### Global Settings
- **telegram_token**: Telegram bot token (required)
- **telegram_chat_id**: Telegram chat/channel ID (required)
- **check_interval_success**: Milliseconds between checks when healthy (default: 60000)
- **check_interval_fail**: Milliseconds between checks when failing (default: 10000)
- **notify_failures**: Consecutive failures before alert (default: 3)
- **rereport**: Re-notify every N failures after initial alert (default: 10)
- **web_port**: Web server port (default: 8080)
- **api_bearer_token**: Bearer token for the admin API (default: none = admin API disabled, see [Authentication](#authentication))

#### Service Configuration

Each service requires:
- **ID** (the YAML key): unique, matching `^[a-z0-9][a-z0-9_-]{0,63}$` (see [Validation](#validation))
- **enabled**: Boolean to enable/disable monitoring
- **name**: Display name
- **description**: Service description
- **check**: Check type (see below)

Optional service-level overrides:
- **check_interval_success**: Override global setting
- **check_interval_fail**: Override global setting
- **notify_failures**: Override global setting
- **rereport**: Override global setting
- **telegram_chat_id**: Send notifications for this service to a different Telegram chat/channel

### Check Types

#### HTTP/HTTPS Check
```yaml
check: !http
  url: "https://api.example.com/health"
  expected_status: 200  # Optional, defaults to 200
  timeout_ms: 10000     # Optional, connect + total timeout, defaults to 10000
```

#### TCP Ping Check
```yaml
check: !tcpPing
  host: "localhost"
  port: 5432
  timeout_ms: 5000
```

#### Certificate Check
```yaml
check: !certificate
  host: "example.com"
  port: 443
  days_before_expiry: 30  # Alert if expires within 30 days
  timeout_ms: 10000       # Optional, connect + handshake timeout, defaults to 10000
```

## Usage

### Running the Server

```bash
# Using default config (healthcheck.yaml)
./target/release/healthcheck

# Specify custom config
HEALTHCHECK_CONFIG=/path/to/config.yaml ./target/release/healthcheck

# Run in development mode
cargo run
```

Access the dashboard at: `http://localhost:8080`

### Using the CLI Tool

#### Test a Service
```bash
# Test by service ID
./target/release/healthcheck_cli test-service my-website

# Output:
# Testing service: My Website
# Description: Main website health
# ✓ Service check PASSED
```

Exit codes:
- `0`: Service check passed
- `1`: Service check failed
- `2`: Service check returned unknown state

#### Send Telegram Messages
```bash
# Send success message
./target/release/healthcheck_cli telegram success "Deployment completed successfully"

# Send error message
./target/release/healthcheck_cli telegram error "Deployment failed: connection timeout"
```

#### Specify Custom Config
```bash
./target/release/healthcheck_cli -c /path/to/config.yaml test-service <ID>
```

### Web Dashboard

#### Configuration Editor

The dashboard includes two editor modes:

1. **Visual Editor** (default):
   - Form-based interface
   - Add/remove services with buttons
   - Dropdown menus for check types
   - Collapsible advanced options
   - Real-time validation

2. **Raw Editor**:
   - Direct JSON editing
   - Useful for bulk changes
   - Copy/paste configurations

Both editors support:
- Live configuration updates
- Automatic service restart
- Validation before saving
- Persistent storage (changes are written to configuration file)

## API Endpoints

All routes live under `/api`. Errors are always JSON: `{"error": "message"}`.

### Authentication

| Route | Auth |
|---|---|
| `GET /api/health`, `GET /api/status`, static frontend | public |
| everything else under `/api`, and `/mcp` | admin: `Authorization: Bearer <api_bearer_token>` |

- `api_bearer_token` unset (or blank) in the YAML → admin routes answer **403** `{"error":"admin API disabled: set api_bearer_token"}` (fail closed); a warning is logged at startup.
- Missing, malformed or wrong token → **401**. Tokens are compared in constant time.
- No CORS headers are sent: the frontend is served from the same origin.

```bash
curl -H "Authorization: Bearer your-secret-token-here" http://localhost:8080/api/services
```

`telegram_token`, `api_bearer_token` and `web_port` are **file-only**: never returned, never writable through the API, and preserved whenever the YAML is rewritten.

### Routes

| Method | Path | Body | Response |
|---|---|---|---|
| GET | `/api/health` | – | `200` text `OK` |
| GET | `/api/status` | – | `200` `[Status]` — enabled services, sorted by name |
| GET | `/api/settings` | – | `200` `Settings` |
| PUT | `/api/settings` | `Settings` | `200` `Settings`; `422` invalid |
| GET | `/api/services` | – | `200` `[ServiceEntry]` — all services incl. disabled, sorted by name |
| POST | `/api/services` | `{"id", ...Service}` | `201` `ServiceEntry`; `409` id exists; `422` invalid |
| GET | `/api/services/{id}` | – | `200` `ServiceEntry`; `404` |
| PUT | `/api/services/{id}` | `Service` | `200` `ServiceEntry`; `404`; `422` |
| DELETE | `/api/services/{id}` | – | `204`; `404` |
| POST | `/api/services/{id}/check` | – | `200` `{"state": State}` — runs the saved check once, not recorded in stats |
| POST | `/api/checks/test` | `CheckType` | `200` `{"state": State}`; `422` — runs an unsaved check once |

Every write is validated, written atomically (`<config>.tmp` + rename) and only then applied in memory. Editing a service restarts only that service's monitor and keeps its live statistics; disabling or deleting it stops the task and drops its status. Settings changes need no restart (monitors read the globals every cycle). If the file cannot be written, nothing changes and the API answers `500 {"error":"failed to persist configuration"}` (details go to the log). The YAML is re-serialized on write, so comments and unknown fields are dropped.

### JSON shapes

`State`: `"Unknown"` | `"Success"` | `{"Failure": "reason"}`

`Status`:
```json
{
  "id": "website",
  "name": "My Website",
  "description": "Main website health",
  "state": {"Failure": "Connection failed: connection refused"},
  "last_check": "2026-01-26T12:30:05Z",
  "consecutive_failures": 5,
  "total_checks": 50,
  "successful_checks": 45,
  "failed_checks": 5,
  "uptime_start": null,
  "failing_since": "2026-01-26T12:25:05Z",
  "alerted": true
}
```
`last_check` is `null` until the first check completes.

`Service` (the YAML service fields; `?` = optional):
```json
{
  "enabled": true,
  "name": "My Website",
  "description": "",
  "check_interval_success?": 60000,
  "check_interval_fail?": 10000,
  "notify_failures?": 3,
  "rereport?": 10,
  "telegram_chat_id?": -100123,
  "check": {"http": {"url": "https://example.com", "expected_status": 200, "timeout_ms": 5000}}
}
```
`check` is one of `{"http": {url, expected_status?, timeout_ms?}}`, `{"certificate": {host, port, days_before_expiry?, timeout_ms?}}`, `{"tcpPing": {host, port, timeout_ms?}}`.

`ServiceEntry` = `{"id", ...Service fields, "status": Status | null}` (`status` is `null` while disabled).

`Settings`:
```json
{"check_interval_success": 60000, "check_interval_fail": 10000, "notify_failures": 3, "rereport": 10, "telegram_chat_id": 123456789}
```

### Validation

Applied at startup (`Config::load` fails with the full message) and on every API write. All violations are reported at once with `422`, e.g. `"check.http.url: must be an http(s) URL; check_interval_fail: must be >= 1000"` (at startup the service fields are prefixed with `services.<id>.`).

| Field | Rule |
|---|---|
| service id | `^[a-z0-9][a-z0-9_-]{0,63}$` |
| `name` | non-empty after trim |
| `check_interval_success` / `check_interval_fail` (global and per service) | `>= 1000` |
| `notify_failures` (global and per service) | `>= 1` |
| `rereport` | `>= 0` (`0` = never re-alert) |
| `timeout_ms` (all check types) | `100..=300000` |
| `http.url` | valid URL with scheme `http` or `https` |
| `http.expected_status` | `100..=599` |
| `port` | `1..=65535` |
| `host` | non-empty after trim |

## MCP

The server exposes a [Model Context Protocol](https://modelcontextprotocol.io) endpoint at **`/mcp`** (streamable HTTP, stateless, JSON responses), so an AI client such as Claude Code or Claude Desktop can inspect and manage the monitor. It sits behind the **same admin auth** as `/api`: send `Authorization: Bearer <api_bearer_token>`; with no token configured it answers 403.

Claude Code:

```bash
claude mcp add --transport http healthcheck http://<host>:<port>/mcp \
  --header "Authorization: Bearer <token>"
```

Equivalent `.mcp.json` (project scope):

```json
{
  "mcpServers": {
    "healthcheck": {
      "type": "http",
      "url": "http://<host>:<port>/mcp",
      "headers": { "Authorization": "Bearer <token>" }
    }
  }
}
```

Tools (thin wrappers over the REST operations; all intervals are in milliseconds, ids match `^[a-z0-9][a-z0-9_-]{0,63}$`):

| Tool | Arguments | Does |
|---|---|---|
| `list_services` | – | all services incl. disabled, with live status |
| `get_service` | `id` | one service + live status |
| `create_service` | `id`, `service` | create (and start if enabled) |
| `update_service` | `id`, `service` | **full replace** of the service config; stats kept |
| `delete_service` | `id` | stop and remove |
| `set_service_enabled` | `id`, `enabled` | toggle monitoring, other fields unchanged |
| `run_service_check` | `id` | run the saved check once (not recorded) |
| `test_check` | `check` | run an unsaved `CheckType` once |
| `get_settings` | – | global settings |
| `update_settings` | `settings` | **full replace** of the global settings |

Validation and lookup errors come back as tool results with `isError: true` and the same message the REST API would return. The resource **`healthcheck://status`** (`application/json`) holds the live status of all enabled services (same as `GET /api/status`).

> claude.ai web connectors require OAuth, which is **not supported** — use a client that can send a static header (Claude Code, Claude Desktop via config).

## Service States

Services can be in one of three states:

- **Success** ✅: Service is healthy
- **Failure** ❌: Service check failed (includes error details)
- **Unknown** ⚠️: Initial state before first check

## Notification Logic

### Initial Alert
- Service fails N times consecutively (configurable via `notify_failures`)
- Telegram alert sent with error details

### Re-notifications
- After initial alert, re-notify every M failures (configurable via `rereport`)
- Message includes "(still failing)" indicator

### Recovery Alert
- Sent on the first successful check **after an alert was sent** (i.e. the failure streak had reached `notify_failures`)
- Failure streaks shorter than `notify_failures` are counted in the statistics but produce no message
- Message includes the number of failed checks and the downtime, e.g. `recovered after 3 failed checks (2m 30s)`
- Resets consecutive failure counter

## Development

### Project Structure
```
healthcheck/
├── src/
│   ├── lib.rs                 # Library exports
│   ├── checks.rs              # Check types (http / certificate / tcpPing) + State
│   ├── config.rs              # YAML config model & per-service monitor loop
│   ├── notify.rs              # ServiceState + notification policy (apply_check)
│   ├── state/                 # AppState: live state, tasks, mutation API
│   ├── validate.rs            # Input validation rules
│   ├── error.rs               # AppError (404/409/422/500)
│   ├── web/                   # Axum router: public, admin, auth
│   ├── mcp.rs                 # MCP server (/mcp)
│   ├── telegram.rs            # Telegram notifications
│   └── bin/
│       ├── healthcheck.rs     # Server binary
│       └── healthcheck_cli.rs # CLI binary
├── frontend/
│   ├── index.html             # Dashboard UI
│   ├── index.js               # AngularJS app
│   ├── index.css              # Styles
│   └── angular.js             # AngularJS library
├── healthcheck.yaml           # Runtime config
├── healthcheck.yaml.example   # Example config
├── Cargo.toml                 # Rust dependencies
└── README.md                  # This file
```

### Adding New Check Types

1. Add variant to `CheckType` enum in [src/checks.rs](src/checks.rs) (and a match arm in `CheckType::run()`):
```rust
#[derive(Deserialize, Serialize, Debug, Clone, Hash, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum CheckType {
    Http(ServiceHttp),
    Certificate(ServiceCertificate),
    TcpPing(ServiceTcpPing),
    YourNewCheck(ServiceYourNewCheck), // Add here
}
```

2. Implement check struct:
```rust
#[derive(Deserialize, Serialize, Debug, Clone, Hash, JsonSchema)]
pub struct ServiceYourNewCheck {
    pub field1: String,
    pub field2: u64,
}

impl ServiceYourNewCheck {
    pub async fn check(&self) -> State {
        // Implement check logic
        State::Success
    }
}
```

3. Add match arm in `Service::run()`:
```rust
let state = match &self.check {
    CheckType::Http(http) => http.check().await,
    CheckType::Certificate(cert) => cert.check().await,
    CheckType::TcpPing(tcp) => tcp.check().await,
    CheckType::YourNewCheck(check) => check.check().await, // Add here
};
```

4. Update frontend visual editor in [frontend/index.html](frontend/index.html)

5. Update configuration example

### Running Tests
```bash
make test              # everything
make test-unit         # unit tests in src/
make test-integration  # tests/ (HTTP check against a local server, API via the router, MCP over a local listener)
```

### Development Server
```bash
# Watch for changes and rebuild
cargo watch -x run
```

## Dependencies

Key Rust crates:
- **axum** (0.8) - Web framework
- **rmcp** (3.5) / **schemars** (1) - MCP server (official Rust SDK) and tool input schemas
- **rust-embed** - Frontend embedded into the binary
- **tokio** (1.49.0) - Async runtime
- **reqwest** (0.13.1) - HTTP client
- **serde** / **serde_yaml** / **serde_json** - Serialization
- **chrono** (0.4.39) - Date/time handling
- **clap** (4.5) - CLI argument parsing
- **rustls** / **tokio-rustls** - TLS support
- **x509-parser** (0.16) - Certificate parsing
- **tracing** / **tracing-subscriber** - Logging

## Troubleshooting

### Server Won't Start
- Check config file exists: `ls healthcheck.yaml`
- Validate YAML syntax: `cargo run` (errors will be printed)
- Verify port not in use: `lsof -i :8080` or `netstat -tuln | grep 8080`

### Services Not Being Checked
- Verify `enabled: true` in config
- Check logs for error messages
- Test service manually: `healthcheck_cli test-service <ID>`

### Telegram Notifications Not Working
- Verify bot token is correct
- Check chat ID is correct (try sending test message via CLI)
- Ensure consecutive failures >= notify_failures threshold
- Check logs for Telegram API errors

### Certificate Checks Failing
- Verify host is correct (without `https://`)
- Check port is correct (usually 443)
- Ensure server supports TLS
- For internal/self-signed CAs: add the CA to the OS trust store — the app loads both Mozilla's bundle and the system CA store automatically
- Test manually: `openssl s_client -connect host:port`

## Examples

### Monitor Multiple Websites
```yaml
services:
  550e8400-e29b-41d4-a716-446655440001:
    enabled: true
    name: "Production API"
    description: "Main production API health"
    check: !http
      url: "https://api.production.com/health"

  550e8400-e29b-41d4-a716-446655440002:
    enabled: true
    name: "Staging API"
    description: "Staging environment API"
    check: !http
      url: "https://api.staging.com/health"
```

### Monitor Database Cluster
```yaml
services:
  660e8400-e29b-41d4-a716-446655440001:
    enabled: true
    name: "PostgreSQL Primary"
    description: "Primary database server"
    check: !tcpPing
      host: "db1.internal"
      port: 5432
      timeout_ms: 3000

  660e8400-e29b-41d4-a716-446655440002:
    enabled: true
    name: "PostgreSQL Replica"
    description: "Read replica database"
    check: !tcpPing
      host: "db2.internal"
      port: 5432
      timeout_ms: 3000
```

### Monitor SSL Certificates
```yaml
services:
  770e8400-e29b-41d4-a716-446655440001:
    enabled: true
    name: "Production SSL"
    description: "Production website SSL certificate"
    check_interval_success: 86400000  # Check once per day
    notify_failures: 1  # Alert immediately
    check: !certificate
      host: "www.production.com"
      port: 443
      days_before_expiry: 30
```

### Continuous Deployment Integration
```bash
#!/bin/bash
# deploy.sh - Example deployment script

set -e

echo "Deploying application..."
# ... deployment steps ...

# Test service after deployment
if ./healthcheck_cli test-service my-website; then
    ./healthcheck_cli telegram success "Deployment completed successfully"
else
    ./healthcheck_cli telegram error "Deployment completed but health check failed"
    exit 1
fi
```

## License

[Your License Here]

## Contributing

Contributions welcome! Please:
1. Fork the repository
2. Create a feature branch
3. Make your changes
4. Add tests if applicable
5. Submit a pull request

## Support

For issues, questions, or suggestions:
- Open an issue on GitHub
- Check existing issues for solutions
- Review logs for error messages

## Acknowledgments

Built with:
- [Rust](https://www.rust-lang.org/)
- [Tokio](https://tokio.rs/)
- [Axum](https://github.com/tokio-rs/axum)
- [AngularJS](https://angularjs.org/)
