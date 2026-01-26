use std::{collections::HashMap, hash::Hash, time::Duration, sync::Arc};

use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use chrono::{DateTime, Utc};

#[derive(Default, Hash, Deserialize, Serialize, Debug, Clone, PartialEq)]
pub enum State {
    #[default]
    Unknown,
    Success,
    Failure(String),
}

#[derive(Deserialize, Serialize, Debug, Clone, Hash)]
pub struct ServiceHttp {
    pub url: String,
    pub expected_status: Option<u16>,
}

impl ServiceHttp {
    pub async fn check(&self) -> State {
        match reqwest::get(&self.url).await {
            Ok(response) => {
                let status = response.status().as_u16();
                let expected = self.expected_status.unwrap_or(200);
                if status == expected {
                    State::Success
                } else {
                    State::Failure(format!("Unexpected status: {}", status))
                }
            }
            Err(e) => State::Failure(format!("Request failed: {}", e)),
        }
    }
}
#[derive(Deserialize, Serialize, Debug, Clone, Hash)]
pub struct ServiceCertificate {
    pub host: String,
    pub port: u16,
    pub days_before_expiry: u64,
}

impl ServiceCertificate {
    pub async fn check(&self) -> State {
        // TODO: Implement certificate expiration check
        // For now, return Unknown
        State::Unknown
    }
}

#[derive(Deserialize, Serialize, Debug, Clone, Hash)]
pub struct ServiceTcpPing {
    pub host: String,
    pub port: u16,
    pub timeout_ms: u64,
}

impl ServiceTcpPing {
    pub async fn check(&self) -> State {
        let addr = format!("{}:{}", self.host, self.port);
        let timeout = Duration::from_millis(self.timeout_ms);

        match tokio::time::timeout(timeout, tokio::net::TcpStream::connect(&addr)).await {
            Ok(Ok(_)) => State::Success,
            Ok(Err(e)) => State::Failure(format!("Connection failed: {}", e)),
            Err(_) => State::Failure(format!("Timeout after {}ms", self.timeout_ms)),
        }
    }
}

#[derive(Deserialize, Serialize, Debug, Clone, Hash)]
#[serde(rename_all = "PascalCase")]
pub enum CheckType {
    Http(ServiceHttp),
    Certificate(ServiceCertificate),
    #[serde(rename = "TCPPing")]
    TcpPing(ServiceTcpPing),
}

#[derive(Deserialize, Serialize, Debug, Clone, Hash)]
pub struct Service {
    pub enabled: bool,
    pub name: String,
    pub description: String,
    pub check_interval_success: Option<u64>,
    pub check_interval_fail: Option<u64>,
    pub notify_failures: Option<u64>,
    pub rereport: Option<u64>,
    pub check: CheckType,
}

impl Service {
    pub async fn run(&self, uuid: uuid::Uuid, app_state: AppState) {
        loop {
            let state = match &self.check {
                CheckType::Certificate(cert) => cert.check().await,
                CheckType::Http(http) => http.check().await,
                CheckType::TcpPing(tcp) => tcp.check().await,
            };

            // Update state in the global store
            app_state.set_state(uuid, state.clone()).await;

            // Determine sleep interval based on state
            let interval = match &state {
                State::Success => self.check_interval_success.unwrap_or(60000),
                State::Failure(_) => self.check_interval_fail.unwrap_or(10000),
                State::Unknown => self.check_interval_success.unwrap_or(60000),
            };

            tokio::time::sleep(Duration::from_millis(interval)).await;
        }
    }
}

// ServiceState represents the current runtime state of a service for API responses
#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct ServiceState {
    pub id: uuid::Uuid,
    pub name: String,
    pub description: String,
    pub state: State,
    pub last_check: DateTime<Utc>,
    pub enabled: bool,
    pub consecutive_failures: u64,
    pub total_checks: u64,
    pub successful_checks: u64,
    pub failed_checks: u64,
    pub uptime_start: Option<DateTime<Utc>>,
}

// Config represents the application configuration loaded from file
#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct Config {
    pub telegram_token: String,
    pub telegram_chat_id: i64,
    pub check_interval_success: u64,
    pub check_interval_fail: u64,
    pub notify_failures: u64,
    pub rereport: u64,
    pub services: HashMap<uuid::Uuid, Service>,
    pub web_port: Option<u16>,
}

impl Config {
    pub fn load(path: &std::path::Path) -> anyhow::Result<Self> {
        let contents = std::fs::read_to_string(path)?;
        let config: Config = serde_yaml::from_str(&contents)?;
        Ok(config)
    }
}

// AppState manages the runtime state of all services
#[derive(Clone)]
pub struct AppState {
    services: Arc<RwLock<HashMap<uuid::Uuid, ServiceState>>>,
    config: Arc<RwLock<Config>>,
}

impl AppState {
    pub fn new(config: Config) -> Self {
        let now = Utc::now();
        let services = config
            .services
            .iter()
            .map(|(id, service)| {
                (
                    *id,
                    ServiceState {
                        id: *id,
                        name: service.name.clone(),
                        description: service.description.clone(),
                        state: State::Unknown,
                        last_check: now,
                        enabled: service.enabled,
                        consecutive_failures: 0,
                        total_checks: 0,
                        successful_checks: 0,
                        failed_checks: 0,
                        uptime_start: None,
                    },
                )
            })
            .collect();

        Self {
            services: Arc::new(RwLock::new(services)),
            config: Arc::new(RwLock::new(config)),
        }
    }

    pub async fn set_state(&self, uuid: uuid::Uuid, state: State) {
        let mut services = self.services.write().await;
        if let Some(service_state) = services.get_mut(&uuid) {
            let now = Utc::now();
            service_state.state = state.clone();
            service_state.last_check = now;
            service_state.total_checks += 1;

            match state {
                State::Success => {
                    service_state.consecutive_failures = 0;
                    service_state.successful_checks += 1;

                    // Set uptime_start only on first successful check
                    if service_state.uptime_start.is_none() {
                        service_state.uptime_start = Some(now);
                    }
                }
                State::Failure(_) => {
                    service_state.consecutive_failures += 1;
                    service_state.failed_checks += 1;
                    // Clear uptime when service fails
                    service_state.uptime_start = None;
                }
                State::Unknown => {}
            }
        }
    }

    pub async fn get_all_services(&self) -> Vec<ServiceState> {
        let services = self.services.read().await;
        let mut result: Vec<ServiceState> = services.values().cloned().collect();
        result.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        result
    }

    pub async fn get_config(&self) -> Config {
        self.config.read().await.clone()
    }
}
