use crate::config::MastodonConfig;
use crate::error::{AlternatorError, ErrorRecovery, MastodonError};
use chrono::{DateTime, Utc};
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tokio::net::TcpStream;
use tokio::time::sleep;
use tokio_tungstenite::{connect_async, tungstenite::Message, MaybeTlsStream, WebSocketStream};
use tracing::{debug, error, info, warn};
use url::Url;

/// Mastodon toot event from WebSocket stream
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TootEvent {
    pub id: String,
    pub account: Account,
    pub content: String,
    pub language: Option<String>,
    pub media_attachments: Vec<MediaAttachment>,
    pub created_at: DateTime<Utc>,
    pub url: Option<String>,
    pub visibility: String,
}

/// Mastodon account information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Account {
    pub id: String,
    pub username: String,
    pub acct: String,
    pub display_name: String,
    pub url: String,
}

/// Media attachment information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MediaAttachment {
    pub id: String,
    #[serde(rename = "type")]
    pub media_type: String,
    pub url: String,
    pub preview_url: Option<String>,
    pub description: Option<String>,
    pub meta: Option<MediaMeta>,
}

/// Media metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MediaMeta {
    pub original: Option<MediaDimensions>,
    pub small: Option<MediaDimensions>,
}

/// Media dimensions
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MediaDimensions {
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub size: Option<String>,
    pub aspect: Option<f64>,
}

/// WebSocket streaming event wrapper
#[derive(Debug, Serialize, Deserialize)]
pub struct StreamEvent {
    pub event: String,
    pub payload: Option<String>,
}

/// Mastodon WebSocket streaming client
pub struct MastodonClient {
    config: MastodonConfig,
    http_client: reqwest::Client,
    websocket: Option<WebSocketStream<MaybeTlsStream<TcpStream>>>,
    reconnect_attempts: u32,
    authenticated_user_id: Option<String>,
}

impl Clone for MastodonClient {
    fn clone(&self) -> Self {
        Self {
            config: self.config.clone(),
            http_client: self.http_client.clone(),
            websocket: None, // WebSocket connections can't be cloned
            reconnect_attempts: self.reconnect_attempts,
            authenticated_user_id: self.authenticated_user_id.clone(),
        }
    }
}

/// Trait for Mastodon streaming operations
#[allow(async_fn_in_trait)] // Internal trait for dependency injection in tests
pub trait MastodonStream {
    async fn connect(&mut self) -> Result<(), MastodonError>;
    async fn listen(&mut self) -> Result<Option<TootEvent>, MastodonError>;
    async fn get_toot(&self, toot_id: &str) -> Result<TootEvent, MastodonError>;
    #[allow(dead_code)] // Kept for backward compatibility in trait
    async fn update_media(
        &self,
        toot_id: &str,
        media_id: &str,
        description: &str,
    ) -> Result<(), MastodonError>;
    async fn update_multiple_media(
        &self,
        toot_id: &str,
        media_updates: Vec<(String, String)>,
    ) -> Result<(), MastodonError>;
    async fn create_media_attachment(
        &self,
        image_data: Vec<u8>,
        description: &str,
        filename: &str,
    ) -> Result<String, MastodonError>;
    async fn recreate_media_with_descriptions(
        &self,
        toot_id: &str,
        media_recreations: Vec<(Vec<u8>, String)>,
        original_media_ids: Vec<String>,
    ) -> Result<(), MastodonError>;
    async fn send_dm(&self, message: &str) -> Result<(), MastodonError>;
    async fn verify_credentials(&mut self) -> Result<Account, MastodonError>;
}

impl MastodonClient {
    /// Create a new Mastodon client
    pub fn new(config: MastodonConfig) -> Self {
        let http_client = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .user_agent(format!("Alternator/{}", env!("CARGO_PKG_VERSION")))
            .build()
            .expect("Failed to create HTTP client");

        Self {
            config,
            http_client,
            websocket: None,
            reconnect_attempts: 0,
            authenticated_user_id: None,
        }
    }

    /// Resolve the WebSocket streaming URL, following any redirects
    async fn resolve_streaming_url(&self) -> Result<Url, MastodonError> {
        let base_url = self.config.instance_url.trim_end_matches('/');
        let http_url = format!(
            "{}/api/v1/streaming?access_token={}&stream=user",
            base_url, self.config.access_token
        );

        // Make a HEAD request to resolve any redirects
        let response = self.http_client.head(&http_url).send().await.map_err(|e| {
            MastodonError::ConnectionFailed(format!("Failed to resolve streaming URL: {e}"))
        })?;

        let final_url = response.url().to_string();
        debug!("Resolved HTTP URL: {} -> {}", http_url, final_url);

        // Convert the final HTTP URL to WebSocket URL
        let streaming_url = final_url
            .replace("https://", "wss://")
            .replace("http://", "ws://");

        Url::parse(&streaming_url)
            .map_err(|e| MastodonError::ConnectionFailed(format!("Invalid streaming URL: {e}")))
    }

