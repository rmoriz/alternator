use crate::config::OpenRouterConfig;
use async_trait::async_trait;
use base64::prelude::*;
use reqwest::{Client, Response};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Semaphore;
use tokio::time::sleep;
use tracing::{debug, error, info, warn};

// Re-export OpenRouterError so tests can access it
pub use crate::error::OpenRouterError;

/// Trait for OpenRouter API operations to enable mocking in tests
#[async_trait]
pub trait OpenRouterApi {
    #[allow(dead_code)]
    async fn get_account_balance(&self) -> Result<f64, OpenRouterError>;
    #[allow(dead_code)]
    async fn list_models(&self) -> Result<Vec<Model>, OpenRouterError>;
    #[allow(dead_code)]
    async fn describe_image(
        &self,
        image_data: &[u8],
        prompt: &str,
    ) -> Result<String, OpenRouterError>;
    #[allow(dead_code)]
    async fn process_text(&self, prompt: &str) -> Result<String, OpenRouterError>;
}

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
pub struct OpenRouterClient {
    config: OpenRouterConfig,
    http_client: Client,
    rate_limiter: Arc<tokio::sync::Mutex<RateLimiter>>,
}

impl std::fmt::Debug for OpenRouterClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpenRouterClient")
            .field("model", &self.config.model)
            .field("base_url", &self.config.base_url)
            .field("max_tokens", &self.config.max_tokens)
            .field("api_key", &"[REDACTED]")
            .finish()
    }
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
    pub usage: f64,
}

/// Image description request
#[derive(Debug, Serialize)]
pub struct ImageDescriptionRequest {
    pub model: String,
    pub messages: Vec<Message>,
    pub max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<ReasoningConfig>,
}

/// Reasoning configuration for controlling reasoning tokens
#[derive(Debug, Serialize, Deserialize)]
pub struct ReasoningConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exclude: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Message {
    pub role: String,
    pub content: Vec<ContentPart>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ContentPart {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "image_url")]
    ImageUrl { image_url: ImageUrl },
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ImageUrl {
    pub url: String,
}

