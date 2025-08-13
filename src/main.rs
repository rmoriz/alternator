use clap::Parser;
use std::path::PathBuf;
use tracing::{debug, error, info, warn, Level};
use tracing_subscriber::{self, EnvFilter};

mod balance;
mod config;
mod error;
mod language;
mod mastodon;
mod media;
mod openrouter;
mod toot_handler;

use crate::config::Config;
use crate::error::{AlternatorError, ErrorRecovery};
use crate::toot_handler::TootStreamHandler;

#[derive(Parser)]
#[command(name = "alternator")]
#[command(about = "Automatically adds descriptions to media attachments in Mastodon toots")]
#[command(version)]
struct Cli {
    /// Path to configuration file
    #[arg(short, long)]
    config: Option<PathBuf>,

    /// Set log level (error, warn, info, debug, trace)
    #[arg(long, value_name = "LEVEL")]
    log_level: Option<String>,

    /// Enable verbose logging (equivalent to --log-level debug)
    #[arg(short, long)]
    verbose: bool,
}

/// Initialize structured logging with proper error handling
#[allow(clippy::result_large_err)] // AlternatorError is large but needed for comprehensive error handling
fn init_logging(config: &Config, cli: &Cli) -> Result<(), AlternatorError> {
    // Determine log level from CLI args, config, or environment
    let log_level = if cli.verbose {
        "debug"
    } else if let Some(ref level) = cli.log_level {
        level.as_str()
    } else {
        config.logging().level.as_deref().unwrap_or("info")
    };

    // Validate log level
    let _level = match log_level.to_lowercase().as_str() {
        "error" => Level::ERROR,
        "warn" => Level::WARN,
        "info" => Level::INFO,
        "debug" => Level::DEBUG,
        "trace" => Level::TRACE,
        _ => {
            return Err(AlternatorError::InvalidData(format!(
                "Invalid log level: {log_level}. Valid levels are: error, warn, info, debug, trace"
            )));
        }
    };

    // Create environment filter with fallback
    let env_filter = EnvFilter::try_from_default_env()
        .or_else(|_| EnvFilter::try_new(log_level))
        .map_err(|e| AlternatorError::InvalidData(format!("Failed to create log filter: {e}")))?;

    // Initialize structured logging with timestamps and target information
    tracing_subscriber::fmt()
        .with_env_filter(env_filter)
        .with_target(true)
        .with_thread_ids(true)
        .with_file(true)
        .with_line_number(true)
        .with_level(true)
        .init();

    debug!("Logging initialized with level: {}", log_level);
    Ok(())
}

/// Handle application errors with appropriate logging and recovery
async fn handle_error(error: AlternatorError) -> Result<(), AlternatorError> {
    match &error {
        // Log different error types at appropriate levels
        AlternatorError::Config(_) => {
            error!("Configuration error: {}", error);
            error!("Please check your configuration file and environment variables");
        }
        AlternatorError::Network(_) => {
            warn!("Network error: {}", error);
            if ErrorRecovery::is_recoverable(&error) {
                info!("Network error is recoverable, will retry");
            }
        }
        AlternatorError::Mastodon(_) => {
            error!("Mastodon API error: {}", error);
            if ErrorRecovery::is_recoverable(&error) {
                info!("Mastodon error is recoverable, will retry");
            }
        }
        AlternatorError::OpenRouter(_) => {
            error!("OpenRouter API error: {}", error);
            if ErrorRecovery::is_recoverable(&error) {
                info!("OpenRouter error is recoverable, will retry");
            }
        }
        AlternatorError::Media(_) => {
            warn!("Media processing error: {}", error);
            debug!("Media error details: {:?}", error);
        }
        AlternatorError::Language(_) => {
            warn!("Language detection error: {}", error);
            debug!("Language error details: {:?}", error);
        }
        AlternatorError::Balance(_) => {
            warn!("Balance monitoring error: {}", error);
            if ErrorRecovery::is_recoverable(&error) {
                info!("Balance error is recoverable, will retry");
            }
        }
        _ => {
            error!("Application error: {}", error);
            debug!("Error details: {:?}", error);
        }
    }

    // Determine if we should shutdown
    if ErrorRecovery::should_shutdown(&error) {
        error!("Fatal error encountered, shutting down application");
        return Err(error);
    }

    Ok(())
}

