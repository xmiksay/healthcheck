use std::path::Path;
use tracing_subscriber::prelude::*;

use healthcheck::{AppState, Config};

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
    app_state.start_monitoring_tasks().await;

    // Start web server
    let web_port = config.web_port.unwrap_or(8080);
    let web_state = app_state.clone();

    let web_handle = tokio::spawn(async move {
        if let Err(e) = healthcheck::web::start_server(web_state, web_port).await {
            tracing::error!("Web server error: {}", e);
        }
    });

    // Wait for web server (monitoring tasks run indefinitely in background)
    web_handle.await?;

    Ok(())
}
