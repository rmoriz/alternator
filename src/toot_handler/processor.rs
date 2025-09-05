use crate::config::RuntimeConfig;
use crate::error::AlternatorError;
use crate::language::LanguageDetector;
use crate::mastodon::{MastodonClient, MediaAttachment, MediaRecreation, TootEvent};
use crate::media::MediaProcessor;
use crate::openrouter::OpenRouterClient;
use tracing::{debug, error, info, warn};

/// Strategy pattern for processing different media types
#[async_trait::async_trait]
trait MediaProcessingStrategy: Send + Sync {
    /// Check if this strategy can handle the given media type
    fn can_handle(&self, media_type: &str) -> bool;

    /// Process the media and return a MediaRecreation if successful
    async fn process_media(
        &self,
        media: &MediaAttachment,
        media_processor: &MediaProcessor,
        config: &RuntimeConfig,
    ) -> Result<Option<MediaRecreation>, AlternatorError>;
}

/// Strategy for processing audio files
struct AudioProcessingStrategy;

#[async_trait::async_trait]
impl MediaProcessingStrategy for AudioProcessingStrategy {
    fn can_handle(&self, media_type: &str) -> bool {
        media_type.to_lowercase().starts_with("audio")
    }

    async fn process_media(
        &self,
        media: &MediaAttachment,
        media_processor: &MediaProcessor,
        config: &RuntimeConfig,
    ) -> Result<Option<MediaRecreation>, AlternatorError> {
        // Check if audio processing is enabled
        if !config.is_audio_enabled() {
            debug!(
                "Audio processing disabled, skipping audio file: {} ({})",
                media.id, media.media_type
            );
            return Ok(None);
        }

        info!("Processing audio file: {} ({})", media.id, media.media_type);

        // Download original audio data for recreation
        let original_audio_data = media_processor
            .download_media_for_recreation(media)
            .await
            .map_err(|e| {
                error!(
                    "Failed to download audio {} for recreation: {}",
                    media.id, e
                );
                e
            })?;

        // Transcribe the audio to get description
        let transcript = match crate::media::process_audio_for_transcript(
            media,
            config.config().whisper(),
            config.config().media(),
            Some(&config.config().openrouter),
        )
        .await
        {
            Ok(transcript) => {
                info!(
                    "Generated transcript for audio {}: {}",
                    media.id, transcript
                );
                transcript
            }
            Err(crate::error::MediaError::UnsupportedType { .. }) => {
                warn!(
                    "Audio type {} not supported for transcription, skipping",
                    media.media_type
                );
                return Ok(None);
            }
            Err(e) => {
                error!("Failed to transcribe audio {}: {}", media.id, e);
                return Err(AlternatorError::Media(e));
            }
        };

        // Determine appropriate file extension for audio
        let extension = get_audio_file_extension(&media.media_type);
        let filename = format!("audio_{}.{}", media.id, extension);

        Ok(Some(MediaRecreation {
            data: original_audio_data,
            description: transcript,
            media_type: media.media_type.clone(),
            filename,
        }))
    }
}

/// Strategy for processing video files
struct VideoProcessingStrategy;

#[async_trait::async_trait]
impl MediaProcessingStrategy for VideoProcessingStrategy {
    fn can_handle(&self, media_type: &str) -> bool {
        media_type.to_lowercase().starts_with("video")
    }

    async fn process_media(
        &self,
        media: &MediaAttachment,
        media_processor: &MediaProcessor,
        config: &RuntimeConfig,
    ) -> Result<Option<MediaRecreation>, AlternatorError> {
        debug!(
            "Detected video media: '{}' (type: '{}')",
            media.id, media.media_type
        );

        // Check if audio processing is enabled (required for video transcription)
        if !config.is_audio_enabled() {
            debug!(
                "Audio processing disabled, skipping video file: {} ({})",
                media.id, media.media_type
            );
            return Ok(None);
        }

        info!("Processing video file: {} ({})", media.id, media.media_type);

        // Download original video data for recreation
        debug!(
            "About to download video data for media: {} with type: '{}'",
            media.id, media.media_type
        );
        let original_video_data = media_processor
            .download_media_for_recreation(media)
            .await
            .map_err(|e| {
                error!(
                    "Failed to download video {} for recreation: {}",
                    media.id, e
                );
                e
            })?;

        // Transcribe the video audio to get description
        let transcript = match crate::media::process_video_for_transcript(
            media,
            config.config().whisper(),
            config.config().media(),
            Some(&config.config().openrouter),
        )
        .await
        {
            Ok(transcript) => {
                info!(
                    "Generated transcript for video {}: {}",
                    media.id, transcript
                );
                transcript
            }
            Err(crate::error::MediaError::UnsupportedType { .. }) => {
                warn!(
                    "Video type {} not supported for transcription, skipping",
                    media.media_type
                );
                return Ok(None);
            }
            Err(e) => {
                error!("Failed to transcribe video {}: {}", media.id, e);
                return Err(AlternatorError::Media(e));
            }
        };

        // Determine appropriate file extension for video
        let extension = get_video_file_extension(&media.media_type);
        let filename = format!("video_{}.{}", media.id, extension);

        Ok(Some(MediaRecreation {
            data: original_video_data,
            description: transcript,
            media_type: media.media_type.clone(),
            filename,
        }))
    }
}

