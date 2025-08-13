use crate::error::{AlternatorError, MastodonError};
use crate::language::LanguageDetector;
use crate::mastodon::{MastodonClient, MastodonStream, TootEvent};
use crate::media::MediaProcessor;
use crate::openrouter::OpenRouterClient;
use std::collections::HashSet;
use tracing::{debug, error, info, warn};

/// Handler for processing incoming toot events from WebSocket stream
pub struct TootStreamHandler {
    mastodon_client: MastodonClient,
    openrouter_client: OpenRouterClient,
    media_processor: MediaProcessor,
    language_detector: LanguageDetector,
    processed_toots: HashSet<String>,
}

impl TootStreamHandler {
    /// Create a new toot stream handler
    pub fn new(
        mastodon_client: MastodonClient,
        openrouter_client: OpenRouterClient,
        media_processor: MediaProcessor,
        language_detector: LanguageDetector,
    ) -> Self {
        Self {
            mastodon_client,
            openrouter_client,
            media_processor,
            language_detector,
            processed_toots: HashSet::new(),
        }
    }

    /// Start processing toot stream - main entry point
    pub async fn start_processing(&mut self) -> Result<(), AlternatorError> {
        info!("Starting toot stream processing");

        // Connect to Mastodon WebSocket stream
        self.mastodon_client
            .connect()
            .await
            .map_err(AlternatorError::Mastodon)?;

        info!("✓ Connected to Mastodon stream - listening for toots");

        // Main processing loop
        loop {
            match self.listen_and_process().await {
                Ok(()) => {
                    // Continue processing
                    continue;
                }
                Err(e) => {
                    error!("Error in toot processing loop: {}", e);

                    // Handle specific error types
                    match &e {
                        AlternatorError::Mastodon(MastodonError::Disconnected(_))
                        | AlternatorError::Mastodon(MastodonError::ConnectionFailed(_)) => {
                            warn!("Connection lost, will attempt to reconnect");
                            // The MastodonClient will handle reconnection automatically
                            continue;
                        }
                        _ => {
                            // For other errors, propagate up to main application
                            return Err(e);
                        }
                    }
                }
            }
        }
    }

    /// Listen for a single toot event and process it
    async fn listen_and_process(&mut self) -> Result<(), AlternatorError> {
        // Listen for toot events
        match self.mastodon_client.listen().await {
            Ok(Some(toot)) => {
                // Verify this is from the authenticated user (already done in MastodonClient)
                // Check for duplicate processing
                if self.is_already_processed(&toot.id) {
                    debug!("Skipping already processed toot: {}", toot.id);
                    return Ok(());
                }

                info!(
                    "Processing toot: {} (media: {})",
                    toot.id,
                    toot.media_attachments.len()
                );

                // Process the toot
                match self.process_toot(&toot).await {
                    Ok(()) => {
                        self.mark_as_processed(toot.id.clone());
                        info!("✓ Successfully processed toot: {}", toot.id);
                    }
                    Err(e) => {
                        // Log error but continue processing other toots
                        error!("Failed to process toot {}: {}", toot.id, e);

                        // Still mark as processed to avoid retry loops for non-recoverable errors
                        self.mark_as_processed(toot.id.clone());

                        // Return error for recoverable issues that should be handled at higher level
                        match &e {
                            AlternatorError::Mastodon(MastodonError::RateLimitExceeded {
                                ..
                            })
                            | AlternatorError::OpenRouter(
                                crate::error::OpenRouterError::RateLimitExceeded { .. },
                            ) => {
                                return Err(e);
                            }
                            _ => {
                                // For other errors, log and continue
                                warn!(
                                    "Non-recoverable error processing toot {}, continuing: {}",
                                    toot.id, e
                                );
                            }
                        }
                    }
                }
            }
            Ok(None) => {
                // No toot received, continue listening
                debug!("No toot received, continuing to listen");
            }
            Err(e) => {
                error!("Error listening for toots: {}", e);
                return Err(AlternatorError::Mastodon(e));
            }
        }

        Ok(())
    }