    /// Get the WebSocket streaming URL (for testing)
    #[cfg(test)]
    fn get_streaming_url(&self) -> Result<Url, MastodonError> {
        let base_url = self.config.instance_url.trim_end_matches('/');
        let streaming_url = format!(
            "{}/api/v1/streaming?access_token={}&stream=user",
            base_url
                .replace("https://", "wss://")
                .replace("http://", "ws://"),
            self.config.access_token
        );

        Url::parse(&streaming_url)
            .map_err(|e| MastodonError::ConnectionFailed(format!("Invalid streaming URL: {e}")))
    }

    /// Reconnect with exponential backoff
    async fn reconnect(&mut self) -> Result<(), MastodonError> {
        loop {
            if self.reconnect_attempts > 0 {
                let delay = ErrorRecovery::retry_delay(
                    &AlternatorError::Mastodon(MastodonError::ConnectionFailed(
                        "reconnect".to_string(),
                    )),
                    self.reconnect_attempts,
                );

                warn!(
                    "Reconnecting to Mastodon WebSocket in {} seconds (attempt {})",
                    delay,
                    self.reconnect_attempts + 1
                );
                sleep(Duration::from_secs(delay)).await;
            }

            match self.connect().await {
                Ok(()) => {
                    info!("Successfully reconnected to Mastodon WebSocket");
                    self.reconnect_attempts = 0;
                    return Ok(());
                }
                Err(e) => {
                    self.reconnect_attempts += 1;
                    let max_retries =
                        ErrorRecovery::max_retries(&AlternatorError::Mastodon(e.clone()));

                    if self.reconnect_attempts >= max_retries {
                        error!("Max reconnection attempts ({}) exceeded", max_retries);
                        return Err(e);
                    } else {
                        warn!(
                            "Reconnection attempt {} failed: {}",
                            self.reconnect_attempts, e
                        );
                        // Continue the loop for next attempt
                    }
                }
            }
        }
    }

    /// Parse streaming event from WebSocket message
    fn parse_streaming_event(&self, message: &str) -> Result<Option<TootEvent>, MastodonError> {
        debug!("Received WebSocket message: {}", message);

        let stream_event: StreamEvent = serde_json::from_str(message).map_err(|e| {
            MastodonError::InvalidTootData(format!("Failed to parse stream event: {e}"))
        })?;

        match stream_event.event.as_str() {
            "update" => {
                if let Some(payload) = stream_event.payload {
                    let toot: TootEvent = serde_json::from_str(&payload).map_err(|e| {
                        MastodonError::InvalidTootData(format!("Failed to parse toot: {e}"))
                    })?;

                    debug!(
                        "Parsed toot event: id={}, account={}, media_count={}",
                        toot.id,
                        toot.account.id,
                        toot.media_attachments.len()
                    );

                    Ok(Some(toot))
                } else {
                    warn!("Received update event without payload");
                    Ok(None)
                }
            }
            "delete" => {
                debug!("Received delete event, ignoring");
                Ok(None)
            }
            "notification" => {
                debug!("Received notification event, ignoring");
                Ok(None)
            }
            _ => {
                debug!("Received unknown event type: {}", stream_event.event);
                Ok(None)
            }
        }
    }

    /// Check if toot is from authenticated user
    fn is_own_toot(&self, toot: &TootEvent) -> Result<bool, MastodonError> {
        match &self.authenticated_user_id {
            Some(user_id) => Ok(toot.account.id == *user_id),
            None => Err(MastodonError::UserVerificationFailed),
        }
    }

    /// Spawn a background task for delayed cleanup of media attachments
    /// This won't block the current operation and handles timing issues with Mastodon
    pub fn spawn_cleanup_task(&self, media_ids: Vec<String>) {
        if media_ids.is_empty() {
            return;
        }

        let client = self.clone();

        tokio::spawn(async move {
            // Initial delay to let Mastodon process the status update
            tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;

            let mut retry_count = 0;
            const MAX_RETRIES: u32 = 3;
            const RETRY_DELAY_BASE: u64 = 2; // seconds

            while retry_count < MAX_RETRIES {
                let mut any_currently_used = false;

                for media_id in &media_ids {
                    match client.delete_media_attachment(media_id).await {
                        Ok(()) => {
                            debug!("Successfully cleaned up media attachment: {}", media_id);
                        }
                        Err(MastodonError::ApiRequestFailed(msg))
                            if msg.contains("422")
                                && msg.contains("currently used by a status") =>
                        {
                            debug!("Media attachment {} still in use, will retry", media_id);
                            any_currently_used = true;
                        }
                        Err(e) => {
                            error!("Failed to delete media attachment {}: {}", media_id, e);
                            // Don't retry for other types of errors
                        }
                    }
                }

                // If no media is currently in use, we're done
                if !any_currently_used {
                    break;
                }

                retry_count += 1;
                if retry_count < MAX_RETRIES {
                    let delay = RETRY_DELAY_BASE.pow(retry_count);
                    debug!(
                        "Retrying media cleanup in {} seconds (attempt {}/{})",
                        delay,
                        retry_count + 1,
                        MAX_RETRIES
                    );
                    tokio::time::sleep(tokio::time::Duration::from_secs(delay)).await;
                }
            }

            if retry_count >= MAX_RETRIES {
                warn!(
                    "Failed to clean up some media attachments after {} retries",
                    MAX_RETRIES
                );
            }
        });
    }

