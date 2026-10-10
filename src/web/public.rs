use axum::{extract::State, http::StatusCode, response::Json, routing::get, Router};

use crate::notify::ServiceState;
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/health", get(health))
        .route("/api/status", get(status))
}

async fn health() -> (StatusCode, &'static str) {
    (StatusCode::OK, "OK")
}

async fn status(State(state): State<AppState>) -> Json<Vec<ServiceState>> {
    Json(state.status().await)
}