/// Strategy for processing image files
struct ImageProcessingStrategy;

#[async_trait::async_trait]
impl MediaProcessingStrategy for ImageProcessingStrategy {
    fn can_handle(&self, media_type: &str) -> bool {
        media_type.to_lowercase().starts_with("image")
    }

    async fn process_media(
        &self,
        _media: &MediaAttachment,
        _media_processor: &MediaProcessor,
        _config: &RuntimeConfig,
    ) -> Result<Option<MediaRecreation>, AlternatorError> {
        // Images are handled separately in the main processing loop
        // due to the need for parallel processing
        Ok(None)
    }
}

/// Get appropriate file extension for audio media type
fn get_audio_file_extension(media_type: &str) -> &'static str {
    match media_type {
        "audio/mpeg" | "audio/mp3" => "mp3",
        "audio/wav" | "audio/wave" | "audio/x-wav" => "wav",
        "audio/m4a" => "m4a",
        "audio/mp4" => "mp4",
        "audio/aac" => "aac",
        "audio/ogg" => "ogg",
        "audio/flac" | "audio/x-flac" => "flac",
        _ => "audio", // fallback
    }
}

/// Get appropriate file extension for video media type
fn get_video_file_extension(media_type: &str) -> &'static str {
    match media_type {
        "video/mp4" => "mp4",
        "video/mpeg" => "mpeg",
        "video/quicktime" => "mov",
        "video/x-msvideo" => "avi",
        "video/webm" => "webm",
        "video/x-ms-wmv" => "wmv",
        "video/x-flv" => "flv",
        "video/3gpp" => "3gp",
        "video/x-matroska" => "mkv",
        _ => "video", // fallback
    }
}

/// Get appropriate file extension for image media type
fn get_image_file_extension(media_type: &str) -> &'static str {
    match media_type {
        "image/jpeg" => "jpg",
        "image/png" => "png",
        "image/gif" => "gif",
        "image/webp" => "webp",
        "image/bmp" => "bmp",
        "image/tiff" => "tiff",
        _ => "jpg", // fallback to jpg for unknown image types
    }
}

/// Process a single toot - check for media, generate descriptions, and update
pub async fn process_toot(
    toot: &TootEvent,
    mastodon_client: &MastodonClient,
    openrouter_client: &OpenRouterClient,
    media_processor: &MediaProcessor,
    language_detector: &LanguageDetector,
    config: &RuntimeConfig,
) -> Result<(), AlternatorError> {
    process_toot_internal(
        toot,
        mastodon_client,
        openrouter_client,
        media_processor,
        language_detector,
        config,
        false,
    )
    .await
}

/// Process an edited toot - focus on new/changed media without descriptions
pub async fn process_edited_toot(
    toot: &TootEvent,
    mastodon_client: &MastodonClient,
    openrouter_client: &OpenRouterClient,
    media_processor: &MediaProcessor,
    language_detector: &LanguageDetector,
    config: &RuntimeConfig,
) -> Result<(), AlternatorError> {
    info!(
        "Processing edited toot {} - checking for new media without descriptions",
        toot.id
    );
    process_toot_internal(
        toot,
        mastodon_client,
        openrouter_client,
        media_processor,
        language_detector,
        config,
        true,
    )
    .await
}

