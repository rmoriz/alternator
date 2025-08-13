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
        let detected_language = self.detect_toot_language(toot)?;
        let prompt_template = self
            .language_detector
            .get_prompt_template(&detected_language)
            .map_err(AlternatorError::Language)?;

        debug!(
            "Using language '{}' with prompt template",
            detected_language
        );

        // Process each media attachment and collect successful descriptions with image data
        let mut media_recreations = Vec::new();
        let mut original_media_ids = Vec::new();

        // First pass: Prepare all media for processing (downloads and preprocessing)
        let mut prepared_media = Vec::new();

        for media in processable_media {
            info!(
                "Preparing media attachment: {} ({})",
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

            // Download original image data for recreation
            let original_image_data = match self
                .media_processor
                .download_media_for_recreation(media)
                .await
            {
                Ok(data) => data,
                Err(e) => {
                    error!(
                        "Failed to download media {} for recreation: {}",
                        media.id, e
                    );
                    continue; // Skip this media but continue with others
                }
            };

            // Process media for analysis (resized/optimized version)
            let processed_media_data =
                match self.media_processor.process_media_for_analysis(media).await {
                    Ok(data) => data,
                    Err(e) => {
                        error!("Failed to process media {} for analysis: {}", media.id, e);
                        continue; // Skip this media but continue with others
                    }
                };

            prepared_media.push((media.id.clone(), original_image_data, processed_media_data));
        }

        if prepared_media.is_empty() {
            debug!("No media could be prepared for processing");
            return Ok(());
        }

        info!(
            "Prepared {} media attachments, starting parallel description generation",
            prepared_media.len()
        );

        // Second pass: Generate descriptions in parallel using OpenRouter
        let description_tasks: Vec<_> = prepared_media
            .iter()
            .map(|(media_id, _original_data, processed_data)| {
                let openrouter_client = &self.openrouter_client;
                let media_id = media_id.clone();
                async move {
                    let result = openrouter_client
                        .describe_image(processed_data, prompt_template)
                        .await;
                    (media_id, result)
                }
            })
            .collect();

        let description_results = futures_util::future::join_all(description_tasks).await;

        // Process results and build media recreations
        for ((media_id, original_data, _processed_data), (result_media_id, description_result)) in
            prepared_media
                .into_iter()
                .zip(description_results.into_iter())
        {
            debug_assert_eq!(
                media_id, result_media_id,
                "Media ID mismatch in parallel processing"
            );

            match description_result {
                Ok(description) => {
                    info!(
                        "Generated description for media {}: {}",
                        media_id, description
                    );
                    // Add to media recreations for batch processing
                    media_recreations.push((original_data, description));
                    // Track original media ID for cleanup
                    original_media_ids.push(media_id);
                }
                Err(crate::error::OpenRouterError::TokenLimitExceeded { .. }) => {
                    warn!("Token limit exceeded for media {}, skipping", media_id);
                    continue; // Skip this media but continue with others
                }
                Err(e) => {
                    error!(
                        "Failed to generate description for media {}: {}",
                        media_id, e
                    );
                    return Err(AlternatorError::OpenRouter(e));
                }
            }
        }

        // If we have media to recreate, do a batch recreation
        if !media_recreations.is_empty() {
            info!(
                "Recreating {} media attachments with descriptions for toot {}",
                media_recreations.len(),
                toot.id
            );

            // Final race condition check and batch recreation
            match self
                .recreate_media_with_race_check(
                    &toot.id,
                    media_recreations.clone(),
                    original_media_ids.clone(),
                )
                .await
            {
                Ok(()) => {
                    info!(
                        "✓ Successfully recreated {} media attachments for toot: {}",
                        media_recreations.len(),
                        toot.id
                    );
                }
                Err(AlternatorError::Mastodon(MastodonError::RaceConditionDetected)) => {
                    info!(
                        "Race condition detected during media recreation for toot {}, operation aborted",
                        toot.id
                    );
                }
                Err(e) => {
                    error!(
                        "Failed to recreate media attachments for toot {}: {}",
                        toot.id, e
                    );
                    return Err(e);
                }
            }
        } else {
            info!("No media attachments to recreate for toot {}", toot.id);
        }

        Ok(())
    }

    /// Detect the language of a toot with fallback handling
    #[allow(clippy::result_large_err)] // AlternatorError is large but needed for comprehensive error handling
    fn detect_toot_language(&self, toot: &TootEvent) -> Result<String, AlternatorError> {
        // First, check if the toot has a language attribute
        if let Some(ref lang) = toot.language {
            if !lang.trim().is_empty() {
                debug!("Using toot language attribute: {}", lang);
                return Ok(lang.clone());
            }
        }

        // Fallback to content-based language detection
        debug!("No toot language attribute found, detecting from content");
        match self.language_detector.detect_language(&toot.content) {
            Ok(lang) => {
                debug!("Detected language from content: {}", lang);
                Ok(lang)
            }
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
                } else {
                    // Media attachment not found in current toot state
                    debug!(
                        "Media {} no longer exists in toot {}, race condition detected",
                        media_id, toot_id
                    );
                    return Err(AlternatorError::Mastodon(
                        MastodonError::RaceConditionDetected,
                    ));
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
    #[allow(dead_code)] // Kept for backward compatibility, replaced by batch update
    async fn update_media_with_race_check(
        &self,
        toot_id: &str,
        media_id: &str,
        description: &str,
    ) -> Result<(), AlternatorError> {
        // Final race condition check before update
        self.check_race_condition(toot_id, media_id).await?;

        // Update media description
        match self
            .mastodon_client
            .update_media(toot_id, media_id, description)
            .await
        {
            Ok(()) => Ok(()),
            Err(MastodonError::MediaNotFound { .. }) => {
                // Treat MediaNotFound as a race condition - the media was removed/changed
                debug!(
                    "Media {} not found during update, treating as race condition",
                    media_id
                );
                Err(AlternatorError::Mastodon(
                    MastodonError::RaceConditionDetected,
                ))
            }
            Err(e) => Err(AlternatorError::Mastodon(e)),
        }
    }

    /// Recreate media attachments with descriptions and race condition checks
    async fn recreate_media_with_race_check(
        &self,
        toot_id: &str,
        media_recreations: Vec<(Vec<u8>, String)>, // Vec of (image_data, description)
        original_media_ids: Vec<String>,           // Original media IDs to clean up after success
    ) -> Result<(), AlternatorError> {
        if media_recreations.is_empty() {
            return Ok(());
        }

        // Get current toot state to verify no race conditions
        let current_toot = self
            .mastodon_client
            .get_toot(toot_id)
            .await
            .map_err(AlternatorError::Mastodon)?;

        // Check if any of the original media attachments now have descriptions
        let processable_media = self
            .media_processor
            .filter_processable_media(&current_toot.media_attachments);

        if processable_media.len() != media_recreations.len() {
            debug!(
                "Media state changed: expected {} processable media, found {}. Race condition detected.",
                media_recreations.len(),
                processable_media.len()
            );
            return Err(AlternatorError::Mastodon(
                MastodonError::RaceConditionDetected,
            ));
        }

        // Recreate all media attachments with descriptions (includes cleanup)
        match self
            .mastodon_client
            .recreate_media_with_descriptions(toot_id, media_recreations, original_media_ids)
            .await
        {
            Ok(()) => Ok(()),
            Err(e) => Err(AlternatorError::Mastodon(e)),
        }
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
    #[allow(dead_code)] // Public API method, may be used in future
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
#[allow(dead_code)] // Stats struct for API completeness
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
            model: "mistralai/mistral-small-3.2-24b-instruct:free".to_string(),
            base_url: Some("https://test.openrouter.ai/api/v1".to_string()),
            max_tokens: Some(150),
        }
    }

    #[allow(dead_code)] // Test helper function
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

    #[allow(dead_code)] // Test helper function
    fn create_test_media(
        id: &str,
        media_type: &str,
        description: Option<String>,
    ) -> MediaAttachment {
        MediaAttachment {
            id: id.to_string(),
            media_type: media_type.to_string(),
            url: format!("https://example.com/media/{id}"),
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
            handler.mark_as_processed(format!("toot{i}"));
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

        // Test with toot that has language attribute
        let toot_with_lang = create_test_toot("123", vec![]);
        let mut toot_with_lang = toot_with_lang;
        toot_with_lang.language = Some("de".to_string());

        let result = handler.detect_toot_language(&toot_with_lang).unwrap();
        assert_eq!(result, "de");

        // Test with toot that has empty language attribute (should fallback to content detection)
        let mut toot_empty_lang = create_test_toot("456", vec![]);
        toot_empty_lang.language = Some("".to_string());
        toot_empty_lang.content = "Hello world, this is a test".to_string();

        let result = handler.detect_toot_language(&toot_empty_lang).unwrap();
        assert_eq!(result, "en");

        // Test with toot without language attribute (should fallback to content detection)
        let mut toot_no_lang = create_test_toot("789", vec![]);
        toot_no_lang.language = None;
        toot_no_lang.content = "Das ist ein Test mit deutschen Wörtern".to_string();

        let result = handler.detect_toot_language(&toot_no_lang).unwrap();
        assert_eq!(result, "de");

        // Test with toot without language and empty content (should fallback to English)
        let mut toot_empty = create_test_toot("000", vec![]);
        toot_empty.language = None;
        toot_empty.content = "".to_string();

        let result = handler.detect_toot_language(&toot_empty).unwrap();
        assert_eq!(result, "en");
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

        let debug_str = format!("{stats:?}");
        assert!(debug_str.contains("processed_toots_count: 42"));
    }
}
