use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

use crate::checks::State;
use crate::config::Service;

/// Runtime state of one monitored service.
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
    /// An alert for the current failure streak was delivered to Telegram.
    #[serde(default)]
    pub alerted: bool,
}

impl ServiceState {
    pub fn new(service: &Service, now: DateTime<Utc>) -> Self {
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
            alerted: false,
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
/// A recovery is only announced when `alerted` is set, i.e. an alert for this
/// streak was actually delivered (the caller sets it after a successful send).
/// Keying on the flag rather than on the failure count keeps alerts and
/// recoveries paired even when the alert send failed or `notify_failures`
/// changed mid-streak. An undelivered alert is retried on the next failure.
pub fn apply_check(
    service: &mut ServiceState,
    state: &State,
    now: DateTime<Utc>,
    policy: NotifyPolicy,
) -> Option<Notification> {
    service.state = state.clone();
    service.last_check = now;
    service.total_checks += 1;

    match state {
        State::Success => {
            let previous_failures = std::mem::take(&mut service.consecutive_failures);
            service.successful_checks += 1;
            let failing_since = service.failing_since.take();
            service.uptime_start.get_or_insert(now);
            if !std::mem::take(&mut service.alerted) {
                return None;
            }
            let downtime = failing_since
                .map(|since| format!(" ({})", format_duration(now - since)))
                .unwrap_or_default();
            Some(Notification::Recovery(format!(
                "recovered after {} failed checks{}",
                previous_failures, downtime
            )))
        }
        State::Failure(reason) => {
            service.consecutive_failures += 1;
            service.failed_checks += 1;
            service.uptime_start = None;
            service.failing_since.get_or_insert(now);

            let failures = service.consecutive_failures;
            if !service.alerted {
                (failures >= policy.notify_failures).then(|| Notification::Alert(reason.clone()))
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
            alerted: false,
        }
    }

    const POLICY: NotifyPolicy = NotifyPolicy {
        notify_failures: 3,
        rereport: 10,
    };

    fn fail() -> State {
        State::Failure("boom".into())
    }

    /// Simulates the caller: a returned alert counts as delivered.
    fn check(s: &mut ServiceState, state: &State, policy: NotifyPolicy) -> Option<Notification> {
        let n = apply_check(s, state, Utc::now(), policy);
        if matches!(n, Some(Notification::Alert(_))) {
            s.alerted = true;
        }
        n
    }

    #[test]
    fn single_blip_is_silent() {
        let mut s = fresh();
        assert_eq!(check(&mut s, &State::Success, POLICY), None);
        assert_eq!(check(&mut s, &fail(), POLICY), None);
        assert_eq!(check(&mut s, &State::Success, POLICY), None);
        assert_eq!(s.failed_checks, 1);
        assert_eq!(s.consecutive_failures, 0);
        assert!(s.failing_since.is_none());
    }

    #[test]
    fn streak_below_threshold_never_recovers() {
        let mut s = fresh();
        for _ in 0..2 {
            assert_eq!(check(&mut s, &fail(), POLICY), None);
        }
        assert_eq!(check(&mut s, &State::Success, POLICY), None);
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
        s.alerted = true;
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
        assert!(!s.alerted);
    }

    #[test]
    fn undelivered_alert_is_retried_and_no_recovery_without_it() {
        let mut s = fresh();
        let t0 = Utc::now();
        for _ in 0..2 {
            apply_check(&mut s, &fail(), t0, POLICY);
        }
        // Alert returned but the send failed: `alerted` stays false.
        assert!(apply_check(&mut s, &fail(), t0, POLICY).is_some());
        assert_eq!(
            apply_check(&mut s, &fail(), t0, POLICY),
            Some(Notification::Alert("boom".into()))
        );
        assert_eq!(apply_check(&mut s, &State::Success, t0, POLICY), None);
    }

    #[test]
    fn threshold_lowered_mid_streak_still_alerts() {
        let mut s = fresh();
        for _ in 0..2 {
            assert_eq!(check(&mut s, &fail(), POLICY), None);
        }
        let lowered = NotifyPolicy {
            notify_failures: 2,
            rereport: 10,
        };
        assert_eq!(
            check(&mut s, &fail(), lowered),
            Some(Notification::Alert("boom".into()))
        );
        assert!(matches!(
            check(&mut s, &State::Success, lowered),
            Some(Notification::Recovery(_))
        ));
    }

    #[test]
    fn threshold_raised_after_alert_still_recovers() {
        let mut s = fresh();
        for _ in 0..3 {
            check(&mut s, &fail(), POLICY);
        }
        assert!(s.alerted);
        let raised = NotifyPolicy {
            notify_failures: 50,
            rereport: 10,
        };
        assert!(matches!(
            check(&mut s, &State::Success, raised),
            Some(Notification::Recovery(_))
        ));
    }

    #[test]
    fn rereport_every_n_failures_after_alert() {
        let mut s = fresh();
        let mut alerts = Vec::new();
        for _ in 0..24 {
            if let Some(n) = check(&mut s, &fail(), POLICY) {
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
        assert!(check(&mut s, &fail(), policy).is_some());
        for _ in 0..5 {
            assert_eq!(check(&mut s, &fail(), policy), None);
        }
    }

    #[test]
    fn duration_formatting() {
        assert_eq!(format_duration(Duration::seconds(45)), "45s");
        assert_eq!(format_duration(Duration::seconds(125)), "2m 5s");
        assert_eq!(format_duration(Duration::seconds(3_725)), "1h 2m");
    }
}
