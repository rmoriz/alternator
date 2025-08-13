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
    let media_processor = crate::media::MediaProcessor::with_image_transformer(
        crate::media::MediaConfig {
            max_size_mb: config.media().max_size_mb.unwrap_or(10) as f64,
            max_dimension: config.media().resize_max_dimension.unwrap_or(1024),
            supported_formats: config.media().supported_formats.as_ref()
                .unwrap_or(&vec![
                    "image/jpeg".to_string(),
                    "image/png".to_string(),
                    "image/gif".to_string(),
                    "image/webp".to_string(),
                ])
                .iter()
                .cloned()
                .collect(),
        }
    );
    let language_detector = crate::language::LanguageDetector::new();
    let mut balance_monitor = crate::balance::BalanceMonitor::new(
        config.balance().clone(),
        crate::openrouter::OpenRouterClient::new(config.openrouter.clone())
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
    let processing_task = tokio::spawn(async move {
        toot_processing_loop(mastodon_client, openrouter_client, media_processor, language_detector).await
    });
    
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
    let account = mastodon_client.verify_credentials().await
        .map_err(|e| AlternatorError::Mastodon(e))?;
    
    info!("✓ Mastodon connection validated - authenticated as: {} (@{})", 
          account.display_name, account.acct);
    
    info!("Validating OpenRouter connectivity");
    
    // Check OpenRouter account balance
    let balance = openrouter_client.get_account_balance().await
        .map_err(|e| AlternatorError::OpenRouter(e))?;
    
    info!("✓ OpenRouter account balance: ${:.2}", balance);
    
    // Verify configured model is available
    let models = openrouter_client.list_models().await
        .map_err(|e| AlternatorError::OpenRouter(e))?;
    
    info!("✓ OpenRouter model validation complete - {} models available", models.len());
    
    // Warn if balance is low
    if balance < 1.0 {
        warn!("⚠️  OpenRouter balance is low (${:.2}) - consider topping up your account", balance);
    }
    
    info!("✓ All startup validations passed successfully");
    Ok(())
}

/// Main toot processing loop that handles incoming toots
async fn toot_processing_loop(
    mut mastodon_client: crate::mastodon::MastodonClient,
    openrouter_client: crate::openrouter::OpenRouterClient,
    media_processor: crate::media::MediaProcessor,
    language_detector: crate::language::LanguageDetector,
) -> Result<(), AlternatorError> {
    use crate::mastodon::MastodonStream;
    
    info!("Connecting to Mastodon WebSocket stream");
    mastodon_client.connect().await
        .map_err(|e| AlternatorError::Mastodon(e))?;
    
    info!("✓ Connected to Mastodon stream - listening for toots");
    
    let mut processed_toots = std::collections::HashSet::new();
    
    loop {
        // Listen for toot events
        match mastodon_client.listen().await {
            Ok(Some(toot)) => {
                // Check if we've already processed this toot
                if processed_toots.contains(&toot.id) {
                    debug!("Skipping already processed toot: {}", toot.id);
                    continue;
                }
                
                info!("Processing toot: {} (media: {})", toot.id, toot.media_attachments.len());
                
                // Process the toot
                match process_toot(&toot, &mastodon_client, &openrouter_client, &media_processor, &language_detector).await {
                    Ok(()) => {
                        processed_toots.insert(toot.id.clone());
                        info!("✓ Successfully processed toot: {}", toot.id);
                    }
                    Err(e) => {
                        // Log error but continue processing other toots
                        error!("Failed to process toot {}: {}", toot.id, e);
                        
                        // Still mark as processed to avoid retry loops
                        processed_toots.insert(toot.id.clone());
                        
                        // Handle specific error types
                        if let Err(handle_err) = handle_error(e).await {
                            // If handle_error returns an error, it means we should shutdown
                            return Err(handle_err);
                        }
                    }
                }
            }
            Ok(None) => {
                // No toot received, continue listening
                continue;
            }
            Err(e) => {
                error!("Error listening for toots: {}", e);
                
                // Handle the error and determine if we should continue
                if let Err(handle_err) = handle_error(AlternatorError::Mastodon(e)).await {
                    return Err(handle_err);
                }
                
                // If error is recoverable, the connection will be re-established automatically
                continue;
            }
        }
    }
}

