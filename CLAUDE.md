# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Commands

Use the Makefile (`CARGO_BUILD_JOBS=4` is set there):

```bash
make build / make release        # dev / release build (binaries in target/release/)
make run                         # run server (requires healthcheck.yaml)
make test                        # all tests; make test-unit / make test-integration
make lint                        # cargo clippy --all-targets -D warnings
make fmt                         # cargo fmt
make check                       # fmt + lint + test — run before declaring done
make cross                       # cross-compile for Turris Omnia/MOX (ARMv7 musl, needs Docker)
HEALTHCHECK_CONFIG=my.yaml cargo run   # custom config file
```

## Architecture

Two binaries share a single library crate:

- **`healthcheck`** (`src/bin/healthcheck.rs`) — starts the web server and spawns monitoring tasks
- **`healthcheck_cli`** (`src/bin/healthcheck_cli.rs`) — CLI for testing a service by ID or sending Telegram messages

### Core library (`src/`)

**`checks.rs`** — the check implementations:

- `State` (`Unknown | Success | Failure(reason)`)
- `CheckType` enum with three variants: `Http`, `Certificate`, `TcpPing` — each has its own struct with an async `check() -> State`; `CheckType::run()` dispatches (used by both the monitor loop and the CLI)
- `Http` and `Certificate` take an optional `timeout_ms` (default `DEFAULT_TIMEOUT_MS` = 10 s) covering connect + response / handshake
- `error_chain()` renders an error with its full `source()` chain — failure reasons must show the root cause (DNS, refused, reset, timeout), not reqwest's generic "error sending request"

**`config.rs`** — `Config` (deserialized from YAML; service keys are arbitrary strings used as IDs), `Service` and `Service::run()`, the per-service loop: check → `AppState::set_state` → sleep

**`notify.rs`** — per-service runtime state and notification policy:

- `ServiceState` (what `GET /api/services` returns; `failing_since` tracks the current failure streak, `alerted` whether its alert was delivered)
- `apply_check()` is a pure function: updates counters and returns `Option<Notification>`; it is unit-tested, keep notification logic there
- Alert once the streak reaches `notify_failures`, re-alert every `rereport` failures; **recovery is only sent if the alert was delivered** (`alerted` is set by the caller after Telegram returns OK). An undelivered alert is retried on the next failure; shorter blips are counted but silent

**`state.rs`** — `AppState` wraps `Config` + live `ServiceState` map + task handles in `Arc<RwLock<_>>`; cheaply cloned and passed everywhere. `set_state()` decides under the locks and sends Telegram messages outside them

**`web.rs`** — Axum router. Frontend files are embedded at compile time via `rust-embed` (`#[folder = "frontend/"]`), served through a `fallback` handler. API routes:

- `GET /api/services` — public, returns `Vec<ServiceState>`
- `GET|PUT /api/config` — optionally protected by bearer token; PUT writes YAML to disk and restarts all tasks
- `GET /api/health` — public liveness probe

**`telegram.rs`** — thin wrapper around the Telegram Bot HTTP API using `reqwest`.

### Frontend (`frontend/`)

AngularJS single-page app embedded into the binary. The visual config editor in `index.html` / `index.js` hits `/api/config` (GET to load, PUT to save). Bearer token is stored in `localStorage` and sent as `Authorization: Bearer ...`.

### Deployment (`deploy/`)

`deploy/healthcheck.init` is the procd init script for the Turris (`/etc/init.d/healthcheck`). stdout/stderr are forwarded to `logread`, so per-check log noise must stay at `debug`; failures log at `warn`, notifications at `info`.

### Key design points

- Service IDs are arbitrary strings in the YAML (not required to be UUIDs)
- `Config` is intentionally re-serialized to YAML on PUT `/api/config`, which means unknown fields from the original file are dropped
- Per-service optional overrides follow the same pattern: `service.field.unwrap_or(config.field)` — applies to `check_interval_success`, `check_interval_fail`, `notify_failures`, `rereport`, and `telegram_chat_id`
- TLS for certificate checks uses both Mozilla's webpki-roots bundle **and** the OS native CA store (`rustls-native-certs`), so internal/custom CAs work out of the box
- Both binaries call `rustls::crypto::ring::default_provider().install_default()` at startup to resolve the ambiguity between `ring` and `aws-lc-rs` (both pulled in transitively)
- Config path is resolved: CLI flag → `HEALTHCHECK_CONFIG` env var → `healthcheck.yaml` default

### Adding a new check type

1. Add struct + `async fn check(&self) -> State` in `checks.rs`, and a match arm in `CheckType::run()`
2. Add variant to `CheckType` enum (keep `#[serde(rename_all = "camelCase")]`)
3. Add UI section in `frontend/index.html` and `frontend/index.js`
4. Document it in `README.md` (Check Types) and add a test