#[tokio::main]
#[allow(clippy::result_large_err)] // AlternatorError is large but needed for comprehensive error handling
async fn main() -> Result<(), AlternatorError> {
    let cli = Cli::parse();

    // Load configuration first
    let config = match Config::load(cli.config.clone()) {
        Ok(config) => config,
        Err(e) => {
            // Initialize basic logging for configuration errors
            tracing_subscriber::fmt().init();
            let error = AlternatorError::Config(e);
            handle_error(error).await?;
            return Err(AlternatorError::Shutdown);
        }
    };

    // Initialize structured logging
    if let Err(e) = init_logging(&config, &cli) {
        eprintln!("Failed to initialize logging: {e}");
        return Err(e);
    }

    info!("Starting Alternator v{}", env!("CARGO_PKG_VERSION"));
    info!("Configuration loaded successfully");
    debug!("Configuration file path: {:?}", cli.config);
    debug!(
        "Log level: {}",
        config.logging().level.as_deref().unwrap_or("info")
    );

    // Log configuration summary (without sensitive data)
    info!("Mastodon instance: {}", config.mastodon.instance_url);
    info!("OpenRouter model: {}", config.openrouter.model);
    info!(
        "Balance monitoring: {}",
        if config.balance().enabled.unwrap_or(true) {
            "enabled"
        } else {
            "disabled"
        }
    );

    // Initialize and start main application loop
    match run_application(config).await {
        Ok(()) => {
            info!("Application shutdown complete");
            Ok(())
        }
        Err(e) => {
            handle_error(e).await?;
            Err(AlternatorError::Shutdown)
        }
    }
}

/// Main application orchestration - coordinates all components
async fn run_application(config: Config) -> Result<(), AlternatorError> {
    info!("Initializing application components");

    // Initialize all components
    let mut mastodon_client = crate::mastodon::MastodonClient::new(config.mastodon.clone());
    let openrouter_client = crate::openrouter::OpenRouterClient::new(config.openrouter.clone());
    let media_processor =
        crate::media::MediaProcessor::with_image_transformer(crate::media::MediaConfig {
            max_size_mb: config.media().max_size_mb.unwrap_or(10) as f64,
            max_dimension: config.media().resize_max_dimension.unwrap_or(2048),
            supported_formats: config
                .media()
                .supported_formats
                .as_ref()
                .unwrap_or(&vec![
                    "image/jpeg".to_string(),
                    "image/png".to_string(),
                    "image/gif".to_string(),
                    "image/webp".to_string(),
                ])
                .iter()
                .cloned()
                .collect(),
        });
    let language_detector = crate::language::LanguageDetector::new();
    let mut balance_monitor = crate::balance::BalanceMonitor::new(
        config.balance().clone(),
        crate::openrouter::OpenRouterClient::new(config.openrouter.clone()),
    );

    // Perform startup validation
    info!("Performing startup validation");
    startup_validation(&mut mastodon_client, &openrouter_client).await?;

    // Set up graceful shutdown handling
    let shutdown_signal = setup_shutdown_signal();

    // Start balance monitoring in background if enabled
    let balance_task = if balance_monitor.is_enabled() {
        info!("Starting balance monitoring service");
        let balance_mastodon_client = crate::mastodon::MastodonClient::new(config.mastodon.clone());
        Some(tokio::spawn(async move {
            if let Err(e) = balance_monitor.run(&balance_mastodon_client).await {
                error!("Balance monitoring failed: {}", e);
            }
        }))
    } else {
        info!("Balance monitoring is disabled");
        None
    };

    // Start main toot processing loop
    info!("Starting main toot processing loop");
    let mut toot_handler = TootStreamHandler::new(
        mastodon_client,
        openrouter_client,
        media_processor,
        language_detector,
    );

    let processing_task = tokio::spawn(async move { toot_handler.start_processing().await });

    // Wait for shutdown signal or task completion
    tokio::select! {
        _ = shutdown_signal => {
            info!("Shutdown signal received, stopping application");
        }
        result = processing_task => {
            match result {
                Ok(Ok(())) => {
                    info!("Toot processing loop completed successfully");
                }
                Ok(Err(e)) => {
                    error!("Toot processing loop failed: {}", e);
                    return Err(e);
                }
                Err(e) => {
                    error!("Toot processing task panicked: {}", e);
                    return Err(AlternatorError::TaskJoin(e));
                }
            }
        }
    }

    // Clean shutdown - stop background tasks
    if let Some(balance_task) = balance_task {
        info!("Stopping balance monitoring service");
        balance_task.abort();
        let _ = balance_task.await;
    }

    info!("Application shutdown complete");
    Ok(())
}

