use std::{collections::HashMap, hash::Hash, sync::Arc, time::Duration};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use tracing;

use crate::telegram::TelegramClient;

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
        tracing::debug!("Starting HTTP check for url: {}", self.url);

        let result = match reqwest::get(&self.url).await {
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
        };

        tracing::debug!(
            "HTTP check for url: {} completed with state: {:?}",
            self.url,
            result
        );
        result
    }
}
#[derive(Deserialize, Serialize, Debug, Clone, Hash)]
pub struct ServiceCertificate {
    pub host: String,
    pub port: u16,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub days_before_expiry: Option<u64>,
}

impl ServiceCertificate {
    pub async fn check(&self) -> State {
        tracing::debug!(
            "Starting certificate check for host: {}:{}",
            self.host,
            self.port
        );

        let result = self.check_certificate().await;

        tracing::debug!(
            "Certificate check for host: {}:{} completed with state: {:?}",
            self.host,
            self.port,
            result
        );
        result
    }

    async fn check_certificate(&self) -> State {
        use native_tls::TlsConnector;
        use tokio::net::TcpStream;

        // Connect to the server
        let addr = format!("{}:{}", self.host, self.port);
        let tcp_stream = match TcpStream::connect(&addr).await {
            Ok(stream) => stream,
            Err(e) => return State::Failure(format!("TCP connection failed: {}", e)),
        };

        // Create TLS connector
        let connector = match TlsConnector::new() {
            Ok(c) => c,
            Err(e) => return State::Failure(format!("Failed to create TLS connector: {}", e)),
        };

        let connector = tokio_native_tls::TlsConnector::from(connector);

        // Perform TLS handshake
        let tls_stream = match connector.connect(&self.host, tcp_stream).await {
            Ok(stream) => stream,
            Err(e) => return State::Failure(format!("TLS handshake failed: {}", e)),
        };

        // Get the peer certificate
        let cert = match tls_stream.get_ref().peer_certificate() {
            Ok(Some(cert)) => cert,
            Ok(None) => return State::Failure("No peer certificate found".to_string()),
            Err(e) => return State::Failure(format!("Failed to get peer certificate: {}", e)),
        };

        // Parse the certificate to get expiration date
        let der = cert.to_der().unwrap();
        let (_, parsed_cert) = match x509_parser::parse_x509_certificate(&der) {
            Ok(result) => result,
            Err(e) => return State::Failure(format!("Failed to parse certificate: {}", e)),
        };

        // Get the not_after timestamp
        let not_after = parsed_cert.validity().not_after;
        let expiry_timestamp = not_after.timestamp();

        // Calculate days until expiration
        let now = chrono::Utc::now().timestamp();
        let seconds_until_expiry = expiry_timestamp - now;
        let days_until_expiry = seconds_until_expiry / 86400; // 86400 seconds in a day

        let threshold = self.days_before_expiry.unwrap_or(30);

        if days_until_expiry < 0 {
            State::Failure(format!("Certificate expired {} days ago", -days_until_expiry))
        } else if days_until_expiry < threshold as i64 {
            State::Failure(format!(
                "Certificate expires in {} days (threshold: {} days)",
                days_until_expiry, threshold
            ))
        } else {
            State::Success
        }
    }
}

#[derive(Deserialize, Serialize, Debug, Clone, Hash)]
pub struct ServiceTcpPing {
    pub host: String,
    pub port: u16,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
}

impl ServiceTcpPing {
    pub async fn check(&self) -> State {
        tracing::debug!("Starting TCP ping for host: {}:{}", self.host, self.port);

        let addr = format!("{}:{}", self.host, self.port);
        let timeout_ms = self.timeout_ms.unwrap_or(1000);
        let timeout = Duration::from_millis(timeout_ms);

        let result =
            match tokio::time::timeout(timeout, tokio::net::TcpStream::connect(&addr)).await {
                Ok(Ok(_)) => State::Success,
                Ok(Err(e)) => State::Failure(format!("Connection failed: {}", e)),
                Err(_) => State::Failure(format!("Timeout after {}ms", timeout_ms)),
            };

        tracing::debug!(
            "TCP ping for host: {}:{} completed with state: {:?}",
            self.host,
            self.port,
            result
        );
        result
    }
}

