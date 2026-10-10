use crate::validate::ValidationError;

/// Errors returned by the `AppState` mutation/query API.
///
/// Transport-agnostic: the HTTP layer maps them to 404/409/422/500, an MCP layer
/// can map them to its own error codes.
#[derive(Debug)]
pub enum AppError {
    NotFound(String),
    Conflict(String),
    Invalid(ValidationError),
    /// Writing the config file failed; nothing was changed in memory.
    Persist(anyhow::Error),
}

impl std::fmt::Display for AppError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AppError::NotFound(id) => write!(f, "service '{id}' not found"),
            AppError::Conflict(id) => write!(f, "service '{id}' already exists"),
            AppError::Invalid(e) => e.fmt(f),
            AppError::Persist(_) => f.write_str("failed to persist configuration"),
        }
    }
}

impl std::error::Error for AppError {}

impl From<ValidationError> for AppError {
    fn from(e: ValidationError) -> Self {
        AppError::Invalid(e)
    }
}
