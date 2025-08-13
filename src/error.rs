use crate::config::ConfigError;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum AlternatorError {
    #[error("Configuration error: {0}")]
    Config(#[from] ConfigError),

    #[error("Mastodon API error: {0}")]
    Mastodon(#[from] MastodonError),

    #[error("OpenRouter API error: {0}")]
    OpenRouter(#[from] OpenRouterError),

    #[error("Media processing error: {0}")]
    Media(#[from] MediaError),

    #[error("Language detection error: {0}")]
    Language(#[from] LanguageError),

    #[error("Balance monitoring error: {0}")]
    Balance(#[from] BalanceError),

    #[error("Network error: {0}")]
    Network(#[from] reqwest::Error),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON parsing error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("WebSocket error: {0}")]
    WebSocket(#[from] tokio_tungstenite::tungstenite::Error),

    #[error("URL parsing error: {0}")]
    Url(#[from] url::ParseError),

    #[error("Task join error: {0}")]
    TaskJoin(#[from] tokio::task::JoinError),

    #[error("Application shutdown requested")]
    Shutdown,

    #[error("Rate limit exceeded: {0}")]
    RateLimit(String),

    #[error("Authentication failed: {0}")]
    Authentication(String),

    #[error("Invalid data: {0}")]
    InvalidData(String),
}

#[derive(Error, Debug, Clone)]
pub enum MastodonError {
    #[error("WebSocket connection failed: {0}")]
    ConnectionFailed(String),

    #[error("WebSocket disconnected: {0}")]
    Disconnected(String),

    #[error("Authentication failed: {0}")]
    AuthenticationFailed(String),

    #[error("API request failed: {0}")]
    ApiRequestFailed(String),

    #[error("Invalid toot data: {0}")]
    InvalidTootData(String),

    #[error("Rate limit exceeded, retry after {retry_after} seconds")]
    RateLimitExceeded { retry_after: u64 },

    #[error("Toot not found: {toot_id}")]
    TootNotFound { toot_id: String },

    #[error("Media attachment not found: {media_id}")]
    MediaNotFound { media_id: String },

    #[error("User verification failed")]
    UserVerificationFailed,

    #[error("Race condition detected: toot was modified")]
    RaceConditionDetected,
}

#[derive(Error, Debug, Clone)]
pub enum OpenRouterError {
    #[error("API request failed: {0}")]
    ApiRequestFailed(String),

    #[error("Authentication failed: invalid API key")]
    AuthenticationFailed,

    #[error("Model not available: {model}")]
    ModelNotAvailable { model: String },

    #[error("Token limit exceeded: {tokens_used}/{max_tokens}")]
    TokenLimitExceeded { tokens_used: u32, max_tokens: u32 },

    #[error("Insufficient balance: ${balance} (minimum: ${minimum})")]
    InsufficientBalance { balance: f64, minimum: f64 },

    #[error("Rate limit exceeded, retry after {retry_after} seconds")]
    RateLimitExceeded { retry_after: u64 },

    #[error("Invalid response format: {0}")]
    InvalidResponse(String),

    #[error("Image too large: {size_mb}MB (max: {max_mb}MB)")]
    ImageTooLarge { size_mb: f64, max_mb: f64 },

    #[error("Unsupported image format: {format}")]
    UnsupportedImageFormat { format: String },
}

#[derive(Error, Debug, Clone)]
pub enum MediaError {
    #[error("Unsupported media type: {media_type}")]
    UnsupportedType { media_type: String },

    #[error("Image processing failed: {0}")]
    ProcessingFailed(String),

    #[error("Image decoding failed: {0}")]
    DecodingFailed(String),

    #[error("Image encoding failed: {0}")]
    EncodingFailed(String),

    #[error("Image too large: {width}x{height} (max dimension: {max_dimension})")]
    ImageTooLarge {
        width: u32,
        height: u32,
        max_dimension: u32,
    },

    #[error("Invalid image data")]
    InvalidImageData,

    #[error("Media download failed: {url}")]
    DownloadFailed { url: String },
}

#[derive(Error, Debug, Clone)]
pub enum LanguageError {
    #[error("Language detection failed: {0}")]
    DetectionFailed(String),

    #[error("Unsupported language: {language}")]
    UnsupportedLanguage { language: String },

    #[error("Prompt template not found for language: {language}")]
    PromptTemplateNotFound { language: String },

    #[error("Invalid language code: {code}")]
    InvalidLanguageCode { code: String },
}

#[derive(Error, Debug, Clone)]
pub enum BalanceError {
    #[error("Balance check failed: {0}")]
    CheckFailed(String),

    #[error("Invalid balance threshold: {threshold}")]
    InvalidThreshold { threshold: f64 },

    #[error("Invalid check time format: {time}")]
    InvalidCheckTime { time: String },

    #[error("Notification sending failed: {0}")]
    NotificationFailed(String),
}

/// Error recovery strategies for different failure scenarios
pub struct ErrorRecovery;

impl ErrorRecovery {
    /// Determine if an error is recoverable and suggest retry strategy
    pub fn is_recoverable(error: &AlternatorError) -> bool {
        match error {
            // Network errors are generally recoverable
            AlternatorError::Network(_) => true,

            // WebSocket errors are recoverable with reconnection
            AlternatorError::WebSocket(_) => true,

            // Specific Mastodon errors
            AlternatorError::Mastodon(mastodon_error) => match mastodon_error {
                MastodonError::ConnectionFailed(_) => true,
                MastodonError::Disconnected(_) => true,
                MastodonError::RateLimitExceeded { .. } => true,
                MastodonError::ApiRequestFailed(_) => true,
                MastodonError::AuthenticationFailed(_) => false, // Not recoverable
                MastodonError::UserVerificationFailed => false,  // Not recoverable
                _ => false,
            },

            // Specific OpenRouter errors
            AlternatorError::OpenRouter(openrouter_error) => match openrouter_error {
                OpenRouterError::RateLimitExceeded { .. } => true,
                OpenRouterError::ApiRequestFailed(_) => true,
                OpenRouterError::TokenLimitExceeded { .. } => false, // Skip this media
                OpenRouterError::AuthenticationFailed => false,      // Not recoverable
                OpenRouterError::InsufficientBalance { .. } => false, // Not recoverable
                _ => false,
            },

            // Media processing errors are generally not recoverable for specific media
            AlternatorError::Media(_) => false,

            // Configuration errors are not recoverable at runtime
            AlternatorError::Config(_) => false,

            // Other errors
            AlternatorError::Io(_) => true,        // May be temporary
            AlternatorError::Json(_) => false,     // Data format issue
            AlternatorError::Url(_) => false,      // Configuration issue
            AlternatorError::TaskJoin(_) => false, // Internal error
            AlternatorError::Shutdown => false,    // Intentional shutdown
            AlternatorError::RateLimit(_) => true,
            AlternatorError::Authentication(_) => false,
            AlternatorError::InvalidData(_) => false,
            AlternatorError::Language(_) => false, // Skip this toot
            AlternatorError::Balance(_) => true,   // May be temporary
        }
    }

    /// Get the recommended retry delay in seconds for recoverable errors
    pub fn retry_delay(error: &AlternatorError, attempt: u32) -> u64 {
        match error {
            AlternatorError::Mastodon(mastodon_error) => match mastodon_error {
                // Rate limit errors should respect the exact retry_after value
                MastodonError::RateLimitExceeded { retry_after } => *retry_after,
                MastodonError::ConnectionFailed(_) | MastodonError::Disconnected(_) => {
                    // Apply exponential backoff, max 60 seconds
                    let base_delay = 1;
                    let exponential_delay = base_delay * 2_u64.pow(attempt.min(6));
                    exponential_delay.min(60)
                }
                _ => {
                    // Apply exponential backoff, max 60 seconds
                    let base_delay = 5;
                    let exponential_delay = base_delay * 2_u64.pow(attempt.min(6));
                    exponential_delay.min(60)
                }
            },
            AlternatorError::OpenRouter(openrouter_error) => match openrouter_error {
                // Rate limit errors should respect the exact retry_after value
                OpenRouterError::RateLimitExceeded { retry_after } => *retry_after,
                _ => {
                    // Apply exponential backoff, max 60 seconds
                    let base_delay = 5;
                    let exponential_delay = base_delay * 2_u64.pow(attempt.min(6));
                    exponential_delay.min(60)
                }
            },
            AlternatorError::Network(_) => {
                // Apply exponential backoff, max 60 seconds
                let base_delay = 2;
                let exponential_delay = base_delay * 2_u64.pow(attempt.min(6));
                exponential_delay.min(60)
            }
            AlternatorError::WebSocket(_) => {
                // Apply exponential backoff, max 60 seconds
                let base_delay = 1;
                let exponential_delay = base_delay * 2_u64.pow(attempt.min(6));
                exponential_delay.min(60)
            }
            _ => {
                // Apply exponential backoff, max 60 seconds
                let base_delay = 5;
                let exponential_delay = base_delay * 2_u64.pow(attempt.min(6));
                exponential_delay.min(60)
            }
        }
    }

    /// Get the maximum number of retry attempts for an error
    pub fn max_retries(error: &AlternatorError) -> u32 {
        match error {
            AlternatorError::Mastodon(mastodon_error) => match mastodon_error {
                MastodonError::ConnectionFailed(_) | MastodonError::Disconnected(_) => 10,
                MastodonError::RateLimitExceeded { .. } => 3,
                _ => 3,
            },
            AlternatorError::OpenRouter(openrouter_error) => match openrouter_error {
                OpenRouterError::RateLimitExceeded { .. } => 3,
                _ => 3,
            },
            AlternatorError::Network(_) => 5,
            AlternatorError::WebSocket(_) => 10,
            _ => 3,
        }
    }

    /// Determine if an error should cause application shutdown
    pub fn should_shutdown(error: &AlternatorError) -> bool {
        match error {
            AlternatorError::Config(_) => true, // Configuration errors are fatal
            AlternatorError::Shutdown => true,  // Intentional shutdown
            AlternatorError::Mastodon(MastodonError::AuthenticationFailed(_)) => true,
            AlternatorError::OpenRouter(OpenRouterError::AuthenticationFailed) => true,
            _ => false,
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_alternator_error_display() {
        let config_error = ConfigError::MissingRequired("test_field".to_string());
        let error = AlternatorError::Config(config_error);
        assert!(error.to_string().contains("Configuration error"));
        assert!(error.to_string().contains("test_field"));
    }

    #[test]
    fn test_mastodon_error_variants() {
        let connection_error = MastodonError::ConnectionFailed("timeout".to_string());
        assert!(connection_error
            .to_string()
            .contains("WebSocket connection failed"));

        let rate_limit_error = MastodonError::RateLimitExceeded { retry_after: 60 };
        assert!(rate_limit_error.to_string().contains("Rate limit exceeded"));
        assert!(rate_limit_error.to_string().contains("60 seconds"));

        let toot_not_found = MastodonError::TootNotFound {
            toot_id: "123".to_string(),
        };
        assert!(toot_not_found.to_string().contains("Toot not found: 123"));
    }

    #[test]
    fn test_openrouter_error_variants() {
        let token_limit_error = OpenRouterError::TokenLimitExceeded {
            tokens_used: 200,
            max_tokens: 150,
        };
        assert!(token_limit_error
            .to_string()
            .contains("Token limit exceeded"));
        assert!(token_limit_error.to_string().contains("200/150"));

        let balance_error = OpenRouterError::InsufficientBalance {
            balance: 2.50,
            minimum: 5.0,
        };
        assert!(balance_error.to_string().contains("Insufficient balance"));
        assert!(balance_error.to_string().contains("$2.5"));

        let image_size_error = OpenRouterError::ImageTooLarge {
            size_mb: 15.0,
            max_mb: 10.0,
        };
        assert!(image_size_error.to_string().contains("Image too large"));
        assert!(image_size_error.to_string().contains("15MB"));
    }

    #[test]
    fn test_media_error_variants() {
        let unsupported_error = MediaError::UnsupportedType {
            media_type: "video/mp4".to_string(),
        };
        assert!(unsupported_error
            .to_string()
            .contains("Unsupported media type"));
        assert!(unsupported_error.to_string().contains("video/mp4"));

        let size_error = MediaError::ImageTooLarge {
            width: 4000,
            height: 3000,
            max_dimension: 1024,
        };
        assert!(size_error.to_string().contains("Image too large"));
        assert!(size_error.to_string().contains("4000x3000"));
        assert!(size_error.to_string().contains("1024"));
    }

    #[test]
    fn test_language_error_variants() {
        let detection_error = LanguageError::DetectionFailed("no text".to_string());
        assert!(detection_error
            .to_string()
            .contains("Language detection failed"));

        let unsupported_error = LanguageError::UnsupportedLanguage {
            language: "xyz".to_string(),
        };
        assert!(unsupported_error
            .to_string()
            .contains("Unsupported language: xyz"));

        let template_error = LanguageError::PromptTemplateNotFound {
            language: "fr".to_string(),
        };
        assert!(template_error
            .to_string()
            .contains("Prompt template not found"));
        assert!(template_error.to_string().contains("fr"));
    }

    #[test]
    fn test_balance_error_variants() {
        let check_error = BalanceError::CheckFailed("network timeout".to_string());
        assert!(check_error.to_string().contains("Balance check failed"));

        let threshold_error = BalanceError::InvalidThreshold { threshold: -1.0 };
        assert!(threshold_error
            .to_string()
            .contains("Invalid balance threshold"));
        assert!(threshold_error.to_string().contains("-1"));

        let time_error = BalanceError::InvalidCheckTime {
            time: "25:00".to_string(),
        };
        assert!(time_error.to_string().contains("Invalid check time format"));
        assert!(time_error.to_string().contains("25:00"));
    }

    #[test]
    fn test_error_recovery_is_recoverable() {
        // Recoverable errors - create a simple network error for testing
        let io_error = std::io::Error::new(std::io::ErrorKind::TimedOut, "timeout");
        let network_error = AlternatorError::Io(io_error);
        assert!(ErrorRecovery::is_recoverable(&network_error));

        let websocket_error =
            AlternatorError::WebSocket(tokio_tungstenite::tungstenite::Error::ConnectionClosed);
        assert!(ErrorRecovery::is_recoverable(&websocket_error));

        let mastodon_connection_error =
            AlternatorError::Mastodon(MastodonError::ConnectionFailed("timeout".to_string()));
        assert!(ErrorRecovery::is_recoverable(&mastodon_connection_error));

        let mastodon_rate_limit =
            AlternatorError::Mastodon(MastodonError::RateLimitExceeded { retry_after: 60 });
        assert!(ErrorRecovery::is_recoverable(&mastodon_rate_limit));

        let openrouter_rate_limit =
            AlternatorError::OpenRouter(OpenRouterError::RateLimitExceeded { retry_after: 30 });
        assert!(ErrorRecovery::is_recoverable(&openrouter_rate_limit));

        // Non-recoverable errors
        let config_error =
            AlternatorError::Config(ConfigError::MissingRequired("test".to_string()));
        assert!(!ErrorRecovery::is_recoverable(&config_error));

        let mastodon_auth_error = AlternatorError::Mastodon(MastodonError::AuthenticationFailed(
            "invalid token".to_string(),
        ));
        assert!(!ErrorRecovery::is_recoverable(&mastodon_auth_error));

        let openrouter_auth_error =
            AlternatorError::OpenRouter(OpenRouterError::AuthenticationFailed);
        assert!(!ErrorRecovery::is_recoverable(&openrouter_auth_error));

        let token_limit_error = AlternatorError::OpenRouter(OpenRouterError::TokenLimitExceeded {
            tokens_used: 200,
            max_tokens: 150,
        });
        assert!(!ErrorRecovery::is_recoverable(&token_limit_error));

        let media_error = AlternatorError::Media(MediaError::UnsupportedType {
            media_type: "video/mp4".to_string(),
        });
        assert!(!ErrorRecovery::is_recoverable(&media_error));
    }

    #[test]
    fn test_error_recovery_retry_delay() {
        // Test exponential backoff - use IO error for testing
        let io_error = std::io::Error::new(std::io::ErrorKind::TimedOut, "timeout");
        let network_error = AlternatorError::Io(io_error);

        assert_eq!(ErrorRecovery::retry_delay(&network_error, 0), 5);
        assert_eq!(ErrorRecovery::retry_delay(&network_error, 1), 10);
        assert_eq!(ErrorRecovery::retry_delay(&network_error, 2), 20);
        assert_eq!(ErrorRecovery::retry_delay(&network_error, 3), 40);

        // Test max delay cap
        assert_eq!(ErrorRecovery::retry_delay(&network_error, 10), 60);

        // Test rate limit specific delay - should respect the retry_after value even if > 60
        let rate_limit_error =
            AlternatorError::Mastodon(MastodonError::RateLimitExceeded { retry_after: 120 });
        assert_eq!(ErrorRecovery::retry_delay(&rate_limit_error, 0), 120);

        // Test WebSocket connection delay
        let websocket_error =
            AlternatorError::WebSocket(tokio_tungstenite::tungstenite::Error::ConnectionClosed);
        assert_eq!(ErrorRecovery::retry_delay(&websocket_error, 0), 1);
        assert_eq!(ErrorRecovery::retry_delay(&websocket_error, 1), 2);
        assert_eq!(ErrorRecovery::retry_delay(&websocket_error, 2), 4);
    }

    #[test]
    fn test_error_recovery_max_retries() {
        let io_error = std::io::Error::new(std::io::ErrorKind::TimedOut, "timeout");
        let network_error = AlternatorError::Io(io_error);
        assert_eq!(ErrorRecovery::max_retries(&network_error), 3);

        let websocket_error =
            AlternatorError::WebSocket(tokio_tungstenite::tungstenite::Error::ConnectionClosed);
        assert_eq!(ErrorRecovery::max_retries(&websocket_error), 10);

        let mastodon_connection_error =
            AlternatorError::Mastodon(MastodonError::ConnectionFailed("timeout".to_string()));
        assert_eq!(ErrorRecovery::max_retries(&mastodon_connection_error), 10);

        let rate_limit_error =
            AlternatorError::Mastodon(MastodonError::RateLimitExceeded { retry_after: 60 });
        assert_eq!(ErrorRecovery::max_retries(&rate_limit_error), 3);

        let config_error =
            AlternatorError::Config(ConfigError::MissingRequired("test".to_string()));
        assert_eq!(ErrorRecovery::max_retries(&config_error), 3);
    }

    #[test]
    fn test_error_recovery_should_shutdown() {
        // Errors that should cause shutdown
        let config_error =
            AlternatorError::Config(ConfigError::MissingRequired("test".to_string()));
        assert!(ErrorRecovery::should_shutdown(&config_error));

        let shutdown_error = AlternatorError::Shutdown;
        assert!(ErrorRecovery::should_shutdown(&shutdown_error));

        let mastodon_auth_error = AlternatorError::Mastodon(MastodonError::AuthenticationFailed(
            "invalid token".to_string(),
        ));
        assert!(ErrorRecovery::should_shutdown(&mastodon_auth_error));

        let openrouter_auth_error =
            AlternatorError::OpenRouter(OpenRouterError::AuthenticationFailed);
        assert!(ErrorRecovery::should_shutdown(&openrouter_auth_error));

        // Errors that should not cause shutdown
        let io_error = std::io::Error::new(std::io::ErrorKind::TimedOut, "timeout");
        let network_error = AlternatorError::Io(io_error);
        assert!(!ErrorRecovery::should_shutdown(&network_error));

        let media_error = AlternatorError::Media(MediaError::UnsupportedType {
            media_type: "video/mp4".to_string(),
        });
        assert!(!ErrorRecovery::should_shutdown(&media_error));

        let rate_limit_error =
            AlternatorError::Mastodon(MastodonError::RateLimitExceeded { retry_after: 60 });
        assert!(!ErrorRecovery::should_shutdown(&rate_limit_error));
    }

    #[test]
    fn test_error_conversion_from_std_errors() {
        // Test conversion from std::io::Error
        let io_error = std::io::Error::new(std::io::ErrorKind::NotFound, "file not found");
        let alternator_error = AlternatorError::from(io_error);
        assert!(matches!(alternator_error, AlternatorError::Io(_)));

        // Test conversion from serde_json::Error
        let json_error = serde_json::from_str::<serde_json::Value>("invalid json").unwrap_err();
        let alternator_error = AlternatorError::from(json_error);
        assert!(matches!(alternator_error, AlternatorError::Json(_)));

        // Test conversion from url::ParseError
        let url_error = url::Url::parse("not a url").unwrap_err();
        let alternator_error = AlternatorError::from(url_error);
        assert!(matches!(alternator_error, AlternatorError::Url(_)));
    }

    #[test]
    fn test_nested_error_conversion() {
        // Test that nested errors are properly converted
        let config_error = ConfigError::MissingRequired("api_key".to_string());
        let alternator_error = AlternatorError::from(config_error);

        match alternator_error {
            AlternatorError::Config(inner) => {
                assert!(inner.to_string().contains("api_key"));
            }
            _ => panic!("Expected Config error variant"),
        }

        let mastodon_error = MastodonError::TootNotFound {
            toot_id: "123".to_string(),
        };
        let alternator_error = AlternatorError::from(mastodon_error);

        match alternator_error {
            AlternatorError::Mastodon(inner) => {
                assert!(inner.to_string().contains("123"));
            }
            _ => panic!("Expected Mastodon error variant"),
        }
    }
}
