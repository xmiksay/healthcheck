//! Integration tests for the MCP endpoint: the real router on a local listener,
//! raw JSON-RPC over HTTP. No real network: only disabled services / local ports.

use healthcheck::{web::create_router, AppState, Config};
use reqwest::{Client, StatusCode};
use serde_json::{json, Value};

const TOKEN: &str = "mcp-secret";
const PROTOCOL: &str = "2025-06-18";

const ALL_TOOLS: [&str; 10] = [
    "list_services",
    "get_service",
    "create_service",
    "update_service",
    "delete_service",
    "set_service_enabled",
    "run_service_check",
    "test_check",
    "get_settings",
    "update_settings",
];

struct Server {
    _dir: tempfile::TempDir,
    url: String,
    client: Client,
}

/// One enabled service on a closed local port (its monitor is never started) and
/// a notify threshold no test reaches, so Telegram is never contacted.
async fn server(token: Option<&str>) -> Server {
    let closed_port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let token_line = token
        .map(|t| format!("api_bearer_token: {t}\n"))
        .unwrap_or_default();
    let yaml = format!(
        "telegram_token: tg
telegram_chat_id: 1
check_interval_success: 60000
check_interval_fail: 60000
notify_failures: 1000000
rereport: 0
{token_line}services:
  local:
    enabled: true
    name: Local
    description: closed port
    check: !tcpPing
      host: 127.0.0.1
      port: {closed_port}
      timeout_ms: 500
"
    );
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("healthcheck.yaml");
    std::fs::write(&path, yaml).unwrap();
    let state = AppState::new(Config::load(&path).unwrap(), path);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, create_router(state)).await });

    Server {
        _dir: dir,
        url: format!("http://{addr}/mcp"),
        client: Client::new(),
    }
}

impl Server {
    async fn post(&self, token: Option<&str>, body: Value) -> reqwest::Response {
        let mut req = self
            .client
            .post(&self.url)
            .header("Accept", "application/json, text/event-stream")
            .header("MCP-Protocol-Version", PROTOCOL)
            .json(&body);
        if let Some(t) = token {
            req = req.bearer_auth(t);
        }
        req.send().await.unwrap()
    }

    /// Sends a JSON-RPC request and returns its `result`.
    async fn rpc(&self, id: u64, method: &str, params: Value) -> Value {
        let body = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
        let resp = self.post(Some(TOKEN), body).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let content_type = resp.headers()["content-type"].to_str().unwrap().to_owned();
        assert!(
            content_type.starts_with("application/json"),
            "{content_type}"
        );
        let msg: Value = resp.json().await.unwrap();
        assert_eq!(msg["id"], id, "{msg}");
        msg.get("result")
            .cloned()
            .unwrap_or_else(|| panic!("no result: {msg}"))
    }

    async fn initialize(&self) -> Value {
        let result = self
            .rpc(
                1,
                "initialize",
                json!({
                    "protocolVersion": PROTOCOL,
                    "capabilities": {},
                    "clientInfo": { "name": "test", "version": "1.0" }
                }),
            )
            .await;
        let note = json!({ "jsonrpc": "2.0", "method": "notifications/initialized" });
        let resp = self.post(Some(TOKEN), note).await;
        assert_eq!(resp.status(), StatusCode::ACCEPTED);
        result
    }

    async fn call(&self, id: u64, tool: &str, args: Value) -> Value {
        self.rpc(id, "tools/call", json!({ "name": tool, "arguments": args }))
            .await
    }
}

/// Parses the JSON text content of a successful tool result.
fn tool_json(result: &Value) -> Value {
    assert_eq!(result["isError"], false, "{result}");
    serde_json::from_str(result["content"][0]["text"].as_str().unwrap()).unwrap()
}

fn init_body() -> Value {
    json!({
        "jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": { "protocolVersion": PROTOCOL, "capabilities": {},
                    "clientInfo": { "name": "test", "version": "1.0" } }
    })
}

#[tokio::test]
async fn rejects_missing_or_wrong_token_with_401() {
    let s = server(Some(TOKEN)).await;
    for token in [None, Some("wrong")] {
        let resp = s.post(token, init_body()).await;
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
        let body: Value = resp.json().await.unwrap();
        assert!(body["error"].is_string(), "{body}");
    }
}