    /// Delete a single media attachment
    async fn delete_media_attachment(&self, media_id: &str) -> Result<(), MastodonError> {
        let url = format!(
            "{}/api/v1/media/{}",
            self.config.instance_url.trim_end_matches('/'),
            media_id
        );

        debug!("Deleting orphaned media attachment: {}", media_id);

        let response = self
            .http_client
            .delete(&url)
            .header(
                "Authorization",
                format!("Bearer {}", self.config.access_token),
            )
            .send()
            .await
            .map_err(|e| {
                MastodonError::ApiRequestFailed(format!("Failed to delete media {media_id}: {e}"))
            })?;

        if response.status() == 404 {
            // Media not found - could have already been deleted or never existed
            debug!(
                "Media {} not found (may have been already deleted)",
                media_id
            );
            return Ok(());
        }

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            warn!("Failed to delete media {media_id}: HTTP {status}: {error_text}");
            return Err(MastodonError::ApiRequestFailed(format!(
                "Media deletion failed with status {status}: {error_text}"
            )));
        }

        debug!("Successfully deleted media attachment: {}", media_id);
        Ok(())
    }

    /// Delete multiple media attachments (cleanup orphaned media)
    async fn delete_multiple_media_attachments(
        &self,
        media_ids: Vec<String>,
    ) -> Result<(), MastodonError> {
        if media_ids.is_empty() {
            return Ok(());
        }

        debug!("Deleting {} orphaned media attachments", media_ids.len());

        let mut deletion_errors = Vec::new();

        for media_id in &media_ids {
            if let Err(e) = self.delete_media_attachment(media_id).await {
                // Log error but continue with other deletions
                warn!("Failed to delete media {media_id}: {e}");
                deletion_errors.push(format!("{media_id}: {e}"));
            }
        }

        if !deletion_errors.is_empty() {
            // Some deletions failed, but we continue - this is cleanup, not critical
            warn!(
                "Failed to delete {} out of {} media attachments: {:?}",
                deletion_errors.len(),
                media_ids.len(),
                deletion_errors
            );
            // We could return an error here, but for cleanup it's better to be permissive
            // return Err(MastodonError::ApiRequestFailed(format!(
            //     "Failed to delete {} media attachments", deletion_errors.len()
            // )));
        }

        info!(
            "Successfully cleaned up {} orphaned media attachments",
            media_ids.len() - deletion_errors.len()
        );
        Ok(())
    }
}

impl MastodonStream for MastodonClient {
    /// Connect to Mastodon WebSocket streaming API
    async fn connect(&mut self) -> Result<(), MastodonError> {
        info!("Connecting to Mastodon WebSocket streaming API");

        // First verify credentials and get authenticated user ID
        if self.authenticated_user_id.is_none() {
            let account = self.verify_credentials().await?;
            self.authenticated_user_id = Some(account.id.clone());
            info!(
                "Authenticated as user: {} (@{})",
                account.display_name, account.acct
            );
        }

        let streaming_url = self.resolve_streaming_url().await?;
        debug!("Connecting to WebSocket URL: {}", streaming_url);

        let (ws_stream, response) = connect_async(streaming_url.as_str()).await.map_err(|e| {
            MastodonError::ConnectionFailed(format!("WebSocket connection failed: {e}"))
        })?;

        debug!(
            "WebSocket connection established, response status: {}",
            response.status()
        );

        self.websocket = Some(ws_stream);
        self.reconnect_attempts = 0;

        info!("Successfully connected to Mastodon WebSocket streaming API");
        Ok(())
    }

    /// Listen for toot events from WebSocket stream
    async fn listen(&mut self) -> Result<Option<TootEvent>, MastodonError> {
        loop {
            let websocket = match &mut self.websocket {
                Some(ws) => ws,
                None => {
                    warn!("WebSocket not connected, attempting to connect");
                    self.reconnect().await?;
                    continue;
                }
            };

            match websocket.next().await {
                Some(Ok(Message::Text(text))) => {
                    match self.parse_streaming_event(&text) {
                        Ok(Some(toot)) => {
                            // Check if this is the authenticated user's toot
                            if self.is_own_toot(&toot)? {
                                debug!("Received own toot: {}", toot.id);
                                return Ok(Some(toot));
                            } else {
                                debug!("Ignoring toot from other user: {}", toot.account.acct);
                                continue;
                            }
                        }
                        Ok(None) => {
                            // Event was parsed but not a toot update, continue listening
                            continue;
                        }
                        Err(e) => {
                            warn!("Failed to parse streaming event: {}", e);
                            continue;
                        }
                    }
                }
                Some(Ok(Message::Close(_))) => {
                    warn!("WebSocket connection closed by server");
                    self.websocket = None;
                    self.reconnect().await?;
                    continue;
                }
                Some(Ok(Message::Ping(data))) => {
                    debug!("Received WebSocket ping, sending pong");
                    if let Err(e) = websocket.send(Message::Pong(data)).await {
                        warn!("Failed to send pong: {}", e);
                        self.websocket = None;
                        self.reconnect().await?;
                        continue;
                    }
                }
                Some(Ok(Message::Pong(_))) => {
                    debug!("Received WebSocket pong");
                    continue;
                }
                Some(Ok(Message::Binary(_))) => {
                    debug!("Received binary WebSocket message, ignoring");
                    continue;
                }
                Some(Ok(Message::Frame(_))) => {
                    debug!("Received WebSocket frame, ignoring");
                    continue;
                }
                Some(Err(e)) => {
                    error!("WebSocket error: {}", e);
                    self.websocket = None;
                    return Err(MastodonError::Disconnected(format!("WebSocket error: {e}")));
                }
                None => {
                    warn!("WebSocket stream ended");
                    self.websocket = None;
                    self.reconnect().await?;
                    continue;
                }
            }
        }
    }

