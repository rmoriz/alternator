use crate::config::OpenRouterConfig;
use crate::error::OpenRouterError;
use base64::prelude::*;
use reqwest::{Client, Response};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Semaphore;
use tokio::time::sleep;
use tracing::{debug, error, info, warn};

/// Rate limiter for API calls with exponential backoff
#[derive(Debug)]
pub struct RateLimiter {
    semaphore: Arc<Semaphore>,
    last_request: Option<Instant>,
    min_interval: Duration,
}

impl RateLimiter {
    pub fn new(max_concurrent: usize, min_interval_ms: u64) -> Self {
        Self {
            semaphore: Arc::new(Semaphore::new(max_concurrent)),
            last_request: None,
            min_interval: Duration::from_millis(min_interval_ms),
        }
    }

    /// Acquire a permit and enforce minimum interval between requests
    pub async fn acquire(&mut self) -> tokio::sync::SemaphorePermit<'_> {
        let permit = self.semaphore.acquire().await.unwrap();

        if let Some(last) = self.last_request {
            let elapsed = last.elapsed();
            if elapsed < self.min_interval {
                let sleep_duration = self.min_interval - elapsed;
                debug!("Rate limiting: sleeping for {:?}", sleep_duration);
                sleep(sleep_duration).await;
            }
        }

        self.last_request = Some(Instant::now());
        permit
    }
}

/// OpenRouter API client with rate limiting and cost controls
#[derive(Debug)]
pub struct OpenRouterClient {
    config: OpenRouterConfig,
    http_client: Client,
    rate_limiter: Arc<tokio::sync::Mutex<RateLimiter>>,
}

/// OpenRouter model information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Model {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub pricing: Option<ModelPricing>,
    pub context_length: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelPricing {
    pub prompt: String,
    pub completion: String,
}

/// Account balance response
#[derive(Debug, Serialize, Deserialize)]
pub struct AccountBalance {
    pub data: BalanceData,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct BalanceData {
    pub balance: f64,
}

/// Image description request
#[derive(Debug, Serialize)]
pub struct ImageDescriptionRequest {
    pub model: String,
    pub messages: Vec<Message>,
    pub max_tokens: Option<u32>,
}

#[derive(Debug, Serialize)]
pub struct Message {
    pub role: String,
    pub content: Vec<ContentPart>,
}

#[derive(Debug, Serialize)]
#[serde(tag = "type")]
pub enum ContentPart {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "image_url")]
    ImageUrl { image_url: ImageUrl },
}

#[derive(Debug, Serialize)]
pub struct ImageUrl {
    pub url: String,
}

/// Image description response
#[derive(Debug, Deserialize)]
pub struct ImageDescriptionResponse {
    pub choices: Vec<Choice>,
    pub usage: Option<Usage>,
}

#[derive(Debug, Deserialize)]
pub struct Choice {
    pub message: ResponseMessage,
    pub finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ResponseMessage {
    pub content: String,
}

#[derive(Debug, Deserialize)]
pub struct Usage {
    pub prompt_tokens: Option<u32>,
    pub completion_tokens: Option<u32>,
    pub total_tokens: Option<u32>,
}

/// Models list response
#[derive(Debug, Deserialize)]
pub struct ModelsResponse {
    pub data: Vec<Model>,
}

/// Error response from OpenRouter API
#[derive(Debug, Deserialize)]
pub struct ErrorResponse {
    pub error: ErrorDetail,
}

#[derive(Debug, Deserialize)]
pub struct ErrorDetail {
    pub message: String,
    pub code: Option<String>,
    #[serde(rename = "type")]
    pub error_type: Option<String>,
}

impl OpenRouterClient {
    /// Create a new OpenRouter client with rate limiting
    pub fn new(config: OpenRouterConfig) -> Self {
        let http_client = Client::builder()
            .timeout(Duration::from_secs(60))
            .build()
            .expect("Failed to create HTTP client");

        // Rate limiter: max 5 concurrent requests, minimum 200ms between requests
        let rate_limiter = Arc::new(tokio::sync::Mutex::new(RateLimiter::new(5, 200)));

        Self {
            config,
            http_client,
            rate_limiter,
        }
    }

