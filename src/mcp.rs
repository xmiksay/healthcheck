//! MCP (Model Context Protocol) server: thin tool wrappers over the `AppState` API,
//! served over streamable HTTP at `/mcp` behind the admin bearer-token auth.

use axum::Router;
use rmcp::{
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{
        CallToolResult, ContentBlock, Implementation, ListResourcesResult, PaginatedRequestParams,
        ReadResourceRequestParams, ReadResourceResponse, ReadResourceResult, Resource,
        ResourceContents, ServerCapabilities, ServerConfig,
    },
    schemars::JsonSchema,
    service::RequestContext,
    tool, tool_handler, tool_router,
    transport::streamable_http_server::{
        session::never::NeverSessionManager, StreamableHttpServerConfig, StreamableHttpService,
    },
    ErrorData, RoleServer, ServerHandler,
};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::checks::CheckType;
use crate::config::{Service, Settings};
use crate::error::AppError;
use crate::state::AppState;

pub const STATUS_URI: &str = "healthcheck://status";

const INSTRUCTIONS: &str = "Manage the healthcheck monitor. \
list_services / get_service show configured services with live status; \
create_service, update_service (full replace), set_service_enabled and delete_service \
change them and are persisted to the YAML config immediately; \
run_service_check runs a saved check once and test_check runs an unsaved check once \
(neither is recorded in stats); get_settings / update_settings read and replace the \
global defaults. All intervals are in milliseconds. \
The resource healthcheck://status holds the live status of all enabled services.";

/// Shared field reference for the service-writing tools (a macro because the
/// `#[tool]` description must be a `concat!` of literals).
macro_rules! service_doc {
    () => {
        "Service fields: enabled (bool), name (non-empty), description, optional \
        per-service overrides check_interval_success / check_interval_fail (milliseconds, \
        >= 1000), notify_failures (>= 1), rereport (>= 0, 0 = never re-alert), \
        telegram_chat_id, and check: one of {\"http\":{url, expected_status?, timeout_ms?}}, \
        {\"certificate\":{host, port, days_before_expiry?, timeout_ms?}}, \
        {\"tcpPing\":{host, port, timeout_ms?}}; timeout_ms in milliseconds, 100..=300000."
    };
}

#[derive(Deserialize, JsonSchema)]
pub struct IdArgs {
    /// Service id.
    pub id: String,
}

#[derive(Deserialize, JsonSchema)]
pub struct ServiceArgs {
    /// Service id: 1-64 chars, lowercase a-z, 0-9, '-' or '_', starting with a letter or digit.
    pub id: String,
    pub service: Service,
}

#[derive(Deserialize, JsonSchema)]
pub struct EnabledArgs {
    /// Service id.
    pub id: String,
    /// true starts monitoring, false stops it and drops the live status.
    pub enabled: bool,
}

#[derive(Deserialize, JsonSchema)]
pub struct CheckArgs {
    pub check: CheckType,
}

#[derive(Deserialize, JsonSchema)]
pub struct SettingsArgs {
    pub settings: Settings,
}

/// The MCP handler; one instance per request (stateless transport), cheap to clone.
#[derive(Clone)]
pub struct HealthcheckMcp {
    state: AppState,
    tool_router: ToolRouter<Self>,
}

/// Domain errors become `is_error` tool results so the model can read and react to
/// them; `AppError`'s `Display` is already client-safe (no persistence details).
fn to_result<T: Serialize>(result: Result<T, AppError>) -> Result<CallToolResult, ErrorData> {
    match result {
        Ok(value) => json_content(&value),
        Err(e) => Ok(CallToolResult::error(vec![ContentBlock::text(
            e.to_string(),
        )])),
    }
}

fn json_content<T: Serialize>(value: &T) -> Result<CallToolResult, ErrorData> {
    let text = to_json(value)?;
    Ok(CallToolResult::success(vec![ContentBlock::text(text)]))
}

fn to_json<T: Serialize>(value: &T) -> Result<String, ErrorData> {
    serde_json::to_string_pretty(value).map_err(|e| {
        tracing::error!("Failed to serialize MCP response: {}", e);
        ErrorData::internal_error("failed to serialize response", None)
    })
}