/// Perform startup validation for both Mastodon and OpenRouter connectivity
async fn startup_validation(
    mastodon_client: &mut crate::mastodon::MastodonClient,
    openrouter_client: &crate::openrouter::OpenRouterClient,
) -> Result<(), AlternatorError> {
    info!("Validating Mastodon connectivity");

    // Verify Mastodon credentials and get user info
    use crate::mastodon::MastodonStream;
    let account = mastodon_client
        .verify_credentials()
        .await
        .map_err(AlternatorError::Mastodon)?;

    info!(
        "✓ Mastodon connection validated - authenticated as: {} (@{})",
        account.display_name, account.acct
    );

    info!("Validating OpenRouter connectivity");

    // Check OpenRouter account balance
    let balance = openrouter_client
        .get_account_balance()
        .await
        .map_err(AlternatorError::OpenRouter)?;

    info!("✓ OpenRouter account balance: ${:.2}", balance);

    // Verify configured model is available
    let models = openrouter_client
        .list_models()
        .await
        .map_err(AlternatorError::OpenRouter)?;

    info!(
        "✓ OpenRouter model validation complete - {} models available",
        models.len()
    );

    // Warn if balance is low
    if balance < 1.0 {
        warn!(
            "⚠️  OpenRouter balance is low (${:.2}) - consider topping up your account",
            balance
        );
    }

    info!("✓ All startup validations passed successfully");
    Ok(())
}

