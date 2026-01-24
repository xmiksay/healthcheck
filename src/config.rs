use std::{collections::HashMap, hash::Hash};

use serde::{Deserialize, Serialize};

#[derive(Default, Hash, Deserialize, Serialize, Debug, Clone)]
pub enum State {
    #[default]
    Unknown,
    Success,
    Failure(Option<String>),
}

#[derive(Deserialize, Serialize, Debug, Clone, Hash)]
pub struct ServiceHttp {}

impl ServiceHttp {
    pub async fn check(&self) -> State {
        State::Unknown
    }
}
#[derive(Deserialize, Serialize, Debug, Clone, Hash)]
pub struct ServiceCertificate {}

impl ServiceCertificate {
    pub async fn check(&self) -> State {
        State::Unknown
    }
}

#[derive(Deserialize, Serialize, Debug, Clone, Hash)]
pub struct ServiceTcpPing {}

impl ServiceTcpPing {
    pub async fn check(&self) -> State {
        State::Unknown
    }
}

#[derive(Deserialize, Serialize, Debug, Clone, Hash)]
pub enum ServiceItem {
    Http(ServiceHttp),
    Certificate(ServiceCertificate),
    TCPPing(ServiceTcpPing),
}

#[derive(Deserialize, Serialize, Debug, Clone, Hash)]
pub struct Service {
    pub enabled: bool,
    pub uptime: Option<u64>,
    pub name: String,
    pub description: String,
    pub state: State,
    pub check_interval_success: Option<u64>,
    pub check_interval_fail: Option<u64>,
    pub notify_failures: Option<u64>,
    pub rereport: Option<u64>,
    pub item: ServiceItem,
}

impl Service {
    pub async fn run(&self, uuid: uuid::Uuid) {
        loop {
            let state = match &self.item {
                ServiceItem::Certificate(cert) => cert.check().await,
                ServiceItem::Http(http) => http.check().await,
                ServiceItem::TCPPing(tcp) => tcp.check().await,
            };

            Config::set_state(uuid, state);
        }
    }
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct Config {
    telegram_token: String,
    telegram_chat_id: i64,
    check_interval_success: u64,
    check_interval_fail: u64,
    notify_failures: u64,
    rereport: u64,
    services: HashMap<uuid::Uuid, Service>,
}

impl Config {
    const SINGLETON: Option<Self> = None;

    pub fn set_state(uuid: uuid::Uuid, state: State) {}

    pub fn load(path: &std::path::Path) -> anyhow::Result<Self> {
        Err(anyhow::anyhow!("Error"))
    }
}
