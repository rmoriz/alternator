use clap::Parser;
use std::path::PathBuf;
use tracing::{debug, error, info, warn, Level};
use tracing_subscriber::{self, EnvFilter};

mod backfill;
mod balance;
mod config;
mod error;
mod language;
mod mastodon;
mod media;
mod openrouter;
mod toot_handler;
mod whisper_cli;

use crate::backfill::BackfillProcessor;
use crate::config::{Config, RuntimeConfig};
use crate::error::{AlternatorError, ErrorRecovery};
use crate::toot_handler::TootStreamHandler;

#[derive(Parser)]
#[command(name = "alternator")]
#[command(about = "Automatically adds descriptions to media attachments in Mastodon toots")]
#[command(version)]
struct Cli {
    /// Path to configuration file (can also be set via ALTERNATOR_CONFIG env var)
    #[arg(short, long)]
    config: Option<PathBuf>,

    /// Set log level (error, warn, info, debug, trace)
    #[arg(long, value_name = "LEVEL")]
    log_level: Option<String>,

    /// Enable verbose logging (equivalent to --log-level debug)
    #[arg(short, long)]
    verbose: bool,
}

impl Cli {
    /// Get config path from CLI arg or ALTERNATOR_CONFIG environment variable
    fn config_path(&self) -> Option<PathBuf> {
        self.config
            .clone()
            .or_else(|| std::env::var("ALTERNATOR_CONFIG").ok().map(PathBuf::from))
    }
}