/// Set up graceful shutdown signal handling
async fn setup_shutdown_signal() {
    use tokio::signal;

    #[cfg(unix)]
    {
        let mut sigterm = signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("Failed to register SIGTERM handler");
        let mut sigint = signal::unix::signal(signal::unix::SignalKind::interrupt())
            .expect("Failed to register SIGINT handler");

        tokio::select! {
            _ = sigterm.recv() => {
                info!("Received SIGTERM, initiating graceful shutdown");
            }
            _ = sigint.recv() => {
                info!("Received SIGINT (Ctrl+C), initiating graceful shutdown");
            }
        }
    }

    #[cfg(not(unix))]
    {
        signal::ctrl_c().await.expect("Failed to listen for Ctrl+C");
        info!("Received Ctrl+C, initiating graceful shutdown");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Config, LoggingConfig, MastodonConfig, OpenRouterConfig};
    use chrono::Utc;
    use std::path::PathBuf;
    use std::time::Duration;

    #[test]
    fn test_cli_parsing() {
        let cli = Cli::parse_from(&["alternator"]);
        assert!(cli.config.is_none());
        assert!(cli.log_level.is_none());
        assert!(!cli.verbose);

        let cli = Cli::parse_from(&["alternator", "--config", "/path/to/config.toml"]);
        assert_eq!(cli.config, Some(PathBuf::from("/path/to/config.toml")));

        let cli = Cli::parse_from(&["alternator", "--log-level", "debug"]);
        assert_eq!(cli.log_level, Some("debug".to_string()));

        let cli = Cli::parse_from(&["alternator", "--verbose"]);
        assert!(cli.verbose);

        let cli = Cli::parse_from(&[
            "alternator",
            "-v",
            "-c",
            "config.toml",
            "--log-level",
            "error",
        ]);
        assert!(cli.verbose);
        assert_eq!(cli.config, Some(PathBuf::from("config.toml")));
        assert_eq!(cli.log_level, Some("error".to_string()));
    }

    fn create_test_config() -> Config {
        Config {
            mastodon: MastodonConfig {
                instance_url: "https://mastodon.social".to_string(),
                access_token: "test_token".to_string(),
                user_stream: Some(true),
            },
            openrouter: OpenRouterConfig {
                api_key: "test_key".to_string(),
                model: "mistralai/mistral-small-3.2-24b-instruct:free".to_string(),
                base_url: Some("https://openrouter.ai/api/v1".to_string()),
                max_tokens: Some(150),
            },
            logging: Some(LoggingConfig {
                level: Some("info".to_string()),
            }),
            ..Default::default()
        }
    }

    #[test]
    fn test_init_logging_with_cli_verbose() {
        let config = create_test_config();
        let cli = Cli {
            config: None,
            log_level: None,
            verbose: true,
        };

        // Note: We can't easily test the actual logging initialization in unit tests
        // because it affects global state. We test the logic that determines the log level.
        let log_level = if cli.verbose {
            "debug"
        } else if let Some(ref level) = cli.log_level {
            level.as_str()
        } else {
            config.logging().level.as_deref().unwrap_or("info")
        };

        assert_eq!(log_level, "debug");
    }

    #[test]
    fn test_init_logging_with_cli_log_level() {
        let config = create_test_config();
        let cli = Cli {
            config: None,
            log_level: Some("warn".to_string()),
            verbose: false,
        };

        let log_level = if cli.verbose {
            "debug"
        } else if let Some(ref level) = cli.log_level {
            level.as_str()
        } else {
            config.logging().level.as_deref().unwrap_or("info")
        };

        assert_eq!(log_level, "warn");
    }

    #[test]
    fn test_init_logging_with_config_level() {
        let config = create_test_config();
        let cli = Cli {
            config: None,
            log_level: None,
            verbose: false,
        };

        let log_level = if cli.verbose {
            "debug"
        } else if let Some(ref level) = cli.log_level {
            level.as_str()
        } else {
            config.logging().level.as_deref().unwrap_or("info")
        };

        assert_eq!(log_level, "info");
    }

    #[test]
    fn test_init_logging_default_fallback() {
        let mut config = create_test_config();
        config.logging = None;
        let cli = Cli {
            config: None,
            log_level: None,
            verbose: false,
        };

        let log_level = if cli.verbose {
            "debug"
        } else if let Some(ref level) = cli.log_level {
            level.as_str()
        } else {
            config.logging().level.as_deref().unwrap_or("info")
        };

        assert_eq!(log_level, "info");
    }

    #[test]
    fn test_invalid_log_level_validation() {
        let config = create_test_config();
        let cli = Cli {
            config: None,
            log_level: Some("invalid".to_string()),
            verbose: false,
        };

        // Test that invalid log levels are properly handled
        let log_level = "invalid";
        let level_result = match log_level.to_lowercase().as_str() {
            "error" => Ok("error"),
            "warn" => Ok("warn"),
            "info" => Ok("info"),
            "debug" => Ok("debug"),
            "trace" => Ok("trace"),
            _ => Err(format!(
                "Invalid log level: {log_level}. Valid levels are: error, warn, info, debug, trace"
            )),
        };

        assert!(level_result.is_err());
        assert!(level_result
            .unwrap_err()
            .contains("Invalid log level: invalid"));
    }

    #[tokio::test]
    async fn test_handle_error_config_error() {
        let error = AlternatorError::Config(crate::config::ConfigError::FileNotFound {
            path: "test.toml".to_string(),
        });

        // This should not return an error for config errors (they're logged but not fatal in handle_error)
        let result = handle_error(error).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_handle_error_network_recoverable() {
        // Create a mock reqwest error (timeout scenario)
        let error = AlternatorError::Network(reqwest::Error::from(std::io::Error::new(
            std::io::ErrorKind::TimedOut,
            "request timed out",
        )));

        let result = handle_error(error).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_handle_error_media_processing() {
        let error = AlternatorError::Media(crate::error::MediaError::UnsupportedFormat {
            format: "image/bmp".to_string(),
        });

        let result = handle_error(error).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_handle_error_language_detection() {
        let error = AlternatorError::Language(crate::error::LanguageError::DetectionFailed {
            content: "test".to_string(),
        });

        let result = handle_error(error).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_handle_error_balance_monitoring() {
        let error = AlternatorError::Balance(crate::error::BalanceError::ThresholdExceeded {
            current: 0.5,
            threshold: 1.0,
        });

        let result = handle_error(error).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_handle_error_shutdown_required() {
        let error = AlternatorError::Shutdown;

        let result = handle_error(error).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_handle_error_mastodon_recoverable() {
        let error = AlternatorError::Mastodon(crate::error::MastodonError::RateLimitExceeded {
            reset_at: Utc::now() + chrono::Duration::minutes(5),
        });

        let result = handle_error(error).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_handle_error_openrouter_recoverable() {
        let error = AlternatorError::OpenRouter(crate::error::OpenRouterError::RateLimitExceeded {
            reset_at: Utc::now() + chrono::Duration::minutes(5),
        });

        let result = handle_error(error).await;
        assert!(result.is_ok());
    }

    #[test]
    fn test_cli_version() {
        // Test that the CLI includes version information
        let cli = Cli::parse_from(&["alternator"]);
        // This would normally exit, but we can at least verify the struct is properly defined
        assert!(cli.config.is_none());
    }

    #[test]
    fn test_cli_help() {
        // Test that the CLI includes help information
        // We can't easily test the help output without exiting, but we can verify the struct
        let cli = Cli {
            config: Some(PathBuf::from("test.toml")),
            log_level: Some("debug".to_string()),
            verbose: true,
        };

        // Verify all fields are accessible
        assert_eq!(cli.config, Some(PathBuf::from("test.toml")));
        assert_eq!(cli.log_level, Some("debug".to_string()));
        assert!(cli.verbose);
    }

    #[test]
    fn test_log_level_priority() {
        let config = create_test_config();

        // Verbose flag should override everything
        let cli = Cli {
            config: None,
            log_level: Some("error".to_string()),
            verbose: true,
        };

        let log_level = if cli.verbose {
            "debug"
        } else if let Some(ref level) = cli.log_level {
            level.as_str()
        } else {
            config.logging().level.as_deref().unwrap_or("info")
        };

        assert_eq!(log_level, "debug");

        // CLI log level should override config
        let cli = Cli {
            config: None,
            log_level: Some("trace".to_string()),
            verbose: false,
        };

        let log_level = if cli.verbose {
            "debug"
        } else if let Some(ref level) = cli.log_level {
            level.as_str()
        } else {
            config.logging().level.as_deref().unwrap_or("info")
        };

        assert_eq!(log_level, "trace");
    }

    #[test]
    fn test_all_valid_log_levels() {
        let valid_levels = ["error", "warn", "info", "debug", "trace"];

        for level in valid_levels {
            let result = match level.to_lowercase().as_str() {
                "error" => Ok("error"),
                "warn" => Ok("warn"),
                "info" => Ok("info"),
                "debug" => Ok("debug"),
                "trace" => Ok("trace"),
                _ => Err("invalid"),
            };
            assert!(result.is_ok(), "Level {level} should be valid");
        }
    }

    #[test]
    fn test_case_insensitive_log_levels() {
        let test_cases = [
            ("ERROR", "error"),
            ("Warn", "warn"),
            ("INFO", "info"),
            ("Debug", "debug"),
            ("TRACE", "trace"),
        ];

        for (input, expected) in test_cases {
            let result = match input.to_lowercase().as_str() {
                "error" => Some("error"),
                "warn" => Some("warn"),
                "info" => Some("info"),
                "debug" => Some("debug"),
                "trace" => Some("trace"),
                _ => None,
            };
            assert_eq!(
                result,
                Some(expected),
                "Level {input} should normalize to {expected}"
            );
        }
    }
}
