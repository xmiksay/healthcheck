use std::time::Duration;

use serde::{Deserialize, Serialize};

/// Default per-check timeout for HTTP and certificate checks.
pub const DEFAULT_TIMEOUT_MS: u64 = 10_000;

#[derive(Default, Hash, Deserialize, Serialize, Debug, Clone, PartialEq)]
pub enum State {
    #[default]
    Unknown,
    Success,
    Failure(String),
}

/// Renders an error together with its whole `source()` chain.
///
/// reqwest/hyper errors only say "error sending request" at the top level; the
/// actionable part (DNS failure, connection refused, timeout) is buried in the
/// sources, and that is what an alert must show.
pub fn error_chain(err: &(dyn std::error::Error + 'static)) -> String {
    let mut out = err.to_string();
    let mut source = err.source();
    while let Some(s) = source {
        let text = s.to_string();
        if !out.contains(&text) {
            out.push_str(": ");
            out.push_str(&text);
        }
        source = s.source();
    }
    out
}

#[derive(Deserialize, Serialize, Debug, Clone, Hash)]
pub struct ServiceHttp {
    pub url: String,
    pub expected_status: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
}

impl ServiceHttp {
    pub async fn check(&self) -> State {
        let timeout = Duration::from_millis(self.timeout_ms.unwrap_or(DEFAULT_TIMEOUT_MS));
        let client = match reqwest::Client::builder()
            .timeout(timeout)
            .connect_timeout(timeout)
            .build()
        {
            Ok(client) => client,
            Err(e) => return State::Failure(format!("Client setup failed: {}", error_chain(&e))),
        };

        match client.get(&self.url).send().await {
            Ok(response) => {
                let status = response.status().as_u16();
                let expected = self.expected_status.unwrap_or(200);
                if status == expected {
                    State::Success
                } else {
                    State::Failure(format!("Unexpected status: {}", status))
                }
            }
            Err(e) if e.is_timeout() => State::Failure(format!(
                "Timeout after {}ms: {}",
                timeout.as_millis(),
                error_chain(&e)
            )),
            Err(e) => State::Failure(format!("Request failed: {}", error_chain(&e))),
        }
    }
}

#[derive(Deserialize, Serialize, Debug, Clone, Hash)]
pub struct ServiceCertificate {
    pub host: String,
    pub port: u16,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub days_before_expiry: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
}

impl ServiceCertificate {
    pub async fn check(&self) -> State {
        let timeout_ms = self.timeout_ms.unwrap_or(DEFAULT_TIMEOUT_MS);
        match tokio::time::timeout(Duration::from_millis(timeout_ms), self.days_until_expiry())
            .await
        {
            Ok(Ok(days)) => self.evaluate(days),
            Ok(Err(reason)) => State::Failure(reason),
            Err(_) => State::Failure(format!("Timeout after {}ms", timeout_ms)),
        }
    }

    fn evaluate(&self, days_until_expiry: i64) -> State {
        let threshold = self.days_before_expiry.unwrap_or(30) as i64;
        if days_until_expiry < 0 {
            State::Failure(format!(
                "Certificate expired {} days ago",
                -days_until_expiry
            ))
        } else if days_until_expiry < threshold {
            State::Failure(format!(
                "Certificate expires in {} days (threshold: {} days)",
                days_until_expiry, threshold
            ))
        } else {
            State::Success
        }
    }

    /// Connects, handshakes and returns the number of days until the leaf certificate expires.
    async fn days_until_expiry(&self) -> Result<i64, String> {
        use rustls::pki_types::ServerName;
        use std::sync::Arc;
        use tokio::net::TcpStream;
        use tokio_rustls::TlsConnector;

        let addr = format!("{}:{}", self.host, self.port);
        let tcp_stream = TcpStream::connect(&addr)
            .await
            .map_err(|e| format!("TCP connection failed: {}", error_chain(&e)))?;

        // Mozilla's bundle plus the OS store, so internal/custom CAs trusted by the OS work too.
        let mut root_cert_store = rustls::RootCertStore::empty();
        root_cert_store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        let native = rustls_native_certs::load_native_certs();
        for e in &native.errors {
            tracing::warn!("Could not load native CA cert: {}", e);
        }
        for cert in native.certs {
            if let Err(e) = root_cert_store.add(cert) {
                tracing::debug!("Skipping native CA cert: {}", e);
            }
        }

        let config = rustls::ClientConfig::builder()
            .with_root_certificates(root_cert_store)
            .with_no_client_auth();
        let connector = TlsConnector::from(Arc::new(config));

        let server_name = ServerName::try_from(self.host.as_str())
            .map_err(|e| format!("Invalid DNS name: {}", e))?
            .to_owned();

        let tls_stream = connector
            .connect(server_name, tcp_stream)
            .await
            .map_err(|e| format!("TLS handshake failed: {}", error_chain(&e)))?;

        let (_, connection) = tls_stream.get_ref();
        let der = connection
            .peer_certificates()
            .and_then(|certs| certs.first())
            .ok_or_else(|| "No peer certificates found".to_string())?;

        let (_, parsed_cert) = x509_parser::parse_x509_certificate(der.as_ref())
            .map_err(|e| format!("Failed to parse certificate: {}", e))?;

        let expiry = parsed_cert.validity().not_after.timestamp();
        let now = chrono::Utc::now().timestamp();
        Ok((expiry - now) / 86_400)
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
        let addr = format!("{}:{}", self.host, self.port);
        let timeout_ms = self.timeout_ms.unwrap_or(1000);
        let timeout = Duration::from_millis(timeout_ms);

        match tokio::time::timeout(timeout, tokio::net::TcpStream::connect(&addr)).await {
            Ok(Ok(_)) => State::Success,
            Ok(Err(e)) => State::Failure(format!("Connection failed: {}", error_chain(&e))),
            Err(_) => State::Failure(format!("Timeout after {}ms", timeout_ms)),
        }
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

impl CheckType {
    pub async fn run(&self) -> State {
        match self {
            CheckType::Certificate(cert) => cert.check().await,
            CheckType::Http(http) => http.check().await,
            CheckType::TcpPing(tcp) => tcp.check().await,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fmt;

    #[derive(Debug)]
    struct Layer(&'static str, Option<Box<Layer>>);

    impl fmt::Display for Layer {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str(self.0)
        }
    }

    impl std::error::Error for Layer {
        fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
            self.1
                .as_deref()
                .map(|l| l as &(dyn std::error::Error + 'static))
        }
    }

    #[test]
    fn error_chain_joins_all_sources() {
        let err = Layer(
            "error sending request",
            Some(Box::new(Layer(
                "client error (Connect)",
                Some(Box::new(Layer("dns error: failed to lookup address", None))),
            ))),
        );
        assert_eq!(
            error_chain(&err),
            "error sending request: client error (Connect): dns error: failed to lookup address"
        );
    }

    #[test]
    fn error_chain_skips_duplicated_text() {
        let err = Layer("outer: inner", Some(Box::new(Layer("inner", None))));
        assert_eq!(error_chain(&err), "outer: inner");
    }

    #[test]
    fn certificate_evaluation_thresholds() {
        let cert = ServiceCertificate {
            host: "example.com".into(),
            port: 443,
            days_before_expiry: Some(21),
            timeout_ms: None,
        };
        assert_eq!(cert.evaluate(60), State::Success);
        assert_eq!(cert.evaluate(21), State::Success);
        assert!(
            matches!(cert.evaluate(20), State::Failure(ref r) if r.contains("expires in 20 days"))
        );
        assert!(
            matches!(cert.evaluate(-3), State::Failure(ref r) if r.contains("expired 3 days ago"))
        );
    }

    #[test]
    fn timeout_ms_is_optional_and_omitted_when_unset() {
        let yaml = "!http\nurl: https://example.com\nexpected_status: 200\n";
        let check: CheckType = serde_yaml::from_str(yaml).unwrap();
        let CheckType::Http(http) = &check else {
            panic!("expected http")
        };
        assert_eq!(http.timeout_ms, None);
        assert!(!serde_yaml::to_string(&check)
            .unwrap()
            .contains("timeout_ms"));

        let yaml = "!certificate\nhost: example.com\nport: 443\ntimeout_ms: 2500\n";
        let check: CheckType = serde_yaml::from_str(yaml).unwrap();
        let CheckType::Certificate(cert) = &check else {
            panic!("expected certificate")
        };
        assert_eq!(cert.timeout_ms, Some(2500));
    }
}
