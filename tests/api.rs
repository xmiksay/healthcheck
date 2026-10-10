//! Integration tests for the HTTP API, driven through the router with `oneshot`.
//! No real network: only local closed ports, no task ever reaches the alert threshold.

use std::path::{Path, PathBuf};

use axum::{
    body::Body,
    http::{header, Method, Request, StatusCode},
    Router,
};
use healthcheck::{web::create_router, AppState, Config, State};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use tower::ServiceExt;

const TOKEN: &str = "admin-secret";
const TG_TOKEN: &str = "tg-secret-token";

fn closed_port() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.local_addr().unwrap().port()
}

fn config_yaml(token: Option<&str>) -> String {
    let port = closed_port();
    let token_line = token
        .map(|t| format!("api_bearer_token: {t}\n"))
        .unwrap_or_default();
    format!(
        "telegram_token: {TG_TOKEN}
telegram_chat_id: 1
check_interval_success: 60000
check_interval_fail: 60000
notify_failures: 1000000
rereport: 0
web_port: 9999
{token_line}services:
  local:
    enabled: true
    name: Local
    description: closed port
    check: !tcpPing
      host: 127.0.0.1
      port: {port}
      timeout_ms: 500
  off:
    enabled: false
    name: alpha off
    description: ''
    check: !http
      url: http://127.0.0.1:{port}/
"
    )
}

struct App {
    _dir: tempfile::TempDir,
    path: PathBuf,
    state: AppState,
    router: Router,
}

/// Monitor tasks are not started, so stats only change when a test says so.
fn app_at(token: Option<&str>, path: Option<&Path>) -> App {
    let dir = tempfile::tempdir().unwrap();
    let real = dir.path().join("healthcheck.yaml");
    std::fs::write(&real, config_yaml(token)).unwrap();
    let config = Config::load(&real).unwrap();
    let path = path.map(Path::to_path_buf).unwrap_or(real);
    let state = AppState::new(config, path.clone());
    App {
        _dir: dir,
        path,
        router: create_router(state.clone()),
        state,
    }
}

fn app(token: Option<&str>) -> App {
    app_at(token, None)
}

async fn call(
    app: &App,
    method: Method,
    uri: &str,
    token: Option<&str>,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut req = Request::builder().method(method).uri(uri);
    if let Some(t) = token {
        req = req.header(header::AUTHORIZATION, format!("Bearer {t}"));
    }
    let req = match body {
        Some(b) => req
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(b.to_string())),
        None => req.body(Body::empty()),
    }
    .unwrap();
    let resp = app.router.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let text = String::from_utf8(bytes.to_vec()).unwrap();
    assert!(!text.contains(TG_TOKEN), "telegram token leaked: {text}");
    assert!(!text.contains(TOKEN), "admin token leaked: {text}");
    let value = serde_json::from_str(&text).unwrap_or(Value::String(text));
    (status, value)
}

async fn admin(app: &App, method: Method, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
    call(app, method, uri, Some(TOKEN), body).await
}

fn on_disk(app: &App) -> Config {
    serde_yaml::from_str(&std::fs::read_to_string(&app.path).unwrap()).unwrap()
}

fn new_service(id: &str, enabled: bool) -> Value {
    json!({
        "id": id,
        "enabled": enabled,
        "name": "Bravo",
        "description": "d",
        "notify_failures": 1000000,
        "check": {"tcpPing": {"host": "127.0.0.1", "port": closed_port(), "timeout_ms": 200}}
    })
}

#[tokio::test]
async fn public_routes_need_no_token() {
    let app = app(Some(TOKEN));
    let (status, body) = call(&app, Method::GET, "/api/health", None, None).await;
    assert_eq!((status, body), (StatusCode::OK, json!("OK")));

    let (status, body) = call(&app, Method::GET, "/api/status", None, None).await;
    assert_eq!(status, StatusCode::OK);
    let list = body.as_array().unwrap();
    assert_eq!(list.len(), 1, "only enabled services: {body}");
    assert_eq!(list[0]["id"], "local");
    assert_eq!(list[0]["last_check"], Value::Null);
    assert_eq!(list[0]["state"], "Unknown");
}

