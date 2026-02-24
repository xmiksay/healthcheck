use axum::{
    async_trait,
    extract::{FromRequestParts, State},
    http::{header, request::Parts, StatusCode},
    response::{IntoResponse, Json, Response},
    routing::get,
    Router,
};
use rust_embed::RustEmbed;
use tower_http::cors::{Any, CorsLayer};

use crate::config::{AppState, Config, ServiceState};

#[derive(RustEmbed)]
#[folder = "frontend/"]
struct Assets;

// Bearer token extractor for authentication
pub struct BearerToken(pub String);

#[async_trait]
impl<S> FromRequestParts<S> for BearerToken
where
    S: Send + Sync,
{
    type Rejection = (StatusCode, &'static str);

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        let auth_header = parts
            .headers
            .get("Authorization")
            .and_then(|h| h.to_str().ok())
            .ok_or((StatusCode::UNAUTHORIZED, "Missing Authorization header"))?;

        if let Some(token) = auth_header.strip_prefix("Bearer ") {
            Ok(BearerToken(token.to_string()))
        } else {
            Err((
                StatusCode::UNAUTHORIZED,
                "Invalid Authorization header format",
            ))
        }
    }
}

// Handler for serving embedded static files
async fn serve_asset(uri: axum::http::Uri) -> impl IntoResponse {
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
            Response::builder()
                .header(header::CONTENT_TYPE, mime)
                .body(axum::body::Body::from(content.data))
                .unwrap()
        }
        None => Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(axum::body::Body::from("404 Not Found"))
            .unwrap(),
    }
}

// Handler for getting all service states
async fn get_services(State(state): State<AppState>) -> Json<Vec<ServiceState>> {
    let services = state.get_all_services().await;
    Json(services)
}

// Handler for health check endpoint
async fn health_check() -> (StatusCode, &'static str) {
    (StatusCode::OK, "OK")
}

// Handler for getting configuration (requires authentication)
async fn get_config(
    State(state): State<AppState>,
    bearer: BearerToken,
) -> Result<Json<Config>, (StatusCode, &'static str)> {
    let config = state.get_config().await;

    // Check if bearer token is required and validate it
    if let Some(expected_token) = &config.api_bearer_token {
        if bearer.0 != *expected_token {
            return Err((StatusCode::UNAUTHORIZED, "Invalid bearer token"));
        }
    }

    Ok(Json(config))
}

// Handler for updating configuration (requires authentication)
async fn update_config(
    State(state): State<AppState>,
    bearer: BearerToken,
    Json(new_config): Json<Config>,
) -> Result<(StatusCode, &'static str), (StatusCode, String)> {
    let current_config = state.get_config().await;

    // Check if bearer token is required and validate it
    if let Some(expected_token) = &current_config.api_bearer_token {
        if bearer.0 != *expected_token {
            return Err((StatusCode::UNAUTHORIZED, "Invalid bearer token".to_string()));
        }
    }

    match state.update_config(new_config).await {
        Ok(_) => {
            tracing::info!("Configuration updated successfully via API");
            Ok((StatusCode::OK, "Configuration updated successfully"))
        }
        Err(e) => {
            tracing::error!("Failed to update configuration: {}", e);
            Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to update configuration: {}", e),
            ))
        }
    }
}

// Create the web server router
pub fn create_router(app_state: AppState) -> Router {
    // Configure CORS to allow requests from any origin
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    Router::new()
        .route("/api/services", get(get_services))
        .route("/api/config", get(get_config).put(update_config))
        .route("/api/health", get(health_check))
        .fallback(serve_asset)
        .layer(cors)
        .with_state(app_state)
}

// Start the web server
pub async fn start_server(app_state: AppState, port: u16) -> anyhow::Result<()> {
    let app = create_router(app_state);
    let addr = format!("0.0.0.0:{}", port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;

    tracing::info!("Web server listening on {}", addr);

    axum::serve(listener, app).await?;

    Ok(())
}