    /// Get current toot state for race condition checking
    async fn get_toot(&self, toot_id: &str) -> Result<TootEvent, MastodonError> {
        let url = format!(
            "{}/api/v1/statuses/{}",
            self.config.instance_url.trim_end_matches('/'),
            toot_id
        );

        debug!("Fetching toot state: {}", url);

        let response = self
            .http_client
            .get(&url)
            .header(
                "Authorization",
                format!("Bearer {}", self.config.access_token),
            )
            .send()
            .await
            .map_err(|e| MastodonError::ApiRequestFailed(format!("Failed to fetch toot: {e}")))?;

        if response.status() == 404 {
            return Err(MastodonError::TootNotFound {
                toot_id: toot_id.to_string(),
            });
        }

        if !response.status().is_success() {
            return Err(MastodonError::ApiRequestFailed(format!(
                "API request failed with status: {}",
                response.status()
            )));
        }

        let toot: TootEvent = response.json().await.map_err(|e| {
            MastodonError::InvalidTootData(format!("Failed to parse toot response: {e}"))
        })?;

        debug!(
            "Retrieved toot state: id={}, media_count={}",
            toot.id,
            toot.media_attachments.len()
        );
        Ok(toot)
    }

    /// Update media attachment description by editing the status
    async fn update_media(
        &self,
        toot_id: &str,
        media_id: &str,
        description: &str,
    ) -> Result<(), MastodonError> {
        // For backward compatibility, wrap single media update in batch update
        let media_updates = vec![(media_id.to_string(), description.to_string())];
        self.update_multiple_media(toot_id, media_updates).await
    }

    /// Update multiple media attachment descriptions by editing the status
    async fn update_multiple_media(
        &self,
        toot_id: &str,
        media_updates: Vec<(String, String)>, // Vec of (media_id, description)
    ) -> Result<(), MastodonError> {
        if media_updates.is_empty() {
            return Ok(());
        }

        debug!(
            "Updating {} media descriptions via status edit: toot_id={}",
            media_updates.len(),
            toot_id
        );

        // First, get the current status to preserve its content
        let current_status = self.get_toot(toot_id).await?;
        let status_content = &current_status.content;

        // Parse HTML content to get plain text
        let status_text = Self::extract_text_from_html(status_content);

        let url = format!(
            "{}/api/v1/statuses/{}",
            self.config.instance_url.trim_end_matches('/'),
            toot_id
        );

        // Prepare form data with the current status text and media attributes
        let mut form_data = std::collections::HashMap::new();
        form_data.insert("status".to_string(), status_text);

        for (index, (media_id, description)) in media_updates.iter().enumerate() {
            form_data.insert(format!("media_attributes[{index}][id]"), media_id.clone());
            form_data.insert(
                format!("media_attributes[{index}][description]"),
                description.clone(),
            );
            debug!(
                "  - media[{index}]: id={media_id}, description_length={}",
                description.len()
            );
        }

        let response = self
            .http_client
            .put(&url)
            .header(
                "Authorization",
                format!("Bearer {}", self.config.access_token),
            )
            .form(&form_data)
            .send()
            .await
            .map_err(|e| {
                MastodonError::ApiRequestFailed(format!("Failed to update status: {e}"))
            })?;

        if response.status() == 404 {
            return Err(MastodonError::MediaNotFound {
                media_id: format!(
                    "one of: {:?}",
                    media_updates.iter().map(|(id, _)| id).collect::<Vec<_>>()
                ),
            });
        }

        if response.status() == 429 {
            let retry_after = response
                .headers()
                .get("retry-after")
                .and_then(|h| h.to_str().ok())
                .and_then(|s| s.parse().ok())
                .unwrap_or(60);

            return Err(MastodonError::RateLimitExceeded { retry_after });
        }

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            error!(
                "Failed to update media descriptions via status edit: HTTP {status}: {error_text}"
            );
            return Err(MastodonError::ApiRequestFailed(format!(
                "Media update failed with status {status}: {error_text}"
            )));
        }