    /// Process a single toot - check for media, generate descriptions, and update
    async fn process_toot(&self, toot: &TootEvent) -> Result<(), AlternatorError> {
        // Check if toot has media attachments
        if toot.media_attachments.is_empty() {
            debug!("Toot {} has no media attachments, skipping", toot.id);
            return Ok(());
        }

        // Filter media that needs processing
        let processable_media = self
            .media_processor
            .filter_processable_media(&toot.media_attachments);

        if processable_media.is_empty() {
            debug!(
                "Toot {} has no processable media (all have descriptions or unsupported types)",
                toot.id
            );
            return Ok(());
        }

        info!(
            "Found {} processable media attachments in toot {}",
            processable_media.len(),
            toot.id
        );

        // Detect language for prompt selection
        let detected_language = self.detect_toot_language(&toot.content)?;
        let prompt_template = self
            .language_detector
            .get_prompt_template(&detected_language)
            .map_err(AlternatorError::Language)?;

        debug!(
            "Using language '{}' with prompt template",
            detected_language
        );

        // Process each media attachment
        for media in processable_media {
            info!(
                "Processing media attachment: {} ({})",
                media.id, media.media_type
            );

            // Check for race conditions before processing
            if let Err(e) = self.check_race_condition(&toot.id, &media.id).await {
                match e {
                    AlternatorError::Mastodon(MastodonError::RaceConditionDetected) => {
                        info!("Race condition detected for media {}, skipping", media.id);
                        continue;
                    }
                    _ => {
                        warn!(
                            "Could not check race condition for media {}: {}",
                            media.id, e
                        );
                        // Continue processing but log the warning
                    }
                }
            }

            // Download and process media
            let processed_media_data =
                match self.media_processor.process_media_for_analysis(media).await {
                    Ok(data) => data,
                    Err(e) => {
                        error!("Failed to process media {}: {}", media.id, e);
                        continue; // Skip this media but continue with others
                    }
                };

            // Generate description using OpenRouter
            let description = match self
                .openrouter_client
                .describe_image(&processed_media_data, prompt_template)
                .await
            {
                Ok(desc) => desc,
                Err(crate::error::OpenRouterError::TokenLimitExceeded { .. }) => {
                    warn!("Token limit exceeded for media {}, skipping", media.id);
                    continue; // Skip this media but continue with others
                }
                Err(e) => {
                    error!(
                        "Failed to generate description for media {}: {}",
                        media.id, e
                    );
                    return Err(AlternatorError::OpenRouter(e));
                }
            };

            info!(
                "Generated description for media {}: {}",
                media.id, description
            );

            // Update media description with final race condition check
            match self
                .update_media_with_race_check(&toot.id, &media.id, &description)
                .await
            {
                Ok(()) => {
                    info!("✓ Updated description for media: {}", media.id);
                }
                Err(AlternatorError::Mastodon(MastodonError::RaceConditionDetected)) => {
                    info!(
                        "Race condition detected when updating media {}, skipping",
                        media.id
                    );
                    continue;
                }
                Err(e) => {
                    error!("Failed to update media description for {}: {}", media.id, e);
                    return Err(e);
                }
            }
        }

        Ok(())
    }

    /// Detect the language of a toot with fallback handling
    fn detect_toot_language(&self, content: &str) -> Result<String, AlternatorError> {
        match self.language_detector.detect_language(content) {
            Ok(lang) => Ok(lang),
            Err(e) => {
                warn!("Language detection failed: {}, defaulting to English", e);
                Ok("en".to_string())
            }
        }
    }

    /// Check for race conditions by retrieving current toot state
    async fn check_race_condition(
        &self,
        toot_id: &str,
        media_id: &str,
    ) -> Result<(), AlternatorError> {
        debug!(
            "Checking for race conditions on toot {} media {}",
            toot_id, media_id
        );

        match self.mastodon_client.get_toot(toot_id).await {
            Ok(current_toot) => {
                // Find the current state of this media attachment
                if let Some(current_media) = current_toot
                    .media_attachments
                    .iter()
                    .find(|m| m.id == *media_id)
                {
                    if current_media.description.is_some()
                        && !current_media
                            .description
                            .as_ref()
                            .unwrap()
                            .trim()
                            .is_empty()
                    {
                        debug!(
                            "Media {} already has description, race condition detected",
                            media_id
                        );
                        return Err(AlternatorError::Mastodon(
                            MastodonError::RaceConditionDetected,
                        ));
                    }
                }
                Ok(())
            }
            Err(e) => {
                warn!(
                    "Could not retrieve current toot state for race condition check: {}",
                    e
                );
                Err(AlternatorError::Mastodon(e))
            }
        }
    }

