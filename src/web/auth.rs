//! Admin authentication: `Authorization: Bearer <api_bearer_token>`.
//!
//! Mount with `route_layer(middleware::from_fn_with_state(state, require_admin))`.
//! Fails closed: with no token configured every admin route answers 403.

use axum::{
    extract::{Request, State},
    http::{header, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};

use super::ApiError;
use crate::state::AppState;

pub async fn require_admin(State(state): State<AppState>, req: Request, next: Next) -> Response {
    let Some(expected) = state.api_bearer_token().await else {
        return ApiError(
            StatusCode::FORBIDDEN,
            "admin API disabled: set api_bearer_token".into(),
        )
        .into_response();
    };

    let provided = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
        .and_then(|h| h.split_once(' '))
        .filter(|(scheme, _)| scheme.eq_ignore_ascii_case("bearer"))
        .map(|(_, token)| token.trim());

    match provided {
        None => ApiError(
            StatusCode::UNAUTHORIZED,
            "missing or malformed Authorization header".into(),
        )
        .into_response(),
        Some(token) if !constant_time_eq(token.as_bytes(), expected.trim().as_bytes()) => {
            ApiError(StatusCode::UNAUTHORIZED, "invalid bearer token".into()).into_response()
        }
        Some(_) => next.run(req).await,
    }
}

/// Compares in time independent of where the inputs differ (only the length leaks).
pub fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

#[cfg(test)]
mod tests {
    use super::constant_time_eq;

    #[test]
    fn constant_time_eq_matches_only_identical_input() {
        assert!(constant_time_eq(b"secret", b"secret"));
        assert!(constant_time_eq(b"", b""));
        assert!(!constant_time_eq(b"secret", b"secreT"));
        assert!(!constant_time_eq(b"secret", b"Secret"));
        assert!(!constant_time_eq(b"secret", b"secret2"));
        assert!(!constant_time_eq(b"secret", b""));
    }
}
