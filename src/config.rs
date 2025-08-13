use serde::{Deserialize, Serialize};
use std::env;
use std::path::PathBuf;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("TOML parsing error: {0}")]
    Toml(#[from] toml::de::Error),
    #[error("Missing required configuration: {0}")]
    MissingRequired(String),
    #[error("Invalid configuration value: {0}")]
    InvalidValue(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub mastodon: MastodonConfig,
    pub openrouter: OpenRouterConfig,
    pub media: Option<MediaConfig>,
    pub balance: Option<BalanceConfig>,
    pub logging: Option<LoggingConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MastodonConfig {
    pub instance_url: String,
    pub access_token: String,
    pub user_stream: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenRouterConfig {
    pub api_key: String,
    pub model: String,
    pub base_url: Option<String>,
    pub max_tokens: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MediaConfig {
    pub max_size_mb: Option<u32>,
    pub supported_formats: Option<Vec<String>>,
    pub resize_max_dimension: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BalanceConfig {
    pub enabled: Option<bool>,
    pub threshold: Option<f64>,
    pub check_time: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoggingConfig {
    pub level: Option<String>,
}

impl Default for MediaConfig {
    fn default() -> Self {
        Self {
            max_size_mb: Some(10),
            supported_formats: Some(vec![
                "image/jpeg".to_string(),
                "image/png".to_string(),
                "image/gif".to_string(),
                "image/webp".to_string(),
            ]),
            resize_max_dimension: Some(1024),
        }
    }
}

impl Default for BalanceConfig {
    fn default() -> Self {
        Self {
            enabled: Some(true),
            threshold: Some(5.0),
            check_time: Some("12:00".to_string()),
        }
    }
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            level: Some("info".to_string()),
        }
    }
}

impl Config {
    /// Load configuration from TOML file with XDG directory support and environment variable overrides
    pub fn load(config_path: Option<PathBuf>) -> Result<Self, ConfigError> {
        let config_file = if let Some(path) = config_path {
            path
        } else {
            Self::find_config_file()?
        };

        let mut config = if config_file.exists() {
            let content = std::fs::read_to_string(&config_file)?;
            toml::from_str::<Config>(&content)?
        } else {
            // Create a minimal config structure that will be populated by env vars
            Config {
                mastodon: MastodonConfig {
                    instance_url: String::new(),
                    access_token: String::new(),
                    user_stream: None,
                },
                openrouter: OpenRouterConfig {
                    api_key: String::new(),
                    model: "anthropic/claude-3-haiku".to_string(),
                    base_url: None,
                    max_tokens: None,
                },
                media: None,
                balance: None,
                logging: None,
            }
        };

        // Apply environment variable overrides
        config.apply_env_overrides()?;

        // Apply defaults for optional sections
        if config.media.is_none() {
            config.media = Some(MediaConfig::default());
        }
        if config.balance.is_none() {
            config.balance = Some(BalanceConfig::default());
        }
        if config.logging.is_none() {
            config.logging = Some(LoggingConfig::default());
        }

        // Validate required fields
        config.validate()?;

        Ok(config)
    }

    /// Find configuration file using XDG directory support
    fn find_config_file() -> Result<PathBuf, ConfigError> {
        // First check current directory
        let current_dir_config = PathBuf::from("alternator.toml");
        if current_dir_config.exists() {
            return Ok(current_dir_config);
        }

        // Then check XDG_CONFIG_HOME/alternator/alternator.toml
        if let Ok(xdg_config_home) = env::var("XDG_CONFIG_HOME") {
            let xdg_config = PathBuf::from(xdg_config_home)
                .join("alternator")
                .join("alternator.toml");
            if xdg_config.exists() {
                return Ok(xdg_config);
            }
        }

        // Default to current directory (file may not exist yet)
        Ok(current_dir_config)
    }

    /// Apply environment variable overrides to configuration
    fn apply_env_overrides(&mut self) -> Result<(), ConfigError> {
        // Mastodon configuration
        if let Ok(instance_url) = env::var("ALTERNATOR_MASTODON_INSTANCE_URL") {
            self.mastodon.instance_url = instance_url;
        }
        if let Ok(access_token) = env::var("ALTERNATOR_MASTODON_ACCESS_TOKEN") {
            self.mastodon.access_token = access_token;
        }
        if let Ok(user_stream) = env::var("ALTERNATOR_MASTODON_USER_STREAM") {
            self.mastodon.user_stream = Some(user_stream.parse().map_err(|_| {
                ConfigError::InvalidValue(
                    "ALTERNATOR_MASTODON_USER_STREAM must be true or false".to_string(),
                )
            })?);
        }

        // OpenRouter configuration
        if let Ok(api_key) = env::var("ALTERNATOR_OPENROUTER_API_KEY") {
            self.openrouter.api_key = api_key;
        }
        if let Ok(model) = env::var("ALTERNATOR_OPENROUTER_MODEL") {
            self.openrouter.model = model;
        }
        if let Ok(base_url) = env::var("ALTERNATOR_OPENROUTER_BASE_URL") {
            self.openrouter.base_url = Some(base_url);
        }
        if let Ok(max_tokens) = env::var("ALTERNATOR_OPENROUTER_MAX_TOKENS") {
            self.openrouter.max_tokens = Some(max_tokens.parse().map_err(|_| {
                ConfigError::InvalidValue(
                    "ALTERNATOR_OPENROUTER_MAX_TOKENS must be a valid number".to_string(),
                )
            })?);
        }

        // Balance configuration
        if let Ok(enabled) = env::var("ALTERNATOR_BALANCE_ENABLED") {
            let balance = self.balance.get_or_insert_with(BalanceConfig::default);
            balance.enabled = Some(enabled.parse().map_err(|_| {
                ConfigError::InvalidValue(
                    "ALTERNATOR_BALANCE_ENABLED must be true or false".to_string(),
                )
            })?);
        }
        if let Ok(threshold) = env::var("ALTERNATOR_BALANCE_THRESHOLD") {
            let balance = self.balance.get_or_insert_with(BalanceConfig::default);
            balance.threshold = Some(threshold.parse().map_err(|_| {
                ConfigError::InvalidValue(
                    "ALTERNATOR_BALANCE_THRESHOLD must be a valid number".to_string(),
                )
            })?);
        }
        if let Ok(check_time) = env::var("ALTERNATOR_BALANCE_CHECK_TIME") {
            let balance = self.balance.get_or_insert_with(BalanceConfig::default);
            balance.check_time = Some(check_time);
        }

        // Logging configuration
        if let Ok(level) = env::var("ALTERNATOR_LOG_LEVEL") {
            let logging = self.logging.get_or_insert_with(LoggingConfig::default);
            logging.level = Some(level);
        }

        Ok(())
    }

    /// Validate that all required configuration is present
    fn validate(&self) -> Result<(), ConfigError> {
        if self.mastodon.instance_url.is_empty() {
            return Err(ConfigError::MissingRequired(
                "mastodon.instance_url or ALTERNATOR_MASTODON_INSTANCE_URL".to_string(),
            ));
        }

        if self.mastodon.access_token.is_empty() {
            return Err(ConfigError::MissingRequired(
                "mastodon.access_token or ALTERNATOR_MASTODON_ACCESS_TOKEN".to_string(),
            ));
        }

        if self.openrouter.api_key.is_empty() {
            return Err(ConfigError::MissingRequired(
                "openrouter.api_key or ALTERNATOR_OPENROUTER_API_KEY".to_string(),
            ));
        }

        if self.openrouter.model.is_empty() {
            return Err(ConfigError::MissingRequired(
                "openrouter.model or ALTERNATOR_OPENROUTER_MODEL".to_string(),
            ));
        }

        // Validate balance check_time format if provided
        if let Some(ref balance) = self.balance {
            if let Some(ref check_time) = balance.check_time {
                if !check_time.contains(':') || check_time.split(':').count() != 2 {
                    return Err(ConfigError::InvalidValue(
                        "balance.check_time must be in HH:MM format".to_string(),
                    ));
                }
            }
        }

        Ok(())
    }

    /// Get the OpenRouter base URL with default fallback
    pub fn openrouter_base_url(&self) -> &str {
        self.openrouter
            .base_url
            .as_deref()
            .unwrap_or("https://openrouter.ai/api/v1")
    }

    /// Get the media configuration with defaults
    pub fn media(&self) -> &MediaConfig {
        self.media.as_ref().unwrap()
    }

    /// Get the balance configuration with defaults
    pub fn balance(&self) -> &BalanceConfig {
        self.balance.as_ref().unwrap()
    }

    /// Get the logging configuration with defaults
    pub fn logging(&self) -> &LoggingConfig {
        self.logging.as_ref().unwrap()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;

    #[test]
    fn test_config_defaults() {
        let media = MediaConfig::default();
        assert_eq!(media.max_size_mb, Some(10));
        assert_eq!(media.resize_max_dimension, Some(1024));
        assert!(media
            .supported_formats
            .as_ref()
            .unwrap()
            .contains(&"image/jpeg".to_string()));

        let balance = BalanceConfig::default();
        assert_eq!(balance.enabled, Some(true));
        assert_eq!(balance.threshold, Some(5.0));
        assert_eq!(balance.check_time, Some("12:00".to_string()));

        let logging = LoggingConfig::default();
        assert_eq!(logging.level, Some("info".to_string()));
    }

    #[test]
    fn test_config_validation_missing_required() {
        let config = Config {
            mastodon: MastodonConfig {
                instance_url: String::new(),
                access_token: "token".to_string(),
                user_stream: None,
            },
            openrouter: OpenRouterConfig {
                api_key: "key".to_string(),
                model: "model".to_string(),
                base_url: None,
                max_tokens: None,
            },
            media: None,
            balance: None,
            logging: None,
        };

        let result = config.validate();
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("mastodon.instance_url"));
    }

    #[test]
    fn test_config_validation_invalid_time_format() {
        let config = Config {
            mastodon: MastodonConfig {
                instance_url: "https://mastodon.social".to_string(),
                access_token: "token".to_string(),
                user_stream: None,
            },
            openrouter: OpenRouterConfig {
                api_key: "key".to_string(),
                model: "model".to_string(),
                base_url: None,
                max_tokens: None,
            },
            media: None,
            balance: Some(BalanceConfig {
                enabled: Some(true),
                threshold: Some(5.0),
                check_time: Some("invalid".to_string()),
            }),
            logging: None,
        };

        let result = config.validate();
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("HH:MM format"));
    }

    #[test]
    fn test_env_var_overrides() {
        // Set environment variables
        env::set_var("ALTERNATOR_MASTODON_INSTANCE_URL", "https://test.social");
        env::set_var("ALTERNATOR_MASTODON_ACCESS_TOKEN", "test_token");
        env::set_var("ALTERNATOR_OPENROUTER_API_KEY", "test_key");
        env::set_var("ALTERNATOR_OPENROUTER_MODEL", "test_model");
        env::set_var("ALTERNATOR_OPENROUTER_MAX_TOKENS", "200");
        env::set_var("ALTERNATOR_BALANCE_ENABLED", "false");
        env::set_var("ALTERNATOR_BALANCE_THRESHOLD", "10.5");

        let mut config = Config {
            mastodon: MastodonConfig {
                instance_url: String::new(),
                access_token: String::new(),
                user_stream: None,
            },
            openrouter: OpenRouterConfig {
                api_key: String::new(),
                model: String::new(),
                base_url: None,
                max_tokens: None,
            },
            media: None,
            balance: None,
            logging: None,
        };

        config.apply_env_overrides().unwrap();

        assert_eq!(config.mastodon.instance_url, "https://test.social");
        assert_eq!(config.mastodon.access_token, "test_token");
        assert_eq!(config.openrouter.api_key, "test_key");
        assert_eq!(config.openrouter.model, "test_model");
        assert_eq!(config.openrouter.max_tokens, Some(200));
        assert_eq!(config.balance.as_ref().unwrap().enabled, Some(false));
        assert_eq!(config.balance.as_ref().unwrap().threshold, Some(10.5));

        // Clean up environment variables
        env::remove_var("ALTERNATOR_MASTODON_INSTANCE_URL");
        env::remove_var("ALTERNATOR_MASTODON_ACCESS_TOKEN");
        env::remove_var("ALTERNATOR_OPENROUTER_API_KEY");
        env::remove_var("ALTERNATOR_OPENROUTER_MODEL");
        env::remove_var("ALTERNATOR_OPENROUTER_MAX_TOKENS");
        env::remove_var("ALTERNATOR_BALANCE_ENABLED");
        env::remove_var("ALTERNATOR_BALANCE_THRESHOLD");
    }

    #[test]
    fn test_toml_parsing() {
        let toml_content = r#"
[mastodon]
instance_url = "https://mastodon.social"
access_token = "your_token_here"
user_stream = true

[openrouter]
api_key = "your_api_key_here"
model = "anthropic/claude-3-haiku"
base_url = "https://openrouter.ai/api/v1"
max_tokens = 150

[media]
max_size_mb = 10
supported_formats = ["image/jpeg", "image/png", "image/gif", "image/webp"]
resize_max_dimension = 1024

[balance]
enabled = true
threshold = 5.0
check_time = "12:00"

[logging]
level = "info"
"#;

        let config: Config = toml::from_str(toml_content).unwrap();

        assert_eq!(config.mastodon.instance_url, "https://mastodon.social");
        assert_eq!(config.mastodon.access_token, "your_token_here");
        assert_eq!(config.mastodon.user_stream, Some(true));

        assert_eq!(config.openrouter.api_key, "your_api_key_here");
        assert_eq!(config.openrouter.model, "anthropic/claude-3-haiku");
        assert_eq!(
            config.openrouter.base_url,
            Some("https://openrouter.ai/api/v1".to_string())
        );
        assert_eq!(config.openrouter.max_tokens, Some(150));

        assert_eq!(config.media.as_ref().unwrap().max_size_mb, Some(10));
        assert_eq!(config.balance.as_ref().unwrap().enabled, Some(true));
        assert_eq!(config.balance.as_ref().unwrap().threshold, Some(5.0));
        assert_eq!(
            config.logging.as_ref().unwrap().level,
            Some("info".to_string())
        );
    }

    #[test]
    fn test_openrouter_base_url_default() {
        let config = Config {
            mastodon: MastodonConfig {
                instance_url: "https://mastodon.social".to_string(),
                access_token: "token".to_string(),
                user_stream: None,
            },
            openrouter: OpenRouterConfig {
                api_key: "key".to_string(),
                model: "model".to_string(),
                base_url: None,
                max_tokens: None,
            },
            media: None,
            balance: None,
            logging: None,
        };

        assert_eq!(config.openrouter_base_url(), "https://openrouter.ai/api/v1");
    }
}