/// Process a single toot - check for media, generate descriptions, and update
async fn process_toot(
    toot: &crate::mastodon::TootEvent,
    mastodon_client: &crate::mastodon::MastodonClient,
    openrouter_client: &crate::openrouter::OpenRouterClient,
    media_processor: &crate::media::MediaProcessor,
    language_detector: &crate::language::LanguageDetector,
) -> Result<(), AlternatorError> {
    use crate::mastodon::MastodonStream;
    
    // Check if toot has media attachments
    if toot.media_attachments.is_empty() {
        debug!("Toot {} has no media attachments, skipping", toot.id);
        return Ok(());
    }
    
    // Filter media that needs processing
    let processable_media = media_processor.filter_processable_media(&toot.media_attachments);
    
    if processable_media.is_empty() {
        debug!("Toot {} has no processable media (all have descriptions or unsupported types)", toot.id);
        return Ok(());
    }
    
    info!("Found {} processable media attachments in toot {}", processable_media.len(), toot.id);
    
    // Detect language for prompt selection
    let detected_language = language_detector.detect_language(&toot.content)
        .unwrap_or_else(|e| {
            warn!("Language detection failed: {}, defaulting to English", e);
            "en".to_string()
        });
    
    let prompt_template = language_detector.get_prompt_template(&detected_language)
        .map_err(|e| AlternatorError::Language(e))?;
    
    debug!("Using language '{}' with prompt template", detected_language);
    
    // Process each media attachment
    for media in processable_media {
        info!("Processing media attachment: {} ({})", media.id, media.media_type);
        
        // Check for race conditions before processing
        match mastodon_client.get_toot(&toot.id).await {
            Ok(current_toot) => {
                // Find the current state of this media attachment
                if let Some(current_media) = current_toot.media_attachments.iter().find(|m| m.id == media.id) {
                    if current_media.description.is_some() && !current_media.description.as_ref().unwrap().trim().is_empty() {
                        info!("Media {} already has description (race condition detected), skipping", media.id);
                        continue;
                    }
                }
            }
            Err(e) => {
                warn!("Could not check current toot state for race condition: {}", e);
                // Continue processing but log the warning
            }
        }
        
        // Download and process media
        let processed_media_data = match media_processor.process_media_for_analysis(media).await {
            Ok(data) => data,
            Err(e) => {
                error!("Failed to process media {}: {}", media.id, e);
                continue; // Skip this media but continue with others
            }
        };
        
        // Generate description using OpenRouter
        let description = match openrouter_client.describe_image(&processed_media_data, prompt_template).await {
            Ok(desc) => desc,
            Err(crate::error::OpenRouterError::TokenLimitExceeded { .. }) => {
                warn!("Token limit exceeded for media {}, skipping", media.id);
                continue; // Skip this media but continue with others
            }
            Err(e) => {
                error!("Failed to generate description for media {}: {}", media.id, e);
                return Err(AlternatorError::OpenRouter(e));
            }
        };
        
        info!("Generated description for media {}: {}", media.id, description);
        
        // Update media description
        match mastodon_client.update_media(&media.id, &description).await {
            Ok(()) => {
                info!("✓ Updated description for media: {}", media.id);
            }
            Err(crate::error::MastodonError::RaceConditionDetected) => {
                info!("Race condition detected when updating media {}, skipping", media.id);
                continue;
            }
            Err(e) => {
                error!("Failed to update media description for {}: {}", media.id, e);
                return Err(AlternatorError::Mastodon(e));
            }
        }
    }
    
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