#[derive(Deserialize, Serialize, Debug, Clone, Hash)]
#[serde(rename_all = "camelCase")]
pub enum CheckType {
    Http(ServiceHttp),
    Certificate(ServiceCertificate),
    #[serde(rename = "tcpPing")]
    TcpPing(ServiceTcpPing),
}

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
    pub check: CheckType,
}

impl Service {
    pub async fn run(&self, id: String, app_state: AppState) {
        loop {
            tracing::info!("Running health check for service: {}", self.name);

            let state = match &self.check {
                CheckType::Certificate(cert) => cert.check().await,
                CheckType::Http(http) => http.check().await,
                CheckType::TcpPing(tcp) => tcp.check().await,
            };

            // Log the result
            match &state {
                State::Success => tracing::info!("Service '{}' check succeeded", self.name),
                State::Failure(reason) => tracing::warn!("Service '{}' check failed: {}", self.name, reason),
                State::Unknown => tracing::info!("Service '{}' check returned unknown state", self.name),
            }

            // Update state in the global store
            app_state.set_state(id.clone(), state.clone()).await;

            // Get global config defaults
            let config = app_state.get_config().await;

            // Determine sleep interval based on state, using service override or global default
            let interval = match &state {
                State::Success => self.check_interval_success.unwrap_or(config.check_interval_success),
                State::Failure(_) => self.check_interval_fail.unwrap_or(config.check_interval_fail),
                State::Unknown => self.check_interval_success.unwrap_or(config.check_interval_success),
            };

            tracing::debug!("Service '{}' next check in {}ms", self.name, interval);
            tokio::time::sleep(Duration::from_millis(interval)).await;
        }
    }
}

// ServiceState represents the current runtime state of a service for API responses
#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct ServiceState {
    pub name: String,
    pub description: String,
    pub state: State,
    pub last_check: DateTime<Utc>,
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
    pub services: HashMap<String, Service>,
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
    services: Arc<RwLock<HashMap<String, ServiceState>>>,
    config: Arc<RwLock<Config>>,
    task_handles: Arc<RwLock<HashMap<String, tokio::task::JoinHandle<()>>>>,
    telegram: Arc<TelegramClient>,
    config_path: Arc<String>,
}

impl AppState {
    pub fn new(config: Config, config_path: String) -> Self {
        let now = Utc::now();
        let services = config
            .services
            .iter()
            .filter(|(_, service)| service.enabled)
            .map(|(id, service)| {
                (
                    id.clone(),
                    ServiceState {
                        name: service.name.clone(),
                        description: service.description.clone(),
                        state: State::Unknown,
                        last_check: now,
                        consecutive_failures: 0,
                        total_checks: 0,
                        successful_checks: 0,
                        failed_checks: 0,
                        uptime_start: None,
                    },
                )
            })
            .collect();

        // Create Telegram client
        let telegram = Arc::new(TelegramClient::new(
            config.telegram_token.clone(),
            config.telegram_chat_id,
        ));

        Self {
            services: Arc::new(RwLock::new(services)),
            config: Arc::new(RwLock::new(config)),
            task_handles: Arc::new(RwLock::new(HashMap::new())),
            telegram,
            config_path: Arc::new(config_path),
        }
    }