        info!(
            "Successfully updated {} media descriptions for toot: {toot_id}",
            media_updates.len()
        );
        Ok(())
    }

    /// Send direct message to authenticated user
    async fn send_dm(&self, message: &str) -> Result<(), MastodonError> {
        let user_id = self
            .authenticated_user_id
            .as_ref()
            .ok_or(MastodonError::UserVerificationFailed)?;

        let url = format!(
            "{}/api/v1/statuses",
            self.config.instance_url.trim_end_matches('/')
        );

        let mut params = std::collections::HashMap::new();
        params.insert("status", message);
        params.insert("visibility", "direct");
        params.insert("in_reply_to_id", user_id);

        let response = self
            .http_client
            .post(&url)
            .header(
                "Authorization",
                format!("Bearer {}", self.config.access_token),
            )
            .form(&params)
            .send()
            .await
            .map_err(|e| MastodonError::ApiRequestFailed(format!("Failed to send DM: {e}")))?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            return Err(MastodonError::ApiRequestFailed(format!(
                "DM failed with status {status}: {error_text}"
            )));
        }

        info!("DM sent successfully");
        Ok(())
    }

    /// Verify user credentials and store user ID for ownership checks
    async fn verify_credentials(&mut self) -> Result<Account, MastodonError> {
        let url = format!(
            "{}/api/v1/accounts/verify_credentials",
            self.config.instance_url.trim_end_matches('/')
        );

        let response = self
            .http_client
            .get(&url)
            .header(
                "Authorization",
                format!("Bearer {}", self.config.access_token),
            )
            .send()
            .await
            .map_err(|e| {
                MastodonError::ApiRequestFailed(format!("Failed to verify credentials: {e}"))
            })?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            return Err(MastodonError::ApiRequestFailed(format!(
                "Credential verification failed with status {status}: {error_text}"
            )));
        }

        let account: Account = response.json().await.map_err(|e| {
            MastodonError::ApiRequestFailed(format!("Failed to parse account response: {e}"))
        })?;

        // Store the authenticated user ID for future ownership checks
        self.authenticated_user_id = Some(account.id.clone());

        info!(
            "Credentials verified for user: {} (@{})",
            account.display_name, account.acct
        );
        Ok(account)
    }

    /// Create a new media attachment with description
    async fn create_media_attachment(
        &self,
        image_data: Vec<u8>,
        description: &str,
        filename: &str,
    ) -> Result<String, MastodonError> {
        let url = format!(
            "{}/api/v2/media",
            self.config.instance_url.trim_end_matches('/')
        );

        // Create multipart form with image data and description
        let form = reqwest::multipart::Form::new()
            .part(
                "file",
                reqwest::multipart::Part::bytes(image_data)
                    .file_name(filename.to_string())
                    .mime_str("image/jpeg") // Default to JPEG, could be improved to detect actual type
                    .map_err(|e| {
                        MastodonError::ApiRequestFailed(format!("Failed to set MIME type: {e}"))
                    })?,
            )
            .text("description", description.to_string());

        let response = self
            .http_client
            .post(&url)
            .header(
                "Authorization",
                format!("Bearer {}", self.config.access_token),
            )
            .multipart(form)
            .send()
            .await
            .map_err(|e| {
                MastodonError::ApiRequestFailed(format!("Failed to create media attachment: {e}"))
            })?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            return Err(MastodonError::ApiRequestFailed(format!(
                "Media creation failed with status {status}: {error_text}"
            )));
        }

        #[derive(Deserialize)]
        struct MediaResponse {
            id: String,
        }

        let media_response: MediaResponse = response.json().await.map_err(|e| {
            MastodonError::ApiRequestFailed(format!("Failed to parse media response: {e}"))
        })?;

        info!("Created new media attachment: id={}", media_response.id);
        Ok(media_response.id)
    }

    /// Recreate media attachments with descriptions and update the status
    async fn recreate_media_with_descriptions(
        &self,
        toot_id: &str,
        media_recreations: Vec<(Vec<u8>, String)>,
        original_media_ids: Vec<String>,
    ) -> Result<(), MastodonError> {
        if media_recreations.is_empty() {
            debug!("No media to recreate for toot: {}", toot_id);
            return Ok(());
        }

        debug!(
            "Recreating {} media attachments for toot: {}",
            media_recreations.len(),
            toot_id
        );

        // Step 1: Create new media attachments with descriptions
        let mut new_media_ids = Vec::new();
        for (index, (image_data, description)) in media_recreations.iter().enumerate() {
            let filename = format!("media_{index}.jpg");
            match self
                .create_media_attachment(image_data.clone(), description, &filename)
                .await
            {
                Ok(new_media_id) => {
                    debug!("Created new media attachment: {}", new_media_id);
                    new_media_ids.push(new_media_id);
                }
                Err(e) => {
                    error!(
                        "Failed to create media attachment {}: {}. Cleaning up created media.",
                        index, e
                    );
                    // Clean up any media we created before failing
                    if !new_media_ids.is_empty() {
                        if let Err(cleanup_error) =
                            self.delete_multiple_media_attachments(new_media_ids).await
                        {
                            warn!(
                                "Failed to clean up partial media during error: {}",
                                cleanup_error
                            );
                        }
                    }
                    return Err(e);
                }
            }
        }

        // Step 2: Update the status to use the new media attachments
        let url = format!(
            "{}/api/v1/statuses/{}",
            self.config.instance_url.trim_end_matches('/'),
            toot_id
        );

        // Get current status to preserve its content
        let current_status = self.get_toot(toot_id).await?;
        let status_text = Self::extract_text_from_html(&current_status.content);

        debug!("Original content HTML: {}", current_status.content);
        debug!("Extracted text: '{}'", status_text);

        let mut form_data = std::collections::HashMap::new();

        // Only include status text if the original had content
        // This avoids Mastodon's validation error for empty text
        if !status_text.trim().is_empty() {
            debug!("Including original status text in update");
            form_data.insert("status".to_string(), status_text);
        } else {
            debug!("Skipping status text (was empty) - updating only media attachments");
        }

        // Add new media IDs to the status update
        for (index, media_id) in new_media_ids.iter().enumerate() {
            form_data.insert(format!("media_ids[{index}]"), media_id.clone());
        }

        let response = self
            .http_client
            .put(&url)
            .header(
                "Authorization",
                format!("Bearer {}", self.config.access_token),
            )
            .form(&form_data)
            .send()
            .await
            .map_err(|e| {
                MastodonError::ApiRequestFailed(format!("Failed to update status: {e}"))
            })?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            error!("Failed to update status with new media: HTTP {status}: {error_text}");
            return Err(MastodonError::ApiRequestFailed(format!(
                "Status update failed with status {status}: {error_text}"
            )));
        }

        info!(
            "Successfully recreated {} media attachments for toot: {}",
            new_media_ids.len(),
            toot_id
        );

        // Schedule non-blocking cleanup of orphaned original media attachments
        if !original_media_ids.is_empty() {
            debug!(
                "Scheduling delayed cleanup of {} original media attachments",
                original_media_ids.len()
            );
            self.spawn_cleanup_task(original_media_ids);
        }

        Ok(())
    }
}

