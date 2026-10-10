use axum::{
    extract::{rejection::JsonRejection, Path, State},
    http::StatusCode,
    response::Json,
    routing::{get, post},
    Router,
};
use serde::Deserialize;
use serde_json::{json, Value};

use super::ApiError;
use crate::checks::{CheckType, State as CheckState};
use crate::config::{Service, Settings};
use crate::state::{AppState, ServiceEntry};

type Body<T> = Result<Json<T>, JsonRejection>;
type ApiResult<T> = Result<T, ApiError>;

/// Admin routes; the caller wraps them in `auth::require_admin`.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/settings", get(get_settings).put(put_settings))
        .route("/api/services", get(list_services).post(create_service))
        .route(
            "/api/services/{id}",
            get(get_service).put(update_service).delete(delete_service),
        )
        .route("/api/services/{id}/check", post(run_check))
        .route("/api/checks/test", post(test_check))
}

#[derive(Deserialize)]
struct NewService {
    id: String,
    #[serde(flatten)]
    service: Service,
}

fn state_body(state: CheckState) -> Json<Value> {
    Json(json!({ "state": state }))
}

async fn get_settings(State(state): State<AppState>) -> Json<Settings> {
    Json(state.get_settings().await)
}

async fn put_settings(
    State(state): State<AppState>,
    body: Body<Settings>,
) -> ApiResult<Json<Settings>> {
    let Json(settings) = body?;
    Ok(Json(state.update_settings(settings).await?))
}

async fn list_services(State(state): State<AppState>) -> Json<Vec<ServiceEntry>> {
    Json(state.list_services().await)
}

async fn create_service(
    State(state): State<AppState>,
    body: Body<NewService>,
) -> ApiResult<(StatusCode, Json<ServiceEntry>)> {
    let Json(new) = body?;
    let entry = state.create_service(new.id, new.service).await?;
    Ok((StatusCode::CREATED, Json(entry)))
}

async fn get_service(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Json<ServiceEntry>> {
    Ok(Json(state.get_service(&id).await?))
}

async fn update_service(
    State(state): State<AppState>,
    Path(id): Path<String>,
    body: Body<Service>,
) -> ApiResult<Json<ServiceEntry>> {
    let Json(service) = body?;
    Ok(Json(state.update_service(&id, service).await?))
}

async fn delete_service(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    state.delete_service(&id).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn run_check(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    Ok(state_body(state.run_service_check(&id).await?))
}

async fn test_check(
    State(state): State<AppState>,
    body: Body<CheckType>,
) -> ApiResult<Json<Value>> {
    let Json(check) = body?;
    Ok(state_body(state.test_check(check).await?))
}
