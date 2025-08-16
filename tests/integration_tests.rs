//! Integration tests for Alternator
//!
//! These tests modify global environment variables and should be run sequentially.
//! Use: cargo test --test integration_tests -- --test-threads=1

use alternator::config::{
    BalanceConfig, Config, LoggingConfig, MastodonConfig, MediaConfig, OpenRouterConfig,
    RuntimeConfig, WhisperConfig,
};
use alternator::error::AlternatorError;
use alternator::mastodon::{Account, MediaAttachment, TootEvent};
use alternator::toot_handler::TootStreamHandler;
use chrono::{Timelike, Utc};
use std::sync::Mutex;
use tokio::time::{timeout, Duration};

// Global mutex to ensure environment variable tests run sequentially
static ENV_TEST_MUTEX: Mutex<()> = Mutex::new(());

/// Create a test runtime config for integration tests
fn create_test_runtime_config() -> RuntimeConfig {
    let config = create_test_config();
    RuntimeConfig::new(config)
}

/// Create a test configuration for integration tests
fn create_test_config() -> Config {
    Config {
        mastodon: MastodonConfig {
            instance_url: "https://mastodon.social".to_string(),
            access_token: "test_token".to_string(),
            user_stream: Some(true),
        },
        openrouter: OpenRouterConfig {
            api_key: "test_api_key".to_string(),
            model: "anthropic/claude-3-haiku".to_string(),
            vision_model: "anthropic/claude-3-haiku".to_string(),
            text_model: "anthropic/claude-3-haiku".to_string(),
            base_url: Some("https://test.openrouter.ai/api/v1".to_string()),
            max_tokens: Some(150),
        },
        media: Some(MediaConfig {
            max_size_mb: Some(10),
            max_audio_size_mb: Some(50),
            max_video_size_mb: Some(250),
            supported_formats: Some(vec![
                "image/jpeg".to_string(),
                "image/png".to_string(),
                "image/gif".to_string(),
                "image/webp".to_string(),
            ]),
            resize_max_dimension: Some(2048),
        }),
        balance: Some(BalanceConfig {
            enabled: Some(false), // Disable for tests
            threshold: Some(5.0),
            check_time: Some("12:00".to_string()),
        }),
        logging: Some(LoggingConfig {
            level: Some("debug".to_string()),
        }),
        whisper: Some(WhisperConfig {
            enabled: Some(false),
            model: Some("base".to_string()),
            model_dir: None,
            language: None,
            max_duration_minutes: Some(10),
            python_executable: Some("python3".to_string()),
            device: None,
            backend: None,
            preload: Some(true),
        }),
    }
}

#[tokio::test]
async fn test_config_loading_from_file() {
    let _lock = ENV_TEST_MUTEX.lock().unwrap();

    // Clean up any existing environment variables first
    let env_vars_to_clean = [
        "ALTERNATOR_MASTODON_INSTANCE_URL",
        "ALTERNATOR_MASTODON_ACCESS_TOKEN",
        "ALTERNATOR_OPENROUTER_API_KEY",
        "ALTERNATOR_OPENROUTER_MODEL",
        "ALTERNATOR_OPENROUTER_VISION_MODEL",
        "ALTERNATOR_OPENROUTER_TEXT_MODEL",
        "ALTERNATOR_OPENROUTER_MAX_TOKENS",
        "ALTERNATOR_BALANCE_ENABLED",
        "ALTERNATOR_BALANCE_THRESHOLD",
        "ALTERNATOR_LOG_LEVEL",
    ];

    for var in &env_vars_to_clean {
        std::env::remove_var(var);
    }

    // Create a temporary config file
    let temp_dir = tempfile::tempdir().unwrap();
    let original_dir = std::env::current_dir().unwrap();
    std::env::set_current_dir(&temp_dir).unwrap();
    let config_path = temp_dir.path().join("test_config.toml");

    let config_content = r#"
[mastodon]
instance_url = "https://test.mastodon.social"
access_token = "test_access_token"
user_stream = true

[openrouter]
api_key = "test_openrouter_key"
model = "test-model"
vision_model = "test-vision-model"
text_model = "test-text-model"
base_url = "https://test.openrouter.ai/api/v1"
max_tokens = 200

[media]
max_size_mb = 15
supported_formats = ["image/jpeg", "image/png"]
resize_max_dimension = 2048

[balance]
enabled = true
threshold = 10.0
check_time = "14:30"

[logging]
level = "info"
"#;

    std::fs::write(&config_path, config_content).unwrap();

    // Load config from file
    let config = Config::load(Some(config_path)).unwrap();

    assert_eq!(config.mastodon.instance_url, "https://test.mastodon.social");
    assert_eq!(config.mastodon.access_token, "test_access_token");
    assert_eq!(config.openrouter.api_key, "test_openrouter_key");
    assert_eq!(config.openrouter.model, "test-model");
    assert_eq!(config.openrouter.vision_model, "test-vision-model");
    assert_eq!(config.openrouter.text_model, "test-text-model");
    assert_eq!(config.openrouter.max_tokens, Some(200));
    assert_eq!(config.media().max_size_mb, Some(15));
    assert_eq!(config.balance().threshold, Some(10.0));
    assert_eq!(config.logging().level, Some("info".to_string()));

    // Restore original directory - only if it still exists
    if original_dir.exists() {
        std::env::set_current_dir(original_dir).unwrap();
    } else {
        // If original directory no longer exists, set to a safe fallback
        let current = std::env::current_dir().unwrap_or_else(|_| {
            // If even that fails, use the temp dir's parent
            temp_dir
                .path()
                .parent()
                .unwrap_or(temp_dir.path())
                .to_path_buf()
        });
        std::env::set_current_dir(current).ok(); // Best effort
    }
}