impl MastodonClient {
    /// Extract plain text from HTML content
    fn extract_text_from_html(html: &str) -> String {
        // Simple HTML tag removal - this is basic but should work for our needs
        let mut text = html.to_string();

        // Remove HTML tags using a simple regex approach
        while let Some(start) = text.find('<') {
            if let Some(end) = text[start..].find('>') {
                text.replace_range(start..start + end + 1, "");
            } else {
                break;
            }
        }

        // Decode common HTML entities
        text = text
            .replace("&amp;", "&")
            .replace("&lt;", "<")
            .replace("&gt;", ">")
            .replace("&quot;", "\"")
            .replace("&#39;", "'")
            .replace("&nbsp;", " ");

        text.trim().to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::{SinkExt, StreamExt};
    use std::sync::Arc;
    use tokio::net::{TcpListener, TcpStream};
    use tokio::sync::Mutex;
    use tokio_tungstenite::tungstenite::Message;
    use tokio_tungstenite::{accept_async, WebSocketStream};

    /// Mock WebSocket server for testing
    struct MockWebSocketServer {
        listener: TcpListener,
        messages_to_send: Arc<Mutex<Vec<String>>>,
    }

    impl MockWebSocketServer {
        async fn new() -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            Self {
                listener,
                messages_to_send: Arc::new(Mutex::new(Vec::new())),
            }
        }

        fn local_addr(&self) -> std::net::SocketAddr {
            self.listener.local_addr().unwrap()
        }

        async fn add_message(&self, message: String) {
            self.messages_to_send.lock().await.push(message);
        }

        async fn run(&self) {
            let (stream, _) = self.listener.accept().await.unwrap();
            let ws_stream = accept_async(stream).await.unwrap();
            self.handle_connection(ws_stream).await;
        }

        async fn handle_connection(&self, mut ws_stream: WebSocketStream<TcpStream>) {
            let messages = self.messages_to_send.clone();

            // Send all queued messages
            let messages_to_send = messages.lock().await.clone();
            for message in messages_to_send {
                if ws_stream.send(Message::Text(message)).await.is_err() {
                    break;
                }
            }

            // Keep connection alive and handle pings
            while let Some(msg) = ws_stream.next().await {
                match msg {
                    Ok(Message::Ping(data)) => {
                        let _ = ws_stream.send(Message::Pong(data)).await;
                    }
                    Ok(Message::Close(_)) => break,
                    Err(_) => break,
                    _ => {}
                }
            }
        }
    }

    fn create_test_config() -> MastodonConfig {
        MastodonConfig {
            instance_url: "https://mastodon.social".to_string(),
            access_token: "test_token".to_string(),
            user_stream: Some(true),
        }
    }

    fn create_test_toot_event() -> String {
        let toot = TootEvent {
            id: "123456789".to_string(),
            account: Account {
                id: "user123".to_string(),
                username: "testuser".to_string(),
                acct: "testuser@mastodon.social".to_string(),
                display_name: "Test User".to_string(),
                url: "https://mastodon.social/@testuser".to_string(),
            },
            content: "Test toot with image".to_string(),
            language: Some("en".to_string()),
            media_attachments: vec![MediaAttachment {
                id: "media123".to_string(),
                media_type: "image".to_string(),
                url: "https://example.com/image.jpg".to_string(),
                preview_url: Some("https://example.com/image_small.jpg".to_string()),
                description: None,
                meta: Some(MediaMeta {
                    original: Some(MediaDimensions {
                        width: Some(1920),
                        height: Some(1080),
                        size: Some("1920x1080".to_string()),
                        aspect: Some(1.777),
                    }),
                    small: Some(MediaDimensions {
                        width: Some(400),
                        height: Some(225),
                        size: Some("400x225".to_string()),
                        aspect: Some(1.777),
                    }),
                }),
            }],
            created_at: Utc::now(),
            url: Some("https://mastodon.social/@testuser/123456789".to_string()),
            visibility: "public".to_string(),
        };

        let stream_event = StreamEvent {
            event: "update".to_string(),
            payload: Some(serde_json::to_string(&toot).unwrap()),
        };

        serde_json::to_string(&stream_event).unwrap()
    }

