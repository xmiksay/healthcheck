//! Input validation shared by `Config::load`, the HTTP API and (later) MCP.
//!
//! Every rule reports into one list so callers see all offending fields at once.

use std::fmt;

use crate::checks::CheckType;
use crate::config::{Config, Service, Settings};

const MIN_INTERVAL_MS: u64 = 1000;
const TIMEOUT_RANGE_MS: std::ops::RangeInclusive<u64> = 100..=300_000;
const STATUS_RANGE: std::ops::RangeInclusive<u16> = 100..=599;
const MAX_ID_LEN: usize = 64;

/// All validation failures, each formatted as `field: message`.
#[derive(Debug, Clone, PartialEq)]
pub struct ValidationError(pub Vec<String>);

impl fmt::Display for ValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0.join("; "))
    }
}

impl std::error::Error for ValidationError {}

struct Errors<'a> {
    prefix: &'a str,
    list: &'a mut Vec<String>,
}

impl Errors<'_> {
    fn push(&mut self, field: &str, msg: &str) {
        self.list.push(format!("{}{}: {}", self.prefix, field, msg));
    }

    fn check(&mut self, ok: bool, field: &str, msg: &str) {
        if !ok {
            self.push(field, msg);
        }
    }
}

fn finish(list: Vec<String>) -> Result<(), ValidationError> {
    if list.is_empty() {
        Ok(())
    } else {
        Err(ValidationError(list))
    }
}

pub fn validate_config(config: &Config) -> Result<(), ValidationError> {
    let mut list = Vec::new();
    settings_rules(
        &config.settings(),
        &mut Errors {
            prefix: "",
            list: &mut list,
        },
    );
    // Sorted so the startup message is deterministic.
    let mut ids: Vec<&String> = config.services.keys().collect();
    ids.sort();
    for id in ids {
        if let Some(service) = config.services.get(id) {
            let prefix = format!("services.{id}.");
            service_rules(
                id,
                service,
                &mut Errors {
                    prefix: &prefix,
                    list: &mut list,
                },
            );
        }
    }
    finish(list)
}

pub fn validate_settings(settings: &Settings) -> Result<(), ValidationError> {
    let mut list = Vec::new();
    settings_rules(
        settings,
        &mut Errors {
            prefix: "",
            list: &mut list,
        },
    );
    finish(list)
}

pub fn validate_service(id: &str, service: &Service) -> Result<(), ValidationError> {
    let mut list = Vec::new();
    service_rules(
        id,
        service,
        &mut Errors {
            prefix: "",
            list: &mut list,
        },
    );
    finish(list)
}

pub fn validate_check(check: &CheckType) -> Result<(), ValidationError> {
    let mut list = Vec::new();
    check_rules(
        check,
        &mut Errors {
            prefix: "",
            list: &mut list,
        },
    );
    finish(list)
}

/// `^[a-z0-9][a-z0-9_-]{0,63}$`
pub fn is_valid_id(id: &str) -> bool {
    let mut chars = id.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    id.len() <= MAX_ID_LEN
        && (first.is_ascii_lowercase() || first.is_ascii_digit())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
}

fn settings_rules(s: &Settings, e: &mut Errors) {
    interval_rule(Some(s.check_interval_success), "check_interval_success", e);
    interval_rule(Some(s.check_interval_fail), "check_interval_fail", e);
    notify_rule(Some(s.notify_failures), "notify_failures", e);
}

fn service_rules(id: &str, s: &Service, e: &mut Errors) {
    e.check(
        is_valid_id(id),
        "id",
        "must match ^[a-z0-9][a-z0-9_-]{0,63}$",
    );
    e.check(!s.name.trim().is_empty(), "name", "must not be empty");
    interval_rule(s.check_interval_success, "check_interval_success", e);
    interval_rule(s.check_interval_fail, "check_interval_fail", e);
    notify_rule(s.notify_failures, "notify_failures", e);
    let prefix = format!("{}check.", e.prefix);
    check_rules(
        &s.check,
        &mut Errors {
            prefix: &prefix,
            list: e.list,
        },
    );
}

fn interval_rule(value: Option<u64>, field: &str, e: &mut Errors) {
    if let Some(v) = value {
        e.check(v >= MIN_INTERVAL_MS, field, "must be >= 1000");
    }
}

fn notify_rule(value: Option<u64>, field: &str, e: &mut Errors) {
    if let Some(v) = value {
        e.check(v >= 1, field, "must be >= 1");
    }
}

fn check_rules(check: &CheckType, e: &mut Errors) {
    match check {
        CheckType::Http(http) => {
            let url_ok = reqwest::Url::parse(&http.url)
                .map(|u| matches!(u.scheme(), "http" | "https"))
                .unwrap_or(false);
            e.check(url_ok, "http.url", "must be an http(s) URL");
            if let Some(status) = http.expected_status {
                e.check(
                    STATUS_RANGE.contains(&status),
                    "http.expected_status",
                    "must be between 100 and 599",
                );
            }
            timeout_rule(http.timeout_ms, "http.timeout_ms", e);
        }
        CheckType::Certificate(cert) => {
            host_port_rules(&cert.host, cert.port, "certificate", e);
            timeout_rule(cert.timeout_ms, "certificate.timeout_ms", e);
        }
        CheckType::TcpPing(tcp) => {
            host_port_rules(&tcp.host, tcp.port, "tcpPing", e);
            timeout_rule(tcp.timeout_ms, "tcpPing.timeout_ms", e);
        }
    }
}

