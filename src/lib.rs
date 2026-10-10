pub mod checks;
pub mod config;
pub mod state;
pub mod telegram;
pub mod web;

pub use checks::{CheckType, State};
pub use config::Config;
pub use state::AppState;
pub use telegram::TelegramClient;

pub const CONFIG_ENV: &str = "HEALTHCHECK_CONFIG";
pub const CONFIG_VAL: &str = "healthcheck.yaml";
