use crate::config::RuntimeConfig;
use crate::error::AlternatorError;
use crate::language::LanguageDetector;
use crate::mastodon::{MastodonClient, MediaRecreation, TootEvent};
use crate::media::MediaProcessor;
use crate::openrouter::OpenRouterClient;
use tracing::{debug, error, info, warn};

/// Process a single toot - check for media, generate descriptions, and update
pub async fn process_toot(
    toot: &TootEvent,
    mastodon_client: &MastodonClient,
    openrouter_client: &OpenRouterClient,
    media_processor: &MediaProcessor,
    language_detector: &LanguageDetector,
    config: &RuntimeConfig,
) -> Result<(), AlternatorError> {
    // Check if toot has media attachments
    if toot.media_attachments.is_empty() {
        debug!("Toot {} has no media attachments, skipping", toot.id);
        return Ok(());
    }

    // Filter media that needs processing (with audio awareness)
    let processable_media = media_processor
        .filter_processable_media_with_audio(&toot.media_attachments, config.is_audio_enabled());

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
    let detected_language =
        crate::toot_handler::processor::detect_toot_language(toot, language_detector)?;
    let prompt_template = language_detector
        .get_prompt_template(&detected_language)
        .map_err(AlternatorError::Language)?;

    debug!(
        "Using language '{}' with prompt template",
        detected_language
    );

    // Process each media attachment and collect successful descriptions with media data
    let mut media_recreations: Vec<MediaRecreation> = Vec::new();
    let mut original_media_ids = Vec::new();

    // First pass: Prepare all media for processing (downloads and preprocessing)
    let mut prepared_media = Vec::new();

    for media in processable_media {
        info!(
            "Preparing media attachment: {} ({})",
            media.id, media.media_type
        );

        // Check for race conditions before processing
        if let Err(e) =
            crate::toot_handler::race::check_race_condition(mastodon_client, &toot.id, &media.id)
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

        // Handle audio files separately from images
        if media.media_type.to_lowercase().starts_with("audio") {
            // Check if audio processing is enabled
            if !config.is_audio_enabled() {
                debug!(
                    "Audio processing disabled, skipping audio file: {} ({})",
                    media.id, media.media_type
                );
                continue;
            }

            info!("Processing audio file: {} ({})", media.id, media.media_type);

            // Download original audio data for recreation
            let original_audio_data =
                match media_processor.download_media_for_recreation(media).await {
                    Ok(data) => data,
                    Err(e) => {
                        error!(
                            "Failed to download audio {} for recreation: {}",
                            media.id, e
                        );
                        continue;
                    }
                };

            // Transcribe the audio to get description
            match crate::media::process_audio_for_transcript(
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
                    // Determine appropriate file extension for audio
                    let extension = match media.media_type.as_str() {
                        "audio/mpeg" | "audio/mp3" => "mp3",
                        "audio/wav" | "audio/wave" | "audio/x-wav" => "wav",
                        "audio/m4a" => "m4a",
                        "audio/mp4" => "mp4",
                        "audio/aac" => "aac",
                        "audio/ogg" => "ogg",
                        "audio/flac" | "audio/x-flac" => "flac",
                        _ => "audio", // fallback
                    };
                    let filename = format!("audio_{}.{}", media.id, extension);

                    // Add to media recreations for batch processing
                    media_recreations.push(MediaRecreation {
                        data: original_audio_data,
                        description: transcript,
                        media_type: media.media_type.clone(),
                        filename,
                    });
                    // Track original media ID for cleanup
                    original_media_ids.push(media.id.clone());
                }
                Err(crate::error::MediaError::UnsupportedType { .. }) => {
                    warn!(
                        "Audio type {} not supported for transcription, skipping",
                        media.media_type
                    );
                    continue;
                }
                Err(e) => {
                    error!("Failed to transcribe audio {}: {}", media.id, e);
                    continue;
                }
            }

            // Skip to next media (don't process as image)
            continue;
        }

        // Handle video files separately from images
        if media.media_type.to_lowercase().starts_with("video") {
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
                continue;
            }

            info!("Processing video file: {} ({})", media.id, media.media_type);

            // Download original video data for recreation
            debug!(
                "About to download video data for media: {} with type: '{}'",
                media.id, media.media_type
            );
            let original_video_data =
                match media_processor.download_media_for_recreation(media).await {
                    Ok(data) => data,
                    Err(e) => {
                        error!(
                            "Failed to download video {} for recreation: {}",
                            media.id, e
                        );
                        continue;
                    }
                };

            // Transcribe the video audio to get description
            match crate::media::process_video_for_transcript(
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
                    // Determine appropriate file extension for video
                    let extension = match media.media_type.as_str() {
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
                    };
                    let filename = format!("video_{}.{}", media.id, extension);

                    // Add to media recreations for batch processing
                    media_recreations.push(MediaRecreation {
                        data: original_video_data,
                        description: transcript,
                        media_type: media.media_type.clone(),
                        filename,
                    });
                    // Track original media ID for cleanup
                    original_media_ids.push(media.id.clone());
                }
                Err(crate::error::MediaError::UnsupportedType { .. }) => {
                    warn!(
                        "Video type {} not supported for transcription, skipping",
                        media.media_type
                    );
                    continue;
                }
                Err(e) => {
                    error!("Failed to transcribe video {}: {}", media.id, e);
                    continue;
                }
            }

            // Skip to next media (don't process as image)
            continue;
        }

        // Process non-audio media (images) through the existing pipeline
        // Download original image data for recreation
        let original_image_data = match media_processor.download_media_for_recreation(media).await {
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
        let processed_media_data = match media_processor.process_media_for_analysis(media).await {
            Ok(data) => data,
            Err(e) => {
                error!("Failed to process media {} for analysis: {}", media.id, e);
                continue; // Skip this media but continue with others
            }
        };

        prepared_media.push((media.clone(), original_image_data, processed_media_data));
    }

    if prepared_media.is_empty() && media_recreations.is_empty() {
        debug!("No media could be prepared for processing");
        return Ok(());
    }

    // Process images if any are prepared
    if !prepared_media.is_empty() {
        info!(
            "Prepared {} media attachments, starting parallel description generation",
            prepared_media.len()
        );

        // Second pass: Generate descriptions in parallel using OpenRouter
        let description_tasks: Vec<_> = prepared_media
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
        for ((media, original_data, _processed_data), (result_media_id, description_result)) in
            prepared_media
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

                    // Determine appropriate file extension for images
                    let extension = match media.media_type.as_str() {
                        "image/jpeg" => "jpg",
                        "image/png" => "png",
                        "image/gif" => "gif",
                        "image/webp" => "webp",
                        "image/bmp" => "bmp",
                        "image/tiff" => "tiff",
                        _ => "jpg", // fallback to jpg for unknown image types
                    };
                    let filename = format!("image_{}.{}", media.id, extension);

                    // Add to media recreations for batch processing
                    media_recreations.push(MediaRecreation {
                        data: original_data,
                        description,
                        media_type: media.media_type.clone(),
                        filename,
                    });
                    // Track original media ID for cleanup
                    original_media_ids.push(media.id);
                }
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
        match crate::toot_handler::coordinator::recreate_media_with_race_check(
            mastodon_client,
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
            Err(AlternatorError::Mastodon(crate::error::MastodonError::RaceConditionDetected)) => {
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
