mod mutate;

use std::{collections::HashMap, path::PathBuf, sync::Arc};

use chrono::Utc;
use serde::{Deserialize, Serialize};
use tokio::sync::{Mutex, RwLock};

use crate::checks::{CheckType, State};
use crate::config::{Config, Service, Settings};
use crate::error::AppError;
use crate::notify::{apply_check, Notification, NotifyPolicy, ServiceState};
use crate::telegram::TelegramClient;
use crate::validate::validate_check;

/// A configured service together with its live status (`None` while disabled).
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ServiceEntry {
    pub id: String,
    #[serde(flatten)]
    pub service: Service,
    pub status: Option<ServiceState>,
}

/// Shared runtime state: config, live per-service state and the monitor task handles.
///
/// Locking rule: never hold two of the `RwLock`s at once (clone and release), so
/// readers, monitor loops and the mutation path cannot deadlock each other.
#[derive(Clone)]
pub struct AppState {
    services: Arc<RwLock<HashMap<String, ServiceState>>>,
    config: Arc<RwLock<Config>>,
    task_handles: Arc<Mutex<HashMap<String, tokio::task::JoinHandle<()>>>>,
    /// Serializes every config mutation (validate → persist → swap → restart).
    write_lock: Arc<Mutex<()>>,
    telegram: Arc<TelegramClient>,
    config_path: Arc<PathBuf>,
}

impl AppState {
    pub fn new(config: Config, config_path: impl Into<PathBuf>) -> Self {
        let services = config
            .services
            .iter()
            .filter(|(_, service)| service.enabled)
            .map(|(id, service)| (id.clone(), ServiceState::new(id, service)))
            .collect();

        let telegram = Arc::new(TelegramClient::new(
            config.telegram_token.clone(),
            config.telegram_chat_id,
        ));

        Self {
            services: Arc::new(RwLock::new(services)),
            config: Arc::new(RwLock::new(config)),
            task_handles: Arc::new(Mutex::new(HashMap::new())),
            write_lock: Arc::new(Mutex::new(())),
            telegram,
            config_path: Arc::new(config_path.into()),
        }
    }

    pub async fn set_state(&self, id: String, state: State) {
        let (policy, chat_id) = {
            let config = self.config.read().await;
            let service = config.services.get(&id);
            let policy = NotifyPolicy {
                notify_failures: service
                    .and_then(|s| s.notify_failures)
                    .unwrap_or(config.notify_failures),
                rereport: service.and_then(|s| s.rereport).unwrap_or(config.rereport),
            };
            let chat_id = service
                .and_then(|s| s.telegram_chat_id)
                .unwrap_or(config.telegram_chat_id);
            (policy, chat_id)
        };

        // Decide under the lock, send outside of it.
        let notification = {
            let mut services = self.services.write().await;
            let Some(service_state) = services.get_mut(&id) else {
                return;
            };
            apply_check(service_state, &state, Utc::now(), policy)
                .map(|n| (service_state.name.clone(), n))
        };

        if let Some((service_name, notification)) = notification {
            tracing::info!("Service '{}': {:?}", service_name, notification);
            let result = match &notification {
                Notification::Recovery(msg) => {
                    self.telegram
                        .send_recovery(chat_id, &service_name, msg)
                        .await
                }
                Notification::Alert(msg) => {
                    self.telegram.send_alert(chat_id, &service_name, msg).await
                }
            };
            match result {
                Ok(()) if matches!(notification, Notification::Alert(_)) => {
                    if let Some(s) = self.services.write().await.get_mut(&id) {
                        s.alerted = true;
                    }
                }
                Ok(()) => {}
                Err(e) => tracing::error!("Failed to send Telegram notification: {}", e),
            }
        }
    }

    /// Snapshot of the whole config (used by the monitor loops each cycle).
    pub async fn get_config(&self) -> Config {
        self.config.read().await.clone()
    }

    /// The admin token, `None` when unset or blank (admin API disabled).
    pub async fn api_bearer_token(&self) -> Option<String> {
        self.config
            .read()
            .await
            .api_bearer_token
            .clone()
            .filter(|t| !t.trim().is_empty())
    }

    /// Live status of all enabled services, sorted by name (case-insensitive).
    pub async fn status(&self) -> Vec<ServiceState> {
        let mut result: Vec<ServiceState> = self.services.read().await.values().cloned().collect();
        result.sort_by_key(|s| s.name.to_lowercase());
        result
    }

    /// All configured services (including disabled), sorted by name (case-insensitive).
    pub async fn list_services(&self) -> Vec<ServiceEntry> {
        let config = self.get_config().await;
        let services = self.services.read().await;
        let mut result: Vec<ServiceEntry> = config
            .services
            .into_iter()
            .map(|(id, service)| ServiceEntry {
                status: services.get(&id).cloned(),
                id,
                service,
            })
            .collect();
        result.sort_by_key(|e| e.service.name.to_lowercase());
        result
    }

    pub async fn get_service(&self, id: &str) -> Result<ServiceEntry, AppError> {
        let service = self.configured_service(id).await?;
        Ok(self.entry(id, service).await)
    }

    pub async fn get_settings(&self) -> Settings {
        self.config.read().await.settings()
    }

    /// Runs the saved check of `id` once; the result is not recorded in the stats.
    pub async fn run_service_check(&self, id: &str) -> Result<State, AppError> {
        let service = self.configured_service(id).await?;
        Ok(service.check.run().await)
    }

    /// Validates and runs an unsaved check once.
    pub async fn test_check(&self, check: CheckType) -> Result<State, AppError> {
        validate_check(&check)?;
        Ok(check.run().await)
    }

    pub async fn start_monitoring_tasks(&self) {
        let config = self.get_config().await;
        for (id, service) in &config.services {
            if service.enabled {
                self.restart_task(id, Some(service)).await;
            } else {
                tracing::info!("Service '{}' is disabled, skipping", service.name);
            }
        }
    }

    async fn configured_service(&self, id: &str) -> Result<Service, AppError> {
        self.config
            .read()
            .await
            .services
            .get(id)
            .cloned()
            .ok_or_else(|| AppError::NotFound(id.to_string()))
    }

    async fn entry(&self, id: &str, service: Service) -> ServiceEntry {
        ServiceEntry {
            id: id.to_string(),
            status: self.services.read().await.get(id).cloned(),
            service,
        }
    }

    /// Brings the live status and the monitor task of `id` in line with `service`
    /// (`None` = deleted). Counters of a service that stays enabled are kept.
    async fn apply_live(&self, id: &str, service: Option<&Service>) {
        let enabled = service.filter(|s| s.enabled);
        {
            let mut services = self.services.write().await;
            match enabled {
                Some(s) => {
                    let entry = services
                        .entry(id.to_string())
                        .or_insert_with(|| ServiceState::new(id, s));
                    entry.name = s.name.clone();
                    entry.description = s.description.clone();
                }
                None => {
                    services.remove(id);
                }
            }
        }
        self.restart_task(id, enabled).await;
    }

    /// Aborts the task of `id` (if any) and spawns a new one when `service` is given.
    async fn restart_task(&self, id: &str, service: Option<&Service>) {
        let mut handles = self.task_handles.lock().await;
        if let Some(handle) = handles.remove(id) {
            tracing::debug!("Stopping monitor for service ID: {}", id);
            handle.abort();
        }
        if let Some(service) = service {
            tracing::info!("Starting monitor for service '{}'", service.name);
            let service = service.clone();
            let state = self.clone();
            let task_id = id.to_string();
            let handle = tokio::spawn(async move { service.run(task_id, state).await });
            handles.insert(id.to_string(), handle);
        }
    }
}