/// Internal implementation for processing toots
async fn process_toot_internal(
    toot: &TootEvent,
    mastodon_client: &MastodonClient,
    openrouter_client: &OpenRouterClient,
    media_processor: &MediaProcessor,
    language_detector: &LanguageDetector,
    config: &RuntimeConfig,
    is_edit: bool,
) -> Result<(), AlternatorError> {
    // Early return if no media attachments
    if toot.media_attachments.is_empty() {
        debug!(
            "{} {} has no media attachments, skipping",
            if is_edit { "Edit" } else { "Toot" },
            toot.id
        );
        return Ok(());
    }

    // Filter media that needs processing
    let processable_media = media_processor
        .filter_processable_media_with_audio(&toot.media_attachments, config.is_audio_enabled());

    if processable_media.is_empty() {
        debug!(
            "{} {} has no processable media (all have descriptions or unsupported types)",
            if is_edit { "Edit" } else { "Toot" },
            toot.id
        );
        return Ok(());
    }

    info!(
        "Found {} processable media attachments in {} {}",
        processable_media.len(),
        if is_edit { "edit" } else { "toot" },
        toot.id
    );

    // Detect language for prompt selection
    let detected_language = detect_toot_language(toot, language_detector)?;
    let prompt_template = language_detector
        .get_prompt_template(&detected_language)
        .map_err(AlternatorError::Language)?;

    debug!(
        "Using language '{}' with prompt template",
        detected_language
    );

    // Process all media using strategies
    let media_processing_result = process_media_attachments(
        &processable_media,
        mastodon_client,
        openrouter_client,
        media_processor,
        prompt_template,
        config,
        &toot.id,
    )
    .await?;

    // Recreate media if we have any successful processing results
    if !media_processing_result.media_recreations.is_empty() {
        recreate_media_attachments(
            mastodon_client,
            &toot.id,
            media_processing_result.media_recreations,
            media_processing_result.original_media_ids,
            is_edit,
        )
        .await?;
    } else {
        info!(
            "No media attachments to recreate for {} {}",
            if is_edit { "edit" } else { "toot" },
            toot.id
        );
    }

    Ok(())
}

/// Result of processing media attachments
struct MediaProcessingResult {
    media_recreations: Vec<MediaRecreation>,
    original_media_ids: Vec<String>,
}

