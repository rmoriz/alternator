use clap::Parser;
use std::path::PathBuf;
use tracing::{info, error, debug, warn, Level};
use tracing_subscriber::{self, EnvFilter};

mod config;
mod error;
mod mastodon;
mod openrouter;
mod media;
mod language;
mod balance;

use crate::config::Config;
use crate::error::{AlternatorError, ErrorRecovery};

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
            return Err(AlternatorError::InvalidData(
                format!("Invalid log level: {}. Valid levels are: error, warn, info, debug, trace", log_level)
            ));
        }
    };
    
    // Create environment filter with fallback
    let env_filter = EnvFilter::try_from_default_env()
        .or_else(|_| EnvFilter::try_new(log_level))
        .map_err(|e| AlternatorError::InvalidData(
            format!("Failed to create log filter: {}", e)
        ))?;
    
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
        eprintln!("Failed to initialize logging: {}", e);
        return Err(e);
    }
    
    info!("Starting Alternator v{}", env!("CARGO_PKG_VERSION"));
    info!("Configuration loaded successfully");
    debug!("Configuration file path: {:?}", cli.config);
    debug!("Log level: {}", config.logging().level.as_deref().unwrap_or("info"));
    
    // Log configuration summary (without sensitive data)
    info!("Mastodon instance: {}", config.mastodon.instance_url);
    info!("OpenRouter model: {}", config.openrouter.model);
    info!("Balance monitoring: {}", 
        if config.balance().enabled.unwrap_or(true) { "enabled" } else { "disabled" }
    );
    
    // TODO: Initialize and start main application loop
    info!("Alternator initialized - ready to process toots");
    
    // Simulate some logging at different levels for demonstration
    debug!("Debug logging is working");
    info!("Application startup complete");
    
    Ok(())
}