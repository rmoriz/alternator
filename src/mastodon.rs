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

/// Zero-width space character used as invisible placeholder for empty text content
/// This allows media descriptions to be updated on posts that originally had no text
const ZERO_WIDTH_SPACE: &str = "\u{200B}";

/// Mastodon toot event from WebSocket stream
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TootEvent {
    pub id: String,
    pub uri: String,
    pub account: Account,
    pub content: String,
    pub language: Option<String>,
    pub media_attachments: Vec<MediaAttachment>,
    pub created_at: DateTime<Utc>,
    pub url: Option<String>,
    pub visibility: String,
    pub sensitive: bool,
    pub spoiler_text: String,
    pub in_reply_to_id: Option<String>,
    pub in_reply_to_account_id: Option<String>,
    pub mentions: Vec<Mention>,
    pub tags: Vec<Tag>,
    pub emojis: Vec<CustomEmoji>,
    pub poll: Option<Poll>,
    /// Indicates if this toot event represents an edit (from status.update)
    /// This field is not part of the Mastodon API but added by Alternator
    #[serde(skip)]
    pub is_edit: bool,
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

/// Media recreation data for uploading with descriptions
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct MediaRecreation {
    pub data: Vec<u8>,
    pub description: String,
    pub media_type: String,
    pub filename: String,
}

/// Mentioned user in a status
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Mention {
    pub id: String,
    pub username: String,
    pub url: String,
    pub acct: String,
}

/// Hashtag in a status
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tag {
    pub name: String,
    pub url: String,
}

/// Custom emoji in a status
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomEmoji {
    pub shortcode: String,
    pub url: String,
    pub static_url: String,
    pub visible_in_picker: bool,
}

/// Poll attached to a status
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Poll {
    pub id: String,
    pub expires_at: Option<DateTime<Utc>>,
    pub expired: bool,
    pub multiple: bool,
    pub votes_count: u32,
    pub voters_count: Option<u32>,
    pub voted: Option<bool>,
    pub own_votes: Option<Vec<u32>>,
    pub options: Vec<PollOption>,
    pub emojis: Vec<CustomEmoji>,
}

/// Poll option
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PollOption {
    pub title: String,
    pub votes_count: Option<u32>,
}

/// Status source for editing
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatusSource {
    pub id: String,
    pub text: String,
    pub spoiler_text: String,
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
    async fn get_status_source(&self, toot_id: &str) -> Result<StatusSource, MastodonError>;
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
        media_data: Vec<u8>,
        description: &str,
        filename: &str,
        media_type: &str,
    ) -> Result<String, MastodonError>;
    async fn recreate_media_with_descriptions(
        &self,
        toot_id: &str,
        media_recreations: Vec<MediaRecreation>,
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
        let http_url = format!("{base_url}/api/v1/streaming");

        // Make a HEAD request to resolve any redirects
        let response = self.http_client.head(&http_url).send().await.map_err(|e| {
            MastodonError::ConnectionFailed(format!("Failed to resolve streaming URL: {e}"))
        })?;

        let final_url = response.url().to_string();
        debug!("Resolved HTTP URL: {} -> {}", http_url, final_url);

        // Convert the final HTTP URL to WebSocket URL and add authentication
        let streaming_url = format!(
            "{}?access_token={}&stream=user",
            final_url
                .replace("https://", "wss://")
                .replace("http://", "ws://"),
            self.config.access_token
        );

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
            "update" | "status.update" => {
                if let Some(payload) = stream_event.payload {
                    let mut toot: TootEvent = serde_json::from_str(&payload).map_err(|e| {
                        MastodonError::InvalidTootData(format!("Failed to parse toot: {e}"))
                    })?;

                    // Set the is_edit flag based on the event type
                    toot.is_edit = stream_event.event == "status.update";

                    debug!(
                        "Parsed {} event: id={}, account={}, media_count={}, is_edit={}",
                        stream_event.event,
                        toot.id,
                        toot.account.id,
                        toot.media_attachments.len(),
                        toot.is_edit
                    );

                    Ok(Some(toot))
                } else {
                    warn!("Received {} event without payload", stream_event.event);
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
            // Initial delay to let Mastodon process the status update (increased from 5s to 10s)
            tokio::time::sleep(tokio::time::Duration::from_secs(10)).await;

            let mut retry_count = 0;
            const MAX_RETRIES: u32 = 3;
            // Exponential backoff: 10s, 20s, 40s
            const RETRY_DELAYS: [u64; 3] = [10, 20, 40];

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
                    let delay = RETRY_DELAYS[retry_count as usize - 1];
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

    /// Delete multiple media attachments (used for cleanup)
    async fn delete_multiple_media_attachments(
        &self,
        media_ids: Vec<String>,
    ) -> Result<(), MastodonError> {
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

    /// Get status source (original plain text) for editing
    async fn get_status_source(&self, toot_id: &str) -> Result<StatusSource, MastodonError> {
        let url = format!(
            "{}/api/v1/statuses/{}/source",
            self.config.instance_url.trim_end_matches('/'),
            toot_id
        );

        debug!("Fetching status source: {}", url);

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
                MastodonError::ApiRequestFailed(format!("Failed to fetch status source: {e}"))
            })?;

        if response.status() == 404 {
            return Err(MastodonError::TootNotFound {
                toot_id: toot_id.to_string(),
            });
        }

        if !response.status().is_success() {
            return Err(MastodonError::ApiRequestFailed(format!(
                "Status source API request failed with status: {}",
                response.status()
            )));
        }

        let source: StatusSource = response.json().await.map_err(|e| {
            MastodonError::InvalidTootData(format!("Failed to parse status source response: {e}"))
        })?;

        debug!(
            "Retrieved status source: id={}, text_length={}",
            source.id,
            source.text.len()
        );
        Ok(source)
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

        // Get original status text from source API to preserve exact original text
        let status_source = self.get_status_source(toot_id).await?;

        // Use zero-width space for empty content to allow media description updates
        // Otherwise use original text exactly as-is without any HTML processing
        let status_text = if status_source.text.trim().is_empty() {
            debug!("Using zero-width space for empty content to enable media description update");
            ZERO_WIDTH_SPACE.to_string()
        } else {
            debug!("Using original status text exactly as-is");
            status_source.text
        };

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
        media_data: Vec<u8>,
        description: &str,
        filename: &str,
        media_type: &str,
    ) -> Result<String, MastodonError> {
        let url = format!(
            "{}/api/v2/media",
            self.config.instance_url.trim_end_matches('/')
        );

        // Validate and sanitize the MIME type
        let mime_type = Self::validate_and_sanitize_mime_type(media_type, filename)?;

        tracing::debug!(
            "Creating media attachment with MIME type: '{mime_type}' for file: '{filename}'"
        );

        // Create multipart form with media data and description
        let form = reqwest::multipart::Form::new()
            .part(
                "file",
                reqwest::multipart::Part::bytes(media_data)
                    .file_name(filename.to_string())
                    .mime_str(&mime_type)
                    .map_err(|e| {
                        tracing::error!("Failed to set MIME type '{mime_type}': {e}");
                        MastodonError::ApiRequestFailed(format!(
                            "Failed to set MIME type '{mime_type}': {e}"
                        ))
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
        media_recreations: Vec<MediaRecreation>,
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
        for (index, recreation) in media_recreations.iter().enumerate() {
            match self
                .create_media_attachment(
                    recreation.data.clone(),
                    &recreation.description,
                    &recreation.filename,
                    &recreation.media_type,
                )
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

        // Step 2: Wait for media processing and update the status with retry logic
        self.update_status_with_media_retry(toot_id, new_media_ids, media_recreations.len())
            .await?;

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
    /// Update status with new media IDs, handling Mastodon processing delays with retries
    async fn update_status_with_media_retry(
        &self,
        toot_id: &str,
        new_media_ids: Vec<String>,
        media_count: usize,
    ) -> Result<(), MastodonError> {
        const MAX_RETRIES: u32 = 4;
        // Initial wait + retry delays: 2s, 5s, 10s, 20s
        const RETRY_DELAYS: [u64; 4] = [2, 5, 10, 20];

        let mut retry_count = 0;

        // Initial delay to let Mastodon process the uploaded media
        debug!(
            "Waiting 2 seconds for Mastodon to process {} media attachments",
            media_count
        );
        tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;

        loop {
            match self.update_status_with_media(toot_id, &new_media_ids).await {
                Ok(()) => {
                    info!(
                        "Successfully recreated {} media attachments for toot: {}",
                        media_count, toot_id
                    );
                    return Ok(());
                }
                Err(MastodonError::ApiRequestFailed(ref error_text))
                    if retry_count < MAX_RETRIES
                        && (error_text.contains("422")
                            && (error_text.contains("nicht verarbeitet wurden")  // German
                            || error_text.contains("not yet been processed")  // English
                            || error_text.contains("cannot be attached"))) =>
                {
                    let delay = RETRY_DELAYS[retry_count as usize];
                    retry_count += 1;

                    warn!(
                        "Media attachments not yet processed by Mastodon (attempt {}), retrying in {} seconds: {}",
                        retry_count, delay, error_text
                    );

                    tokio::time::sleep(tokio::time::Duration::from_secs(delay)).await;
                }
                Err(e) => {
                    error!(
                        "Failed to update status with new media after {} attempts: {}",
                        retry_count + 1,
                        e
                    );

                    // Clean up the media we created since we couldn't attach them
                    if !new_media_ids.is_empty() {
                        if let Err(cleanup_error) =
                            self.delete_multiple_media_attachments(new_media_ids).await
                        {
                            warn!(
                                "Failed to clean up media after status update failure: {}",
                                cleanup_error
                            );
                        }
                    }

                    return Err(e);
                }
            }
        }
    }

    /// Update status with new media attachments (single attempt)
    async fn update_status_with_media(
        &self,
        toot_id: &str,
        new_media_ids: &[String],
    ) -> Result<(), MastodonError> {
        // Step 2: Update the status to use the new media attachments
        let url = format!(
            "{}/api/v1/statuses/{}",
            self.config.instance_url.trim_end_matches('/'),
            toot_id
        );

        // Get current status to preserve its metadata
        let current_status = self.get_toot(toot_id).await?;

        // Get original status text from source API to preserve mentions properly
        let status_source = self.get_status_source(toot_id).await?;

        debug!("Original content HTML: {}", current_status.content);
        debug!("Source text: '{}'", status_source.text);

        // Use zero-width space for empty content to allow media description updates
        // Mastodon requires text content when updating a status, but we want to support
        // adding descriptions to media-only posts
        let status_content = if status_source.text.trim().is_empty() {
            debug!("Using zero-width space for empty content to enable media description update");
            ZERO_WIDTH_SPACE.to_string()
        } else {
            debug!("Including original status text from source in update");
            status_source.text
        };

        // Create form data as a vector of tuples to properly handle array parameters
        let mut form_data = Vec::new();
        form_data.push(("status", status_content.as_str()));

        // Preserve sensitivity and spoiler text (use source for spoiler_text to get original)
        if current_status.sensitive {
            form_data.push(("sensitive", "true"));
        }
        if !status_source.spoiler_text.is_empty() {
            form_data.push(("spoiler_text", status_source.spoiler_text.as_str()));
        }

        // Preserve language if specified
        if let Some(ref lang) = current_status.language {
            form_data.push(("language", lang.as_str()));
        }

        // Note: visibility and in_reply_to_id are NOT supported by the edit status API
        // These fields are immutable after status creation and sending them causes 422 errors

        // Add new media IDs as array parameters
        for media_id in new_media_ids.iter() {
            form_data.push(("media_ids[]", media_id.as_str()));
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
            return Err(MastodonError::ApiRequestFailed(format!(
                "Status update failed with status {status}: {error_text}"
            )));
        }

        Ok(())
    }
}

impl MastodonClient {
    /// Determine MIME type from media type and filename
    #[allow(dead_code)]
    fn determine_mime_type(media_type: &str, filename: &str) -> String {
        // Try to match based on the Mastodon media_type first
        match media_type {
            // Audio types
            "audio/mpeg" | "audio/mp3" => "audio/mpeg".to_string(),
            "audio/wav" | "audio/wave" | "audio/x-wav" => "audio/wav".to_string(),
            "audio/m4a" => "audio/mp4".to_string(),
            "audio/mp4" => "audio/mp4".to_string(),
            "audio/aac" => "audio/aac".to_string(),
            "audio/ogg" => "audio/ogg".to_string(),
            "audio/flac" | "audio/x-flac" => "audio/flac".to_string(),
            // Generic audio fallback
            "audio" => {
                // Try to determine from filename extension
                let extension = filename.split('.').next_back().unwrap_or("").to_lowercase();
                match extension.as_str() {
                    "mp3" => "audio/mpeg",
                    "wav" => "audio/wav",
                    "m4a" => "audio/mp4",
                    "mp4" => "audio/mp4",
                    "aac" => "audio/aac",
                    "ogg" => "audio/ogg",
                    "flac" => "audio/flac",
                    _ => "audio/mpeg", // Default to MP3
                }
                .to_string()
            }
            // Image types (existing logic)
            "image/jpeg" | "image/jpg" => "image/jpeg".to_string(),
            "image/png" => "image/png".to_string(),
            "image/gif" => "image/gif".to_string(),
            "image/webp" => "image/webp".to_string(),
            // Generic image fallback
            "image" => {
                // Try to determine from filename extension
                let extension = filename.split('.').next_back().unwrap_or("").to_lowercase();
                match extension.as_str() {
                    "jpg" | "jpeg" => "image/jpeg",
                    "png" => "image/png",
                    "gif" => "image/gif",
                    "webp" => "image/webp",
                    _ => "image/jpeg", // Default to JPEG
                }
                .to_string()
            }
            // Video types (future support)
            "video/mp4" => "video/mp4".to_string(),
            "video/webm" => "video/webm".to_string(),
            "video" => "video/mp4".to_string(), // Default to MP4
            // Fallback for unknown types
            _ => {
                // Try to determine from filename extension as last resort
                let extension = filename.split('.').next_back().unwrap_or("").to_lowercase();
                match extension.as_str() {
                    // Audio extensions
                    "mp3" => "audio/mpeg",
                    "wav" => "audio/wav",
                    "m4a" | "mp4" => "audio/mp4", // Note: mp4 could be video or audio
                    "aac" => "audio/aac",
                    "ogg" => "audio/ogg",
                    "flac" => "audio/flac",
                    // Image extensions
                    "jpg" | "jpeg" => "image/jpeg",
                    "png" => "image/png",
                    "gif" => "image/gif",
                    "webp" => "image/webp",
                    // Video extensions
                    "webm" => "video/webm",
                    // Default fallback
                    _ => "application/octet-stream",
                }
                .to_string()
            }
        }
    }

    /// Validate and sanitize MIME type, with fallback for invalid types
    fn validate_and_sanitize_mime_type(
        media_type: &str,
        filename: &str,
    ) -> Result<String, MastodonError> {
        // Check for empty or whitespace-only MIME type
        let trimmed_type = media_type.trim();
        if trimmed_type.is_empty() {
            tracing::warn!(
                "Empty MIME type provided, falling back to filename detection for: {filename}"
            );
            return Ok(Self::determine_mime_type("", filename));
        }

        // Check for basic MIME type format (type/subtype)
        if !trimmed_type.contains('/') {
            // This is a common case for Mastodon media types like "audio", "image", "video"
            if matches!(trimmed_type, "audio" | "image" | "video") {
                tracing::debug!("Generic media type '{trimmed_type}' detected, using filename detection for: {filename}");
            } else {
                tracing::warn!("Invalid MIME type format '{trimmed_type}', falling back to filename detection for: {filename}");
            }
            return Ok(Self::determine_mime_type(trimmed_type, filename));
        }

        // Check for invalid characters that could cause reqwest to fail
        if trimmed_type.contains('\n') || trimmed_type.contains('\r') || trimmed_type.contains('\0')
        {
            tracing::warn!("MIME type contains invalid characters '{trimmed_type}', falling back to filename detection for: {filename}");
            return Ok(Self::determine_mime_type("", filename));
        }

        // Validate basic MIME type structure
        let parts: Vec<&str> = trimmed_type.split('/').collect();
        if parts.len() != 2 || parts[0].is_empty() || parts[1].is_empty() {
            tracing::warn!("Malformed MIME type '{trimmed_type}', falling back to filename detection for: {filename}");
            return Ok(Self::determine_mime_type("", filename));
        }

        // MIME type appears valid, return it
        Ok(trimmed_type.to_string())
    }

    /// Extract plain text from HTML content
    #[allow(dead_code)] // Used in tests and integration tests
    pub fn extract_text_from_html(html: &str) -> String {
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
            uri: "https://mastodon.social/users/testuser/statuses/123456789".to_string(),
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
            in_reply_to_id: None,
            in_reply_to_account_id: None,
            mentions: Vec::new(),
            sensitive: false,
            spoiler_text: "".to_string(),
            tags: Vec::new(),
            emojis: Vec::new(),
            poll: None,
            is_edit: false,
        };

        let stream_event = StreamEvent {
            event: "update".to_string(),
            payload: Some(serde_json::to_string(&toot).unwrap()),
        };

        serde_json::to_string(&stream_event).unwrap()
    }

    #[test]
    fn test_is_own_toot_matching_user() {
        let config = create_test_config();
        let mut client = MastodonClient::new(config);
        client.authenticated_user_id = Some("user123".to_string());

        let toot = TootEvent {
            id: "123".to_string(),
            uri: "https://example.com/users/testuser/statuses/123".to_string(),
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
            in_reply_to_id: None,
            in_reply_to_account_id: None,
            mentions: Vec::new(),
            sensitive: false,
            spoiler_text: "".to_string(),
            tags: Vec::new(),
            emojis: Vec::new(),
            poll: None,
            is_edit: false,
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
            uri: "https://example.com/users/testuser/statuses/123".to_string(),
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
            in_reply_to_id: None,
            in_reply_to_account_id: None,
            mentions: Vec::new(),
            sensitive: false,
            spoiler_text: "".to_string(),
            tags: Vec::new(),
            emojis: Vec::new(),
            poll: None,
            is_edit: false,
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
            uri: "https://example.com/users/testuser/statuses/123".to_string(),
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
            in_reply_to_id: None,
            in_reply_to_account_id: None,
            mentions: Vec::new(),
            sensitive: false,
            spoiler_text: "".to_string(),
            tags: Vec::new(),
            emojis: Vec::new(),
            poll: None,
            is_edit: false,
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
            uri: "https://mastodon.social/users/testuser/statuses/123456789".to_string(),
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
            in_reply_to_id: None,
            in_reply_to_account_id: None,
            mentions: Vec::new(),
            sensitive: false,
            spoiler_text: "".to_string(),
            tags: Vec::new(),
            emojis: Vec::new(),
            poll: None,
            is_edit: false,
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
    fn test_extract_text_from_html_empty_content() {
        // Test the HTML text extraction with empty content
        assert_eq!(MastodonClient::extract_text_from_html(""), "");
        assert_eq!(MastodonClient::extract_text_from_html("   "), "");
        assert_eq!(MastodonClient::extract_text_from_html("\n\t "), "");

        // Test with only HTML tags (media-only posts)
        assert_eq!(MastodonClient::extract_text_from_html("<p></p>"), "");
        assert_eq!(MastodonClient::extract_text_from_html("<p>   </p>"), "");
        assert_eq!(MastodonClient::extract_text_from_html("<br/>"), "");
        assert_eq!(
            MastodonClient::extract_text_from_html("<div><span></span></div>"),
            ""
        );

        // Test with actual content
        assert_eq!(
            MastodonClient::extract_text_from_html("<p>Hello world</p>"),
            "Hello world"
        );
        assert_eq!(
            MastodonClient::extract_text_from_html("Plain text"),
            "Plain text"
        );

        // Test with HTML entities
        assert_eq!(
            MastodonClient::extract_text_from_html("&quot;quoted&quot; &amp; escaped"),
            "\"quoted\" & escaped"
        );
    }

    #[test]
    fn test_media_only_post_validation_behavior() {
        // This test documents the current behavior where media-only posts
        // (empty text content) cannot be updated via the status API
        // This is the scenario that would benefit from unicode space character solution

        let config = create_test_config();
        let _client = MastodonClient::new(config);

        // Test empty content scenarios that would trigger the validation issue
        let empty_html_cases = vec![
            "",                         // Completely empty
            "   ",                      // Only whitespace
            "<p></p>",                  // Empty HTML tags
            "<p>   </p>",               // HTML tags with only whitespace
            "<br/>",                    // Self-closing tags
            "<div><span></span></div>", // Nested empty tags
        ];

        for empty_content in empty_html_cases {
            let extracted_text = MastodonClient::extract_text_from_html(empty_content);

            // Current behavior: empty text means the update will be skipped
            // This is where a unicode space character could be used instead
            assert!(
                extracted_text.trim().is_empty(),
                "Expected empty text for '{empty_content}', got '{extracted_text}'"
            );
        }

        // Test that non-empty content works as expected
        let non_empty_cases = vec![
            ("Hello world", "Hello world"),
            ("<p>Test content</p>", "Test content"),
            ("Mixed <strong>content</strong> here", "Mixed content here"),
        ];

        for (input, expected) in non_empty_cases {
            let extracted_text = MastodonClient::extract_text_from_html(input);
            assert_eq!(extracted_text, expected);
            assert!(!extracted_text.trim().is_empty());
        }
    }

    #[test]
    fn test_unicode_space_solution_for_empty_posts() {
        // Test the proposed unicode space solution for empty posts
        // This demonstrates how specific unicode characters could be used
        // to satisfy Mastodon's validation while being minimally visible

        // Only test characters that are NOT trimmed by Rust (i.e., that work for our purpose)
        let working_unicode_chars = vec![
            ("\u{200B}", "zero-width space"), // Invisible, not trimmed
            ("\u{2060}", "word joiner"),      // Invisible, no-break, not trimmed
        ];

        // Test characters that are trimmed (these won't work for our solution)
        let trimmed_unicode_chars = vec![
            ("\u{2009}", "thin space"),         // Minimal visible, but trimmed
            ("\u{200A}", "hair space"),         // Thinnest visible, but trimmed
            ("\u{00A0}", "non-breaking space"), // Standard alternative, but trimmed
        ];

        // Test working characters
        for (space_char, description) in working_unicode_chars {
            // These characters should not be considered "empty" by trim()
            assert!(
                !space_char.trim().is_empty(),
                "{description} should not be considered empty by trim()"
            );

            // They should be very short (1 character)
            assert_eq!(
                space_char.chars().count(),
                1,
                "{description} should be exactly 1 character"
            );

            // Test that they would pass the validation check
            // This simulates what the validation logic would see
            let would_pass_validation = !space_char.trim().is_empty();
            assert!(
                would_pass_validation,
                "{description} should pass the validation check"
            );
        }

        // Test trimmed characters (document that they won't work)
        for (space_char, description) in trimmed_unicode_chars {
            // These characters ARE considered "empty" by trim() so won't work for our solution
            assert!(
                space_char.trim().is_empty(),
                "{description} should be considered empty by trim() - won't work for our solution"
            );
        }

        // Test the recommended zero-width space specifically
        let zero_width_space = "\u{200B}";
        assert!(!zero_width_space.trim().is_empty());
        assert_eq!(zero_width_space.len(), 3); // UTF-8 encoding length
        assert_eq!(zero_width_space.chars().count(), 1); // Unicode character count

        // Verify it's not whitespace in Rust's definition (so it won't be trimmed)
        assert!(!zero_width_space.chars().all(|c| c.is_whitespace()));

        // Test the word joiner as an alternative
        let word_joiner = "\u{2060}";
        assert!(!word_joiner.trim().is_empty());
        assert_eq!(word_joiner.chars().count(), 1);
        assert!(!word_joiner.chars().all(|c| c.is_whitespace()));
    }

    #[test]
    fn test_zero_width_space_implementation() {
        // Test that the zero-width space constant is correctly defined
        assert_eq!(ZERO_WIDTH_SPACE, "\u{200B}");
        assert!(!ZERO_WIDTH_SPACE.trim().is_empty());
        assert_eq!(ZERO_WIDTH_SPACE.chars().count(), 1);

        // Test that it would pass validation
        let would_pass_validation = !ZERO_WIDTH_SPACE.trim().is_empty();
        assert!(would_pass_validation);

        // Test that it's invisible (not ASCII graphic)
        assert!(ZERO_WIDTH_SPACE.chars().all(|c| !c.is_ascii_graphic()));
    }

    #[test]
    fn test_status_content_logic_with_zero_width_space() {
        // Test the logic that would be used in both update functions
        // We now work directly with source text, not HTML
        let test_cases = vec![
            ("", ZERO_WIDTH_SPACE),                       // Empty -> zero-width space
            ("   ", ZERO_WIDTH_SPACE),                    // Whitespace -> zero-width space
            ("Hello world", "Hello world"),               // Normal text -> unchanged
            ("Test content", "Test content"),             // Regular text -> unchanged
            ("Multi\nline\ntext", "Multi\nline\ntext"),   // Preserve formatting
            ("Text with emoji 🎉", "Text with emoji 🎉"), // Preserve emojis
        ];

        for (source_text, expected_status_content) in test_cases {
            // Simulate the logic from both update functions
            let status_content = if source_text.trim().is_empty() {
                ZERO_WIDTH_SPACE.to_string()
            } else {
                source_text.to_string()
            };

            assert_eq!(
                status_content, expected_status_content,
                "For input '{source_text}', expected '{expected_status_content}' but got '{status_content}'"
            );

            // Verify that the result always passes validation
            assert!(
                !status_content.trim().is_empty(),
                "Status content '{status_content}' should pass validation"
            );
        }
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

    #[test]
    fn test_client_clone() {
        let config = create_test_config();
        let mut client = MastodonClient::new(config.clone());
        client.authenticated_user_id = Some("test_user".to_string());
        client.reconnect_attempts = 5;

        let cloned_client = client.clone();

        assert_eq!(cloned_client.config.instance_url, config.instance_url);
        assert_eq!(cloned_client.config.access_token, config.access_token);
        assert_eq!(
            cloned_client.authenticated_user_id,
            Some("test_user".to_string())
        );
        assert_eq!(cloned_client.reconnect_attempts, 5);
        assert!(cloned_client.websocket.is_none()); // WebSocket connections can't be cloned
    }

    #[test]
    fn test_stream_event_serialization() {
        let stream_event = StreamEvent {
            event: "update".to_string(),
            payload: Some("test payload".to_string()),
        };

        let json = serde_json::to_string(&stream_event).unwrap();
        assert!(json.contains("update"));
        assert!(json.contains("test payload"));

        let deserialized: StreamEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.event, "update");
        assert_eq!(deserialized.payload, Some("test payload".to_string()));
    }

    #[test]
    fn test_stream_event_without_payload() {
        let stream_event = StreamEvent {
            event: "heartbeat".to_string(),
            payload: None,
        };

        let json = serde_json::to_string(&stream_event).unwrap();
        assert!(json.contains("heartbeat"));

        let deserialized: StreamEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.event, "heartbeat");
        assert!(deserialized.payload.is_none());
    }

    #[test]
    fn test_media_dimensions_serialization() {
        let dimensions = MediaDimensions {
            width: Some(1920),
            height: Some(1080),
            size: Some("1920x1080".to_string()),
            aspect: Some(1.777),
        };

        let json = serde_json::to_string(&dimensions).unwrap();
        assert!(json.contains("1920"));
        assert!(json.contains("1080"));
        assert!(json.contains("1.777"));

        let deserialized: MediaDimensions = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.width, Some(1920));
        assert_eq!(deserialized.height, Some(1080));
        assert_eq!(deserialized.aspect, Some(1.777));
    }

    #[test]
    fn test_media_meta_serialization() {
        let meta = MediaMeta {
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
        };

        let json = serde_json::to_string(&meta).unwrap();
        assert!(json.contains("1920"));
        assert!(json.contains("400"));

        let deserialized: MediaMeta = serde_json::from_str(&json).unwrap();
        assert!(deserialized.original.is_some());
        assert!(deserialized.small.is_some());
    }

    #[test]
    fn test_account_serialization() {
        let account = Account {
            id: "user123".to_string(),
            username: "testuser".to_string(),
            acct: "testuser@mastodon.social".to_string(),
            display_name: "Test User".to_string(),
            url: "https://mastodon.social/@testuser".to_string(),
        };

        let json = serde_json::to_string(&account).unwrap();
        assert!(json.contains("user123"));
        assert!(json.contains("testuser"));
        assert!(json.contains("Test User"));

        let deserialized: Account = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.id, "user123");
        assert_eq!(deserialized.username, "testuser");
        assert_eq!(deserialized.display_name, "Test User");
    }

    #[test]
    fn test_extract_text_from_html_complex_cases() {
        // Test nested tags
        assert_eq!(
            MastodonClient::extract_text_from_html("<p>Hello <strong>world</strong>!</p>"),
            "Hello world!"
        );

        // Test multiple HTML entities
        assert_eq!(
            MastodonClient::extract_text_from_html(
                "&lt;test&gt; &amp; &quot;quoted&quot; &#39;text&#39; &nbsp;space"
            ),
            "<test> & \"quoted\" 'text'  space"
        );

        // Test malformed HTML
        assert_eq!(
            MastodonClient::extract_text_from_html("<p>Unclosed tag"),
            "Unclosed tag"
        );

        // Test mixed content
        assert_eq!(
            MastodonClient::extract_text_from_html("Before <span>middle</span> after"),
            "Before middle after"
        );
    }

    #[test]
    fn test_parse_streaming_event_malformed_payload() {
        let config = create_test_config();
        let client = MastodonClient::new(config);

        // Test event with malformed JSON payload
        let malformed_event = StreamEvent {
            event: "update".to_string(),
            payload: Some("{invalid json}".to_string()),
        };
        let message = serde_json::to_string(&malformed_event).unwrap();

        let result = client.parse_streaming_event(&message);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            MastodonError::InvalidTootData(_)
        ));
    }

    #[test]
    fn test_parse_streaming_event_empty_payload() {
        let config = create_test_config();
        let client = MastodonClient::new(config);

        // Test event with empty payload
        let empty_event = StreamEvent {
            event: "update".to_string(),
            payload: Some("".to_string()),
        };
        let message = serde_json::to_string(&empty_event).unwrap();

        let result = client.parse_streaming_event(&message);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_streaming_event_status_update() {
        let config = create_test_config();
        let client = MastodonClient::new(config);

        // Create a test toot for status update event
        let toot = TootEvent {
            id: "test_edit_123".to_string(),
            uri: "https://mastodon.social/users/testuser/statuses/test_edit_123".to_string(),
            account: Account {
                id: "user123".to_string(),
                username: "testuser".to_string(),
                acct: "testuser@mastodon.social".to_string(),
                display_name: "Test User".to_string(),
                url: "https://mastodon.social/@testuser".to_string(),
            },
            content: "This is an edited toot".to_string(),
            language: Some("en".to_string()),
            media_attachments: vec![],
            created_at: Utc::now(),
            url: Some("https://mastodon.social/@testuser/test_edit_123".to_string()),
            visibility: "public".to_string(),
            in_reply_to_id: None,
            in_reply_to_account_id: None,
            mentions: Vec::new(),
            sensitive: false,
            spoiler_text: "".to_string(),
            tags: Vec::new(),
            emojis: Vec::new(),
            poll: None,
            is_edit: false, // This will be set by the parser
        };

        // Test status.update event
        let status_update_event = StreamEvent {
            event: "status.update".to_string(),
            payload: Some(serde_json::to_string(&toot).unwrap()),
        };
        let message = serde_json::to_string(&status_update_event).unwrap();

        let result = client.parse_streaming_event(&message).unwrap();
        assert!(result.is_some());
        let parsed_toot = result.unwrap();
        assert_eq!(parsed_toot.id, "test_edit_123");
        assert!(
            parsed_toot.is_edit,
            "Status update event should set is_edit to true"
        );

        // Test regular update event for comparison
        let regular_update_event = StreamEvent {
            event: "update".to_string(),
            payload: Some(serde_json::to_string(&toot).unwrap()),
        };
        let message = serde_json::to_string(&regular_update_event).unwrap();

        let result = client.parse_streaming_event(&message).unwrap();
        assert!(result.is_some());
        let parsed_toot = result.unwrap();
        assert_eq!(parsed_toot.id, "test_edit_123");
        assert!(
            !parsed_toot.is_edit,
            "Regular update event should set is_edit to false"
        );
    }

    #[test]
    fn test_streaming_url_with_custom_stream() {
        let mut config = create_test_config();
        config.user_stream = Some(false);
        let client = MastodonClient::new(config);

        let url = client.get_streaming_url().unwrap();
        // Should still default to user stream even when config says false
        // (this is the current behavior based on the implementation)
        assert!(url.as_str().contains("stream=user"));
    }

    #[test]
    fn test_streaming_url_with_trailing_slash() {
        let mut config = create_test_config();
        config.instance_url = "https://mastodon.social/".to_string();
        let client = MastodonClient::new(config);

        let url = client.get_streaming_url().unwrap();
        // Should handle trailing slash correctly
        assert!(url
            .as_str()
            .starts_with("wss://mastodon.social/api/v1/streaming"));
    }

    #[test]
    fn test_streaming_url_invalid_instance_url() {
        let mut config = create_test_config();
        config.instance_url = "not-a-valid-url".to_string();
        let client = MastodonClient::new(config);

        let result = client.get_streaming_url();
        assert!(result.is_err());
        // The exact error type will depend on URL parsing implementation
    }

    #[test]
    fn test_toot_event_with_different_visibility() {
        let visibilities = ["public", "unlisted", "private", "direct"];

        for visibility in visibilities {
            let toot = TootEvent {
                id: "test".to_string(),
                uri: "https://example.com/users/user/statuses/test".to_string(),
                account: Account {
                    id: "user".to_string(),
                    username: "user".to_string(),
                    acct: "user".to_string(),
                    display_name: "User".to_string(),
                    url: "https://example.com".to_string(),
                },
                content: "test".to_string(),
                language: Some("en".to_string()),
                media_attachments: vec![],
                created_at: Utc::now(),
                url: Some("https://example.com/test".to_string()),
                visibility: visibility.to_string(),
                in_reply_to_id: None,
                in_reply_to_account_id: None,
                mentions: Vec::new(),
                sensitive: false,
                spoiler_text: "".to_string(),
                tags: Vec::new(),
                emojis: Vec::new(),
                poll: None,
                is_edit: false,
            };

            assert_eq!(toot.visibility, visibility);

            // Test serialization/deserialization
            let json = serde_json::to_string(&toot).unwrap();
            let deserialized: TootEvent = serde_json::from_str(&json).unwrap();
            assert_eq!(deserialized.visibility, visibility);
        }
    }

    #[test]
    fn test_media_attachment_different_types() {
        let media_types = ["image", "video", "gifv", "audio", "unknown"];

        for media_type in media_types {
            let media = MediaAttachment {
                id: "test".to_string(),
                media_type: media_type.to_string(),
                url: "https://example.com/media".to_string(),
                preview_url: None,
                description: None,
                meta: None,
            };

            assert_eq!(media.media_type, media_type);

            // Test serialization uses correct field name "type"
            let json = serde_json::to_string(&media).unwrap();
            assert!(json.contains(&format!("\"type\":\"{media_type}\"")));

            let deserialized: MediaAttachment = serde_json::from_str(&json).unwrap();
            assert_eq!(deserialized.media_type, media_type);
        }
    }

    #[test]
    fn test_toot_event_optional_fields() {
        // Test toot with minimal fields
        let minimal_toot = TootEvent {
            id: "test".to_string(),
            uri: "https://example.com/users/user/statuses/test".to_string(),
            account: Account {
                id: "user".to_string(),
                username: "user".to_string(),
                acct: "user".to_string(),
                display_name: "User".to_string(),
                url: "https://example.com".to_string(),
            },
            content: "test".to_string(),
            language: None,
            media_attachments: vec![],
            created_at: Utc::now(),
            url: None,
            visibility: "public".to_string(),
            in_reply_to_id: None,
            in_reply_to_account_id: None,
            mentions: Vec::new(),
            sensitive: false,
            spoiler_text: "".to_string(),
            tags: Vec::new(),
            emojis: Vec::new(),
            poll: None,
            is_edit: false,
        };

        assert!(minimal_toot.language.is_none());
        assert!(minimal_toot.url.is_none());
        assert!(minimal_toot.media_attachments.is_empty());

        // Test serialization/deserialization
        let json = serde_json::to_string(&minimal_toot).unwrap();
        let deserialized: TootEvent = serde_json::from_str(&json).unwrap();
        assert!(deserialized.language.is_none());
        assert!(deserialized.url.is_none());
    }

    #[test]
    fn test_determine_mime_type() {
        // Test audio types
        assert_eq!(
            MastodonClient::determine_mime_type("audio/mpeg", "test.mp3"),
            "audio/mpeg"
        );
        assert_eq!(
            MastodonClient::determine_mime_type("audio/wav", "test.wav"),
            "audio/wav"
        );
        assert_eq!(
            MastodonClient::determine_mime_type("audio/m4a", "test.m4a"),
            "audio/mp4"
        );
        assert_eq!(
            MastodonClient::determine_mime_type("audio/flac", "test.flac"),
            "audio/flac"
        );

        // Test generic audio with filename fallback
        assert_eq!(
            MastodonClient::determine_mime_type("audio", "test.mp3"),
            "audio/mpeg"
        );
        assert_eq!(
            MastodonClient::determine_mime_type("audio", "test.wav"),
            "audio/wav"
        );
        assert_eq!(
            MastodonClient::determine_mime_type("audio", "test.unknown"),
            "audio/mpeg" // Default fallback
        );

        // Test image types
        assert_eq!(
            MastodonClient::determine_mime_type("image/jpeg", "test.jpg"),
            "image/jpeg"
        );
        assert_eq!(
            MastodonClient::determine_mime_type("image/png", "test.png"),
            "image/png"
        );

        // Test generic image with filename fallback
        assert_eq!(
            MastodonClient::determine_mime_type("image", "test.jpg"),
            "image/jpeg"
        );
        assert_eq!(
            MastodonClient::determine_mime_type("image", "test.png"),
            "image/png"
        );

        // Test unknown type with filename fallback
        assert_eq!(
            MastodonClient::determine_mime_type("unknown", "test.mp3"),
            "audio/mpeg"
        );
        assert_eq!(
            MastodonClient::determine_mime_type("unknown", "test.jpg"),
            "image/jpeg"
        );
        assert_eq!(
            MastodonClient::determine_mime_type("unknown", "test.unknown"),
            "application/octet-stream" // Ultimate fallback
        );
    }
}