    /// Update media description with a final race condition check
    async fn update_media_with_race_check(
        &self,
        toot_id: &str,
        media_id: &str,
        description: &str,
    ) -> Result<(), AlternatorError> {
        // Final race condition check before update
        self.check_race_condition(toot_id, media_id).await?;

        // Update media description
        self.mastodon_client
            .update_media(media_id, description)
            .await
            .map_err(AlternatorError::Mastodon)
    }

    /// Check if a toot has already been processed
    fn is_already_processed(&self, toot_id: &str) -> bool {
        self.processed_toots.contains(toot_id)
    }

    /// Mark a toot as processed to prevent duplicate processing
    fn mark_as_processed(&mut self, toot_id: String) {
        self.processed_toots.insert(toot_id);

        // Prevent memory growth by limiting the size of processed toots set
        if self.processed_toots.len() > 10000 {
            // Remove oldest entries (this is a simple approach, could be improved with LRU)
            let excess = self.processed_toots.len() - 5000;
            let to_remove: Vec<String> =
                self.processed_toots.iter().take(excess).cloned().collect();
            for id in to_remove {
                self.processed_toots.remove(&id);
            }
            debug!(
                "Cleaned up processed toots cache, now contains {} entries",
                self.processed_toots.len()
            );
        }
    }

    /// Get statistics about processed toots
    pub fn get_processing_stats(&self) -> ProcessingStats {
        ProcessingStats {
            processed_toots_count: self.processed_toots.len(),
        }
    }

    /// Clear the processed toots cache (useful for testing)
    #[cfg(test)]
    pub fn clear_processed_cache(&mut self) {
        self.processed_toots.clear();
    }
}

/// Statistics about toot processing
#[derive(Debug, Clone)]
pub struct ProcessingStats {
    pub processed_toots_count: usize,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{MastodonConfig, OpenRouterConfig};
    use crate::mastodon::{Account, MediaAttachment};
    use chrono::Utc;

    fn create_test_mastodon_config() -> MastodonConfig {
        MastodonConfig {
            instance_url: "https://mastodon.social".to_string(),
            access_token: "test_token".to_string(),
            user_stream: Some(true),
        }
    }

    fn create_test_openrouter_config() -> OpenRouterConfig {
        OpenRouterConfig {
            api_key: "test_key".to_string(),
            model: "anthropic/claude-3-haiku".to_string(),
            base_url: Some("https://test.openrouter.ai/api/v1".to_string()),
            max_tokens: Some(150),
        }
    }

    fn create_test_toot(id: &str, media_attachments: Vec<MediaAttachment>) -> TootEvent {
        TootEvent {
            id: id.to_string(),
            account: Account {
                id: "user123".to_string(),
                username: "testuser".to_string(),
                acct: "testuser@mastodon.social".to_string(),
                display_name: "Test User".to_string(),
                url: "https://mastodon.social/@testuser".to_string(),
            },
            content: "Test toot with image".to_string(),
            language: Some("en".to_string()),
            media_attachments,
            created_at: Utc::now(),
            url: Some("https://mastodon.social/@testuser/123456789".to_string()),
            visibility: "public".to_string(),
        }
    }

    fn create_test_media(
        id: &str,
        media_type: &str,
        description: Option<String>,
    ) -> MediaAttachment {
        MediaAttachment {
            id: id.to_string(),
            media_type: media_type.to_string(),
            url: format!("https://example.com/media/{}", id),
            preview_url: None,
            description,
            meta: None,
        }
    }

    #[test]
    fn test_toot_stream_handler_creation() {
        let mastodon_client = MastodonClient::new(create_test_mastodon_config());
        let openrouter_client = OpenRouterClient::new(create_test_openrouter_config());
        let media_processor = MediaProcessor::with_default_config();
        let language_detector = LanguageDetector::new();

        let handler = TootStreamHandler::new(
            mastodon_client,
            openrouter_client,
            media_processor,
            language_detector,
        );

        assert_eq!(handler.processed_toots.len(), 0);
    }

