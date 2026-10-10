mod admin;
pub mod auth;
mod public;

use axum::{
    extract::rejection::JsonRejection,
    http::{header, StatusCode, Uri},
    middleware,
    response::{IntoResponse, Json, Response},
    Router,
};
use rust_embed::RustEmbed;
use serde_json::json;

use crate::error::AppError;
use crate::state::AppState;

#[derive(RustEmbed)]
#[folder = "frontend/"]
struct Assets;

/// Every API error is rendered as `{"error": "..."}`.
#[derive(Debug)]
pub struct ApiError(pub StatusCode, pub String);

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(json!({ "error": self.1 }))).into_response()
    }
}

impl From<AppError> for ApiError {
    fn from(e: AppError) -> Self {
        let status = match &e {
            AppError::NotFound(_) => StatusCode::NOT_FOUND,
            AppError::Conflict(_) => StatusCode::CONFLICT,
            AppError::Invalid(_) => StatusCode::UNPROCESSABLE_ENTITY,
            // Already logged with details in AppState; the client gets the generic text.
            AppError::Persist(_) => StatusCode::INTERNAL_SERVER_ERROR,
        };
        ApiError(status, e.to_string())
    }
}

impl From<JsonRejection> for ApiError {
    fn from(e: JsonRejection) -> Self {
        ApiError(e.status(), e.body_text())
    }
}

async fn serve_asset(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    let path = if path.is_empty() { "index.html" } else { path };

    match Assets::get(path) {
        Some(content) => {
            let mime = match path.rsplit('.').next() {
                Some("html") => "text/html; charset=utf-8",
                Some("css") => "text/css",
                Some("js") => "application/javascript",
                _ => "application/octet-stream",
            };
            ([(header::CONTENT_TYPE, mime)], content.data).into_response()
        }
        None => (StatusCode::NOT_FOUND, "404 Not Found").into_response(),
    }
}

/// Full application router: public API, admin API (behind `auth::require_admin`)
/// and the embedded frontend. Further admin surfaces (e.g. `/mcp`) should be
/// merged into the admin part so they share the same auth layer.
pub fn create_router(app_state: AppState) -> Router {
    let admin = admin::router().route_layer(middleware::from_fn_with_state(
        app_state.clone(),
        auth::require_admin,
    ));

    Router::new()
        .merge(public::router())
        .merge(admin)
        .fallback(serve_asset)
        .with_state(app_state)
}

pub async fn start_server(app_state: AppState, port: u16) -> anyhow::Result<()> {
    use anyhow::Context;

    let app = create_router(app_state);
    let addr = format!("0.0.0.0:{}", port);
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .with_context(|| format!("failed to bind {addr}"))?;

    tracing::info!("Web server listening on {}", addr);
    axum::serve(listener, app)
        .await
        .context("web server failed")?;
    Ok(())
}