    #[test]
    fn test_mastodon_client_creation() {
        let config = create_test_config();
        let client = MastodonClient::new(config.clone());

        assert_eq!(client.config.instance_url, config.instance_url);
        assert_eq!(client.config.access_token, config.access_token);
        assert_eq!(client.reconnect_attempts, 0);
        assert!(client.websocket.is_none());
        assert!(client.authenticated_user_id.is_none());
    }

    #[test]
    fn test_streaming_url_generation() {
        let config = create_test_config();
        let client = MastodonClient::new(config);

        let url = client.get_streaming_url().unwrap();
        assert_eq!(url.scheme(), "wss");
        assert!(url.as_str().contains("api/v1/streaming"));
        assert!(url.as_str().contains("stream=user"));
        assert!(url.as_str().contains("access_token=test_token"));
    }

    #[test]
    fn test_streaming_url_http_to_ws_conversion() {
        let mut config = create_test_config();
        config.instance_url = "http://localhost:3000".to_string();
        let client = MastodonClient::new(config);

        let url = client.get_streaming_url().unwrap();
        assert_eq!(url.scheme(), "ws");
    }

    #[test]
    fn test_parse_streaming_event_update() {
        let config = create_test_config();
        let client = MastodonClient::new(config);

        let message = create_test_toot_event();
        let result = client.parse_streaming_event(&message).unwrap();

        assert!(result.is_some());
        let toot = result.unwrap();
        assert_eq!(toot.id, "123456789");
        assert_eq!(toot.account.id, "user123");
        assert_eq!(toot.media_attachments.len(), 1);
        assert_eq!(toot.media_attachments[0].id, "media123");
    }