    /// Get the base URL for OpenRouter API
    fn base_url(&self) -> &str {
        self.config
            .base_url
            .as_deref()
            .unwrap_or("https://openrouter.ai/api/v1")
    }

    /// Handle API response and extract errors
    async fn handle_response<T>(&self, response: Response) -> Result<T, OpenRouterError>
    where
        T: for<'de> Deserialize<'de>,
    {
        let status = response.status();
        let headers = response.headers().clone();

        // Check for rate limiting
        if status == 429 {
            let retry_after = headers
                .get("retry-after")
                .and_then(|h| h.to_str().ok())
                .and_then(|s| s.parse::<u64>().ok())
                .unwrap_or(60);

            warn!(
                "OpenRouter rate limit exceeded, retry after {} seconds",
                retry_after
            );
            return Err(OpenRouterError::RateLimitExceeded { retry_after });
        }

        // Check for authentication errors
        if status == 401 {
            error!("OpenRouter authentication failed - check API key");
            return Err(OpenRouterError::AuthenticationFailed);
        }

        let response_text = response.text().await.map_err(|e| {
            OpenRouterError::ApiRequestFailed(format!("Failed to read response: {}", e))
        })?;

        if !status.is_success() {
            // Try to parse error response
            if let Ok(error_response) = serde_json::from_str::<ErrorResponse>(&response_text) {
                let error_msg = error_response.error.message;

                // Check for token limit exceeded
                if error_msg.to_lowercase().contains("token")
                    && error_msg.to_lowercase().contains("limit")
                {
                    // Extract token information if available
                    let max_tokens = self.config.max_tokens.unwrap_or(150);
                    return Err(OpenRouterError::TokenLimitExceeded {
                        tokens_used: max_tokens,
                        max_tokens,
                    });
                }

                // Check for insufficient balance
                if error_msg.to_lowercase().contains("balance")
                    || error_msg.to_lowercase().contains("credit")
                {
                    return Err(OpenRouterError::InsufficientBalance {
                        balance: 0.0,
                        minimum: 0.01,
                    });
                }

                return Err(OpenRouterError::ApiRequestFailed(error_msg));
            }

            return Err(OpenRouterError::ApiRequestFailed(format!(
                "HTTP {} - {}",
                status, response_text
            )));
        }

        serde_json::from_str(&response_text).map_err(|e| {
            error!("Failed to parse OpenRouter response: {}", e);
            debug!("Response text: {}", response_text);
            OpenRouterError::InvalidResponse(format!("JSON parsing failed: {}", e))
        })
    }

    /// Perform API request with exponential backoff
    async fn api_request_with_retry<T>(
        &self,
        request_fn: impl Fn() -> reqwest::RequestBuilder,
        max_retries: u32,
    ) -> Result<T, OpenRouterError>
    where
        T: for<'de> Deserialize<'de>,
    {
        let mut attempt = 0;

        loop {
            // Acquire rate limiting permit
            {
                let mut limiter = self.rate_limiter.lock().await;
                let _permit = limiter.acquire().await;
                // Permit is dropped here, but rate limiting is enforced by the acquire() call
            }

            debug!("Making OpenRouter API request (attempt {})", attempt + 1);

            let response = request_fn()
                .header("Authorization", format!("Bearer {}", self.config.api_key))
                .header("Content-Type", "application/json")
                .header("HTTP-Referer", "https://github.com/rmoriz/alternator")
                .header("X-Title", "Alternator - Mastodon Media Describer")
                .send()
                .await
                .map_err(|e| OpenRouterError::ApiRequestFailed(format!("Request failed: {}", e)))?;

            match self.handle_response::<T>(response).await {
                Ok(result) => {
                    debug!("OpenRouter API request successful");
                    return Ok(result);
                }
                Err(OpenRouterError::RateLimitExceeded { retry_after }) => {
                    if attempt >= max_retries {
                        error!("Max retries exceeded for rate limited request");
                        return Err(OpenRouterError::RateLimitExceeded { retry_after });
                    }

                    warn!("Rate limited, waiting {} seconds before retry", retry_after);
                    sleep(Duration::from_secs(retry_after)).await;
                    attempt += 1;
                    continue;
                }
                Err(OpenRouterError::TokenLimitExceeded { .. }) => {
                    // Don't retry token limit errors - this is a configuration issue
                    warn!("Token limit exceeded - skipping this request");
                    return Err(OpenRouterError::TokenLimitExceeded {
                        tokens_used: self.config.max_tokens.unwrap_or(150),
                        max_tokens: self.config.max_tokens.unwrap_or(150),
                    });
                }
                Err(OpenRouterError::AuthenticationFailed) => {
                    // Don't retry authentication failures
                    error!("Authentication failed - check API key");
                    return Err(OpenRouterError::AuthenticationFailed);
                }
                Err(OpenRouterError::InsufficientBalance { .. }) => {
                    // Don't retry balance errors
                    error!("Insufficient balance - please top up your account");
                    return Err(OpenRouterError::InsufficientBalance {
                        balance: 0.0,
                        minimum: 0.01,
                    });
                }
                Err(e) => {
                    if attempt >= max_retries {
                        error!("Max retries exceeded: {}", e);
                        return Err(e);
                    }

                    let delay = 2_u64.pow(attempt) * 1000; // Exponential backoff in ms
                    let delay = delay.min(30000); // Cap at 30 seconds

                    warn!(
                        "API request failed (attempt {}): {}, retrying in {}ms",
                        attempt + 1,
                        e,
                        delay
                    );
                    sleep(Duration::from_millis(delay)).await;
                    attempt += 1;
                }
            }
        }
    }

