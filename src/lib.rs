pub mod checks;
pub mod config;
pub mod error;
pub mod mcp;
pub mod notify;
pub mod state;
pub mod telegram;
pub mod validate;
pub mod web;

pub use checks::{CheckType, State};
pub use config::{Config, Service, Settings};
pub use error::AppError;
pub use state::{AppState, ServiceEntry};
pub use telegram::TelegramClient;

pub const CONFIG_ENV: &str = "HEALTHCHECK_CONFIG";
pub const CONFIG_VAL: &str = "healthcheck.yaml";