#[tokio::test]
async fn admin_routes_reject_missing_or_wrong_token() {
    let app = app(Some(TOKEN));
    for token in [None, Some("wrong"), Some("")] {
        let (status, body) = call(&app, Method::GET, "/api/services", token, None).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{token:?}");
        assert!(body["error"].is_string());
    }
    let req = Request::get("/api/settings")
        .header(header::AUTHORIZATION, format!("Basic {TOKEN}"))
        .body(Body::empty())
        .unwrap();
    let resp = app.router.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn admin_routes_forbidden_without_configured_token() {
    let app = app(None);
    let (status, body) = call(&app, Method::GET, "/api/settings", Some("x"), None).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(
        body,
        json!({"error": "admin API disabled: set api_bearer_token"})
    );
    let (status, _) = call(&app, Method::GET, "/api/status", None, None).await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn service_crud_round_trip_persists_and_keeps_secrets() {
    let app = app(Some(TOKEN));

    let (status, body) = admin(
        &app,
        Method::POST,
        "/api/services",
        Some(new_service("bravo", true)),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert_eq!(body["id"], "bravo");
    assert_eq!(body["status"]["id"], "bravo");
    assert!(body["check"]["tcpPing"].is_object());

    let (_, list) = admin(&app, Method::GET, "/api/services", None).await;
    let ids: Vec<&str> = list
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["id"].as_str().unwrap())
        .collect();
    assert_eq!(
        ids,
        ["off", "bravo", "local"],
        "sorted by name, incl. disabled"
    );
    assert_eq!(list[0]["status"], Value::Null);

    let disk = on_disk(&app);
    assert!(disk.services.contains_key("bravo"));
    assert_eq!(disk.telegram_token, TG_TOKEN);
    assert_eq!(disk.api_bearer_token.as_deref(), Some(TOKEN));
    assert_eq!(disk.web_port, Some(9999));

    // Live stats survive an edit that keeps the service enabled. The running task
    // may add a check of its own, so a fresh state would show at most 1-2 checks.
    for _ in 0..5 {
        app.state.set_state("bravo".into(), State::Success).await;
    }
    let mut edit = new_service("ignored", true);
    edit["name"] = json!("Bravo renamed");
    let (status, body) = admin(&app, Method::PUT, "/api/services/bravo", Some(edit.clone())).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(
        body["status"]["total_checks"].as_u64().unwrap() >= 5,
        "{body}"
    );
    assert_eq!(body["status"]["name"], "Bravo renamed");
    assert!(body["status"]["last_check"].is_string());

    edit["enabled"] = json!(false);
    let (_, body) = admin(&app, Method::PUT, "/api/services/bravo", Some(edit)).await;
    assert_eq!(body["status"], Value::Null);
    let (_, status_list) = call(&app, Method::GET, "/api/status", None, None).await;
    assert_eq!(status_list.as_array().unwrap().len(), 1);
    assert!(!on_disk(&app).services["bravo"].enabled);

    let (status, body) = admin(&app, Method::GET, "/api/services/bravo", None).await;
    assert_eq!(
        (status, &body["name"]),
        (StatusCode::OK, &json!("Bravo renamed"))
    );

    let (status, _) = admin(&app, Method::DELETE, "/api/services/bravo", None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _) = admin(&app, Method::GET, "/api/services/bravo", None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let disk = on_disk(&app);
    assert!(!disk.services.contains_key("bravo"));
    assert_eq!(disk.telegram_token, TG_TOKEN);
}

#[tokio::test]
async fn settings_round_trip() {
    let app = app(Some(TOKEN));
    let (status, body) = admin(&app, Method::GET, "/api/settings", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body.as_object().unwrap().len(), 5, "{body}");

    let new = json!({
        "check_interval_success": 5000, "check_interval_fail": 2000,
        "notify_failures": 7, "rereport": 0, "telegram_chat_id": -42,
        "telegram_token": "attempted-overwrite"
    });
    let (status, body) = admin(&app, Method::PUT, "/api/settings", Some(new)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["telegram_chat_id"], -42);
    let disk = on_disk(&app);
    assert_eq!((disk.notify_failures, disk.telegram_chat_id), (7, -42));
    assert_eq!(disk.telegram_token, TG_TOKEN);

    let bad = json!({
        "check_interval_success": 1, "check_interval_fail": 2000,
        "notify_failures": 0, "rereport": 0, "telegram_chat_id": 1
    });
    let (status, body) = admin(&app, Method::PUT, "/api/settings", Some(bad)).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        body["error"],
        "check_interval_success: must be >= 1000; notify_failures: must be >= 1"
    );
}

#[tokio::test]
async fn duplicate_invalid_and_missing() {
    let app = app(Some(TOKEN));
    let (status, body) = admin(
        &app,
        Method::POST,
        "/api/services",
        Some(new_service("local", true)),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["error"], "service 'local' already exists");

    let invalid = json!({
        "id": "Bad Id", "enabled": true, "name": " ", "description": "",
        "check_interval_fail": 10,
        "check": {"http": {"url": "ftp://x", "timeout_ms": 5}}
    });
    let (status, body) = admin(&app, Method::POST, "/api/services", Some(invalid)).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    let msg = body["error"].as_str().unwrap();
    for field in [
        "id:",
        "name:",
        "check_interval_fail:",
        "check.http.url:",
        "check.http.timeout_ms:",
    ] {
        assert!(msg.contains(field), "{field} missing in {msg}");
    }

    let (status, body) = admin(
        &app,
        Method::POST,
        "/api/services",
        Some(json!({"id": "x"})),
    )
    .await;
    assert!(status.is_client_error());
    assert!(
        body["error"].is_string(),
        "json rejection must be wrapped: {body}"
    );

    let svc = new_service("x", true);
    for (method, uri, body) in [
        (Method::GET, "/api/services/nope", None),
        (Method::PUT, "/api/services/nope", Some(svc)),
        (Method::DELETE, "/api/services/nope", None),
        (Method::POST, "/api/services/nope/check", None),
    ] {
        let (status, body) = admin(&app, method, uri, body).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(body["error"], "service 'nope' not found");
    }
}

#[tokio::test]
async fn persistence_failure_changes_nothing() {
    let bad_path = Path::new("/nonexistent-healthcheck-dir/sub/healthcheck.yaml");
    let app = app_at(Some(TOKEN), Some(bad_path));

    let (status, body) = admin(
        &app,
        Method::POST,
        "/api/services",
        Some(new_service("bravo", true)),
    )
    .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(body, json!({"error": "failed to persist configuration"}));

    let (status, body) = admin(&app, Method::DELETE, "/api/services/local", None).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "{body}");

    let (_, list) = admin(&app, Method::GET, "/api/services", None).await;
    let ids: Vec<&str> = list
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, ["off", "local"]);
    let (_, status_list) = call(&app, Method::GET, "/api/status", None, None).await;
    assert_eq!(status_list[0]["id"], "local");
    assert!(!bad_path.exists());
}

#[tokio::test]
async fn checks_run_once_without_recording() {
    let app = app(Some(TOKEN));
    let check = json!({"tcpPing": {"host": "127.0.0.1", "port": closed_port(), "timeout_ms": 500}});
    let (status, body) = admin(&app, Method::POST, "/api/checks/test", Some(check)).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body["state"]["Failure"].is_string(), "{body}");

    let invalid = json!({"tcpPing": {"host": "", "port": 0}});
    let (status, body) = admin(&app, Method::POST, "/api/checks/test", Some(invalid)).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        body["error"],
        "tcpPing.host: must not be empty; tcpPing.port: must be between 1 and 65535"
    );

    let (status, body) = admin(&app, Method::POST, "/api/services/local/check", None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body["state"]["Failure"].is_string(), "{body}");
    let (_, entry) = admin(&app, Method::GET, "/api/services/local", None).await;
    assert_eq!(entry["status"]["total_checks"], 0);
}