/// The device is reached by LAN name/IP, so rmcp's loopback-only Host check must be off.
#[tokio::test]
async fn accepts_non_loopback_host_header() {
    let s = server(Some(TOKEN)).await;
    let resp = s
        .client
        .post(&s.url)
        .header("Accept", "application/json, text/event-stream")
        .header("Host", "turris.lan:8080")
        .bearer_auth(TOKEN)
        .json(&init_body())
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn fails_closed_with_403_when_no_token_configured() {
    let s = server(None).await;
    let resp = s.post(Some(TOKEN), init_body()).await;
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["error"], "admin API disabled: set api_bearer_token");
}

#[tokio::test]
async fn initialize_advertises_server_and_lists_all_tools() {
    let s = server(Some(TOKEN)).await;
    let init = s.initialize().await;
    assert_eq!(init["serverInfo"]["name"], "healthcheck");
    assert_eq!(init["serverInfo"]["version"], env!("CARGO_PKG_VERSION"));
    assert!(init["capabilities"]["tools"].is_object(), "{init}");
    assert!(init["capabilities"]["resources"].is_object(), "{init}");
    assert!(init["instructions"]
        .as_str()
        .unwrap()
        .contains("milliseconds"));

    let list = s.rpc(2, "tools/list", json!({})).await;
    let mut names: Vec<&str> = list["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    names.sort_unstable();
    let mut expected = ALL_TOOLS.to_vec();
    expected.sort_unstable();
    assert_eq!(names, expected);
}

#[tokio::test]
async fn create_service_round_trips_through_list_services() {
    let s = server(Some(TOKEN)).await;
    s.initialize().await;

    let service = json!({
        "enabled": false,
        "name": "New one",
        "description": "",
        "check": { "http": { "url": "http://127.0.0.1:1/", "timeout_ms": 500 } }
    });
    let created = s
        .call(
            2,
            "create_service",
            json!({ "id": "new-one", "service": service }),
        )
        .await;
    let created = tool_json(&created);
    assert_eq!(created["id"], "new-one");
    assert_eq!(created["status"], Value::Null);

    let enabled = s
        .call(
            3,
            "set_service_enabled",
            json!({ "id": "new-one", "enabled": false }),
        )
        .await;
    assert_eq!(tool_json(&enabled)["enabled"], false);

    let list = tool_json(&s.call(4, "list_services", json!({})).await);
    let ids: Vec<&str> = list
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, ["local", "new-one"]);
    assert_eq!(list[1]["name"], "New one");
}

#[tokio::test]
async fn invalid_id_and_unknown_service_are_tool_errors() {
    let s = server(Some(TOKEN)).await;
    s.initialize().await;

    let service = json!({
        "enabled": false, "name": "Bad", "description": "",
        "check": { "tcpPing": { "host": "127.0.0.1", "port": 1 } }
    });
    let result = s
        .call(
            2,
            "create_service",
            json!({ "id": "Bad ID", "service": service }),
        )
        .await;
    assert_eq!(result["isError"], true, "{result}");
    assert!(result["content"][0]["text"]
        .as_str()
        .unwrap()
        .starts_with("id:"));

    let result = s.call(3, "get_service", json!({ "id": "missing" })).await;
    assert_eq!(result["isError"], true, "{result}");
    assert_eq!(result["content"][0]["text"], "service 'missing' not found");
}

#[tokio::test]
async fn status_resource_returns_live_status_json() {
    let s = server(Some(TOKEN)).await;
    s.initialize().await;

    let list = s.rpc(2, "resources/list", json!({})).await;
    assert_eq!(list["resources"][0]["uri"], "healthcheck://status");

    let read = s
        .rpc(
            3,
            "resources/read",
            json!({ "uri": "healthcheck://status" }),
        )
        .await;
    let contents = &read["contents"][0];
    assert_eq!(contents["mimeType"], "application/json");
    let status: Value = serde_json::from_str(contents["text"].as_str().unwrap()).unwrap();
    assert_eq!(status[0]["id"], "local");
    assert_eq!(status[0]["state"], "Unknown");
    assert_eq!(status[0]["last_check"], Value::Null);
}
