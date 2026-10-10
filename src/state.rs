use std::{collections::HashMap, sync::Arc};

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

use crate::checks::State;
use crate::config::{Config, Service};
use crate::telegram::TelegramClient;

/// Runtime state of one monitored service, as exposed by `GET /api/services`.
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
    #[serde(default)]
    pub failing_since: Option<DateTime<Utc>>,
}

impl ServiceState {
    fn new(service: &Service, now: DateTime<Utc>) -> Self {
        Self {
            name: service.name.clone(),
            description: service.description.clone(),
            state: State::Unknown,
            last_check: now,
            consecutive_failures: 0,
            total_checks: 0,
            successful_checks: 0,
            failed_checks: 0,
            uptime_start: None,
            failing_since: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Notification {
    Alert(String),
    Recovery(String),
}

/// Thresholds that decide when a state change turns into a Telegram message.
#[derive(Debug, Clone, Copy)]
pub struct NotifyPolicy {
    pub notify_failures: u64,
    pub rereport: u64,
}

/// Applies a check result to the service state and decides whether to notify.
///
/// A recovery is only announced when an alert was actually sent (the failure
/// streak reached `notify_failures`); shorter blips are counted but stay silent.
pub fn apply_check(
    service: &mut ServiceState,
    state: &State,
    now: DateTime<Utc>,
    policy: NotifyPolicy,
) -> Option<Notification> {
    let previous_failures = service.consecutive_failures;
    service.state = state.clone();
    service.last_check = now;
    service.total_checks += 1;

    match state {
        State::Success => {
            service.consecutive_failures = 0;
            service.successful_checks += 1;
            let failing_since = service.failing_since.take();
            if service.uptime_start.is_none() {
                service.uptime_start = Some(now);
            }
            if previous_failures >= policy.notify_failures {
                let downtime = failing_since
                    .map(|since| format!(" ({})", format_duration(now - since)))
                    .unwrap_or_default();
                Some(Notification::Recovery(format!(
                    "recovered after {} failed checks{}",
                    previous_failures, downtime
                )))
            } else {
                None
            }
        }
        State::Failure(reason) => {
            service.consecutive_failures += 1;
            service.failed_checks += 1;
            service.uptime_start = None;
            service.failing_since.get_or_insert(now);

            let failures = service.consecutive_failures;
            if failures == policy.notify_failures {
                Some(Notification::Alert(reason.clone()))
            } else if failures > policy.notify_failures
                && policy.rereport > 0
                && (failures - policy.notify_failures).is_multiple_of(policy.rereport)
            {
                Some(Notification::Alert(format!("{} (still failing)", reason)))
            } else {
                None
            }
        }
        State::Unknown => None,
    }
}

fn format_duration(d: Duration) -> String {
    let secs = d.num_seconds().max(0);
    if secs < 60 {
        format!("{}s", secs)
    } else if secs < 3600 {
        format!("{}m {}s", secs / 60, secs % 60)
    } else {
        format!("{}h {}m", secs / 3600, (secs % 3600) / 60)
    }
}

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
            if let Err(e) = result {
                tracing::error!("Failed to send Telegram notification: {}", e);
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

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh() -> ServiceState {
        ServiceState {
            name: "svc".into(),
            description: String::new(),
            state: State::Unknown,
            last_check: Utc::now(),
            consecutive_failures: 0,
            total_checks: 0,
            successful_checks: 0,
            failed_checks: 0,
            uptime_start: None,
            failing_since: None,
        }
    }

    const POLICY: NotifyPolicy = NotifyPolicy {
        notify_failures: 3,
        rereport: 10,
    };

    fn fail() -> State {
        State::Failure("boom".into())
    }

    #[test]
    fn single_blip_is_silent() {
        let mut s = fresh();
        let t0 = Utc::now();
        assert_eq!(apply_check(&mut s, &State::Success, t0, POLICY), None);
        assert_eq!(apply_check(&mut s, &fail(), t0, POLICY), None);
        assert_eq!(apply_check(&mut s, &State::Success, t0, POLICY), None);
        assert_eq!(s.failed_checks, 1);
        assert_eq!(s.consecutive_failures, 0);
        assert!(s.failing_since.is_none());
    }

    #[test]
    fn alert_then_recovery_with_duration() {
        let mut s = fresh();
        let t0 = Utc::now();
        assert_eq!(apply_check(&mut s, &fail(), t0, POLICY), None);
        assert_eq!(apply_check(&mut s, &fail(), t0, POLICY), None);
        assert_eq!(
            apply_check(&mut s, &fail(), t0, POLICY),
            Some(Notification::Alert("boom".into()))
        );
        assert_eq!(s.failing_since, Some(t0));
        assert!(s.uptime_start.is_none());

        let t1 = t0 + Duration::seconds(150);
        assert_eq!(
            apply_check(&mut s, &State::Success, t1, POLICY),
            Some(Notification::Recovery(
                "recovered after 3 failed checks (2m 30s)".into()
            ))
        );
        assert_eq!(s.uptime_start, Some(t1));
        assert!(s.failing_since.is_none());
    }

    #[test]
    fn rereport_every_n_failures_after_alert() {
        let mut s = fresh();
        let t0 = Utc::now();
        let mut alerts = Vec::new();
        for _ in 0..24 {
            if let Some(n) = apply_check(&mut s, &fail(), t0, POLICY) {
                alerts.push((s.consecutive_failures, n));
            }
        }
        assert_eq!(
            alerts,
            vec![
                (3, Notification::Alert("boom".into())),
                (13, Notification::Alert("boom (still failing)".into())),
                (23, Notification::Alert("boom (still failing)".into())),
            ]
        );
    }

    #[test]
    fn rereport_zero_never_repeats() {
        let policy = NotifyPolicy {
            notify_failures: 1,
            rereport: 0,
        };
        let mut s = fresh();
        let t0 = Utc::now();
        assert!(apply_check(&mut s, &fail(), t0, policy).is_some());
        for _ in 0..5 {
            assert_eq!(apply_check(&mut s, &fail(), t0, policy), None);
        }
    }

    #[test]
    fn duration_formatting() {
        assert_eq!(format_duration(Duration::seconds(45)), "45s");
        assert_eq!(format_duration(Duration::seconds(125)), "2m 5s");
        assert_eq!(format_duration(Duration::seconds(3_725)), "1h 2m");
    }
}