fn host_port_rules(host: &str, port: u16, kind: &str, e: &mut Errors) {
    e.check(
        !host.trim().is_empty(),
        &format!("{kind}.host"),
        "must not be empty",
    );
    e.check(
        port != 0,
        &format!("{kind}.port"),
        "must be between 1 and 65535",
    );
}

fn timeout_rule(value: Option<u64>, field: &str, e: &mut Errors) {
    if let Some(v) = value {
        e.check(
            TIMEOUT_RANGE_MS.contains(&v),
            field,
            "must be between 100 and 300000",
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::checks::{ServiceCertificate, ServiceHttp, ServiceTcpPing};

    fn http(url: &str) -> CheckType {
        CheckType::Http(ServiceHttp {
            url: url.into(),
            expected_status: None,
            timeout_ms: None,
        })
    }

    fn service(check: CheckType) -> Service {
        Service {
            enabled: true,
            name: "Web".into(),
            description: String::new(),
            check_interval_success: None,
            check_interval_fail: None,
            notify_failures: None,
            rereport: None,
            telegram_chat_id: None,
            check,
        }
    }

    #[test]
    fn id_rules() {
        for ok in ["a", "0", "web-1", "db_primary", &"a".repeat(64)] {
            assert!(is_valid_id(ok), "{ok}");
        }
        for bad in ["", "-a", "_a", "Web", "a b", "a.b", "č", &"a".repeat(65)] {
            assert!(!is_valid_id(bad), "{bad}");
        }
    }

    #[test]
    fn valid_service_passes() {
        assert_eq!(
            validate_service("web", &service(http("https://example.com"))),
            Ok(())
        );
    }

    #[test]
    fn collects_every_error() {
        let mut s = service(CheckType::Http(ServiceHttp {
            url: "ftp://example.com".into(),
            expected_status: Some(99),
            timeout_ms: Some(50),
        }));
        s.name = "  ".into();
        s.check_interval_fail = Some(999);
        s.check_interval_success = Some(1000);
        s.notify_failures = Some(0);
        let err = validate_service("Bad", &s).unwrap_err();
        assert_eq!(
            err.0,
            vec![
                "id: must match ^[a-z0-9][a-z0-9_-]{0,63}$",
                "name: must not be empty",
                "check_interval_fail: must be >= 1000",
                "notify_failures: must be >= 1",
                "check.http.url: must be an http(s) URL",
                "check.http.expected_status: must be between 100 and 599",
                "check.http.timeout_ms: must be between 100 and 300000",
            ]
        );
        assert!(err.to_string().contains("; name: must not be empty; "));
    }

    #[test]
    fn host_port_and_timeout_rules() {
        let cert = CheckType::Certificate(ServiceCertificate {
            host: " ".into(),
            port: 0,
            days_before_expiry: None,
            timeout_ms: Some(300_001),
        });
        assert_eq!(
            validate_check(&cert).unwrap_err().0,
            vec![
                "certificate.host: must not be empty",
                "certificate.port: must be between 1 and 65535",
                "certificate.timeout_ms: must be between 100 and 300000",
            ]
        );
        let tcp = CheckType::TcpPing(ServiceTcpPing {
            host: "localhost".into(),
            port: 22,
            timeout_ms: Some(100),
        });
        assert_eq!(validate_check(&tcp), Ok(()));
        assert!(validate_check(&http("not a url")).is_err());
        assert!(validate_check(&http("http://127.0.0.1:8080/x")).is_ok());
    }

    #[test]
    fn settings_rules_apply() {
        let s = Settings {
            check_interval_success: 999,
            check_interval_fail: 1000,
            notify_failures: 0,
            rereport: 0,
            telegram_chat_id: 1,
        };
        assert_eq!(
            validate_settings(&s).unwrap_err().0,
            vec![
                "check_interval_success: must be >= 1000",
                "notify_failures: must be >= 1",
            ]
        );
    }

    #[test]
    fn config_errors_are_prefixed_with_service_id() {
        let yaml = include_str!("../healthcheck.yaml.example");
        let mut config: Config = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(validate_config(&config), Ok(()));

        config.check_interval_fail = 10;
        config
            .services
            .insert("Bad".into(), service(http("https://example.com")));
        assert_eq!(
            validate_config(&config).unwrap_err().0,
            vec![
                "check_interval_fail: must be >= 1000",
                "services.Bad.id: must match ^[a-z0-9][a-z0-9_-]{0,63}$",
            ]
        );
    }
}