    /// Get account balance for startup validation
    pub async fn get_account_balance(&self) -> Result<f64, OpenRouterError> {
        info!("Checking OpenRouter account balance");

        let response: AccountBalance = self
            .api_request_with_retry(
                || {
                    self.http_client
                        .get(&format!("{}/auth/key", self.base_url()))
                },
                3,
            )
            .await?;

        let balance = response.data.balance;
        info!("OpenRouter account balance: ${:.2}", balance);

        Ok(balance)
    }

    /// List available models for startup validation
    pub async fn list_models(&self) -> Result<Vec<Model>, OpenRouterError> {
        info!("Fetching OpenRouter model list");

        let response: ModelsResponse = self
            .api_request_with_retry(
                || self.http_client.get(&format!("{}/models", self.base_url())),
                3,
            )
            .await?;

        let models = response.data;
        info!("Retrieved {} models from OpenRouter", models.len());

        // Check if configured model is available
        let configured_model = &self.config.model;
        let model_available = models.iter().any(|m| m.id == *configured_model);

        if !model_available {
            warn!(
                "Configured model '{}' not found in available models",
                configured_model
            );
            return Err(OpenRouterError::ModelNotAvailable {
                model: configured_model.clone(),
            });
        }

        info!("Configured model '{}' is available", configured_model);
        Ok(models)
    }

