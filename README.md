# Healthcheck Service Monitor

A Rust-based service monitoring application with a real-time AngularJS web dashboard.

## Features

- **Multiple Check Types:**
  - HTTP/HTTPS endpoint monitoring with expected status codes
  - TCP connectivity checks (databases, services, etc.)
  - SSL certificate expiration monitoring (planned)

- **Real-time Monitoring:**
  - Periodic health checks with configurable intervals
  - Separate intervals for healthy vs. failing services
  - UUID-based service identification

- **Web Dashboard:**
  - AngularJS frontend (no compilation required)
  - Real-time service status display
  - Auto-refresh every 5 seconds
  - Services sorted alphabetically
  - Visual status indicators (Success/Failure/Unknown)

- **Telegram Notifications:**
  - Alert on service failures
  - Configurable failure thresholds
  - Periodic re-notifications

## Architecture

### Backend (Rust)
- **[config.rs](src/config.rs)**: Service configuration, state management, and health check logic
- **[web.rs](src/web.rs)**: REST API and static file serving (Axum framework)
- **[main.rs](src/main.rs)**: Application entry point and service orchestration

### Frontend (AngularJS)
- **[frontend/index.html](frontend/index.html)**: Main dashboard UI
- **[frontend/index.js](frontend/index.js)**: AngularJS controller with API integration
- **[frontend/index.css](frontend/index.css)**: Responsive styling

### API Endpoints
- `GET /api/services` - Returns all service states (sorted alphabetically)
- `GET /api/config` - Returns current configuration
- `GET /api/health` - Simple health check endpoint
- `GET /` - Serves the AngularJS frontend

## Configuration

Create a `healthcheck.yaml` file (see [healthcheck.yaml.example](healthcheck.yaml.example)):

```yaml
# Telegram settings
telegram_token: "YOUR_BOT_TOKEN"
telegram_chat_id: 123456789

# Global defaults (milliseconds)
check_interval_success: 60000
check_interval_fail: 10000
notify_failures: 3
rereport: 10

# Web server port
web_port: 8080

# Services
services:
  # HTTP Check
  550e8400-e29b-41d4-a716-446655440001:
    enabled: true
    name: "My Website"
    description: "Main website health"
    check_interval_success: 30000
    item:
      Http:
        url: "https://example.com"
        expected_status: 200

  # TCP Check
  660e8400-e29b-41d4-a716-446655440001:
    enabled: true
    name: "Database"
    description: "PostgreSQL connectivity"
    item:
      TCPPing:
        host: "localhost"
        port: 5432
        timeout_ms: 5000
```

### Service Structure

Each service requires:
- **UUID**: Unique identifier (use `uuidgen` or online generator)
- **enabled**: Whether to monitor this service
- **name**: Display name
- **description**: Service description
- **item**: One of:
  - `Http`: URL monitoring
  - `TCPPing`: TCP connectivity check
  - `Certificate`: SSL certificate check (not yet implemented)

### Configuration Options

- **check_interval_success**: Milliseconds between checks when service is healthy
- **check_interval_fail**: Milliseconds between checks when service is failing
- **notify_failures**: Number of consecutive failures before notification
- **rereport**: Re-notify every N failures after initial alert

## Installation & Running

### Prerequisites
- Rust 1.70+ (install from [rustup.rs](https://rustup.rs))
- Telegram bot token (optional, for notifications)

### Build
```bash
cargo build --release
```

### Run
```bash
# Using default config (healthcheck.yaml)
cargo run --release

# Or specify config location
HEALTHCHECK_CONFIG=/path/to/config.yaml cargo run --release
```

### Access Dashboard
Open your browser to: `http://localhost:8080`

## Service States

Services can be in one of three states:

- **Success** ✅ - Service is healthy
- **Failure** ❌ - Service check failed (includes error message)
- **Unknown** ⚠️ - Initial state or check not yet completed

## ServiceState Structure

The runtime state includes:
```rust
{
  "id": "uuid",
  "name": "Service Name",
  "description": "Description",
  "state": "Success" | { "Failure": "error message" } | "Unknown",
  "last_check": "2026-01-24T10:30:00Z",
  "enabled": true,
  "consecutive_failures": 0,
  "total_checks": 42,
  "successful_checks": 40
}
```

## Development

### Project Structure
```
healthcheck/
├── src/
│   ├── main.rs         # Entry point
│   ├── config.rs       # Config & state management
│   ├── web.rs          # Web server & API
│   └── telegram.rs     # Telegram notifications (if needed)
├── frontend/
│   ├── index.html      # Dashboard UI
│   ├── index.js        # AngularJS app
│   ├── index.css       # Styles
│   └── angular.js      # AngularJS library
├── healthcheck.yaml    # Runtime config
└── Cargo.toml          # Rust dependencies
```

### Adding New Check Types

1. Add variant to `ServiceItem` enum in [config.rs](src/config.rs#L73)
2. Implement the check struct with async `check()` method
3. Add match arm in `Service::run()` method
4. Update configuration example

## Dependencies

Key Rust crates:
- `axum` - Web framework
- `tower-http` - CORS and static file serving
- `tokio` - Async runtime
- `serde` / `serde_yaml` - Configuration parsing
- `reqwest` - HTTP client
- `teloxide` - Telegram bot API
- `uuid` - UUID generation and parsing
- `chrono` - Date/time handling

## License

[Your License Here]

## Contributing

Contributions welcome! Please submit issues and pull requests.

## Future Enhancements

- [ ] SSL certificate expiration monitoring
- [ ] Service uptime tracking and statistics
- [ ] Historical data and charts
- [ ] Email notifications
- [ ] WebSocket for real-time updates
- [ ] Service groups/categories
- [ ] Configurable alert thresholds per service
- [ ] Export metrics to Prometheus/Grafana
