use std::{collections::HashMap, time::Duration};

use anyhow::Context;
use serde::{Deserialize, Serialize};

use crate::checks::{CheckType, State};
use crate::state::AppState;
use crate::validate::validate_config;

#[derive(Deserialize, Serialize, Debug, Clone, Hash)]
pub struct Service {
    pub enabled: bool,
    pub name: String,
    pub description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub check_interval_success: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub check_interval_fail: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notify_failures: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rereport: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub telegram_chat_id: Option<i64>,
    pub check: CheckType,
}

impl Service {
    /// Per-service monitor loop: check, record, sleep — forever.
    pub async fn run(&self, id: String, app_state: AppState) {
        loop {
            tracing::debug!("Running health check for service: {}", self.name);
            let state = self.check.run().await;

            match &state {
                State::Success => tracing::debug!("Service '{}' check succeeded", self.name),
                State::Failure(reason) => {
                    tracing::warn!("Service '{}' check failed: {}", self.name, reason)
                }
                State::Unknown => {
                    tracing::info!("Service '{}' check returned unknown state", self.name)
                }
            }

            app_state.set_state(id.clone(), state.clone()).await;

            let config = app_state.get_config().await;
            let interval = match &state {
                State::Failure(_) => self
                    .check_interval_fail
                    .unwrap_or(config.check_interval_fail),
                State::Success | State::Unknown => self
                    .check_interval_success
                    .unwrap_or(config.check_interval_success),
            };

            tracing::debug!("Service '{}' next check in {}ms", self.name, interval);
            tokio::time::sleep(Duration::from_millis(interval)).await;
        }
    }
}

/// Application configuration as loaded from (and re-serialized to) the YAML file.
#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct Config {
    pub telegram_token: String,
    pub telegram_chat_id: i64,
    pub check_interval_success: u64,
    pub check_interval_fail: u64,
    pub notify_failures: u64,
    pub rereport: u64,
    pub services: HashMap<String, Service>,
    pub web_port: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_bearer_token: Option<String>,
}

/// The globals editable through the API. Secrets and `web_port` are file-only.
#[derive(Deserialize, Serialize, Debug, Clone, PartialEq)]
pub struct Settings {
    pub check_interval_success: u64,
    pub check_interval_fail: u64,
    pub notify_failures: u64,
    pub rereport: u64,
    pub telegram_chat_id: i64,
}

impl Config {
    /// Reads, parses and validates the config; any rule violation fails startup.
    pub fn load(path: &std::path::Path) -> anyhow::Result<Self> {
        let contents = std::fs::read_to_string(path)
            .with_context(|| format!("failed to read config {}", path.display()))?;
        let config: Config = serde_yaml::from_str(&contents)
            .with_context(|| format!("failed to parse config {}", path.display()))?;
        validate_config(&config).with_context(|| format!("invalid config {}", path.display()))?;
        Ok(config)
    }

    pub fn settings(&self) -> Settings {
        Settings {
            check_interval_success: self.check_interval_success,
            check_interval_fail: self.check_interval_fail,
            notify_failures: self.notify_failures,
            rereport: self.rereport,
            telegram_chat_id: self.telegram_chat_id,
        }
    }

    pub fn apply_settings(&mut self, s: Settings) {
        self.check_interval_success = s.check_interval_success;
        self.check_interval_fail = s.check_interval_fail;
        self.notify_failures = s.notify_failures;
        self.rereport = s.rereport;
        self.telegram_chat_id = s.telegram_chat_id;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_example_config_with_timeouts() {
        let yaml = r#"
telegram_token: t
telegram_chat_id: 1
check_interval_success: 300000
check_interval_fail: 60000
notify_failures: 3
rereport: 100
services:
  web:
    enabled: true
    name: Web
    description: ''
    check: !http
      url: https://example.com
      expected_status: 200
      timeout_ms: 5000
  cert:
    enabled: true
    name: Cert
    description: ''
    check: !certificate
      host: example.com
      port: 443
      days_before_expiry: 21
"#;
        let config: Config = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.services.len(), 2);
        let CheckType::Http(http) = &config.services["web"].check else {
            panic!("expected http")
        };
        assert_eq!(http.timeout_ms, Some(5000));
        let CheckType::Certificate(cert) = &config.services["cert"].check else {
            panic!("expected certificate")
        };
        assert_eq!(cert.timeout_ms, None);
    }

    #[test]
    fn load_fails_fast_on_invalid_config() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bad.yaml");
        std::fs::write(
            &path,
            "telegram_token: t\ntelegram_chat_id: 1\ncheck_interval_success: 5\n\
             check_interval_fail: 60000\nnotify_failures: 3\nrereport: 0\nservices: {}\n",
        )
        .unwrap();
        let err = Config::load(&path).unwrap_err();
        assert!(
            format!("{err:#}").contains("check_interval_success: must be >= 1000"),
            "{err:#}"
        );
    }
}