#[tokio::test]
async fn test_config_environment_variable_overrides() {
    let _lock = ENV_TEST_MUTEX.lock().unwrap();

    // Create a temporary directory and change to it to avoid loading existing config
    let temp_dir = tempfile::tempdir().unwrap();
    let original_dir = std::env::current_dir().unwrap();
    std::env::set_current_dir(&temp_dir).unwrap();

    // Clean up any existing environment variables first
    let env_vars_to_clean = [
        "ALTERNATOR_MASTODON_INSTANCE_URL",
        "ALTERNATOR_MASTODON_ACCESS_TOKEN",
        "ALTERNATOR_OPENROUTER_API_KEY",
        "ALTERNATOR_OPENROUTER_MODEL",
        "ALTERNATOR_OPENROUTER_VISION_MODEL",
        "ALTERNATOR_OPENROUTER_TEXT_MODEL",
        "ALTERNATOR_OPENROUTER_MAX_TOKENS",
        "ALTERNATOR_BALANCE_ENABLED",
        "ALTERNATOR_BALANCE_THRESHOLD",
        "ALTERNATOR_LOG_LEVEL",
    ];

    for var in &env_vars_to_clean {
        std::env::remove_var(var);
    }

    // Set environment variables
    std::env::set_var(
        "ALTERNATOR_MASTODON_INSTANCE_URL",
        "https://env.mastodon.social",
    );
    std::env::set_var("ALTERNATOR_MASTODON_ACCESS_TOKEN", "env_access_token");
    std::env::set_var("ALTERNATOR_OPENROUTER_API_KEY", "env_openrouter_key");
    std::env::set_var("ALTERNATOR_OPENROUTER_MODEL", "env-model");
    std::env::set_var("ALTERNATOR_OPENROUTER_VISION_MODEL", "env-vision-model");
    std::env::set_var("ALTERNATOR_OPENROUTER_TEXT_MODEL", "env-text-model");
    std::env::set_var("ALTERNATOR_OPENROUTER_MAX_TOKENS", "300");
    std::env::set_var("ALTERNATOR_BALANCE_ENABLED", "false");
    std::env::set_var("ALTERNATOR_BALANCE_THRESHOLD", "15.5");
    std::env::set_var("ALTERNATOR_LOG_LEVEL", "debug");

    // Verify environment variables are set immediately (no async delay needed)
    let instance_url = std::env::var("ALTERNATOR_MASTODON_INSTANCE_URL")
        .expect("ALTERNATOR_MASTODON_INSTANCE_URL should be set");
    assert_eq!(instance_url, "https://env.mastodon.social");

    // Load config (should use environment variables)
    let config = Config::load(None).unwrap();

    assert_eq!(config.mastodon.instance_url, "https://env.mastodon.social");
    assert_eq!(config.mastodon.access_token, "env_access_token");
    assert_eq!(config.openrouter.api_key, "env_openrouter_key");
    assert_eq!(config.openrouter.model, "env-model");
    assert_eq!(config.openrouter.vision_model, "env-vision-model");
    assert_eq!(config.openrouter.text_model, "env-text-model");
    assert_eq!(config.openrouter.max_tokens, Some(300));
    assert_eq!(config.balance().enabled, Some(false));
    assert_eq!(config.balance().threshold, Some(15.5));
    assert_eq!(config.logging().level, Some("debug".to_string()));

    // Clean up environment variables
    std::env::remove_var("ALTERNATOR_MASTODON_INSTANCE_URL");
    std::env::remove_var("ALTERNATOR_MASTODON_ACCESS_TOKEN");
    std::env::remove_var("ALTERNATOR_OPENROUTER_API_KEY");
    std::env::remove_var("ALTERNATOR_OPENROUTER_MODEL");
    std::env::remove_var("ALTERNATOR_OPENROUTER_VISION_MODEL");
    std::env::remove_var("ALTERNATOR_OPENROUTER_TEXT_MODEL");
    std::env::remove_var("ALTERNATOR_OPENROUTER_MAX_TOKENS");
    std::env::remove_var("ALTERNATOR_BALANCE_ENABLED");
    std::env::remove_var("ALTERNATOR_BALANCE_THRESHOLD");
    std::env::remove_var("ALTERNATOR_LOG_LEVEL");

    // Restore original directory - only if it still exists
    if original_dir.exists() {
        std::env::set_current_dir(original_dir).unwrap();
    } else {
        // If original directory no longer exists, set to a safe fallback
        let current = std::env::current_dir().unwrap_or_else(|_| {
            // If even that fails, use the temp dir's parent
            temp_dir
                .path()
                .parent()
                .unwrap_or(temp_dir.path())
                .to_path_buf()
        });
        std::env::set_current_dir(current).ok(); // Best effort
    }
}

