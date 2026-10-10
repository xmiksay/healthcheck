use std::{collections::HashMap, sync::Arc};

use chrono::Utc;
use tokio::sync::RwLock;

use crate::checks::State;
use crate::config::Config;
use crate::notify::{apply_check, Notification, NotifyPolicy, ServiceState};
use crate::telegram::TelegramClient;

/// Shared runtime state: config, live per-service state and the monitor task handles.
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
            .map(|(id, service)| (id.clone(), ServiceState::new(service, now)))
            .collect();

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
        // Decide under the locks, send outside of them.
        let notification = {
            let mut services = self.services.write().await;
            let Some(service_state) = services.get_mut(&id) else {
                return;
            };
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

            apply_check(service_state, &state, Utc::now(), policy)
                .map(|n| (service_state.name.clone(), n, chat_id))
        };

        if let Some((service_name, notification, chat_id)) = notification {
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

    pub async fn get_all_services(&self) -> Vec<ServiceState> {
        let services = self.services.read().await;
        let mut result: Vec<ServiceState> = services.values().cloned().collect();
        result.sort_by_key(|s| s.name.to_lowercase());
        result
    }

    pub async fn get_config(&self) -> Config {
        self.config.read().await.clone()
    }

    pub async fn start_monitoring_tasks(&self) {
        let config = self.config.read().await;
        let mut handles = self.task_handles.write().await;

        for (id, service) in config.services.iter() {
            if !service.enabled {
                tracing::info!("Service '{}' is disabled, skipping", service.name);
                continue;
            }

            tracing::info!("Starting monitor for service '{}'", service.name);
            let service = service.clone();
            let state = self.clone();
            let task_id = id.clone();
            let handle = tokio::spawn(async move { service.run(task_id, state).await });
            handles.insert(id.clone(), handle);
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
        self.stop_all_tasks().await;

        tracing::info!("Writing configuration to {}", self.config_path);
        let yaml_content = serde_yaml::to_string(&new_config)?;
        std::fs::write(self.config_path.as_ref(), yaml_content)?;

        {
            let mut config = self.config.write().await;
            *config = new_config.clone();
        }

        // Keep counters of services that survive the update; drop removed/disabled ones.
        {
            let mut services = self.services.write().await;
            let now = Utc::now();
            services.retain(|id, _| {
                new_config
                    .services
                    .get(id)
                    .map(|s| s.enabled)
                    .unwrap_or(false)
            });
            for (id, service) in new_config.services.iter().filter(|(_, s)| s.enabled) {
                let entry = services
                    .entry(id.clone())
                    .or_insert_with(|| ServiceState::new(service, now));
                entry.name = service.name.clone();
                entry.description = service.description.clone();
            }
        }

        self.start_monitoring_tasks().await;
        tracing::info!("Configuration updated and tasks restarted");
        Ok(())
    }
}
