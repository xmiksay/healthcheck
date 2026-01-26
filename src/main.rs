use std::path::Path;
use tracing_subscriber::prelude::*;

mod config;
mod web;

use config::{AppState, Config};

const CONFIG_ENV: &str = "HEALTHCHECK_CONFIG";
const CONFIG_VAL: &str = "healthcheck.yaml";

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize tracing
    let fmt_layer = tracing_subscriber::fmt::layer();
    let rust_tls = tracing_subscriber::filter::Targets::new()
        .with_target("rustls", tracing::Level::ERROR)
        .with_default(tracing_subscriber::fmt::Subscriber::DEFAULT_MAX_LEVEL);

    tracing_subscriber::registry()
        .with(fmt_layer)
        .with(rust_tls)
        .init();

    // Load configuration
    let config_path = std::env::var(CONFIG_ENV).unwrap_or_else(|_| CONFIG_VAL.to_string());
    let config = Config::load(Path::new(&config_path))?;

    tracing::info!("Loaded configuration from {}", config_path);
    tracing::info!("Monitoring {} services", config.services.len());

    // Create application state
    let app_state = AppState::new(config.clone());

    // Start service monitoring tasks
    let mut handles = vec![];

    for (uuid, service) in config.services.iter() {
        if !service.enabled {
            tracing::info!("Service '{}' is disabled, skipping", service.name);
            continue;
        }

        tracing::info!("Starting monitor for service '{}'", service.name);
        let service_clone = service.clone();
        let state_clone = app_state.clone();
        let uuid_clone = *uuid;

        let handle = tokio::spawn(async move {
            service_clone.run(uuid_clone, state_clone).await;
        });

        handles.push(handle);
    }

    // Start web server
    let web_port = config.web_port.unwrap_or(8080);
    let web_state = app_state.clone();

    let web_handle = tokio::spawn(async move {
        if let Err(e) = web::start_server(web_state, web_port).await {
            tracing::error!("Web server error: {}", e);
        }
    });

    handles.push(web_handle);

    // Wait for all tasks
    futures::future::join_all(handles).await;

    Ok(())
}
