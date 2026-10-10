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

**`config.rs`** — `Config` (deserialized from YAML; service keys are the IDs), `Service` and `Service::run()` (the per-service loop: check → `AppState::set_state` → sleep), `Settings` (the API-editable globals) and `Config::load()` which validates and fails startup on any rule violation

**`validate.rs`** — all input rules (id `^[a-z0-9][a-z0-9_-]{0,63}$`, intervals >= 1000, notify_failures >= 1, timeout_ms 100..=300000, http(s) URL, status 100..=599, port != 0, non-empty name/host). Collects **every** violation into `ValidationError` (`field: msg; field: msg`); applied in `Config::load` and on every write

**`notify.rs`** — per-service runtime state and notification policy:

- `ServiceState` (the public `Status` object: `id`, counters, `last_check` = `None` until the first check, `failing_since` for the current streak, `alerted` whether its alert was delivered)
- `apply_check()` is a pure function: updates counters and returns `Option<Notification>`; it is unit-tested, keep notification logic there
- Alert once the streak reaches `notify_failures`, re-alert every `rereport` failures; **recovery is only sent if the alert was delivered** (`alerted` is set by the caller after Telegram returns OK). An undelivered alert is retried on the next failure; shorter blips are counted but silent

**`state/`** — `AppState` (cheaply cloned, passed everywhere): `Config`, live `ServiceState` map, task handles. Its public methods are **the** application API, shared by HTTP and the MCP layer: `status`, `list_services`, `get_service`, `get_settings`, `create_service`, `update_service`, `delete_service`, `update_settings`, `run_service_check`, `test_check`; errors are `error::AppError` (`NotFound`/`Conflict`/`Invalid`/`Persist` → 404/409/422/500)

- `mutate.rs` — the single serialized write path: `write_lock` → clone config → change → validate → write `<config>.tmp` + rename → swap in memory → restart/stop only the affected task. Any failure before the swap changes nothing. Editing a service keeps its live stats; disable/delete drops its status; settings need no restart (loops read globals every cycle)
- Never hold two of the `RwLock`s at once (clone, release, then take the next) — `set_state()` reads config first, then decides under the services lock and sends Telegram outside it

**`web/`** — Axum 0.8 router; errors are always `{"error": "..."}` (`ApiError`, also wraps JSON body rejections; internal details only in the log). Frontend files are embedded via `rust-embed` (`#[folder = "frontend/"]`) and served by the `fallback`. No CORS (same origin).

- `public.rs` — `GET /api/health` (text `OK`), `GET /api/status` (enabled services' `Status`, sorted by name)
- `admin.rs` — `GET|PUT /api/settings`, `GET|POST /api/services`, `GET|PUT|DELETE /api/services/{id}`, `POST /api/services/{id}/check`, `POST /api/checks/test`
- `auth.rs` — `require_admin` middleware (`route_layer(middleware::from_fn_with_state(..))` on the admin router, which also carries `/mcp`). Bearer token compared in constant time; missing/wrong → 401; `api_bearer_token` unset/blank → 403 for every admin route (fail closed, warned at startup)
- `telegram_token`, `api_bearer_token`, `web_port` are file-only: never returned or writable via the API, preserved on every rewrite

**`mcp.rs`** — MCP server (`rmcp` crate, `#[tool_router]`/`#[tool_handler]` macros) at `/mcp` (streamable HTTP), merged into the admin router so it sits behind `require_admin` (static `Authorization: Bearer` header, no OAuth):

- Transport: **stateless** (`legacy_session_mode(false)`, `NeverSessionManager`) with plain JSON responses — no session state, survives restarts; `GET` answers 405. rmcp's loopback-only `Host` check is disabled (the device is reached by LAN name/IP; the bearer token already blocks DNS rebinding)
- Tools are thin wrappers over the `AppState` methods (no logic of their own): `list_services`, `get_service`, `create_service`, `update_service` (full replace), `delete_service`, `set_service_enabled` (read-modify-write via `update_service`), `run_service_check`, `test_check`, `get_settings`, `update_settings`. Results are JSON text content; `AppError` becomes an `is_error` result with its client-safe `Display` (never `Persist` details)
- Resource `healthcheck://status` (application/json) = live `AppState::status()`
- Tool input schemas come from `schemars::JsonSchema`, derived on `Service`, `CheckType` + its structs and `Settings` — keep the derive on any new config/check type

**`telegram.rs`** — thin wrapper around the Telegram Bot HTTP API using `reqwest`. Every send has a 10 s timeout (a monitor loop awaits it) and HTML-escapes the service name and message (`parse_mode: HTML` rejects stray `&`/`<`/`>`). Chat id is always passed explicitly — it is a hot-editable setting

### Frontend (`frontend/`)

AngularJS single-page app embedded into the binary. It reads the public `/api/status` and uses the admin `/api/settings` / `/api/services` routes for editing. Bearer token is stored in `localStorage` and sent as `Authorization: Bearer ...`.

### Deployment (`deploy/`)

`deploy/healthcheck.init` is the procd init script for the Turris (`/etc/init.d/healthcheck`). stdout/stderr are forwarded to `logread`, so per-check log noise must stay at `debug`; failures log at `warn`, notifications at `info`.

### Key design points

- Service IDs are the YAML keys and must match `^[a-z0-9][a-z0-9_-]{0,63}$` (lowercase UUIDs pass)
- `Config` is intentionally re-serialized to YAML on every API write, which means comments and unknown fields from the original file are dropped
- Per-service optional overrides follow the same pattern: `service.field.unwrap_or(config.field)` — applies to `check_interval_success`, `check_interval_fail`, `notify_failures`, `rereport`, and `telegram_chat_id`
- TLS for certificate checks uses both Mozilla's webpki-roots bundle **and** the OS native CA store (`rustls-native-certs`), so internal/custom CAs work out of the box. The rustls `ClientConfig` (certificate checks) and the `reqwest::Client` (HTTP checks) are built once in `OnceLock`s — OS CA changes need a restart. The HTTP client has pooling disabled on purpose: every check must do a fresh DNS lookup + connect + handshake, a reused connection would mask exactly the failures being monitored
- Both binaries call `rustls::crypto::ring::default_provider().install_default()` at startup to resolve the ambiguity between `ring` and `aws-lc-rs` (both pulled in transitively)
- Config path is resolved: CLI flag → `HEALTHCHECK_CONFIG` env var → `healthcheck.yaml` default

### Adding a new check type

1. Add struct + `async fn check(&self) -> State` in `checks.rs`, and a match arm in `CheckType::run()`
2. Add variant to `CheckType` enum (keep `#[serde(rename_all = "camelCase")]`; derive `JsonSchema` on the new struct for the MCP tool schemas)
3. Add UI section in `frontend/index.html` and `frontend/index.js`
4. Add its rules to `validate.rs` (`check_rules`)
5. Document it in `README.md` (Check Types) and add a test