/// Image description response
#[derive(Debug, Deserialize)]
pub struct ImageDescriptionResponse {
    pub choices: Vec<Choice>,
    pub usage: Option<Usage>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Choice {
    pub message: ResponseMessage,
    #[allow(dead_code)] // May be used for response validation in future
    pub finish_reason: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ResponseMessage {
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
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

#[derive(Debug, Serialize, Deserialize)]
pub struct ErrorDetail {
    pub message: String,
    #[allow(dead_code)] // May be used for error categorization in future
    pub code: Option<String>,
    #[serde(rename = "type")]
    #[allow(dead_code)] // May be used for error categorization in future
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

    /// Sanitize text for safe Mastodon API usage
    fn sanitize_description(text: &str) -> String {
        // Remove any null bytes and non-printable control characters (except newlines/tabs)
        let cleaned: String = text
            .chars()
            .filter(|&c| c != '\0' && (c == '\n' || c == '\t' || (!c.is_control())))
            .collect();

        // Normalize Unicode using NFC (Canonical Composition) to ensure consistent encoding
        use unicode_normalization::UnicodeNormalization;
        let normalized: String = cleaned.nfc().collect();

        // Trim whitespace and return
        normalized.trim().to_string()
    }

    /// Safely truncate text at character boundaries, preferring word boundaries
    fn safe_truncate(text: &str, max_chars: usize) -> String {
        if text.chars().count() <= max_chars {
            return text.to_string();
        }

        // Take only the allowed number of characters (Unicode-safe)
        let char_vec: Vec<char> = text.chars().take(max_chars).collect();
        let truncated: String = char_vec.iter().collect();

        // Try to find the last space to avoid cutting words
        if let Some(last_space_byte_pos) = truncated.rfind(' ') {
            // Count characters to the last space position (Unicode-safe)
            let last_space_char_pos = truncated[..last_space_byte_pos].chars().count();

            // Only use space if it's not too early (at least 75% of the limit)
            if last_space_char_pos > max_chars * 3 / 4 {
                return format!("{}…", &truncated[..last_space_byte_pos]);
            }
        }

        format!("{truncated}…")
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
            OpenRouterError::ApiRequestFailed(format!("Failed to read response: {e}"))
        })?;

        // Log the complete OpenRouter response verbatim for debugging
        debug!(
            "OpenRouter API response (status: {}): {}",
            status, response_text
        );

        if !status.is_success() {
            // Try to parse error response
            if let Ok(error_response) = serde_json::from_str::<ErrorResponse>(&response_text) {
                let error_msg = error_response.error.message;

                // Check for token limit exceeded
                if error_msg.to_lowercase().contains("token")
                    && error_msg.to_lowercase().contains("limit")
                {
                    // Extract token information if available
                    let max_tokens = self.config.max_tokens.unwrap_or(1500);
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

                // Check for provider failures (common with OpenRouter)
                if error_msg.to_lowercase().contains("provider")
                    && (error_msg.to_lowercase().contains("error")
                        || error_msg.to_lowercase().contains("failed")
                        || error_msg.to_lowercase().contains("exhausted")
                        || error_msg.to_lowercase().contains("unavailable"))
                {
                    // Extract provider name if available
                    let provider = if let Some(start) = error_msg.find("Provider: ") {
                        let provider_part = &error_msg[start + 10..];
                        provider_part
                            .split(|c: char| c == ')' || c == ',' || c.is_whitespace())
                            .next()
                            .unwrap_or("Unknown")
                            .to_string()
                    } else {
                        "Unknown".to_string()
                    };

                    warn!(
                        "OpenRouter provider failure detected: {} (Provider: {})",
                        error_msg, provider
                    );
                    return Err(OpenRouterError::ProviderFailure {
                        provider,
                        message: error_msg,
                    });
                }

                return Err(OpenRouterError::ApiRequestFailed(error_msg));
            }

            return Err(OpenRouterError::ApiRequestFailed(format!(
                "HTTP {status} - {response_text}"
            )));
        }

        serde_json::from_str(&response_text).map_err(|e| {
            error!("Failed to parse OpenRouter response: {}", e);
            // Only log raw response text if it's reasonably short and safe
            if response_text.len() <= 1000 && response_text.chars().all(|c| c.is_ascii() || c.is_whitespace()) {
                debug!("Raw OpenRouter response text: {}", response_text);
            } else {
                debug!("Raw OpenRouter response text too large or contains non-ASCII characters (length: {})", response_text.len());
            }
            OpenRouterError::InvalidResponse(format!("JSON parsing failed: {e}"))
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

            // Log request details for debugging
            info!("=== HTTP Request Debug (attempt {}) ===", attempt + 1);
            info!(
                "Authorization: Bearer {}...{}",
                if self.config.api_key.len() > 8 {
                    &self.config.api_key[..4]
                } else {
                    "****"
                },
                if self.config.api_key.len() > 8 {
                    &self.config.api_key[self.config.api_key.len() - 4..]
                } else {
                    "****"
                }
            );
            info!("Content-Type: application/json");
            info!("HTTP-Referer: https://github.com/rmoriz/alternator");
            info!("X-Title: Alternator - Mastodon Media Describer");
            info!("=== End HTTP Request Debug ===");

            let response = request_fn()
                .header("Authorization", format!("Bearer {}", self.config.api_key))
                .header("Content-Type", "application/json")
                .header("HTTP-Referer", "https://github.com/rmoriz/alternator")
                .header("X-Title", "Alternator - Mastodon Media Describer")
                .send()
                .await
                .map_err(|e| OpenRouterError::ApiRequestFailed(format!("Request failed: {e}")))?;

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
                        tokens_used: self.config.max_tokens.unwrap_or(1500),
                        max_tokens: self.config.max_tokens.unwrap_or(1500),
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
                        .get(format!("{}/auth/key", self.base_url()))
                },
                3,
            )
            .await?;

        let balance = response.data.usage;
        info!("OpenRouter account balance: ${:.2}", balance);

        Ok(balance)
    }

    /// List available models for startup validation
    pub async fn list_models(&self) -> Result<Vec<Model>, OpenRouterError> {
        info!("Fetching OpenRouter model list");

        let response: ModelsResponse = self
            .api_request_with_retry(
                || self.http_client.get(format!("{}/models", self.base_url())),
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
        // Validate input parameters
        if image_data.is_empty() {
            return Err(OpenRouterError::InvalidResponse(
                "Empty image data provided".to_string(),
            ));
        }

        if prompt.trim().is_empty() {
            return Err(OpenRouterError::InvalidResponse(
                "Empty prompt provided".to_string(),
            ));
        }

        // Replace {model} placeholder in prompt with actual model name
        let processed_prompt = prompt.replace("{model}", &self.config.model);

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
        let data_url = format!("data:image/jpeg;base64,{base64_image}");

        let request = ImageDescriptionRequest {
            model: self.config.model.clone(),
            messages: vec![Message {
                role: "user".to_string(),
                content: vec![
                    ContentPart::Text {
                        text: processed_prompt,
                    },
                    ContentPart::ImageUrl {
                        image_url: ImageUrl { url: data_url },
                    },
                ],
            }],
            max_tokens: self.config.max_tokens,
            reasoning: Some(ReasoningConfig {
                exclude: Some(true), // Exclude reasoning tokens to save costs and get cleaner responses
                enabled: None,
                effort: None,
                max_tokens: None,
            }),
        };

        let response: ImageDescriptionResponse = self
            .api_request_with_retry(
                || {
                    self.http_client
                        .post(format!("{}/chat/completions", self.base_url()))
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

        // Validate that we have at least one choice with content
        let choice = &response.choices[0];
        if choice.message.content.trim().is_empty() {
            return Err(OpenRouterError::InvalidResponse(
                "Empty content in response choice".to_string(),
            ));
        }

        // Extract the main content (not reasoning tokens) from the response
        let raw_description = choice.message.content.trim();

        // Sanitize the description to remove any problematic characters
        let description = Self::sanitize_description(raw_description);

        debug!(
            "OpenRouter response - raw length: {}, sanitized length: {}, content preview: '{}'",
            raw_description.len(),
            description.len(),
            // Use safe_truncate for Unicode-safe preview
            if description.chars().count() > 100 {
                Self::safe_truncate(&description, 100)
            } else {
                description.to_string()
            }
        );

        // Log if reasoning tokens were present but excluded
        if let Some(reasoning) = &choice.message.reasoning {
            debug!(
                "Reasoning tokens were present but excluded: {} chars",
                reasoning.len()
            );
        }

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

        // Ensure description respects character limit (1500 chars total including AI attribution)
        const MAX_DESCRIPTION_LENGTH: usize = 1500;
        let final_description = if description.chars().count() > MAX_DESCRIPTION_LENGTH {
            warn!(
                "Description too long ({} chars), truncating to {} chars",
                description.chars().count(),
                MAX_DESCRIPTION_LENGTH
            );

            Self::safe_truncate(&description, MAX_DESCRIPTION_LENGTH)
        } else {
            description
        };

        debug!("Generated description: {}", final_description);
        Ok(final_description)
    }

    /// Process text using OpenRouter API (for transcript summarization)
    pub async fn process_text(&self, prompt: &str) -> Result<String, OpenRouterError> {
        // Validate input parameters
        if prompt.trim().is_empty() {
            return Err(OpenRouterError::InvalidResponse(
                "Empty prompt provided".to_string(),
            ));
        }

        debug!("Processing text using model: {}", self.config.text_model);

        // Build the request for text processing
        let request = serde_json::json!({
            "model": self.config.text_model,
            "messages": [
                {
                    "role": "user",
                    "content": prompt
                }
            ],
            "max_tokens": self.config.max_tokens,
            "reasoning": {
                "exclude": true
            }
        });

        // Log the complete request for debugging
        info!("=== OpenRouter Request Debug ===");
        info!("URL: {}/chat/completions", self.base_url());
        info!("Headers:");
        info!(
            "  Authorization: Bearer {}",
            if self.config.api_key.len() > 10 {
                format!(
                    "{}...{}",
                    &self.config.api_key[..4],
                    &self.config.api_key[self.config.api_key.len() - 4..]
                )
            } else {
                "[REDACTED]".to_string()
            }
        );
        info!("  Content-Type: application/json");
        info!("  HTTP-Referer: https://github.com/rmoriz/alternator");
        info!("  X-Title: Alternator - Mastodon Media Describer");
        info!("Request Body:");
        info!(
            "{}",
            serde_json::to_string_pretty(&request)
                .unwrap_or_else(|_| "Failed to serialize request".to_string())
        );
        info!("=== End OpenRouter Request Debug ===");

        let response: ImageDescriptionResponse = self
            .api_request_with_retry(
                || {
                    self.http_client
                        .post(format!("{}/chat/completions", self.base_url()))
                        .json(&request)
                },
                2, // Only retry twice for text processing to avoid excessive costs
            )
            .await?;

        if response.choices.is_empty() {
            return Err(OpenRouterError::InvalidResponse(
                "No choices in response".to_string(),
            ));
        }

        // Validate that we have at least one choice with content
        let choice = &response.choices[0];
        if choice.message.content.trim().is_empty() {
            return Err(OpenRouterError::InvalidResponse(
                "Empty content in response choice".to_string(),
            ));
        }

        // Extract the content from the response
        let raw_text = choice.message.content.trim();

        // Sanitize the text to remove any problematic characters
        let processed_text = Self::sanitize_description(raw_text);

        debug!(
            "OpenRouter text processing - raw length: {}, sanitized length: {}, content preview: '{}'",
            raw_text.len(),
            processed_text.len(),
            // Use safe_truncate for Unicode-safe preview
            if processed_text.chars().count() > 100 {
                Self::safe_truncate(&processed_text, 100)
            } else {
                processed_text.to_string()
            }
        );

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

        if processed_text.is_empty() {
            return Err(OpenRouterError::InvalidResponse(
                "Empty processed text returned".to_string(),
            ));
        }

        debug!("Processed text: {}", processed_text);
        Ok(processed_text)
    }
}

#[async_trait]
impl OpenRouterApi for OpenRouterClient {
    async fn get_account_balance(&self) -> Result<f64, OpenRouterError> {
        self.get_account_balance().await
    }

    async fn list_models(&self) -> Result<Vec<Model>, OpenRouterError> {
        self.list_models().await
    }

    async fn describe_image(
        &self,
        image_data: &[u8],
        prompt: &str,
    ) -> Result<String, OpenRouterError> {
        self.describe_image(image_data, prompt).await
    }

    async fn process_text(&self, prompt: &str) -> Result<String, OpenRouterError> {
        self.process_text(prompt).await
    }
}

/// Mock OpenRouter client for testing
#[derive(Debug)]
pub struct MockOpenRouterClient {
    pub balance: f64,
    #[allow(dead_code)]
    // Used in test configurations but may not be detected by clippy in --all-targets mode
    pub models: Vec<Model>,
    pub description_response: String,
    pub text_response: String,
    #[allow(dead_code)]
    // Used in test implementations but may not be detected by clippy in --all-targets mode
    pub should_fail: bool,
    #[allow(dead_code)]
    // Used in test implementations but may not be detected by clippy in --all-targets mode
    pub error_type: Option<OpenRouterError>,
}

impl Default for MockOpenRouterClient {
    fn default() -> Self {
        Self::new()
    }
}

impl MockOpenRouterClient {
    pub fn new() -> Self {
        Self {
            balance: 25.50,
            models: vec![
                Model {
                    id: "anthropic/claude-3-haiku".to_string(),
                    name: "Claude 3 Haiku".to_string(),
                    description: Some("Fast and efficient model".to_string()),
                    pricing: Some(ModelPricing {
                        prompt: "0.00025".to_string(),
                        completion: "0.00125".to_string(),
                    }),
                    context_length: Some(200000),
                },
                Model {
                    id: "mistralai/mistral-small-3.2-24b-instruct:free".to_string(),
                    name: "Mistral Small".to_string(),
                    description: Some("Free model for testing".to_string()),
                    pricing: Some(ModelPricing {
                        prompt: "0.0".to_string(),
                        completion: "0.0".to_string(),
                    }),
                    context_length: Some(32768),
                },
            ],
            description_response: "A beautiful sunset over the ocean with warm orange and pink colors reflecting on the water.".to_string(),
            text_response: "This is a summarized version of the provided text.".to_string(),
            should_fail: false,
            error_type: None,
        }
    }

    #[allow(dead_code)]
    pub fn with_error(error: OpenRouterError) -> Self {
        Self {
            balance: 25.50,
            models: vec![],
            description_response: String::new(),
            text_response: String::new(),
            should_fail: true,
            error_type: Some(error),
        }
    }

    #[allow(dead_code)]
    pub fn with_balance(mut self, balance: f64) -> Self {
        self.balance = balance;
        self
    }

    #[allow(dead_code)]
    pub fn with_description(mut self, description: String) -> Self {
        self.description_response = description;
        self
    }

    #[allow(dead_code)]
    pub fn with_text_response(mut self, text_response: String) -> Self {
        self.text_response = text_response;
        self
    }
}

#[async_trait]
impl OpenRouterApi for MockOpenRouterClient {
    async fn get_account_balance(&self) -> Result<f64, OpenRouterError> {
        if self.should_fail {
            return Err(self.error_type.as_ref().unwrap().clone());
        }
        Ok(self.balance)
    }

    async fn list_models(&self) -> Result<Vec<Model>, OpenRouterError> {
        if self.should_fail {
            return Err(self.error_type.as_ref().unwrap().clone());
        }
        Ok(self.models.clone())
    }

    async fn describe_image(
        &self,
        _image_data: &[u8],
        _prompt: &str,
    ) -> Result<String, OpenRouterError> {
        if self.should_fail {
            return Err(self.error_type.as_ref().unwrap().clone());
        }
        Ok(self.description_response.clone())
    }

    async fn process_text(&self, _prompt: &str) -> Result<String, OpenRouterError> {
        if self.should_fail {
            return Err(self.error_type.as_ref().unwrap().clone());
        }
        Ok(self.text_response.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn create_test_config() -> OpenRouterConfig {
        OpenRouterConfig {
            api_key: "test_key".to_string(),
            model: "mistralai/mistral-small-3.2-24b-instruct:free".to_string(),
            vision_model: "mistralai/mistral-small-3.2-24b-instruct:free".to_string(),
            text_model: "mistralai/mistral-small-3.2-24b-instruct:free".to_string(),
            base_url: Some("https://test.openrouter.ai/api/v1".to_string()),
            max_tokens: Some(150),
        }
    }

    #[test]
    fn test_openrouter_client_creation() {
        let config = create_test_config();
        let client = OpenRouterClient::new(config.clone());

        assert_eq!(client.config.api_key, "test_key");
        assert_eq!(
            client.config.model,
            "mistralai/mistral-small-3.2-24b-instruct:free"
        );
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
            reasoning: Some(ReasoningConfig {
                exclude: Some(true),
                enabled: None,
                effort: None,
                max_tokens: None,
            }),
        };

        let json = serde_json::to_value(&request).unwrap();
        println!(
            "Serialized request: {}",
            serde_json::to_string_pretty(&json).unwrap()
        );

        assert_eq!(json["model"], "test-model");
        assert_eq!(json["max_tokens"], 150);
        assert_eq!(json["reasoning"]["exclude"], true);
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
                "id": "mistralai/mistral-small-3.2-24b-instruct:free",
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
        assert_eq!(
            response.data[0].id,
            "mistralai/mistral-small-3.2-24b-instruct:free"
        );
        assert_eq!(response.data[0].name, "Claude 3 Haiku");
        assert_eq!(response.data[0].context_length, Some(200000));
    }

    #[test]
    fn test_account_balance_deserialization() {
        let json_response = json!({
            "data": {
                "usage": 25.50
            }
        });

        let response: AccountBalance = serde_json::from_value(json_response).unwrap();
        assert_eq!(response.data.usage, 25.50);
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

    #[test]
    fn test_sanitize_description() {
        // Test normal text
        let input = "A beautiful sunset over the ocean";
        let result = OpenRouterClient::sanitize_description(input);
        assert_eq!(result, "A beautiful sunset over the ocean");

        // Test text with control characters
        let input = format!("Text{}with{}control{}chars", '\0', '\x01', '\x02');
        let result = OpenRouterClient::sanitize_description(&input);
        assert_eq!(result, "Textwithcontrolchars");

        // Test text with valid whitespace
        let input = "  Text\nwith\ttabs  ";
        let result = OpenRouterClient::sanitize_description(input);
        assert_eq!(result, "Text\nwith\ttabs");

        // Test empty string
        let input = "";
        let result = OpenRouterClient::sanitize_description(input);
        assert_eq!(result, "");

        // Test unicode text
        let input = "Schönes Bild mit Umlauten";
        let result = OpenRouterClient::sanitize_description(input);
        assert_eq!(result, "Schönes Bild mit Umlauten");
    }

    #[test]
    fn test_safe_truncate_basic() {
        // Test text shorter than limit
        let short_text = "Short text";
        assert_eq!(
            OpenRouterClient::safe_truncate(short_text, 20),
            "Short text"
        );

        // Test text exactly at limit
        let exact_text = "Exactly twenty chars";
        assert_eq!(
            OpenRouterClient::safe_truncate(exact_text, 20),
            "Exactly twenty chars"
        );

        // Test text longer than limit (no spaces)
        let long_text = "ThisIsAVeryLongTextWithoutSpaces";
        let result = OpenRouterClient::safe_truncate(long_text, 10);
        assert_eq!(result, "ThisIsAVer…");
        assert_eq!(result.chars().count(), 11); // 10 chars + ellipsis
    }

    #[test]
    fn test_safe_truncate_with_spaces() {
        // Test truncation at word boundary
        let text = "This is a long sentence that needs truncation";
        let result = OpenRouterClient::safe_truncate(text, 20);

        // Should break at word boundary and add ellipsis
        assert!(result.ends_with('…'));
        assert!(result.chars().count() <= 21); // 20 + ellipsis
        assert!(!result.contains("truncation")); // Should be cut before this word
    }

    #[test]
    fn test_safe_truncate_japanese() {
        // Japanese text from the error log
        let japanese_text = "木製のテーブルに半分ほどビールが注がれた透明なグラスと、中に角切りのチェダーチーズスナックが入ったガラスのボウルが置かれている。グラスとボウルは、トーンがかかった柴編みのコースターの上にあり、そのコースターはテーブルの上に置かれています。背景は落ち着いた灰色で、飲食品を目立たせています。";

        // Test various truncation lengths
        for max_chars in [50, 100, 150] {
            let result = OpenRouterClient::safe_truncate(japanese_text, max_chars);

            // Should not panic (this was the original issue)
            assert!(result.chars().count() <= max_chars + 1); // +1 for ellipsis

            // Should be valid UTF-8 (no broken characters)
            assert!(result.is_ascii() || result.chars().all(|c| c.is_alphanumeric() || c.is_whitespace() || "。、…のとにが入っグラスボウル置れている上あり灰色で目立せます透明なチダーズナックガーターブー".contains(c)));
        }

        // Test that 100 characters doesn't panic (original error point)
        let result = OpenRouterClient::safe_truncate(japanese_text, 100);
        assert!(!result.is_empty());
        assert!(!result.is_empty());
    }

    #[test]
    fn test_safe_truncate_mixed_unicode() {
        // Mix of ASCII, Japanese, and emoji
        let mixed_text = "Hello 世界! This is a test 🌍 with mixed characters 日本語";

        let result = OpenRouterClient::safe_truncate(mixed_text, 25);
        assert!(result.chars().count() <= 26); // 25 + ellipsis

        // Should handle all character types without panicking
        let result2 = OpenRouterClient::safe_truncate(mixed_text, 10);
        assert!(result2.chars().count() <= 11); // 10 + ellipsis
    }

    #[test]
    fn test_safe_truncate_edge_cases() {
        // Empty string
        assert_eq!(OpenRouterClient::safe_truncate("", 10), "");

        // Single character
        assert_eq!(OpenRouterClient::safe_truncate("A", 10), "A");

        // Only spaces
        let spaces = "     ";
        let result = OpenRouterClient::safe_truncate(spaces, 3);
        assert_eq!(result, "   …");

        // Limit of 0
        let result = OpenRouterClient::safe_truncate("test", 0);
        assert_eq!(result, "…");
    }

    #[test]
    fn test_debug_formatting_japanese() {
        // Test the debug formatting that was causing the panic
        let japanese_text = "木製のテーブルに半分ほどビールが注がれた透明なグラスと、中に角切りのチェダーチーズスナックが入ったガラスのボウルが置かれている。";

        // This simulates the debug formatting logic that was fixed
        let preview = if japanese_text.chars().count() > 100 {
            format!("{}...", japanese_text.chars().take(100).collect::<String>())
        } else {
            japanese_text.to_string()
        };

        // Should not panic and should be valid
        assert!(!preview.is_empty());
        assert!(preview.chars().count() <= 103); // 100 + "..."
    }

    #[test]
    fn test_rate_limiter_concurrent_permits() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let rate_limiter = Arc::new(tokio::sync::Mutex::new(RateLimiter::new(2, 50)));

            // Test that we can acquire multiple permits concurrently
            let rate_limiter1 = rate_limiter.clone();
            let rate_limiter2 = rate_limiter.clone();

            let task1 = tokio::spawn(async move {
                let mut guard = rate_limiter1.lock().await;
                let permit = guard.acquire().await;
                drop(permit);
            });

            let task2 = tokio::spawn(async move {
                let mut guard = rate_limiter2.lock().await;
                let permit = guard.acquire().await;
                drop(permit);
            });

            // Both tasks should complete without hanging
            tokio::try_join!(task1, task2).unwrap();
        });
    }

    #[test]
    fn test_openrouter_config_defaults() {
        let config = OpenRouterConfig {
            api_key: "test".to_string(),
            model: "test-model".to_string(),
            vision_model: "test-vision-model".to_string(),
            text_model: "test-text-model".to_string(),
            base_url: None,
            max_tokens: None,
        };

        let client = OpenRouterClient::new(config);
        assert_eq!(client.base_url(), "https://openrouter.ai/api/v1");
        assert!(client.config.max_tokens.is_none());
    }

    #[test]
    fn test_model_pricing_serialization() {
        let pricing = ModelPricing {
            prompt: "0.001".to_string(),
            completion: "0.003".to_string(),
        };

        let json = serde_json::to_string(&pricing).unwrap();
        assert!(json.contains("0.001"));
        assert!(json.contains("0.003"));

        let deserialized: ModelPricing = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.prompt, "0.001");
        assert_eq!(deserialized.completion, "0.003");
    }

    #[test]
    fn test_model_serialization() {
        let model = Model {
            id: "test-model".to_string(),
            name: "Test Model".to_string(),
            description: Some("A test model".to_string()),
            pricing: Some(ModelPricing {
                prompt: "0.001".to_string(),
                completion: "0.003".to_string(),
            }),
            context_length: Some(4096),
        };

        let json = serde_json::to_string(&model).unwrap();
        assert!(json.contains("test-model"));
        assert!(json.contains("Test Model"));
        assert!(json.contains("4096"));

        let deserialized: Model = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.id, "test-model");
        assert_eq!(deserialized.name, "Test Model");
        assert_eq!(deserialized.context_length, Some(4096));
    }

    #[test]
    fn test_reasoning_config_serialization() {
        let reasoning = ReasoningConfig {
            exclude: Some(true),
            enabled: Some(false),
            effort: Some("high".to_string()),
            max_tokens: Some(100),
        };

        let json = serde_json::to_string(&reasoning).unwrap();
        assert!(json.contains("true"));
        assert!(json.contains("false"));
        assert!(json.contains("high"));
        assert!(json.contains("100"));

        let deserialized: ReasoningConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.exclude, Some(true));
        assert_eq!(deserialized.enabled, Some(false));
        assert_eq!(deserialized.effort, Some("high".to_string()));
        assert_eq!(deserialized.max_tokens, Some(100));
    }

    #[test]
    fn test_content_part_text_serialization() {
        let content = ContentPart::Text {
            text: "Describe this image".to_string(),
        };

        let json = serde_json::to_value(&content).unwrap();
        assert_eq!(json["type"], "text");
        assert_eq!(json["text"], "Describe this image");

        let deserialized: ContentPart = serde_json::from_value(json).unwrap();
        match deserialized {
            ContentPart::Text { text } => assert_eq!(text, "Describe this image"),
            _ => panic!("Expected Text variant"),
        }
    }

    #[test]
    fn test_content_part_image_url_serialization() {
        let content = ContentPart::ImageUrl {
            image_url: ImageUrl {
                url: "data:image/jpeg;base64,test".to_string(),
            },
        };

        let json = serde_json::to_value(&content).unwrap();
        assert_eq!(json["type"], "image_url");
        assert_eq!(json["image_url"]["url"], "data:image/jpeg;base64,test");

        let deserialized: ContentPart = serde_json::from_value(json).unwrap();
        match deserialized {
            ContentPart::ImageUrl { image_url } => {
                assert_eq!(image_url.url, "data:image/jpeg;base64,test")
            }
            _ => panic!("Expected ImageUrl variant"),
        }
    }

    #[test]
    fn test_message_serialization() {
        let message = Message {
            role: "user".to_string(),
            content: vec![
                ContentPart::Text {
                    text: "Describe".to_string(),
                },
                ContentPart::ImageUrl {
                    image_url: ImageUrl {
                        url: "data:image/jpeg;base64,test".to_string(),
                    },
                },
            ],
        };

        let json = serde_json::to_value(&message).unwrap();
        assert_eq!(json["role"], "user");
        assert_eq!(json["content"][0]["type"], "text");
        assert_eq!(json["content"][1]["type"], "image_url");

        let deserialized: Message = serde_json::from_value(json).unwrap();
        assert_eq!(deserialized.role, "user");
        assert_eq!(deserialized.content.len(), 2);
    }

    #[test]
    fn test_token_usage_serialization() {
        let usage = Usage {
            prompt_tokens: Some(100),
            completion_tokens: Some(50),
            total_tokens: Some(150),
        };

        let json = serde_json::to_string(&usage).unwrap();
        assert!(json.contains("100"));
        assert!(json.contains("50"));
        assert!(json.contains("150"));

        let deserialized: Usage = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.prompt_tokens, Some(100));
        assert_eq!(deserialized.completion_tokens, Some(50));
        assert_eq!(deserialized.total_tokens, Some(150));
    }