    pub async fn set_state(&self, id: String, state: State) {
        // Determine notification action before modifying state
        let notification = {
            let mut services = self.services.write().await;
            if let Some(service_state) = services.get_mut(&id) {
                let now = Utc::now();
                let previous_failures = service_state.consecutive_failures;
                let was_failing = previous_failures > 0;

                service_state.state = state.clone();
                service_state.last_check = now;
                service_state.total_checks += 1;

                let config = self.config.read().await;
                let service = config.services.get(&id);
                let notify_failures = service
                    .and_then(|s| s.notify_failures)
                    .unwrap_or(config.notify_failures);
                let rereport = service
                    .and_then(|s| s.rereport)
                    .unwrap_or(config.rereport);

                let notification = match &state {
                    State::Success => {
                        service_state.consecutive_failures = 0;
                        service_state.successful_checks += 1;

                        // Set uptime_start only on first successful check
                        if service_state.uptime_start.is_none() {
                            service_state.uptime_start = Some(now);
                        }

                        // Send recovery notification if was previously failing
                        if was_failing {
                            Some((service_state.name.clone(), "recovered".to_string(), true))
                        } else {
                            None
                        }
                    }
                    State::Failure(reason) => {
                        service_state.consecutive_failures += 1;
                        service_state.failed_checks += 1;
                        // Clear uptime when service fails
                        service_state.uptime_start = None;

                        // Send alert if consecutive failures reached threshold
                        if service_state.consecutive_failures == notify_failures {
                            Some((service_state.name.clone(), reason.clone(), false))
                        }
                        // Resend alert at rereport intervals
                        else if service_state.consecutive_failures > notify_failures
                            && (service_state.consecutive_failures - notify_failures) % rereport == 0 {
                            Some((service_state.name.clone(), format!("{} (still failing)", reason), false))
                        } else {
                            None
                        }
                    }
                    State::Unknown => None,
                };

                notification
            } else {
                None
            }
        }; // Release locks before sending notification

        // Send notification if needed (outside of locks)
        if let Some((service_name, message, is_recovery)) = notification {
            let result = if is_recovery {
                self.telegram.send_recovery(&service_name, &message).await
            } else {
                self.telegram.send_alert(&service_name, &message).await
            };

            if let Err(e) = result {
                tracing::error!("Failed to send Telegram notification: {}", e);
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

    pub async fn start_monitoring_tasks(&self) {
        let config = self.config.read().await;
        let mut handles = self.task_handles.write().await;

        for (uuid, service) in config.services.iter() {
            if !service.enabled {
                tracing::info!("Service '{}' is disabled, skipping", service.name);
                continue;
            }

            tracing::info!("Starting monitor for service '{}'", service.name);
            let service_clone = service.clone();
            let state_clone = self.clone();
            let id_clone = uuid.clone();

            let handle = tokio::spawn(async move {
                service_clone.run(id_clone, state_clone).await;
            });

            handles.insert(uuid.clone(), handle);
        }
    }

    pub async fn stop_all_tasks(&self) {
        tracing::info!("Stopping all monitoring tasks");
        let mut handles = self.task_handles.write().await;

        for (id, handle) in handles.drain() {
            tracing::debug!("Aborting task for service ID: {}", id);
            handle.abort();
        }
    }

    pub async fn update_config(&self, new_config: Config) -> anyhow::Result<()> {
        tracing::info!("Updating configuration and restarting tasks");

        // Stop all existing tasks
        self.stop_all_tasks().await;

        // Write configuration to file
        tracing::info!("Writing configuration to {}", self.config_path);
        let yaml_content = serde_yaml::to_string(&new_config)?;
        std::fs::write(self.config_path.as_ref(), yaml_content)?;
        tracing::info!("Configuration file updated successfully");

        // Update the configuration
        {
            let mut config = self.config.write().await;
            *config = new_config.clone();
        }

        // Update service states, preserving existing data where possible
        {
            let mut services = self.services.write().await;
            let now = Utc::now();

            // Remove services that no longer exist or are now disabled
            services.retain(|id, _| {
                new_config.services.get(id)
                    .map(|s| s.enabled)
                    .unwrap_or(false)
            });

            // Add or update enabled services only
            for (id, service) in new_config.services.iter().filter(|(_, s)| s.enabled) {
                services.entry(id.clone()).or_insert_with(|| ServiceState {
                    name: service.name.clone(),
                    description: service.description.clone(),
                    state: State::Unknown,
                    last_check: now,
                    consecutive_failures: 0,
                    total_checks: 0,
                    successful_checks: 0,
                    failed_checks: 0,
                    uptime_start: None,
                });

                // Update name and description for existing services
                if let Some(service_state) = services.get_mut(id) {
                    service_state.name = service.name.clone();
                    service_state.description = service.description.clone();
                }
            }
        }

        // Start new tasks
        self.start_monitoring_tasks().await;

        tracing::info!("Configuration updated and tasks restarted");
        Ok(())
    }
}