    /// Generate description for an image using OpenRouter API
    pub async fn describe_image(
        &self,
        image_data: &[u8],
        prompt: &str,
    ) -> Result<String, OpenRouterError> {
        debug!(
            "Generating image description using model: {}",
            self.config.model
        );

        // Validate image size
        let size_mb = image_data.len() as f64 / (1024.0 * 1024.0);
        if size_mb > 10.0 {
            return Err(OpenRouterError::ImageTooLarge {
                size_mb,
                max_mb: 10.0,
            });
        }

        // Convert image to base64 data URL
        let base64_image = base64::prelude::BASE64_STANDARD.encode(image_data);
        let data_url = format!("data:image/jpeg;base64,{}", base64_image);

        let request = ImageDescriptionRequest {
            model: self.config.model.clone(),
            messages: vec![Message {
                role: "user".to_string(),
                content: vec![
                    ContentPart::Text {
                        text: prompt.to_string(),
                    },
                    ContentPart::ImageUrl {
                        image_url: ImageUrl { url: data_url },
                    },
                ],
            }],
            max_tokens: self.config.max_tokens,
        };

        let response: ImageDescriptionResponse = self
            .api_request_with_retry(
                || {
                    self.http_client
                        .post(&format!("{}/chat/completions", self.base_url()))
                        .json(&request)
                },
                2, // Only retry twice for image description to avoid excessive costs
            )
            .await?;

        if response.choices.is_empty() {
            return Err(OpenRouterError::InvalidResponse(
                "No choices in response".to_string(),
            ));
        }

        let description = response.choices[0].message.content.trim().to_string();

        // Log token usage if available
        if let Some(usage) = response.usage {
            debug!(
                "Token usage - Prompt: {:?}, Completion: {:?}, Total: {:?}",
                usage.prompt_tokens, usage.completion_tokens, usage.total_tokens
            );

            // Check if we hit the token limit
            if let Some(max_tokens) = self.config.max_tokens {
                if let Some(total) = usage.total_tokens {
                    if total >= max_tokens {
                        warn!("Token limit reached: {}/{}", total, max_tokens);
                    }
                }
            }
        }

        if description.is_empty() {
            return Err(OpenRouterError::InvalidResponse(
                "Empty description returned".to_string(),
            ));
        }

        debug!("Generated description: {}", description);
        Ok(description)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn create_test_config() -> OpenRouterConfig {
        OpenRouterConfig {
            api_key: "test_key".to_string(),
            model: "anthropic/claude-3-haiku".to_string(),
            base_url: Some("https://test.openrouter.ai/api/v1".to_string()),
            max_tokens: Some(150),
        }
    }

    #[test]
    fn test_openrouter_client_creation() {
        let config = create_test_config();
        let client = OpenRouterClient::new(config.clone());

        assert_eq!(client.config.api_key, "test_key");
        assert_eq!(client.config.model, "anthropic/claude-3-haiku");
        assert_eq!(client.base_url(), "https://test.openrouter.ai/api/v1");
    }

    #[test]
    fn test_base_url_default() {
        let mut config = create_test_config();
        config.base_url = None;
        let client = OpenRouterClient::new(config);

        assert_eq!(client.base_url(), "https://openrouter.ai/api/v1");
    }

    #[tokio::test]
    async fn test_rate_limiter() {
        let mut rate_limiter = RateLimiter::new(2, 100);

        let start = Instant::now();
        let _permit1 = rate_limiter.acquire().await;
        drop(_permit1); // Explicitly drop the first permit
        let _permit2 = rate_limiter.acquire().await;
        let elapsed = start.elapsed();

        // Second request should be delayed by at least 100ms
        assert!(elapsed >= Duration::from_millis(100));
    }

    #[test]
    fn test_image_description_request_serialization() {
        let request = ImageDescriptionRequest {
            model: "test-model".to_string(),
            messages: vec![Message {
                role: "user".to_string(),
                content: vec![
                    ContentPart::Text {
                        text: "Describe this image".to_string(),
                    },
                    ContentPart::ImageUrl {
                        image_url: ImageUrl {
                            url: "data:image/jpeg;base64,test".to_string(),
                        },
                    },
                ],
            }],
            max_tokens: Some(150),
        };

        let json = serde_json::to_value(&request).unwrap();
        assert_eq!(json["model"], "test-model");
        assert_eq!(json["max_tokens"], 150);
        assert_eq!(json["messages"][0]["role"], "user");
        assert_eq!(json["messages"][0]["content"][0]["type"], "text");
        assert_eq!(json["messages"][0]["content"][1]["type"], "image_url");
    }

    #[test]
    fn test_image_description_response_deserialization() {
        let json_response = json!({
            "choices": [{
                "message": {
                    "content": "A beautiful sunset over the ocean"
                },
                "finish_reason": "stop"
            }],
            "usage": {
                "prompt_tokens": 50,
                "completion_tokens": 10,
                "total_tokens": 60
            }
        });

        let response: ImageDescriptionResponse = serde_json::from_value(json_response).unwrap();
        assert_eq!(response.choices.len(), 1);
        assert_eq!(
            response.choices[0].message.content,
            "A beautiful sunset over the ocean"
        );
        assert_eq!(response.usage.as_ref().unwrap().total_tokens, Some(60));
    }

    #[test]
    fn test_models_response_deserialization() {
        let json_response = json!({
            "data": [{
                "id": "anthropic/claude-3-haiku",
                "name": "Claude 3 Haiku",
                "description": "Fast and efficient model",
                "pricing": {
                    "prompt": "0.00025",
                    "completion": "0.00125"
                },
                "context_length": 200000
            }]
        });

        let response: ModelsResponse = serde_json::from_value(json_response).unwrap();
        assert_eq!(response.data.len(), 1);
        assert_eq!(response.data[0].id, "anthropic/claude-3-haiku");
        assert_eq!(response.data[0].name, "Claude 3 Haiku");
        assert_eq!(response.data[0].context_length, Some(200000));
    }

    #[test]
    fn test_account_balance_deserialization() {
        let json_response = json!({
            "data": {
                "balance": 25.50
            }
        });

        let response: AccountBalance = serde_json::from_value(json_response).unwrap();
        assert_eq!(response.data.balance, 25.50);
    }

    #[test]
    fn test_error_response_deserialization() {
        let json_response = json!({
            "error": {
                "message": "Invalid API key",
                "code": "invalid_api_key",
                "type": "authentication_error"
            }
        });

        let response: ErrorResponse = serde_json::from_value(json_response).unwrap();
        assert_eq!(response.error.message, "Invalid API key");
        assert_eq!(response.error.code, Some("invalid_api_key".to_string()));
        assert_eq!(
            response.error.error_type,
            Some("authentication_error".to_string())
        );
    }

    #[test]
    fn test_base64_encoding() {
        let input = b"Hello, World!";
        let encoded = BASE64_STANDARD.encode(input);
        assert_eq!(encoded, "SGVsbG8sIFdvcmxkIQ==");

        let input = b"A";
        let encoded = BASE64_STANDARD.encode(input);
        assert_eq!(encoded, "QQ==");

        let input = b"AB";
        let encoded = BASE64_STANDARD.encode(input);
        assert_eq!(encoded, "QUI=");
    }

    // Mock tests would require a more complex setup with wiremock or similar
    // For now, we'll focus on unit tests for the data structures and basic functionality

    #[tokio::test]
    async fn test_image_size_validation() {
        let config = create_test_config();
        let client = OpenRouterClient::new(config);

        // Create a large image (> 10MB)
        let large_image = vec![0u8; 11 * 1024 * 1024]; // 11MB

        let result = client
            .describe_image(&large_image, "Describe this image")
            .await;

        match result {
            Err(OpenRouterError::ImageTooLarge { size_mb, max_mb }) => {
                assert!(size_mb > 10.0);
                assert_eq!(max_mb, 10.0);
            }
            _ => panic!("Expected ImageTooLarge error"),
        }
    }

    #[test]
    fn test_openrouter_error_display() {
        let token_error = OpenRouterError::TokenLimitExceeded {
            tokens_used: 200,
            max_tokens: 150,
        };
        assert!(token_error.to_string().contains("Token limit exceeded"));
        assert!(token_error.to_string().contains("200/150"));

        let balance_error = OpenRouterError::InsufficientBalance {
            balance: 2.50,
            minimum: 5.0,
        };
        assert!(balance_error.to_string().contains("Insufficient balance"));
        assert!(balance_error.to_string().contains("$2.5"));

        let rate_limit_error = OpenRouterError::RateLimitExceeded { retry_after: 60 };
        assert!(rate_limit_error.to_string().contains("Rate limit exceeded"));
        assert!(rate_limit_error.to_string().contains("60 seconds"));

        let model_error = OpenRouterError::ModelNotAvailable {
            model: "test-model".to_string(),
        };
        assert!(model_error.to_string().contains("Model not available"));
        assert!(model_error.to_string().contains("test-model"));

        let image_error = OpenRouterError::ImageTooLarge {
            size_mb: 15.0,
            max_mb: 10.0,
        };
        assert!(image_error.to_string().contains("Image too large"));
        assert!(image_error.to_string().contains("15MB"));

        let format_error = OpenRouterError::UnsupportedImageFormat {
            format: "image/bmp".to_string(),
        };
        assert!(format_error
            .to_string()
            .contains("Unsupported image format"));
        assert!(format_error.to_string().contains("image/bmp"));
    }
}
