pub mod config;
pub mod telegram;
pub mod web;

pub use config::{AppState, CheckType, Config, State};
pub use telegram::TelegramClient;

pub const CONFIG_ENV: &str = "HEALTHCHECK_CONFIG";
pub const CONFIG_VAL: &str = "healthcheck.yaml";