/// Initialize structured logging with proper error handling
#[allow(clippy::result_large_err)] // AlternatorError is large but needed for comprehensive error handling
fn init_logging(config: &RuntimeConfig, cli: &Cli) -> Result<(), AlternatorError> {
    // Determine log level from CLI args, config, or environment
    let log_level = if cli.verbose {
        "debug"
    } else if let Some(ref level) = cli.log_level {
        level.as_str()
    } else {
        config.config().logging().level.as_deref().unwrap_or("info")
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
    let base_config = match Config::load(cli.config_path()) {
        Ok(config) => config,
        Err(e) => {
            // Initialize basic logging for configuration errors
            tracing_subscriber::fmt().init();
            let error = AlternatorError::Config(e);
            handle_error(error).await?;
            return Err(AlternatorError::Shutdown);
        }
    };

    // Create runtime config with FFmpeg/audio availability check
    let config = RuntimeConfig::new(base_config);

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
        config.config().logging().level.as_deref().unwrap_or("info")
    );

    // Log configuration summary (without sensitive data)
    info!(
        "Mastodon instance: {}",
        config.config().mastodon.instance_url
    );
    info!("OpenRouter model: {}", config.config().openrouter.model);
    info!(
        "Balance monitoring: {}",
        if config.config().balance().enabled.unwrap_or(true) {
            "enabled"
        } else {
            "disabled"
        }
    );

    // Log audio/FFmpeg status
    if config.is_audio_enabled() {
        info!("✓ Audio transcription: enabled (FFmpeg available)");
    } else if crate::media::is_ffmpeg_available() {
        info!("Audio transcription: disabled (Whisper disabled in config)");
    } else {
        warn!("Audio transcription: disabled (FFmpeg not found in PATH)");
        info!("To enable audio transcription, install FFmpeg and enable Whisper in config");
    }

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
async fn run_application(config: RuntimeConfig) -> Result<(), AlternatorError> {
    info!("Initializing application components");

    // Initialize all components
    let mut mastodon_client =
        crate::mastodon::MastodonClient::new(config.config().mastodon.clone());
    let openrouter_client =
        crate::openrouter::OpenRouterClient::new(config.config().openrouter.clone());
    let media_processor =
        crate::media::MediaProcessor::with_image_transformer(crate::media::MediaConfig {
            max_size_mb: config.config().media().max_size_mb.unwrap_or(10) as f64,
            max_dimension: config.config().media().resize_max_dimension.unwrap_or(2048),
            supported_formats: config
                .config()
                .media()
                .supported_formats
                .as_ref()
                .map(|formats| formats.iter().cloned().collect())
                .unwrap_or_else(|| {
                    // Use default supported formats from MediaConfig to avoid hardcoding
                    crate::media::MediaConfig::default().supported_formats
                }),
        });
    let language_detector = crate::language::LanguageDetector::new();
    let mut balance_monitor = crate::balance::BalanceMonitor::new(
        config.config().balance().clone(),
        crate::openrouter::OpenRouterClient::new(config.config().openrouter.clone()),
    );

    // Perform startup validation
    info!("Performing startup validation");
    startup_validation(&mut mastodon_client, &openrouter_client).await?;

    // Check and download Whisper model if needed
    if config.is_audio_enabled() {
        info!("Checking Whisper model availability");
        check_whisper_model(&config).await?;
    }

    // Set up graceful shutdown handling
    let shutdown_signal = setup_shutdown_signal();

    // Process backfill if enabled
    if let Err(e) =
        BackfillProcessor::process_backfill(config.config(), &mastodon_client, &toot_handler).await
    {
        warn!("Backfill processing failed: {}", e);
        // Don't fail startup if backfill fails - just log and continue
    }

    // Start balance monitoring in background if enabled
    let balance_task = if balance_monitor.is_enabled() {
        info!("Starting balance monitoring service");
        let balance_mastodon_client =
            crate::mastodon::MastodonClient::new(config.config().mastodon.clone());
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
        config,
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

/// Check Whisper model availability and preload if configured
async fn check_whisper_model(config: &RuntimeConfig) -> Result<(), AlternatorError> {
    let whisper_config = config.config().whisper();

    // Skip if Whisper is disabled
    if !whisper_config.enabled.unwrap_or(false) {
        return Ok(());
    }

    let model_name = whisper_config.model.as_deref().unwrap_or("base");
    info!("Initializing Whisper CLI with model: {}", model_name);

    // Create WhisperCli instance to validate configuration
    let whisper_cli =
        crate::whisper_cli::WhisperCli::new(whisper_config).map_err(AlternatorError::Media)?;

    info!(
        "✓ Whisper CLI initialized - Model: {}, Device: {}",
        whisper_cli.model(),
        whisper_cli.device()
    );

    // Preload model if configured to do so
    if whisper_config.preload.unwrap_or(true) {
        info!("Preloading Whisper model for faster transcriptions...");
        whisper_cli
            .preload_model()
            .await
            .map_err(AlternatorError::Media)?;
        info!("✓ Whisper model preloaded successfully");
    } else {
        info!("Whisper model preloading disabled - models will be loaded on demand");
    }

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
    use std::path::PathBuf;

    #[allow(dead_code)]
    fn create_test_config() -> Config {
        Config {
            mastodon: MastodonConfig {
                instance_url: "https://mastodon.example".to_string(),
                access_token: "test_token".to_string(),
                user_stream: Some(true),
                backfill_count: Some(25),
                backfill_pause: Some(60),
            },
            openrouter: OpenRouterConfig {
                api_key: "test_key".to_string(),
                model: "mistralai/mistral-small-3.2-24b-instruct:free".to_string(),
                vision_model: "mistralai/mistral-small-3.2-24b-instruct:free".to_string(),
                vision_fallback_model: "google/gemma-3-27b-it:free".to_string(),
                text_model: "mistralai/mistral-small-3.2-24b-instruct:free".to_string(),
                text_fallback_model: "moonshotai/kimi-k2:free".to_string(),
                base_url: Some("https://openrouter.ai/api/v1".to_string()),
                max_tokens: Some(150),
            },
            media: None,
            balance: None,
            logging: Some(LoggingConfig {
                level: Some("info".to_string()),
            }),
            whisper: None,
        }
    }

    #[test]
    fn test_cli_parsing() {
        let cli = Cli::parse_from(["alternator"]);
        assert!(cli.config.is_none());
        assert!(cli.log_level.is_none());
        assert!(!cli.verbose);

        let cli = Cli::parse_from(["alternator", "--config", "/path/to/config.toml"]);
        assert_eq!(cli.config, Some(PathBuf::from("/path/to/config.toml")));

        let cli = Cli::parse_from(["alternator", "--log-level", "debug"]);
        assert_eq!(cli.log_level, Some("debug".to_string()));

        let cli = Cli::parse_from(["alternator", "--verbose"]);
        assert!(cli.verbose);
    }

    #[test]
    fn test_alternator_config_env_var() {
        // Test that ALTERNATOR_CONFIG environment variable is used when no CLI arg provided
        std::env::set_var("ALTERNATOR_CONFIG", "/env/path/to/config.toml");

        let cli = Cli::parse_from(["alternator"]);
        assert_eq!(
            cli.config_path(),
            Some(PathBuf::from("/env/path/to/config.toml"))
        );

        // Clean up
        std::env::remove_var("ALTERNATOR_CONFIG");

        // Test that CLI arg overrides environment variable
        std::env::set_var("ALTERNATOR_CONFIG", "/env/path/to/config.toml");

        let cli = Cli::parse_from(["alternator", "--config", "/cli/path/to/config.toml"]);
        assert_eq!(
            cli.config_path(),
            Some(PathBuf::from("/cli/path/to/config.toml"))
        );

        // Clean up
        std::env::remove_var("ALTERNATOR_CONFIG");

        // Test that no config is returned when neither CLI arg nor env var is set
        let cli = Cli::parse_from(["alternator"]);
        assert_eq!(cli.config_path(), None);
    }
}
