use axum::{
    extract::State,
    http::StatusCode,
    response::Json,
    routing::get,
    Router,
};
use tower_http::cors::{Any, CorsLayer};
use tower_http::services::ServeDir;

use crate::config::{AppState, ServiceState, Config};

// Handler for getting all service states
async fn get_services(State(state): State<AppState>) -> Json<Vec<ServiceState>> {
    let services = state.get_all_services().await;
    Json(services)
}

// Handler for health check endpoint
async fn health_check() -> (StatusCode, &'static str) {
    (StatusCode::OK, "OK")
}

// Handler for getting configuration
async fn get_config(State(state): State<AppState>) -> Json<Config> {
    let config = state.get_config().await;
    Json(config)
}

// Handler for updating configuration
async fn update_config(
    State(state): State<AppState>,
    Json(new_config): Json<Config>,
) -> Result<(StatusCode, &'static str), (StatusCode, String)> {
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
        .nest_service("/", ServeDir::new("frontend"))
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
