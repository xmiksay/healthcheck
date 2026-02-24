# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Commands

```bash
cargo build                          # dev build
cargo build --release                # release build (binaries in target/release/)
cargo run                            # run server (requires healthcheck.yaml)
cargo test                           # run tests
HEALTHCHECK_CONFIG=my.yaml cargo run # use custom config file

# Cross-compile for Turris Omnia/MOX (ARMv7, requires Docker)
cross build --release --target armv7-unknown-linux-musleabihf
```

## Architecture

Two binaries share a single library crate:

- **`healthcheck`** (`src/bin/healthcheck.rs`) — starts the web server and spawns monitoring tasks
- **`healthcheck_cli`** (`src/bin/healthcheck_cli.rs`) — CLI for testing a service by ID or sending Telegram messages

### Core library (`src/`)

**`config.rs`** is the heart of the application. It contains:

- The `Config` struct (deserialized from YAML) — service keys are arbitrary strings used as IDs
- `CheckType` enum with three variants: `Http`, `Certificate`, `TcpPing` — each has its own struct implementing an async `check() -> State` method
- `AppState` wraps `Config` + live `ServiceState` map + task handles in `Arc<RwLock<_>>`; it is cheaply cloned and passed everywhere
- `Service::run()` is the per-service loop: checks, updates state, sends Telegram alerts, sleeps

**`web.rs`** — Axum router. Frontend files are embedded at compile time via `rust-embed` (`#[folder = "frontend/"]`), served through a `fallback` handler. API routes:

- `GET /api/services` — public, returns `Vec<ServiceState>`
- `GET|PUT /api/config` — optionally protected by bearer token; PUT writes YAML to disk and restarts all tasks
- `GET /api/health` — public liveness probe

**`telegram.rs`** — thin wrapper around the Telegram Bot HTTP API using `reqwest`.

### Frontend (`frontend/`)

AngularJS single-page app embedded into the binary. The visual config editor in `index.html` / `index.js` hits `/api/config` (GET to load, PUT to save). Bearer token is stored in `localStorage` and sent as `Authorization: Bearer ...`.

### Key design points

- Service IDs are arbitrary strings in the YAML (not required to be UUIDs)
- `Config` is intentionally re-serialized to YAML on PUT `/api/config`, which means unknown fields from the original file are dropped
- Per-service optional overrides follow the same pattern: `service.field.unwrap_or(config.field)` — applies to `check_interval_success`, `check_interval_fail`, `notify_failures`, `rereport`, and `telegram_chat_id`
- TLS for certificate checks uses both Mozilla's webpki-roots bundle **and** the OS native CA store (`rustls-native-certs`), so internal/custom CAs work out of the box
- Both binaries call `rustls::crypto::ring::default_provider().install_default()` at startup to resolve the ambiguity between `ring` and `aws-lc-rs` (both pulled in transitively)
- Config path is resolved: CLI flag → `HEALTHCHECK_CONFIG` env var → `healthcheck.yaml` default

### Adding a new check type

1. Add struct + `async fn check(&self) -> State` in `config.rs`
2. Add variant to `CheckType` enum (keep `#[serde(rename_all = "camelCase")]`)
3. Add match arm in `Service::run()` and in `healthcheck_cli.rs`
4. Add UI section in `frontend/index.html` and `frontend/index.js`