    #[test]
    fn test_duplicate_processing_prevention() {
        let mastodon_client = MastodonClient::new(create_test_mastodon_config());
        let openrouter_client = OpenRouterClient::new(create_test_openrouter_config());
        let media_processor = MediaProcessor::with_default_config();
        let language_detector = LanguageDetector::new();

        let mut handler = TootStreamHandler::new(
            mastodon_client,
            openrouter_client,
            media_processor,
            language_detector,
        );

        // Initially not processed
        assert!(!handler.is_already_processed("toot123"));

        // Mark as processed
        handler.mark_as_processed("toot123".to_string());
        assert!(handler.is_already_processed("toot123"));

        // Different toot should not be marked
        assert!(!handler.is_already_processed("toot456"));
    }

    #[test]
    fn test_processed_cache_cleanup() {
        let mastodon_client = MastodonClient::new(create_test_mastodon_config());
        let openrouter_client = OpenRouterClient::new(create_test_openrouter_config());
        let media_processor = MediaProcessor::with_default_config();
        let language_detector = LanguageDetector::new();

        let mut handler = TootStreamHandler::new(
            mastodon_client,
            openrouter_client,
            media_processor,
            language_detector,
        );

        // Add many processed toots to trigger cleanup
        for i in 0..10001 {
            handler.mark_as_processed(format!("toot{}", i));
        }

        // Should have triggered cleanup
        assert!(handler.processed_toots.len() <= 5000);
    }

    #[test]
    fn test_language_detection_fallback() {
        let mastodon_client = MastodonClient::new(create_test_mastodon_config());
        let openrouter_client = OpenRouterClient::new(create_test_openrouter_config());
        let media_processor = MediaProcessor::with_default_config();
        let language_detector = LanguageDetector::new();

        let handler = TootStreamHandler::new(
            mastodon_client,
            openrouter_client,
            media_processor,
            language_detector,
        );

        // Test with English text
        let result = handler
            .detect_toot_language("Hello world, this is a test")
            .unwrap();
        assert_eq!(result, "en");

        // Test with empty text (should fallback to English)
        let result = handler.detect_toot_language("").unwrap();
        assert_eq!(result, "en");

        // Test with German text
        let result = handler
            .detect_toot_language("Das ist ein Test mit deutschen Wörtern")
            .unwrap();
        assert_eq!(result, "de");
    }

    #[test]
    fn test_processing_stats() {
        let mastodon_client = MastodonClient::new(create_test_mastodon_config());
        let openrouter_client = OpenRouterClient::new(create_test_openrouter_config());
        let media_processor = MediaProcessor::with_default_config();
        let language_detector = LanguageDetector::new();

        let mut handler = TootStreamHandler::new(
            mastodon_client,
            openrouter_client,
            media_processor,
            language_detector,
        );

        let stats = handler.get_processing_stats();
        assert_eq!(stats.processed_toots_count, 0);

        handler.mark_as_processed("toot1".to_string());
        handler.mark_as_processed("toot2".to_string());

        let stats = handler.get_processing_stats();
        assert_eq!(stats.processed_toots_count, 2);
    }

    #[test]
    fn test_clear_processed_cache() {
        let mastodon_client = MastodonClient::new(create_test_mastodon_config());
        let openrouter_client = OpenRouterClient::new(create_test_openrouter_config());
        let media_processor = MediaProcessor::with_default_config();
        let language_detector = LanguageDetector::new();

        let mut handler = TootStreamHandler::new(
            mastodon_client,
            openrouter_client,
            media_processor,
            language_detector,
        );

        handler.mark_as_processed("toot1".to_string());
        handler.mark_as_processed("toot2".to_string());
        assert_eq!(handler.processed_toots.len(), 2);

        handler.clear_processed_cache();
        assert_eq!(handler.processed_toots.len(), 0);
    }

    #[test]
    fn test_processing_stats_debug() {
        let stats = ProcessingStats {
            processed_toots_count: 42,
        };

        let debug_str = format!("{:?}", stats);
        assert!(debug_str.contains("processed_toots_count: 42"));
    }
}