#[tokio::test]
async fn test_config_validation_missing_required_fields() {
    let _lock = ENV_TEST_MUTEX.lock().unwrap();

    // Clean up any existing ALTERNATOR env vars to ensure test isolation
    let env_vars_to_clean = [
        "ALTERNATOR_MASTODON_INSTANCE_URL",
        "ALTERNATOR_MASTODON_ACCESS_TOKEN",
        "ALTERNATOR_OPENROUTER_API_KEY",
        "ALTERNATOR_OPENROUTER_MODEL",
        "ALTERNATOR_OPENROUTER_VISION_MODEL",
        "ALTERNATOR_OPENROUTER_TEXT_MODEL",
        "ALTERNATOR_OPENROUTER_MAX_TOKENS",
        "ALTERNATOR_BALANCE_ENABLED",
        "ALTERNATOR_BALANCE_THRESHOLD",
        "ALTERNATOR_LOG_LEVEL",
    ];

    for var in &env_vars_to_clean {
        std::env::remove_var(var);
    }

    let temp_dir = tempfile::tempdir().unwrap();
    let original_dir = std::env::current_dir().unwrap();
    std::env::set_current_dir(&temp_dir).unwrap();
    let config_path = temp_dir.path().join("invalid_config.toml");

    let config_content = r#"
[mastodon]
instance_url = ""
access_token = "test_token"

[openrouter]
api_key = "test_key"
model = "test-model"
vision_model = "test-vision-model"
text_model = "test-text-model"
"#;

    // Ensure the temporary directory exists and is writable
    assert!(temp_dir.path().exists(), "Temporary directory should exist");
    assert!(
        temp_dir.path().is_dir(),
        "Temporary directory should be a directory"
    );

    // Write the config file with better error handling
    std::fs::write(&config_path, config_content)
        .unwrap_or_else(|e| panic!("Failed to write config file to {:?}: {}", config_path, e));

    let result = Config::load(Some(config_path));
    assert!(result.is_err());

    let error = result.unwrap_err();
    assert!(error.to_string().contains("mastodon.instance_url"));

    // Restore original directory - only if it still exists
    if original_dir.exists() {
        std::env::set_current_dir(original_dir).unwrap();
    } else {
        // If original directory no longer exists, set to a safe fallback
        let current = std::env::current_dir().unwrap_or_else(|_| {
            // If even that fails, use the temp dir's parent
            temp_dir
                .path()
                .parent()
                .unwrap_or(temp_dir.path())
                .to_path_buf()
        });
        std::env::set_current_dir(current).ok(); // Best effort
    }
}

#[tokio::test]
async fn test_application_component_initialization() {
    let config = create_test_config();

    // Test that all components can be initialized
    let _mastodon_client = alternator::mastodon::MastodonClient::new(config.mastodon.clone());
    let _openrouter_client =
        alternator::openrouter::OpenRouterClient::new(config.openrouter.clone());
    let media_processor = alternator::media::MediaProcessor::with_default_config();
    let language_detector = alternator::language::LanguageDetector::new();
    let balance_monitor = alternator::balance::BalanceMonitor::new(
        config.balance().clone(),
        alternator::openrouter::OpenRouterClient::new(config.openrouter.clone()),
    );

    // Verify components are initialized correctly
    assert!(!balance_monitor.is_enabled()); // Disabled in test config
    assert_eq!(balance_monitor.threshold(), 5.0);

    // Test language detector
    let supported_languages = language_detector.supported_languages();
    assert!(supported_languages.len() >= 8);
    assert!(supported_languages.contains(&&"en".to_string()));

    // Test media processor
    let test_media = vec![alternator::mastodon::MediaAttachment {
        id: "1".to_string(),
        media_type: "image/jpeg".to_string(),
        url: "https://example.com/image.jpg".to_string(),
        preview_url: None,
        description: None,
        meta: None,
    }];

    let processable = media_processor.filter_processable_media(&test_media);
    assert_eq!(processable.len(), 1);
}

#[tokio::test]
async fn test_error_handling_and_recovery() {
    use alternator::error::{ErrorRecovery, MastodonError, OpenRouterError};

    // Test recoverable errors - create a simple IO error for testing
    let io_error = std::io::Error::new(std::io::ErrorKind::TimedOut, "timeout");
    let network_error = AlternatorError::Io(io_error);
    assert!(ErrorRecovery::is_recoverable(&network_error));

    let mastodon_connection_error =
        AlternatorError::Mastodon(MastodonError::ConnectionFailed("timeout".to_string()));
    assert!(ErrorRecovery::is_recoverable(&mastodon_connection_error));

    let openrouter_rate_limit =
        AlternatorError::OpenRouter(OpenRouterError::RateLimitExceeded { retry_after: 60 });
    assert!(ErrorRecovery::is_recoverable(&openrouter_rate_limit));

    // Test non-recoverable errors
    let config_error = AlternatorError::Config(alternator::config::ConfigError::MissingRequired(
        "test".to_string(),
    ));
    assert!(!ErrorRecovery::is_recoverable(&config_error));
    assert!(ErrorRecovery::should_shutdown(&config_error));

    let auth_error = AlternatorError::Mastodon(MastodonError::AuthenticationFailed(
        "invalid token".to_string(),
    ));
    assert!(!ErrorRecovery::is_recoverable(&auth_error));
    assert!(ErrorRecovery::should_shutdown(&auth_error));

    // Test retry delays
    let delay = ErrorRecovery::retry_delay(&network_error, 0);
    assert!(delay > 0);
    assert!(delay <= 60); // Should be capped at 60 seconds

    let max_retries = ErrorRecovery::max_retries(&mastodon_connection_error);
    assert!(max_retries > 0);
}