    #[test]
    fn test_response_message_serialization() {
        let message = ResponseMessage {
            content: "A beautiful image".to_string(),
            reasoning: Some("I can see a beautiful sunset".to_string()),
        };

        let json = serde_json::to_string(&message).unwrap();
        assert!(json.contains("A beautiful image"));
        assert!(json.contains("I can see a beautiful sunset"));

        let deserialized: ResponseMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.content, "A beautiful image");
        assert_eq!(
            deserialized.reasoning,
            Some("I can see a beautiful sunset".to_string())
        );
    }

    #[test]
    fn test_choice_serialization() {
        let choice = Choice {
            message: ResponseMessage {
                content: "Description".to_string(),
                reasoning: None,
            },
            finish_reason: Some("stop".to_string()),
        };

        let json = serde_json::to_string(&choice).unwrap();
        assert!(json.contains("Description"));
        assert!(json.contains("stop"));

        let deserialized: Choice = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.message.content, "Description");
        assert_eq!(deserialized.finish_reason, Some("stop".to_string()));
    }

    #[test]
    fn test_error_detail_serialization() {
        let error = ErrorDetail {
            message: "Invalid request".to_string(),
            code: Some("invalid_request".to_string()),
            error_type: Some("validation_error".to_string()),
        };

        let json = serde_json::to_string(&error).unwrap();
        assert!(json.contains("Invalid request"));
        assert!(json.contains("invalid_request"));
        assert!(json.contains("validation_error"));

        let deserialized: ErrorDetail = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.message, "Invalid request");
        assert_eq!(deserialized.code, Some("invalid_request".to_string()));
        assert_eq!(
            deserialized.error_type,
            Some("validation_error".to_string())
        );
    }

    #[test]
    fn test_openrouter_client_debug() {
        let config = create_test_config();
        let client = OpenRouterClient::new(config);

        // Test that the client can be debug formatted
        let debug_str = format!("{client:?}");
        assert!(debug_str.contains("OpenRouterClient"));
        // Should not contain sensitive information like API key in plain text
        // The debug implementation should hide the API key
        assert!(!debug_str.contains("test_key"));
    }

    #[test]
    fn test_rate_limiter_debug() {
        let rate_limiter = RateLimiter::new(5, 1000);
        let debug_str = format!("{rate_limiter:?}");
        assert!(debug_str.contains("RateLimiter"));
    }

    #[test]
    fn test_model_without_optional_fields() {
        let model = Model {
            id: "simple-model".to_string(),
            name: "Simple Model".to_string(),
            description: None,
            pricing: None,
            context_length: None,
        };

        let json = serde_json::to_string(&model).unwrap();
        let deserialized: Model = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.id, "simple-model");
        assert_eq!(deserialized.name, "Simple Model");
        assert!(deserialized.description.is_none());
        assert!(deserialized.pricing.is_none());
        assert!(deserialized.context_length.is_none());
    }

    #[test]
    fn test_reasoning_config_skip_serialization() {
        // Test that None values are skipped in serialization
        let reasoning = ReasoningConfig {
            exclude: Some(true),
            enabled: None,
            effort: None,
            max_tokens: None,
        };

        let json = serde_json::to_value(&reasoning).unwrap();
        assert_eq!(json["exclude"], true);
        assert!(!json.as_object().unwrap().contains_key("enabled"));
        assert!(!json.as_object().unwrap().contains_key("effort"));
        assert!(!json.as_object().unwrap().contains_key("max_tokens"));
    }

    #[test]
    fn test_response_message_without_reasoning() {
        let message = ResponseMessage {
            content: "Simple description".to_string(),
            reasoning: None,
        };

        let json = serde_json::to_string(&message).unwrap();
        let deserialized: ResponseMessage = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.content, "Simple description");
        assert!(deserialized.reasoning.is_none());
    }

    #[test]
    fn test_choice_without_finish_reason() {
        let choice = Choice {
            message: ResponseMessage {
                content: "Description".to_string(),
                reasoning: None,
            },
            finish_reason: None,
        };

        let json = serde_json::to_string(&choice).unwrap();
        let deserialized: Choice = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.message.content, "Description");
        assert!(deserialized.finish_reason.is_none());
    }

    #[test]
    fn test_token_usage_optional_fields() {
        let usage = Usage {
            prompt_tokens: Some(100),
            completion_tokens: None,
            total_tokens: None,
        };

        let json = serde_json::to_string(&usage).unwrap();
        let deserialized: Usage = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.prompt_tokens, Some(100));
        assert!(deserialized.completion_tokens.is_none());
        assert!(deserialized.total_tokens.is_none());
    }

    #[test]
    fn test_error_detail_optional_fields() {
        let error = ErrorDetail {
            message: "Simple error".to_string(),
            code: None,
            error_type: None,
        };

        let json = serde_json::to_string(&error).unwrap();
        let deserialized: ErrorDetail = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.message, "Simple error");
        assert!(deserialized.code.is_none());
        assert!(deserialized.error_type.is_none());
    }

    #[test]
    fn test_safe_truncate_boundary_conditions() {
        // Test truncation exactly at word boundary
        let text = "Hello world test";
        let result = OpenRouterClient::safe_truncate(text, 11); // "Hello world" is 11 chars
        assert_eq!(result, "Hello world…");

        // Test truncation just before word boundary
        let result = OpenRouterClient::safe_truncate(text, 10); // One char short
        assert_eq!(result, "Hello worl…");

        // Test truncation with multiple consecutive spaces
        let text_with_spaces = "Hello    world    test";
        let result = OpenRouterClient::safe_truncate(text_with_spaces, 10);
        assert!(result.chars().count() <= 11); // Should handle multiple spaces correctly
    }

    #[test]
    fn test_sanitize_description_comprehensive() {
        // Test various control characters
        let input = "Text\x00with\x01various\x02control\x03chars\x1F";
        let result = OpenRouterClient::sanitize_description(input);
        assert_eq!(result, "Textwithvariouscontrolchars");

        // Test with mixed valid and invalid characters
        let input = "Valid text\twith\ntabs and\nnewlinesbutalsocontrol";
        let result = OpenRouterClient::sanitize_description(input);
        assert_eq!(result, "Valid text\twith\ntabs and\nnewlinesbutalsocontrol");

        // Test empty and whitespace-only strings
        let input = "   \t\n  ";
        let result = OpenRouterClient::sanitize_description(input);
        assert_eq!(result, "");
    }
}