#[tool_router]
impl HealthcheckMcp {
    pub fn new(state: AppState) -> Self {
        Self {
            state,
            tool_router: Self::tool_router(),
        }
    }

    #[tool(
        description = "List all configured services (including disabled ones), sorted by name. \
        Each entry has the id, the service config fields and `status` (live counters and \
        current state, null while disabled). Intervals are in milliseconds."
    )]
    async fn list_services(&self) -> Result<CallToolResult, ErrorData> {
        json_content(&self.state.list_services().await)
    }

    #[tool(
        description = "Get one service by id: its config fields plus live `status` \
        (null while disabled). Errors if the id does not exist."
    )]
    async fn get_service(
        &self,
        Parameters(IdArgs { id }): Parameters<IdArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        to_result(self.state.get_service(&id).await)
    }

    #[tool(description = concat!(
        "Create a new service and start monitoring it if enabled. Persisted immediately. \
        Errors if the id already exists or any field is invalid (all offending fields are \
        named). The id must match ^[a-z0-9][a-z0-9_-]{0,63}$. ",
        service_doc!()
    ))]
    async fn create_service(
        &self,
        Parameters(ServiceArgs { id, service }): Parameters<ServiceArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        to_result(self.state.create_service(id, service).await)
    }

    #[tool(description = concat!(
        "Replace an existing service's whole config (full replace, not a merge: omitted \
        optional overrides are removed). Live stats are kept; only this service's monitor \
        restarts. Persisted immediately. Errors if the id does not exist or a field is invalid. \
        To only toggle monitoring use set_service_enabled. ",
        service_doc!()
    ))]
    async fn update_service(
        &self,
        Parameters(ServiceArgs { id, service }): Parameters<ServiceArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        to_result(self.state.update_service(&id, service).await)
    }

    #[tool(
        description = "Delete a service: stops its monitor, drops its live status and \
        removes it from the config file. Errors if the id does not exist."
    )]
    async fn delete_service(
        &self,
        Parameters(IdArgs { id }): Parameters<IdArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let result = self.state.delete_service(&id).await;
        to_result(result.map(|()| json!({ "deleted": id })))
    }

    #[tool(
        description = "Enable or disable monitoring of a service, leaving every other \
        field unchanged. Disabling stops its monitor and drops its live status; enabling \
        starts it. Persisted immediately. Errors if the id does not exist."
    )]
    async fn set_service_enabled(
        &self,
        Parameters(EnabledArgs { id, enabled }): Parameters<EnabledArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let result = async {
            let mut service = self.state.get_service(&id).await?.service;
            service.enabled = enabled;
            self.state.update_service(&id, service).await
        };
        to_result(result.await)
    }

    #[tool(description = "Run a saved service's check once right now and return \
        {\"state\": \"Success\" | \"Unknown\" | {\"Failure\": reason}}. The result is NOT \
        recorded in the service's stats and sends no notification. Errors if the id does not exist.")]
    async fn run_service_check(
        &self,
        Parameters(IdArgs { id }): Parameters<IdArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let result = self.state.run_service_check(&id).await;
        to_result(result.map(|state| json!({ "state": state })))
    }

    #[tool(
        description = "Validate and run an unsaved check once (e.g. before creating a \
        service) and return {\"state\": \"Success\" | \"Unknown\" | {\"Failure\": reason}}. \
        check is one of {\"http\":{url, expected_status?, timeout_ms?}}, \
        {\"certificate\":{host, port, days_before_expiry?, timeout_ms?}}, \
        {\"tcpPing\":{host, port, timeout_ms?}}; timeout_ms is in milliseconds (100..=300000)."
    )]
    async fn test_check(
        &self,
        Parameters(CheckArgs { check }): Parameters<CheckArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let result = self.state.test_check(check).await;
        to_result(result.map(|state| json!({ "state": state })))
    }

    #[tool(description = "Get the global settings: check_interval_success and \
        check_interval_fail (milliseconds), notify_failures, rereport and telegram_chat_id. \
        Services without an override use these.")]
    async fn get_settings(&self) -> Result<CallToolResult, ErrorData> {
        json_content(&self.state.get_settings().await)
    }

    #[tool(
        description = "Replace the global settings (full replace: all five fields are \
        required). check_interval_success / check_interval_fail in milliseconds (>= 1000), \
        notify_failures >= 1, rereport >= 0 (0 = never re-alert), telegram_chat_id. Takes \
        effect on each monitor's next cycle. Secrets and the web port are file-only and \
        cannot be changed here."
    )]
    async fn update_settings(
        &self,
        Parameters(SettingsArgs { settings }): Parameters<SettingsArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        to_result(self.state.update_settings(settings).await)
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for HealthcheckMcp {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(
            ServerCapabilities::builder()
                .enable_tools()
                .enable_resources()
                .build(),
        )
        .with_server_info(Implementation::new(
            "healthcheck",
            env!("CARGO_PKG_VERSION"),
        ))
        .with_instructions(INSTRUCTIONS)
    }

    async fn list_resources(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListResourcesResult, ErrorData> {
        let status = Resource::new(STATUS_URI, "status")
            .with_description("Live status of all enabled services (same as GET /api/status)")
            .with_mime_type("application/json");
        Ok(ListResourcesResult::with_all_items(vec![status]))
    }

    async fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResponse, ErrorData> {
        if request.uri != STATUS_URI {
            return Err(ErrorData::resource_not_found(
                format!("unknown resource: {}", request.uri),
                None,
            ));
        }
        let text = to_json(&self.state.status().await)?;
        let contents = ResourceContents::text(text, STATUS_URI).with_mime_type("application/json");
        Ok(ReadResourceResult::new(vec![contents]).into())
    }
}

/// `/mcp` route; the caller mounts it inside the admin auth layer.
pub fn router(state: AppState) -> Router<AppState> {
    // Stateless + plain JSON responses: no session map to lose on restart, no SSE.
    // Host validation is off because the device is reached by LAN IP/hostname; DNS
    // rebinding is already blocked by the bearer token a rebinding page cannot know.
    let config = StreamableHttpServerConfig::default()
        .with_legacy_session_mode(false)
        .with_json_response(true)
        .with_sse_keep_alive(None)
        .disable_allowed_hosts();
    let service = StreamableHttpService::new(
        move || Ok(HealthcheckMcp::new(state.clone())),
        std::sync::Arc::new(NeverSessionManager::default()),
        config,
    );
    Router::new().route_service("/mcp", service)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(result: &CallToolResult) -> String {
        let value = serde_json::to_value(result).unwrap();
        value["content"][0]["text"].as_str().unwrap().to_owned()
    }

    #[test]
    fn ok_becomes_json_text_content() {
        let result = to_result::<_>(Ok(json!({ "a": 1 }))).unwrap();
        assert_eq!(result.is_error, Some(false));
        let parsed: serde_json::Value = serde_json::from_str(&text(&result)).unwrap();
        assert_eq!(parsed, json!({ "a": 1 }));
    }

    #[test]
    fn app_errors_become_tool_errors_without_internal_details() {
        let result = to_result::<()>(Err(AppError::Persist(anyhow::anyhow!(
            "/etc/secret: EACCES"
        ))))
        .unwrap();
        assert_eq!(result.is_error, Some(true));
        assert_eq!(text(&result), "failed to persist configuration");

        let result = to_result::<()>(Err(AppError::Conflict("web".into()))).unwrap();
        assert_eq!(result.is_error, Some(true));
        assert_eq!(text(&result), "service 'web' already exists");
    }

    #[test]
    fn tool_input_schemas_describe_nested_config_types() {
        let tool = HealthcheckMcp::create_service_tool_attr();
        let schema = serde_json::to_string(&tool.input_schema).unwrap();
        for needle in ["\"service\"", "tcpPing", "certificate", "timeout_ms"] {
            assert!(schema.contains(needle), "{needle} missing in {schema}");
        }
    }
}