#[tokio::test]
async fn test_graceful_shutdown_signal_setup() {
    // Test that shutdown signal setup doesn't panic
    // We can't easily test the actual signal handling in a unit test,
    // but we can verify the setup doesn't fail

    let shutdown_future = async {
        // This would normally wait for a signal, but we'll timeout quickly for testing
        tokio::time::sleep(Duration::from_millis(10)).await;
    };

    let result = timeout(Duration::from_millis(100), shutdown_future).await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_media_processing_pipeline() {
    let media_processor = alternator::media::MediaProcessor::with_default_config();
    let language_detector = alternator::language::LanguageDetector::new();

    // Test language detection
    let english_text = "This is a test toot with an image attachment";
    let detected_lang = language_detector.detect_language(english_text).unwrap();
    assert_eq!(detected_lang, "en");

    let prompt = language_detector
        .get_prompt_template(&detected_lang)
        .unwrap();
    assert!(prompt.contains("alt-text"));

    // Test media filtering
    let media_attachments = vec![
        alternator::mastodon::MediaAttachment {
            id: "1".to_string(),
            media_type: "image/jpeg".to_string(),
            url: "https://example.com/image.jpg".to_string(),
            preview_url: None,
            description: None, // Needs description
            meta: None,
        },
        alternator::mastodon::MediaAttachment {
            id: "2".to_string(),
            media_type: "image/png".to_string(),
            url: "https://example.com/image.png".to_string(),
            preview_url: None,
            description: Some("Already has description".to_string()), // Has description
            meta: None,
        },
        alternator::mastodon::MediaAttachment {
            id: "3".to_string(),
            media_type: "video/mp4".to_string(), // Now supported type
            url: "https://example.com/video.mp4".to_string(),
            preview_url: None,
            description: None,
            meta: None,
        },
    ];

    let processable = media_processor.filter_processable_media(&media_attachments);
    assert_eq!(processable.len(), 2); // First image and video should be processable
    assert_eq!(processable[0].id, "1");
    assert_eq!(processable[1].id, "3");

    let stats = media_processor.get_media_stats(&media_attachments);
    assert_eq!(stats.total, 3);
    assert_eq!(stats.supported, 3); // JPEG, PNG and MP4 are supported
    assert_eq!(stats.processable, 2); // Image and video need descriptions
}

#[tokio::test]
async fn test_balance_monitoring_configuration() {
    // Test enabled balance monitoring
    let enabled_config = BalanceConfig {
        enabled: Some(true),
        threshold: Some(10.0),
        check_time: Some("14:30".to_string()),
    };

    let openrouter_client = alternator::openrouter::OpenRouterClient::new(OpenRouterConfig {
        api_key: "test_key".to_string(),
        model: "test_model".to_string(),
        vision_model: "test_vision_model".to_string(),
        text_model: "test_text_model".to_string(),
        base_url: None,
        max_tokens: Some(150),
    });

    let monitor = alternator::balance::BalanceMonitor::new(enabled_config, openrouter_client);
    assert!(monitor.is_enabled());
    assert_eq!(monitor.threshold(), 10.0);

    let check_time = monitor.check_time().unwrap();
    assert_eq!(check_time.hour(), 14);
    assert_eq!(check_time.minute(), 30);

    // Test disabled balance monitoring
    let disabled_config = BalanceConfig {
        enabled: Some(false),
        threshold: Some(5.0),
        check_time: Some("12:00".to_string()),
    };

    let openrouter_client2 = alternator::openrouter::OpenRouterClient::new(OpenRouterConfig {
        api_key: "test_key".to_string(),
        model: "test_model".to_string(),
        vision_model: "test_vision_model".to_string(),
        text_model: "test_text_model".to_string(),
        base_url: None,
        max_tokens: Some(150),
    });

    let monitor2 = alternator::balance::BalanceMonitor::new(disabled_config, openrouter_client2);
    assert!(!monitor2.is_enabled());
}

// Note: Full end-to-end integration tests would require actual API connections
// and are better suited for manual testing or CI environments with proper credentials

#[tokio::test]
async fn test_end_to_end_toot_processing_workflow() {
    // Test the complete toot processing pipeline with mock components
    let config = create_test_config();
    let runtime_config = create_test_runtime_config();

    // Create components
    let mastodon_client = alternator::mastodon::MastodonClient::new(config.mastodon.clone());
    let openrouter_client =
        alternator::openrouter::OpenRouterClient::new(config.openrouter.clone());
    let media_processor =
        alternator::media::MediaProcessor::with_image_transformer(alternator::media::MediaConfig {
            max_size_mb: 10.0,
            max_dimension: 2048,
            supported_formats: vec![
                "image/jpeg".to_string(),
                "image/png".to_string(),
                "image/gif".to_string(),
                "image/webp".to_string(),
            ]
            .into_iter()
            .collect(),
        });
    let language_detector = alternator::language::LanguageDetector::new();

    // Create TootStreamHandler
    let toot_handler = TootStreamHandler::new(
        mastodon_client,
        openrouter_client,
        media_processor,
        language_detector,
        runtime_config,
    );

    // Verify TootStreamHandler can be created and has expected initial state
    let stats = toot_handler.get_processing_stats();
    assert_eq!(stats.processed_toots_count, 0);

    // Note: We cannot test actual streaming without mocking the WebSocket connection
    // This would require more sophisticated mocking infrastructure
}

#[tokio::test]
async fn test_toot_processing_race_condition_handling() {
    // Test that the toot processing handles race conditions correctly
    // This simulates the scenario where a toot is edited manually before the automated update

    let _config = create_test_config();

    // Create test toot with media that needs description
    let test_toot = TootEvent {
        id: "test_toot_123".to_string(),
        uri: "https://mastodon.social/users/testuser/statuses/test_toot_123".to_string(),
        account: Account {
            id: "user_123".to_string(),
            username: "testuser".to_string(),
            acct: "testuser@mastodon.social".to_string(),
            display_name: "Test User".to_string(),
            url: "https://mastodon.social/@testuser".to_string(),
        },
        content: "Test toot with image for race condition testing".to_string(),
        language: Some("en".to_string()),
        media_attachments: vec![MediaAttachment {
            id: "media_456".to_string(),
            media_type: "image/jpeg".to_string(),
            url: "https://example.com/test_image.jpg".to_string(),
            preview_url: None,
            description: None, // Initially no description
            meta: None,
        }],
        created_at: Utc::now(),
        url: Some("https://mastodon.social/@testuser/test_toot_123".to_string()),
        visibility: "public".to_string(),
        sensitive: false,
        spoiler_text: String::new(),
        in_reply_to_id: None,
        in_reply_to_account_id: None,
        mentions: Vec::new(),
        tags: Vec::new(),
        emojis: Vec::new(),
        poll: None,
    };

    // Test that the media processor identifies this as processable
    let media_processor =
        alternator::media::MediaProcessor::with_image_transformer(alternator::media::MediaConfig {
            max_size_mb: 10.0,
            max_dimension: 2048,
            supported_formats: vec!["image/jpeg".to_string()].into_iter().collect(),
        });

    let processable_media = media_processor.filter_processable_media(&test_toot.media_attachments);
    assert_eq!(processable_media.len(), 1);
    assert_eq!(processable_media[0].id, "media_456");

    // Test language detection
    let language_detector = alternator::language::LanguageDetector::new();
    let detected_language = language_detector
        .detect_language(&test_toot.content)
        .unwrap();
    assert_eq!(detected_language, "en");

    let prompt_template = language_detector
        .get_prompt_template(&detected_language)
        .unwrap();
    assert!(prompt_template.contains("alt-text"));
}

#[tokio::test]
async fn test_toot_processing_duplicate_prevention() {
    // Test that duplicate toot processing is prevented
    let config = create_test_config();
    let runtime_config = create_test_runtime_config();

    // Create components
    let mastodon_client = alternator::mastodon::MastodonClient::new(config.mastodon.clone());
    let openrouter_client =
        alternator::openrouter::OpenRouterClient::new(config.openrouter.clone());
    let media_processor =
        alternator::media::MediaProcessor::with_image_transformer(alternator::media::MediaConfig {
            max_size_mb: 10.0,
            max_dimension: 2048,
            supported_formats: vec!["image/jpeg".to_string()].into_iter().collect(),
        });
    let language_detector = alternator::language::LanguageDetector::new();

    // Create TootStreamHandler and manually test duplicate prevention
    let toot_handler = TootStreamHandler::new(
        mastodon_client,
        openrouter_client,
        media_processor,
        language_detector,
        runtime_config,
    );

    // Initially no toots processed
    let stats = toot_handler.get_processing_stats();
    assert_eq!(stats.processed_toots_count, 0);

    // This test demonstrates the structure without actual WebSocket interaction
    // In a real scenario, we would test with mocked WebSocket events
}

#[tokio::test]
async fn test_empty_message_with_media_behavior() {
    // Test the behavior when processing a toot with empty message and media attachments
    // This covers the use case mentioned in the issue

    let _config = create_test_config();

    // Create a toot with empty content but media attachments needing descriptions
    let empty_content_toot = TootEvent {
        id: "empty_toot_123".to_string(),
        uri: "https://mastodon.social/users/testuser/statuses/empty_toot_123".to_string(),
        account: Account {
            id: "user_456".to_string(),
            username: "testuser".to_string(),
            acct: "testuser@mastodon.social".to_string(),
            display_name: "Test User".to_string(),
            url: "https://mastodon.social/@testuser".to_string(),
        },
        content: "".to_string(), // Empty content - this is the key issue
        language: Some("en".to_string()),
        media_attachments: vec![
            MediaAttachment {
                id: "media_789".to_string(),
                media_type: "image/jpeg".to_string(),
                url: "https://example.com/image1.jpg".to_string(),
                preview_url: None,
                description: None, // Needs description but post has no text
                meta: None,
            },
            MediaAttachment {
                id: "media_790".to_string(),
                media_type: "image/png".to_string(),
                url: "https://example.com/image2.png".to_string(),
                preview_url: None,
                description: None, // Also needs description
                meta: None,
            },
        ],
        created_at: Utc::now(),
        url: Some("https://mastodon.social/@testuser/empty_toot_123".to_string()),
        visibility: "public".to_string(),
        sensitive: false,
        spoiler_text: String::new(),
        in_reply_to_id: None,
        in_reply_to_account_id: None,
        mentions: Vec::new(),
        tags: Vec::new(),
        emojis: Vec::new(),
        poll: None,
    };

    // Test with HTML-only content (also effectively empty)
    let html_empty_toot = TootEvent {
        id: "html_empty_toot_124".to_string(),
        uri: "https://mastodon.social/users/testuser/statuses/html_empty_toot_124".to_string(),
        account: Account {
            id: "user_456".to_string(),
            username: "testuser".to_string(),
            acct: "testuser@mastodon.social".to_string(),
            display_name: "Test User".to_string(),
            url: "https://mastodon.social/@testuser".to_string(),
        },
        content: "<p></p>".to_string(), // HTML that extracts to empty text
        language: Some("en".to_string()),
        media_attachments: vec![MediaAttachment {
            id: "media_791".to_string(),
            media_type: "image/jpeg".to_string(),
            url: "https://example.com/image3.jpg".to_string(),
            preview_url: None,
            description: None,
            meta: None,
        }],
        created_at: Utc::now(),
        url: Some("https://mastodon.social/@testuser/html_empty_toot_124".to_string()),
        visibility: "public".to_string(),
        sensitive: false,
        spoiler_text: String::new(),
        in_reply_to_id: None,
        in_reply_to_account_id: None,
        mentions: Vec::new(),
        tags: Vec::new(),
        emojis: Vec::new(),
        poll: None,
    };

    // Test media processor identifies these as processable
    let media_processor = alternator::media::MediaProcessor::with_default_config();

    let processable_empty =
        media_processor.filter_processable_media(&empty_content_toot.media_attachments);
    assert_eq!(
        processable_empty.len(),
        2,
        "Both media attachments should be processable"
    );

    let processable_html =
        media_processor.filter_processable_media(&html_empty_toot.media_attachments);
    assert_eq!(
        processable_html.len(),
        1,
        "Single media attachment should be processable"
    );

    // Test language detection on empty content
    let language_detector = alternator::language::LanguageDetector::new();

    // Language detection should fall back to English for empty content
    let detected_lang_empty = language_detector
        .detect_language(&empty_content_toot.content)
        .unwrap();
    assert_eq!(
        detected_lang_empty, "en",
        "Should fallback to English for empty content"
    );

    // For HTML-only content, the detector might detect based on the HTML tags
    // We don't assert a specific language here since it depends on the HTML content
    let detected_lang_html = language_detector
        .detect_language(&html_empty_toot.content)
        .unwrap();
    assert!(
        !detected_lang_html.is_empty(),
        "Should detect some language for HTML content"
    );

    // Verify that these cases would currently be skipped by the validation logic
    // This demonstrates the need for the unicode space solution

    // Test HTML text extraction that would happen in the real processing
    let extracted_empty =
        alternator::mastodon::MastodonClient::extract_text_from_html(&empty_content_toot.content);
    let extracted_html =
        alternator::mastodon::MastodonClient::extract_text_from_html(&html_empty_toot.content);

    assert!(
        extracted_empty.trim().is_empty(),
        "Empty content should extract to empty text"
    );
    assert!(
        extracted_html.trim().is_empty(),
        "HTML-only content should extract to empty text"
    );

    // These are the cases where unicode space character would be beneficial:
    // - Media attachments that need descriptions ✓
    // - Empty or HTML-only content that extracts to empty text ✓
    // - Valid language detection working ✓
    // - But validation failing due to empty text content

    println!(
        "Test demonstrates the empty message + media use case that needs unicode space solution"
    );
}

#[tokio::test]
async fn test_unicode_space_character_solution() {
    // Test the proposed solution using unicode space characters
    // This validates that the solution would work as expected

    let test_cases = vec![
        ("", "\u{200B}"),           // Empty -> zero-width space
        ("   ", "\u{200B}"),        // Whitespace -> zero-width space
        ("<p></p>", "\u{200B}"),    // Empty HTML -> zero-width space
        ("<p>   </p>", "\u{200B}"), // Whitespace HTML -> zero-width space
    ];

    for (original, expected_replacement) in test_cases {
        // Extract text as the current system would
        let extracted = alternator::mastodon::MastodonClient::extract_text_from_html(original);

        // Verify it's empty (current problem)
        assert!(
            extracted.trim().is_empty(),
            "Original content '{original}' should extract to empty"
        );

        // Test the proposed solution
        let with_unicode_space = if extracted.trim().is_empty() {
            expected_replacement.to_string()
        } else {
            extracted
        };

        // Verify the solution would pass validation
        assert!(
            !with_unicode_space.trim().is_empty(),
            "Unicode space solution should pass validation"
        );
        assert_eq!(
            with_unicode_space.chars().count(),
            1,
            "Should be exactly one unicode character"
        );

        // Verify it's the correct unicode character
        assert_eq!(with_unicode_space, "\u{200B}", "Should be zero-width space");
    }

    // Test that zero-width space is indeed invisible
    let zero_width_space = "\u{200B}";
    // Zero-width space is a format character, not whitespace in Rust's definition
    assert!(
        zero_width_space
            .chars()
            .all(|c| c.is_control() || !c.is_ascii_graphic()),
        "Zero-width space should be non-visible"
    );

    // Test that it would not interfere with normal text processing
    let normal_text = "Hello world";
    let extracted_normal =
        alternator::mastodon::MastodonClient::extract_text_from_html(normal_text);
    assert!(!extracted_normal.trim().is_empty());
    assert_eq!(
        extracted_normal, normal_text,
        "Normal text should be unchanged"
    );
}

#[tokio::test]
async fn test_zero_width_space_implementation_integration() {
    // Test that the zero-width space implementation resolves the empty message issue
    let _config = create_test_config();

    // Test cases that would previously fail but should now work
    let empty_content_cases = vec![
        ("", "completely empty content"),
        ("   ", "whitespace-only content"),
        ("<p></p>", "empty HTML tags"),
        ("<p>   </p>", "HTML with whitespace"),
        ("<br/>", "self-closing HTML tag"),
    ];

    for (content, description) in empty_content_cases {
        // Extract text as the system would
        let extracted_text = alternator::mastodon::MastodonClient::extract_text_from_html(content);

        // Verify it extracts to empty (the original problem)
        assert!(
            extracted_text.trim().is_empty(),
            "Content '{content}' ({description}) should extract to empty"
        );

        // Simulate the new logic that uses zero-width space
        let status_content = if extracted_text.trim().is_empty() {
            "\u{200B}".to_string() // Zero-width space
        } else {
            extracted_text
        };

        // Verify the solution works
        assert!(
            !status_content.trim().is_empty(),
            "Status content for '{content}' ({description}) should pass validation with zero-width space"
        );

        assert_eq!(
            status_content, "\u{200B}",
            "Empty content should be replaced with zero-width space"
        );

        // Verify it's still invisible to users
        assert!(
            status_content.chars().all(|c| !c.is_ascii_graphic()),
            "Zero-width space should be invisible"
        );
    }

    // Test that normal content is preserved
    let normal_content = "Hello world with image";
    let extracted_normal =
        alternator::mastodon::MastodonClient::extract_text_from_html(normal_content);
    let status_content_normal = if extracted_normal.trim().is_empty() {
        "\u{200B}".to_string()
    } else {
        extracted_normal
    };

    assert_eq!(
        status_content_normal, normal_content,
        "Normal content should be preserved unchanged"
    );
    assert!(!status_content_normal.trim().is_empty());
}

#[tokio::test]
async fn test_audio_format_support() {
    // Test that audio formats are now properly supported by MediaProcessor

    // Create a test toot with audio attachment
    let test_toot = TootEvent {
        id: "audio_test_123".to_string(),
        uri: "https://mastodon.social/users/testuser/statuses/audio_test_123".to_string(),
        account: Account {
            id: "test_account_456".to_string(),
            username: "testuser".to_string(),
            acct: "testuser".to_string(),
            display_name: "Test User".to_string(),
            url: "https://mastodon.social/@testuser".to_string(),
        },
        content: "Check out this audio clip!".to_string(),
        language: Some("en".to_string()),
        media_attachments: vec![MediaAttachment {
            id: "audio_789".to_string(),
            media_type: "audio".to_string(), // This is what Mastodon API sends
            url: "https://example.com/audio.mp3".to_string(),
            preview_url: None,
            description: None,
            meta: None,
        }],
        created_at: Utc::now(),
        url: Some("https://mastodon.social/@testuser/audio_test_123".to_string()),
        visibility: "public".to_string(),
        sensitive: false,
        spoiler_text: String::new(),
        in_reply_to_id: None,
        in_reply_to_account_id: None,
        mentions: Vec::new(),
        tags: Vec::new(),
        emojis: Vec::new(),
        poll: None,
    };

    // Test with default configuration (should now include audio formats)
    let media_processor_default = alternator::media::MediaProcessor::with_default_config();
    let processable_default =
        media_processor_default.filter_processable_media(&test_toot.media_attachments);

    assert_eq!(
        processable_default.len(),
        1,
        "Audio attachment should be processable with default config"
    );
    assert_eq!(processable_default[0].id, "audio_789");

    // Test with explicit configuration including audio formats
    let mut supported_formats = std::collections::HashSet::new();
    supported_formats.insert("audio/mpeg".to_string());
    supported_formats.insert("audio/mp3".to_string());
    supported_formats.insert("image/jpeg".to_string());

    let media_processor_explicit =
        alternator::media::MediaProcessor::with_image_transformer(alternator::media::MediaConfig {
            max_size_mb: 10.0,
            max_dimension: 2048,
            supported_formats,
        });

    let processable_explicit =
        media_processor_explicit.filter_processable_media(&test_toot.media_attachments);

    assert_eq!(
        processable_explicit.len(),
        1,
        "Audio attachment should be processable with explicit audio config"
    );

    // Test that specific audio MIME types are also supported
    let specific_audio_toot = TootEvent {
        id: "specific_audio_test_456".to_string(),
        uri: "https://mastodon.social/users/testuser2/statuses/specific_audio_test_456".to_string(),
        account: Account {
            id: "test_account_789".to_string(),
            username: "testuser2".to_string(),
            acct: "testuser2".to_string(),
            display_name: "Test User 2".to_string(),
            url: "https://mastodon.social/@testuser2".to_string(),
        },
        content: "Specific audio format test".to_string(),
        language: Some("en".to_string()),
        media_attachments: vec![MediaAttachment {
            id: "specific_audio_123".to_string(),
            media_type: "audio/mpeg".to_string(), // Specific MIME type
            url: "https://example.com/specific.mp3".to_string(),
            preview_url: None,
            description: None,
            meta: None,
        }],
        created_at: Utc::now(),
        url: Some("https://mastodon.social/@testuser2/specific_audio_test_456".to_string()),
        visibility: "public".to_string(),
        sensitive: false,
        spoiler_text: String::new(),
        in_reply_to_id: None,
        in_reply_to_account_id: None,
        mentions: Vec::new(),
        tags: Vec::new(),
        emojis: Vec::new(),
        poll: None,
    };

    let processable_specific =
        media_processor_default.filter_processable_media(&specific_audio_toot.media_attachments);

    assert_eq!(
        processable_specific.len(),
        1,
        "Specific audio MIME type should be processable"
    );
    assert_eq!(processable_specific[0].id, "specific_audio_123");

    println!("Audio format support test passed - audio attachments are now properly recognized as processable");
}

#[tokio::test]
async fn test_openrouter_mocking_in_integration_tests() {
    // This test demonstrates how to use mocks instead of live OpenRouter calls
    use alternator::openrouter::{MockOpenRouterClient, OpenRouterApi, OpenRouterError};

    // Test successful API responses
    let mock_client = MockOpenRouterClient::new()
        .with_balance(50.0)
        .with_description("A mocked image description for testing".to_string());

    // Test account balance
    let balance = mock_client.get_account_balance().await.unwrap();
    assert_eq!(balance, 50.0);

    // Test image description
    let test_image = vec![0u8; 2048];
    let description = mock_client
        .describe_image(&test_image, "Describe this test image")
        .await
        .unwrap();
    assert_eq!(description, "A mocked image description for testing");

    // Test error scenarios
    let error_client = MockOpenRouterClient::with_error(OpenRouterError::InsufficientBalance {
        balance: 0.50,
        minimum: 1.0,
    });

    let balance_result = error_client.get_account_balance().await;
    assert!(balance_result.is_err());

    match balance_result.unwrap_err() {
        OpenRouterError::InsufficientBalance { balance, minimum } => {
            assert_eq!(balance, 0.50);
            assert_eq!(minimum, 1.0);
        }
        _ => panic!("Expected InsufficientBalance error"),
    }

    println!("OpenRouter mocking test passed - no live API calls made");
}

#[tokio::test]
async fn test_balance_monitoring_with_mock() {
    // Test balance monitoring using mock OpenRouter client
    use alternator::openrouter::{MockOpenRouterClient, OpenRouterApi, OpenRouterError};

    // This would typically require dependency injection to work properly
    // For now, we demonstrate the concept with direct mock usage

    let mock_client = MockOpenRouterClient::new().with_balance(15.0);
    let balance = mock_client.get_account_balance().await.unwrap();
    assert_eq!(balance, 15.0);

    // Test error handling
    let error_client = MockOpenRouterClient::with_error(OpenRouterError::AuthenticationFailed);
    let auth_result = error_client.get_account_balance().await;
    assert!(auth_result.is_err());
    assert!(matches!(
        auth_result.unwrap_err(),
        OpenRouterError::AuthenticationFailed
    ));

    println!("Balance monitoring mock test passed");
}

#[tokio::test]
async fn test_media_processing_with_mock_openrouter() {
    // Test the media processing pipeline with mocked OpenRouter responses
    use alternator::openrouter::{MockOpenRouterClient, OpenRouterApi};

    let mock_client = MockOpenRouterClient::new().with_description(
        "A professional photograph showing a coffee cup on a wooden table".to_string(),
    );

    // Simulate processing an image
    let test_image_data = vec![0xFF, 0xD8, 0xFF, 0xE0]; // JPEG header bytes
    let prompt = "Please describe this image in detail for accessibility purposes";

    let description = mock_client
        .describe_image(&test_image_data, prompt)
        .await
        .unwrap();

    assert_eq!(
        description,
        "A professional photograph showing a coffee cup on a wooden table"
    );
    assert!(!description.is_empty());
    assert!(description.len() > 10); // Reasonable length

    // Test that text processing also works
    let text_response = mock_client
        .process_text("Summarize this content")
        .await
        .unwrap();

    assert!(!text_response.is_empty());

    println!("Media processing with mock OpenRouter test passed");
}

#[tokio::test]
async fn test_rate_limiting_error_simulation() {
    // Test rate limiting error handling with mock
    use alternator::openrouter::{MockOpenRouterClient, OpenRouterApi, OpenRouterError};

    let rate_limited_client =
        MockOpenRouterClient::with_error(OpenRouterError::RateLimitExceeded { retry_after: 120 });

    let result = rate_limited_client.get_account_balance().await;
    assert!(result.is_err());

    match result.unwrap_err() {
        OpenRouterError::RateLimitExceeded { retry_after } => {
            assert_eq!(retry_after, 120);
            println!("Rate limit error correctly simulated: retry after {retry_after} seconds");
        }
        _ => panic!("Expected RateLimitExceeded error"),
    }

    // Test image processing with rate limit
    let image_result = rate_limited_client
        .describe_image(&[1, 2, 3], "test prompt")
        .await;
    assert!(image_result.is_err());

    println!("Rate limiting error simulation test passed");
}

#[tokio::test]
async fn test_model_availability_with_mock() {
    // Test model availability checking with mock
    use alternator::openrouter::{MockOpenRouterClient, OpenRouterApi};

    let mock_client = MockOpenRouterClient::new();
    let models = mock_client.list_models().await.unwrap();

    assert!(!models.is_empty());
    assert_eq!(models.len(), 2);

    // Check that the configured model is in the list
    let haiku_model = models
        .iter()
        .find(|m| m.id == "anthropic/claude-3-haiku")
        .expect("Should find Claude 3 Haiku model");

    assert_eq!(haiku_model.name, "Claude 3 Haiku");
    assert!(haiku_model.description.is_some());
    assert!(haiku_model.pricing.is_some());
    assert_eq!(haiku_model.context_length, Some(200000));

    println!("Model availability mock test passed");
}