    #[test]
    fn test_parse_streaming_event_delete() {
        let config = create_test_config();
        let client = MastodonClient::new(config);

        let delete_event = StreamEvent {
            event: "delete".to_string(),
            payload: Some("123456789".to_string()),
        };
        let message = serde_json::to_string(&delete_event).unwrap();

        let result = client.parse_streaming_event(&message).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_parse_streaming_event_notification() {
        let config = create_test_config();
        let client = MastodonClient::new(config);

        let notification_event = StreamEvent {
            event: "notification".to_string(),
            payload: Some("{}".to_string()),
        };
        let message = serde_json::to_string(&notification_event).unwrap();

        let result = client.parse_streaming_event(&message).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_parse_streaming_event_unknown() {
        let config = create_test_config();
        let client = MastodonClient::new(config);

        let unknown_event = StreamEvent {
            event: "unknown_event".to_string(),
            payload: None,
        };
        let message = serde_json::to_string(&unknown_event).unwrap();

        let result = client.parse_streaming_event(&message).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_parse_streaming_event_invalid_json() {
        let config = create_test_config();
        let client = MastodonClient::new(config);

        let result = client.parse_streaming_event("invalid json");
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            MastodonError::InvalidTootData(_)
        ));
    }

    #[test]
    fn test_is_own_toot_matching_user() {
        let config = create_test_config();
        let mut client = MastodonClient::new(config);
        client.authenticated_user_id = Some("user123".to_string());

        let toot = TootEvent {
            id: "123".to_string(),
            account: Account {
                id: "user123".to_string(),
                username: "testuser".to_string(),
                acct: "testuser".to_string(),
                display_name: "Test User".to_string(),
                url: "https://example.com".to_string(),
            },
            content: "test".to_string(),
            language: None,
            media_attachments: vec![],
            created_at: Utc::now(),
            url: None,
            visibility: "public".to_string(),
        };

        let result = client.is_own_toot(&toot).unwrap();
        assert!(result);
    }

    #[test]
    fn test_is_own_toot_different_user() {
        let config = create_test_config();
        let mut client = MastodonClient::new(config);
        client.authenticated_user_id = Some("user123".to_string());

        let toot = TootEvent {
            id: "123".to_string(),
            account: Account {
                id: "user456".to_string(),
                username: "otheruser".to_string(),
                acct: "otheruser".to_string(),
                display_name: "Other User".to_string(),
                url: "https://example.com".to_string(),
            },
            content: "test".to_string(),
            language: None,
            media_attachments: vec![],
            created_at: Utc::now(),
            url: None,
            visibility: "public".to_string(),
        };

        let result = client.is_own_toot(&toot).unwrap();
        assert!(!result);
    }

    #[test]
    fn test_is_own_toot_no_authenticated_user() {
        let config = create_test_config();
        let client = MastodonClient::new(config);

        let toot = TootEvent {
            id: "123".to_string(),
            account: Account {
                id: "user123".to_string(),
                username: "testuser".to_string(),
                acct: "testuser".to_string(),
                display_name: "Test User".to_string(),
                url: "https://example.com".to_string(),
            },
            content: "test".to_string(),
            language: None,
            media_attachments: vec![],
            created_at: Utc::now(),
            url: None,
            visibility: "public".to_string(),
        };

        let result = client.is_own_toot(&toot);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            MastodonError::UserVerificationFailed
        ));
    }

    #[test]
    fn test_toot_event_serialization() {
        let toot = TootEvent {
            id: "123456789".to_string(),
            account: Account {
                id: "user123".to_string(),
                username: "testuser".to_string(),
                acct: "testuser@mastodon.social".to_string(),
                display_name: "Test User".to_string(),
                url: "https://mastodon.social/@testuser".to_string(),
            },
            content: "Test toot".to_string(),
            language: Some("en".to_string()),
            media_attachments: vec![],
            created_at: Utc::now(),
            url: Some("https://mastodon.social/@testuser/123456789".to_string()),
            visibility: "public".to_string(),
        };

        // Test serialization
        let json = serde_json::to_string(&toot).unwrap();
        assert!(json.contains("123456789"));
        assert!(json.contains("testuser"));

        // Test deserialization
        let deserialized: TootEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.id, toot.id);
        assert_eq!(deserialized.account.username, toot.account.username);
    }

    #[test]
    fn test_media_attachment_serialization() {
        let media = MediaAttachment {
            id: "media123".to_string(),
            media_type: "image".to_string(),
            url: "https://example.com/image.jpg".to_string(),
            preview_url: Some("https://example.com/image_small.jpg".to_string()),
            description: Some("A test image".to_string()),
            meta: Some(MediaMeta {
                original: Some(MediaDimensions {
                    width: Some(1920),
                    height: Some(1080),
                    size: Some("1920x1080".to_string()),
                    aspect: Some(1.777),
                }),
                small: None,
            }),
        };

        // Test serialization
        let json = serde_json::to_string(&media).unwrap();
        assert!(json.contains("media123"));
        assert!(json.contains("image"));
        assert!(json.contains("A test image"));

        // Test deserialization
        let deserialized: MediaAttachment = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.id, media.id);
        assert_eq!(deserialized.media_type, media.media_type);
        assert_eq!(deserialized.description, media.description);
    }

    // Integration test with mock WebSocket server
    #[tokio::test]
    async fn test_websocket_connection_and_message_parsing() {
        let server = MockWebSocketServer::new().await;
        let addr = server.local_addr();

        // Add a test message to the server
        server.add_message(create_test_toot_event()).await;

        // Start the server in a background task
        let server_handle = tokio::spawn(async move {
            server.run().await;
        });

        // Give the server a moment to start
        tokio::time::sleep(Duration::from_millis(100)).await;

        // Create a client with a custom WebSocket URL pointing to our mock server
        let mut config = create_test_config();
        config.instance_url = format!("ws://127.0.0.1:{}", addr.port());

        let mut client = MastodonClient::new(config);
        client.authenticated_user_id = Some("user123".to_string()); // Set authenticated user

        // Connect to the mock server
        let streaming_url = format!(
            "ws://127.0.0.1:{}/api/v1/streaming?access_token=test_token&stream=user",
            addr.port()
        );
        let url = Url::parse(&streaming_url).unwrap();

        let (ws_stream, _) = tokio_tungstenite::connect_async(url.as_str())
            .await
            .unwrap();
        client.websocket = Some(ws_stream);

        // Listen for a message
        let result = client.listen().await;

        // Clean up
        server_handle.abort();

        // Verify we received the expected toot
        assert!(result.is_ok());
        let toot = result.unwrap();
        assert!(toot.is_some());
        let toot = toot.unwrap();
        assert_eq!(toot.id, "123456789");
        assert_eq!(toot.account.id, "user123");
        assert_eq!(toot.media_attachments.len(), 1);
    }

    #[test]
    fn test_error_recovery_integration() {
        // Test that MastodonError variants work with ErrorRecovery
        let connection_error =
            AlternatorError::Mastodon(MastodonError::ConnectionFailed("timeout".to_string()));
        assert!(ErrorRecovery::is_recoverable(&connection_error));
        assert_eq!(ErrorRecovery::retry_delay(&connection_error, 0), 1);
        assert_eq!(ErrorRecovery::max_retries(&connection_error), 10);

        let rate_limit_error =
            AlternatorError::Mastodon(MastodonError::RateLimitExceeded { retry_after: 120 });
        assert!(ErrorRecovery::is_recoverable(&rate_limit_error));
        assert_eq!(ErrorRecovery::retry_delay(&rate_limit_error, 0), 120);

        let auth_error = AlternatorError::Mastodon(MastodonError::AuthenticationFailed(
            "invalid token".to_string(),
        ));
        assert!(!ErrorRecovery::is_recoverable(&auth_error));
        assert!(ErrorRecovery::should_shutdown(&auth_error));
    }
}