/// Process all media attachments using appropriate strategies
async fn process_media_attachments(
    processable_media: &[&MediaAttachment],
    mastodon_client: &MastodonClient,
    openrouter_client: &OpenRouterClient,
    media_processor: &MediaProcessor,
    prompt_template: &str,
    config: &RuntimeConfig,
    toot_id: &str,
) -> Result<MediaProcessingResult, AlternatorError> {
    let strategies: Vec<Box<dyn MediaProcessingStrategy>> = vec![
        Box::new(AudioProcessingStrategy),
        Box::new(VideoProcessingStrategy),
        Box::new(ImageProcessingStrategy),
    ];

    let mut media_recreations = Vec::new();
    let mut original_media_ids = Vec::new();
    let mut prepared_images = Vec::new();

    for &media in processable_media {
        info!(
            "Preparing media attachment: {} ({})",
            media.id, media.media_type
        );

        // Check for race conditions before processing
        if let Err(e) =
            crate::toot_handler::race::check_race_condition(mastodon_client, toot_id, &media.id)
                .await
        {
            match e {
                AlternatorError::Mastodon(crate::error::MastodonError::RaceConditionDetected) => {
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

        // Find appropriate strategy and process
        let mut processed = false;
        for strategy in &strategies {
            if strategy.can_handle(&media.media_type) {
                match strategy
                    .process_media(media, media_processor, config)
                    .await?
                {
                    Some(media_recreation) => {
                        // Direct recreation (audio/video)
                        media_recreations.push(media_recreation);
                        original_media_ids.push(media.id.clone());
                    }
                    None => {
                        // Handle images separately (they need parallel processing)
                        if media.media_type.to_lowercase().starts_with("image") {
                            // Download original image data for recreation
                            let original_image_data =
                                match media_processor.download_media_for_recreation(media).await {
                                    Ok(data) => data,
                                    Err(e) => {
                                        error!(
                                            "Failed to download media {} for recreation: {}",
                                            media.id, e
                                        );
                                        continue;
                                    }
                                };

                            // Process media for analysis (resized/optimized version)
                            let processed_media_data =
                                match media_processor.process_media_for_analysis(media).await {
                                    Ok(data) => data,
                                    Err(e) => {
                                        error!(
                                            "Failed to process media {} for analysis: {}",
                                            media.id, e
                                        );
                                        continue;
                                    }
                                };

                            prepared_images.push((
                                media.clone(),
                                original_image_data,
                                processed_media_data,
                            ));
                        }
                        // Strategy handled but returned None (e.g., disabled processing)
                    }
                }
                processed = true;
                break;
            }
        }

        if !processed {
            debug!(
                "No strategy found for media type: {} ({})",
                media.id, media.media_type
            );
        }
    }

    // Process images in parallel if any were prepared
    if !prepared_images.is_empty() {
        info!(
            "Prepared {} image attachments, starting parallel description generation",
            prepared_images.len()
        );

        let image_recreations =
            process_images_in_parallel(prepared_images, openrouter_client, prompt_template).await?;

        media_recreations.extend(image_recreations);
    }

    Ok(MediaProcessingResult {
        media_recreations,
        original_media_ids,
    })
}

/// Process images in parallel using OpenRouter
async fn process_images_in_parallel(
    prepared_images: Vec<(MediaAttachment, Vec<u8>, Vec<u8>)>,
    openrouter_client: &OpenRouterClient,
    prompt_template: &str,
) -> Result<Vec<MediaRecreation>, AlternatorError> {
    // Generate descriptions in parallel
    let description_tasks: Vec<_> = prepared_images
        .iter()
        .map(|(media, _original_data, processed_data)| {
            let media_id = media.id.clone();
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
    let mut media_recreations = Vec::new();

    for ((media, original_data, _processed_data), (result_media_id, description_result)) in
        prepared_images
            .into_iter()
            .zip(description_results.into_iter())
    {
        debug_assert_eq!(
            media.id, result_media_id,
            "Media ID mismatch in parallel processing"
        );

        match description_result {
            Ok(description) => {
                info!(
                    "Generated description for media {}: {}",
                    media.id, description
                );

                let extension = get_image_file_extension(&media.media_type);
                let filename = format!("image_{}.{}", media.id, extension);

                media_recreations.push(MediaRecreation {
                    data: original_data,
                    description,
                    media_type: media.media_type.clone(),
                    filename,
                });
            }
            Err(crate::error::OpenRouterError::TokenLimitExceeded { .. }) => {
                warn!("Token limit exceeded for media {}, skipping", media.id);
            }
            Err(e) => {
                error!(
                    "Failed to generate description for media {}: {}",
                    media.id, e
                );
                return Err(AlternatorError::OpenRouter(e));
            }
        }
    }

    Ok(media_recreations)
}

/// Recreate media attachments with descriptions
async fn recreate_media_attachments(
    mastodon_client: &MastodonClient,
    toot_id: &str,
    media_recreations: Vec<MediaRecreation>,
    original_media_ids: Vec<String>,
    is_edit: bool,
) -> Result<(), AlternatorError> {
    info!(
        "Recreating {} media attachments with descriptions for toot {}",
        media_recreations.len(),
        toot_id
    );

    // Final race condition check and batch recreation
    match crate::toot_handler::coordinator::recreate_media_with_race_check(
        mastodon_client,
        toot_id,
        media_recreations.clone(),
        original_media_ids.clone(),
    )
    .await
    {
        Ok(()) => {
            info!(
                "✓ Successfully recreated {} media attachments for {}: {}",
                media_recreations.len(),
                if is_edit { "edit" } else { "toot" },
                toot_id
            );
        }
        Err(AlternatorError::Mastodon(crate::error::MastodonError::RaceConditionDetected)) => {
            info!(
                "Race condition detected during media recreation for {} {}, operation aborted",
                if is_edit { "edit" } else { "toot" },
                toot_id
            );
        }
        Err(e) => {
            error!(
                "Failed to recreate media attachments for {} {}: {}",
                if is_edit { "edit" } else { "toot" },
                toot_id,
                e
            );
            return Err(e);
        }
    }

    Ok(())
}

/// Detect the language of a toot with fallback handling
#[allow(clippy::result_large_err)] // AlternatorError is large but needed for comprehensive error handling
pub fn detect_toot_language(
    toot: &TootEvent,
    language_detector: &LanguageDetector,
) -> Result<String, AlternatorError> {
    // First, check if the toot has a language attribute
    if let Some(ref lang) = toot.language {
        if !lang.trim().is_empty() {
            debug!("Using toot language attribute: {}", lang);
            return Ok(lang.clone());
        }
    }

    // Fallback to content-based language detection
    debug!("No toot language attribute found, detecting from content");
    match language_detector.detect_language(&toot.content) {
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
