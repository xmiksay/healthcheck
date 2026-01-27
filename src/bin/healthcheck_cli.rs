use std::path::Path;
use clap::{Parser, Subcommand};
use healthcheck::{Config, TelegramClient};

const CONFIG_ENV: &str = "HEALTHCHECK_CONFIG";
const CONFIG_VAL: &str = "healthcheck.yaml";

#[derive(Parser)]
#[command(name = "healthcheck_cli")]
#[command(about = "Health check CLI tool for testing services and sending notifications")]
struct Cli {
    /// Path to configuration file
    #[arg(short, long, default_value = CONFIG_VAL)]
    config: String,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Send a Telegram message
    Telegram {
        /// Message type: success or error
        #[arg(value_parser = ["success", "error"])]
        message_type: String,

        /// Message text to send
        message: String,
    },

    /// Test a service by ID
    TestService {
        /// ID of the service to test
        id: String,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize basic tracing
    tracing_subscriber::fmt::init();

    let cli = Cli::parse();

    // Load configuration - check environment variable first
    let config_path = if cli.config == CONFIG_VAL {
        std::env::var(CONFIG_ENV).unwrap_or_else(|_| CONFIG_VAL.to_string())
    } else {
        cli.config
    };
    let config = Config::load(Path::new(&config_path))?;

    match &cli.command {
        Commands::Telegram { message_type, message } => {
            handle_telegram_command(&config, message_type, message).await?;
        }
        Commands::TestService { id } => {
            handle_test_service_command(&config, id).await?;
        }
    }

    Ok(())
}

async fn handle_telegram_command(
    config: &Config,
    message_type: &str,
    message: &str,
) -> anyhow::Result<()> {
    let telegram = TelegramClient::new(
        config.telegram_token.clone(),
        config.telegram_chat_id,
    );

    match message_type {
        "success" => {
            telegram.send_recovery("CLI", message).await?;
            println!("Success message sent to Telegram");
        }
        "error" => {
            telegram.send_alert("CLI", message).await?;
            println!("Error message sent to Telegram");
        }
        _ => {
            anyhow::bail!("Invalid message type: {}", message_type);
        }
    }

    Ok(())
}

async fn handle_test_service_command(
    config: &Config,
    id: &str,
) -> anyhow::Result<()> {
    // Find service in config
    let service = config
        .services
        .get(id)
        .ok_or_else(|| anyhow::anyhow!("Service with ID '{}' not found", id))?;

    println!("Testing service: {}", service.name);
    println!("Description: {}", service.description);

    if !service.enabled {
        println!("Warning: Service is disabled in configuration");
    }

    // Run the check
    use healthcheck::config::{CheckType, State};
    let state = match &service.check {
        CheckType::Certificate(cert) => cert.check().await,
        CheckType::Http(http) => http.check().await,
        CheckType::TcpPing(tcp) => tcp.check().await,
    };

    // Display result
    match state {
        State::Success => {
            println!("✓ Service check PASSED");
            Ok(())
        }
        State::Failure(reason) => {
            println!("✗ Service check FAILED: {}", reason);
            std::process::exit(1);
        }
        State::Unknown => {
            println!("? Service check returned UNKNOWN state");
            std::process::exit(2);
        }
    }
}
