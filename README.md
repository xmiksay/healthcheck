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
- **Recovery Notifications**: Automatic alerts when services recover
- **HTML Formatting**: Rich message formatting with emojis (🚨 alerts, ✅ recovery)

### CLI Tools
- **Service Testing**: Test individual services by UUID without starting the server
- **Telegram Messaging**: Send success/error messages via Telegram from scripts
- **Configuration Sharing**: Uses same config file as server

## Architecture

### Binaries
- **`healthcheck`**: Main server with web dashboard and monitoring
- **`healthcheck_cli`**: Command-line tool for testing and notifications

### Backend Components (Rust)
- **[src/lib.rs](src/lib.rs)**: Shared library exports
- **[src/config.rs](src/config.rs)**: Configuration, state management, and health check implementations
- **[src/web.rs](src/web.rs)**: REST API and static file serving (Axum framework)
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
- Telegram chat ID (get from [@userinfobot](https://t.me/userinfobot))

### Build
```bash
git clone <repository-url>
cd healthcheck
cargo build --release
```

Binaries will be in `target/release/`:
- `healthcheck` (server)
- `healthcheck_cli` (CLI tool)

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

#### Service Configuration

Each service requires:
- **UUID**: Unique identifier (generate with `uuidgen` or online)
- **enabled**: Boolean to enable/disable monitoring
- **name**: Display name
- **description**: Service description
- **check**: Check type (see below)

Optional service-level overrides:
- **check_interval_success**: Override global setting
- **check_interval_fail**: Override global setting
- **notify_failures**: Override global setting
- **rereport**: Override global setting

### Check Types

#### HTTP/HTTPS Check
```yaml
check: !http
  url: "https://api.example.com/health"
  expected_status: 200  # Optional, defaults to 200
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
# Test by UUID
./target/release/healthcheck_cli test-service 550e8400-e29b-41d4-a716-446655440001

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
./target/release/healthcheck_cli -c /path/to/config.yaml test-service <UUID>
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

## API Endpoints

### GET /api/services
Returns all monitored services with their current state.

**Response:**
```json
[
  {
    "name": "My Website",
    "description": "Main website health",
    "state": "Success",
    "last_check": "2026-01-26T12:30:00Z",
    "consecutive_failures": 0,
    "total_checks": 142,
    "successful_checks": 140,
    "failed_checks": 2,
    "uptime_start": "2026-01-26T10:00:00Z"
  },
  {
    "name": "Database",
    "description": "PostgreSQL connectivity",
    "state": {
      "Failure": "Connection failed: connection refused"
    },
    "last_check": "2026-01-26T12:30:05Z",
    "consecutive_failures": 5,
    "total_checks": 50,
    "successful_checks": 45,
    "failed_checks": 5,
    "uptime_start": null
  }
]
```

### GET /api/config
Returns current configuration.

**Response:**
```json
{
  "telegram_token": "...",
  "telegram_chat_id": 123456789,
  "check_interval_success": 60000,
  "check_interval_fail": 10000,
  "notify_failures": 3,
  "rereport": 10,
  "web_port": 8080,
  "services": { ... }
}
```

### PUT /api/config
Update configuration (triggers service restart).

**Request:**
```json
{
  "telegram_token": "...",
  "telegram_chat_id": 123456789,
  ...
}
```

**Response:**
- `200 OK`: Configuration updated
- `500 Internal Server Error`: Update failed

### GET /api/health
Simple health check endpoint.

**Response:**
```json
{
  "status": "healthy"
}
```

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
- Sent immediately when service recovers from failure state
- Resets consecutive failure counter

## Development

### Project Structure
```
healthcheck/
├── src/
│   ├── lib.rs                 # Library exports
│   ├── config.rs              # Config & state management
│   ├── web.rs                 # Web server & API
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

1. Add variant to `CheckType` enum in [src/config.rs](src/config.rs):
```rust
#[derive(Deserialize, Serialize, Debug, Clone, Hash)]
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
#[derive(Deserialize, Serialize, Debug, Clone, Hash)]
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
cargo test
```

### Development Server
```bash
# Watch for changes and rebuild
cargo watch -x run
```

## Dependencies

Key Rust crates:
- **axum** (0.7.9) - Web framework
- **tower-http** (0.6.2) - CORS and static file serving
- **tokio** (1.49.0) - Async runtime
- **reqwest** (0.13.1) - HTTP client
- **serde** / **serde_yaml** / **serde_json** - Serialization
- **uuid** (1.19.0) - UUID handling
- **chrono** (0.4.39) - Date/time handling
- **clap** (4.5) - CLI argument parsing
- **native-tls** / **tokio-native-tls** - TLS support
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
- Test service manually: `healthcheck_cli test-service <UUID>`

### Telegram Notifications Not Working
- Verify bot token is correct
- Check chat ID is correct (try sending test message via CLI)
- Ensure consecutive failures >= notify_failures threshold
- Check logs for Telegram API errors

### Certificate Checks Failing
- Verify host is correct (without `https://`)
- Check port is correct (usually 443)
- Ensure server supports TLS
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
if ./healthcheck_cli test-service 550e8400-e29b-41d4-a716-446655440001; then
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
