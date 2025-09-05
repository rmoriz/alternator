use crate::config::RuntimeConfig;
use crate::error::{AlternatorError, MastodonError};
use crate::language::LanguageDetector;
use crate::mastodon::{MastodonClient, MastodonStream, TootEvent};
use crate::media::MediaProcessor;
use crate::openrouter::OpenRouterClient;
use crate::toot_handler::processor;
use crate::toot_handler::stats::ProcessingStats;
use lru::LruCache;
use std::num::NonZeroUsize;
use tracing::{debug, error, info, warn};

/// Handler for processing incoming toot events from WebSocket stream
pub struct TootStreamHandler {
    mastodon_client: MastodonClient,
    openrouter_client: OpenRouterClient,
    media_processor: MediaProcessor,
    language_detector: LanguageDetector,
    processed_toots: LruCache<String, ()>,
    processed_edits: LruCache<String, ()>,
    config: RuntimeConfig,
}

impl TootStreamHandler {
    /// Create a new toot stream handler
    pub fn new(
        mastodon_client: MastodonClient,
        openrouter_client: OpenRouterClient,
        media_processor: MediaProcessor,
        language_detector: LanguageDetector,
        config: RuntimeConfig,
    ) -> Self {
        // Use LRU cache with capacity of 5000 entries to prevent memory leaks
        let capacity = NonZeroUsize::new(5000).unwrap();

        Self {
            mastodon_client,
            openrouter_client,
            media_processor,
            language_detector,
            processed_toots: LruCache::new(capacity),
            processed_edits: LruCache::new(capacity),
            config,
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
                        AlternatorError::Mastodon(
                            MastodonError::Disconnected(_) | MastodonError::ConnectionFailed(_),
                        ) => {
                            warn!("Connection lost, will attempt to reconnect");
                            // The MastodonClient will handle reconnection automatically
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

                if toot.is_edit {
                    // Handle edit events with content-aware deduplication
                    if self.is_edit_already_processed(&toot) {
                        debug!(
                            "Skipping already processed edit: {} (media: {})",
                            toot.id,
                            toot.media_attachments.len()
                        );
                        return Ok(());
                    }

                    info!(
                        "Processing edited toot: {} (media: {})",
                        toot.id,
                        toot.media_attachments.len()
                    );

                    // Process the edited toot
                    match processor::process_edited_toot(
                        &toot,
                        &self.mastodon_client,
                        &self.openrouter_client,
                        &self.media_processor,
                        &self.language_detector,
                        &self.config,
                    )
                    .await
                    {
                        Ok(()) => {
                            self.mark_edit_as_processed(&toot);
                            info!("✓ Successfully processed edited toot: {}", toot.id);
                        }
                        Err(e) => {
                            // Log error but continue processing other toots
                            error!("Failed to process edited toot {}: {}", toot.id, e);

                            // Still mark as processed to avoid retry loops for non-recoverable errors
                            self.mark_edit_as_processed(&toot);

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
                                        "Non-recoverable error processing edited toot {}, continuing: {}",
                                        toot.id, e
                                    );
                                }
                            }
                        }
                    }
                } else {
                    // Handle new toot events with existing logic
                    if self.is_already_processed(toot.id.as_str()) {
                        debug!("Skipping already processed toot: {}", toot.id);
                        return Ok(());
                    }

                    info!(
                        "Processing toot: {} (media: {})",
                        toot.id,
                        toot.media_attachments.len()
                    );

                    // Process the toot
                    match processor::process_toot(
                        &toot,
                        &self.mastodon_client,
                        &self.openrouter_client,
                        &self.media_processor,
                        &self.language_detector,
                        &self.config,
                    )
                    .await
                    {
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

    /// Check if a toot has already been processed
    fn is_already_processed(&mut self, toot_id: &str) -> bool {
        self.processed_toots.get(toot_id).is_some()
    }

    /// Mark a toot as processed to prevent duplicate processing
    fn mark_as_processed(&mut self, toot_id: String) {
        // LRU cache automatically manages size and evicts least recently used entries
        self.processed_toots.put(toot_id, ());
    }

    /// Check if an edit has already been processed
    fn is_edit_already_processed(&mut self, toot: &TootEvent) -> bool {
        let edit_key = self.generate_edit_key(toot);
        self.processed_edits.get(&edit_key).is_some()
    }

    /// Mark an edit as processed to prevent duplicate processing
    fn mark_edit_as_processed(&mut self, toot: &TootEvent) {
        let edit_key = self.generate_edit_key(toot);
        // LRU cache automatically manages size and evicts least recently used entries
        self.processed_edits.put(edit_key, ());
    }

    /// Generate a unique key for an edit based on toot ID and media attachment IDs
    /// This ensures that adding new media to an existing toot will be processed
    fn generate_edit_key(&self, toot: &TootEvent) -> String {
        let mut media_ids: Vec<String> = toot
            .media_attachments
            .iter()
            .map(|m| m.id.clone())
            .collect();
        media_ids.sort(); // Ensure consistent ordering
        format!("{}:{}", toot.id, media_ids.join(","))
    }

    /// Get statistics about processed toots
    #[allow(dead_code)] // Public API method, may be used in future
    pub fn get_processing_stats(&self) -> ProcessingStats {
        ProcessingStats {
            processed_toots_count: self.processed_toots.len(),
        }
    }
}
