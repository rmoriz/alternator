use crate::config::{OpenRouterConfig, WhisperConfig};
use crate::error::MediaError;
use crate::mastodon::MediaAttachment;
use crate::openrouter::OpenRouterClient;
use crate::whisper_cli::WhisperCli;
use std::process::Command;
use tempfile::NamedTempFile;

/// Supported audio formats for transcription  
pub const SUPPORTED_AUDIO_FORMATS: &[&str] = &[
    "audio/mpeg",
    "audio/mp3",
    "audio/wav",
    "audio/wave",
    "audio/x-wav",
    "audio/m4a",
    "audio/mp4",
    "audio/aac",
    "audio/ogg",
    "audio/flac",
    "audio/x-flac",
];

/// Check if FFmpeg is available on the system
pub fn is_ffmpeg_available() -> bool {
    Command::new("ffmpeg")
        .arg("-version")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

/// Process audio file for transcription using Whisper CLI
pub async fn process_audio_for_transcript(
    media: &MediaAttachment,
    whisper_config: &WhisperConfig,
    media_config: &crate::config::MediaConfig,
    openrouter_config: Option<&OpenRouterConfig>,
) -> Result<String, MediaError> {
    // Check if it's an audio file
    let is_audio = media.media_type.to_lowercase().starts_with("audio")
        || media.media_type.to_lowercase() == "audio";

    if !is_audio {
        return Err(MediaError::UnsupportedType {
            media_type: media.media_type.clone(),
        });
    }

    if media.description.is_some() {
        return Err(MediaError::ProcessingFailed(
            "Audio already has description".to_string(),
        ));
    }

    // Validate URL before attempting download
    let parsed_url = match url::Url::parse(&media.url) {
        Ok(u) => u,
        Err(e) => {
            tracing::warn!("Invalid audio URL format: {}: {}", media.url, e);
            return Err(MediaError::DownloadFailed {
                url: media.url.clone(),
            });
        }
    };

    // Validate URL scheme
    if !matches!(parsed_url.scheme(), "http" | "https") {
        tracing::warn!("Unsupported audio URL scheme: {}", parsed_url.scheme());
        return Err(MediaError::DownloadFailed {
            url: media.url.clone(),
        });
    }

    // Download audio data
    let http_client = reqwest::Client::new();
    let url_string = media.url.clone(); // Clone early to avoid borrow issues

    let response = http_client.get(&media.url).send().await.map_err(|e| {
        tracing::warn!("Failed to download audio from {}: {}", url_string, e);
        MediaError::DownloadFailed {
            url: url_string.clone(),
        }
    })?;

    if !response.status().is_success() {
        tracing::warn!(
            "HTTP error {} for audio URL: {}",
            response.status(),
            url_string
        );
        return Err(MediaError::DownloadFailed { url: url_string });
    }

    let audio_data = response.bytes().await.map_err(|e| {
        tracing::warn!(
            "Failed to read audio response bytes from {}: {}",
            url_string,
            e
        );
        MediaError::DownloadFailed {
            url: url_string.clone(),
        }
    })?;

    // Check audio size limits
    let size_mb = audio_data.len() as f64 / (1024.0 * 1024.0);
    if let Some(max_duration) = whisper_config.max_duration_minutes {
        let estimated_duration = size_mb; // Simple estimation
        if estimated_duration > max_duration as f64 {
            return Err(MediaError::ProcessingFailed(format!(
                  "Audio estimated duration {estimated_duration:.1} minutes exceeds limit of {max_duration} minutes"
              )));
        }
    }

    // Also check against media config size limit
    let max_size_mb = media_config.max_audio_size_mb.unwrap_or(50) as f64;
    if size_mb > max_size_mb {
        return Err(MediaError::ProcessingFailed(format!(
            "Audio size {size_mb:.2}MB exceeds limit of {max_size_mb:.2}MB"
        )));
    }

    // Convert audio to WAV format using FFmpeg
    let wav_data = convert_audio_to_wav(&audio_data).await?;

    // Transcribe audio using Whisper CLI
    let transcript =
        transcribe_audio_with_whisper_cli(&wav_data, whisper_config, openrouter_config).await?;

    Ok(transcript)
}

/// Convert audio data to WAV format using FFmpeg
async fn convert_audio_to_wav(audio_data: &[u8]) -> Result<Vec<u8>, MediaError> {
    let input_file = NamedTempFile::new()
        .map_err(|e| MediaError::ProcessingFailed(format!("Failed to create temp file: {e}")))?;

    // Write data and sync to ensure it's on disk before FFmpeg reads it
    tokio::fs::write(input_file.path(), audio_data)
        .await
        .map_err(|e| MediaError::ProcessingFailed(format!("Failed to write audio data: {e}")))?;

    // Sync to ensure data is written before FFmpeg processes it
    let input_file_path = input_file.path().to_path_buf();

    let output_file = NamedTempFile::with_suffix(".wav").map_err(|e| {
        MediaError::ProcessingFailed(format!("Failed to create output temp file: {e}"))
    })?;

    let output_file_path = output_file.path().to_path_buf();

    // Convert audio to WAV format using FFmpeg (non-blocking)
    let input_path_clone = input_file_path.clone();
    let output_path_clone = output_file_path.clone();

    let output = tokio::task::spawn_blocking(move || {
        Command::new("ffmpeg")
            .args([
                "-i",
                input_path_clone.to_str().ok_or_else(|| {
                    std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        "Invalid input file path encoding",
                    )
                })?,
                "-ar",
                "16000",
                "-ac",
                "1",
                "-c:a",
                "pcm_s16le",
                "-y",
                output_path_clone.to_str().ok_or_else(|| {
                    std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        "Invalid output file path encoding",
                    )
                })?,
            ])
            .output()
    })
    .await
    .map_err(|e| MediaError::ProcessingFailed(format!("FFmpeg task failed: {e}")))?
    .map_err(|e| MediaError::ProcessingFailed(format!("FFmpeg execution failed: {e}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(MediaError::ProcessingFailed(format!(
            "FFmpeg conversion failed: {stderr}"
        )));
    }

    // Read output before temp files are dropped
    let result = tokio::fs::read(&output_file_path)
        .await
        .map_err(|e| MediaError::ProcessingFailed(format!("Failed to read converted WAV: {e}")));

    // Explicit cleanup is handled by NamedTempFile Drop
    result
}

/// Transcribe audio data using Whisper CLI
async fn transcribe_audio_with_whisper_cli(
    wav_data: &[u8],
    whisper_config: &WhisperConfig,
    openrouter_config: Option<&OpenRouterConfig>,
) -> Result<String, MediaError> {
    // Create Whisper CLI instance
    let whisper_cli = WhisperCli::new(whisper_config)?;

    // Save WAV data to a temporary file
    let wav_file = NamedTempFile::with_suffix(".wav").map_err(|e| {
        MediaError::ProcessingFailed(format!("Failed to create WAV temp file: {e}"))
    })?;

    tokio::fs::write(wav_file.path(), wav_data)
        .await
        .map_err(|e| MediaError::ProcessingFailed(format!("Failed to write WAV data: {e}")))?;

    // Transcribe using Whisper CLI
    let transcript = whisper_cli
        .transcribe_audio(wav_file.path(), whisper_config.language.as_deref())
        .await?;

    // Normalize Unicode and clean the transcript
    let transcript = transcript
        .chars()
        .filter(|&c| c != '\0' && (c.is_whitespace() || !c.is_control()))
        .collect::<String>()
        .trim()
        .to_string();

    // Apply hard limit of 1500 characters for audio descriptions
    let transcript = if transcript.len() > 1500 {
        // Try to summarize using LLM if OpenRouter config is available
        if let Some(openrouter_config) = openrouter_config {
            match summarize_transcript(&transcript, openrouter_config).await {
                Ok(summary) => summary,
                Err(e) => {
                    tracing::warn!(
                        "Failed to summarize transcript using LLM: {e}, falling back to truncation"
                    );
                    let truncated = transcript.chars().take(1497).collect::<String>();
                    format!("{truncated}...")
                }
            }
        } else {
            let truncated = transcript.chars().take(1497).collect::<String>();
            format!("{truncated}...")
        }
    } else {
        transcript
    };

    if transcript.is_empty() {
        return Err(MediaError::ProcessingFailed(
            "No transcribable speech found in audio".to_string(),
        ));
    }

    Ok(transcript)
}

/// Summarize a long transcript using OpenRouter LLM with fallback
pub async fn summarize_transcript(
    transcript: &str,
    openrouter_config: &OpenRouterConfig,
) -> Result<String, MediaError> {
    let openrouter_client = OpenRouterClient::new(openrouter_config.clone());

    // Detect the primary language of the transcript for better language preservation
    let detected_language = crate::language::detect_text_language(transcript);

    let prompt = format!(
        "IMPORTANT: You MUST respond in the EXACT SAME LANGUAGE as the transcript below. Do NOT translate or change the language.

Your task:
1. Summarize the following transcript in under 1500 characters
2. Keep the EXACT SAME LANGUAGE as the original transcript  
3. Add a brief note that this is a summary due to length (in the same language)
4. Preserve the main content and meaning

Detected language: {detected_language}

Transcript to summarize:
{transcript}

Remember: Your entire response must be in the same language as the transcript above."
    );

    // Try summarization with retries for provider failures
    const MAX_RETRIES: u32 = 3;
    const INITIAL_DELAY_MS: u64 = 2000;

    for attempt in 0..=MAX_RETRIES {
        match openrouter_client.process_text(&prompt).await {
            Ok(summary) => {
                tracing::info!(
                    "Successfully summarized transcript from {} to {} characters on attempt {}",
                    transcript.len(),
                    summary.len(),
                    attempt + 1
                );
                return Ok(summary);
            }
            Err(crate::error::OpenRouterError::ProviderFailure { provider, message }) => {
                if attempt < MAX_RETRIES {
                    let delay = INITIAL_DELAY_MS * 2_u64.pow(attempt);
                    tracing::warn!(
                        "OpenRouter provider '{}' failed (attempt {}): {}. Retrying in {}ms...",
                        provider,
                        attempt + 1,
                        message,
                        delay
                    );
                    tokio::time::sleep(tokio::time::Duration::from_millis(delay)).await;
                    continue;
                } else {
                    tracing::error!(
                        "OpenRouter provider '{}' failed after {} attempts: {}",
                        provider,
                        MAX_RETRIES + 1,
                        message
                    );
                    return Err(MediaError::ProcessingFailed(format!(
                        "LLM summarization failed after {} retries - provider '{}' unavailable: {}",
                        MAX_RETRIES + 1,
                        provider,
                        message
                    )));
                }
            }
            Err(crate::error::OpenRouterError::RateLimitExceeded { retry_after }) => {
                if attempt < MAX_RETRIES {
                    tracing::warn!(
                        "OpenRouter rate limited (attempt {}). Waiting {} seconds...",
                        attempt + 1,
                        retry_after
                    );
                    tokio::time::sleep(tokio::time::Duration::from_secs(retry_after)).await;
                    continue;
                } else {
                    return Err(MediaError::ProcessingFailed(format!(
                        "LLM summarization failed - rate limited after {} attempts",
                        MAX_RETRIES + 1
                    )));
                }
            }
            Err(e) => {
                tracing::warn!("OpenRouter summarization failed: {e}");
                return Err(MediaError::ProcessingFailed(format!(
                    "LLM summarization failed: {e}"
                )));
            }
        }
    }

    unreachable!("Should have returned in the loop")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_audio_formats_list() {
        assert!(SUPPORTED_AUDIO_FORMATS.contains(&"audio/mp3"));
        assert!(SUPPORTED_AUDIO_FORMATS.contains(&"audio/wav"));
        assert!(SUPPORTED_AUDIO_FORMATS.contains(&"audio/flac"));
        assert_eq!(SUPPORTED_AUDIO_FORMATS.len(), 11);
    }

    #[test]
    fn test_ffmpeg_availability_check() {
        // This test will pass or fail based on whether FFmpeg is installed
        // We're just testing that the function doesn't panic
        let _result = is_ffmpeg_available();
    }

    /// Helper function to test transcript character limiting logic
    fn apply_transcript_limit(input: String) -> String {
        // Apply the same logic as in transcribe_wav_audio
        let transcript = input
            .chars()
            .filter(|&c| c != '\0' && (c.is_whitespace() || !c.is_control()))
            .collect::<String>()
            .trim()
            .to_string();

        // Apply hard limit of 1500 characters for audio descriptions
        if transcript.len() > 1500 {
            let truncated = transcript.chars().take(1497).collect::<String>();
            format!("{truncated}...")
        } else {
            transcript
        }
    }

    #[test]
    fn test_transcript_character_limit() {
        // Test short transcript (under limit)
        let short_text = "This is a short transcript that should not be truncated.";
        let result = apply_transcript_limit(short_text.to_string());
        assert_eq!(result, short_text);
        assert!(result.len() <= 1500);

        // Test exactly 1500 characters
        let exact_1500 = "a".repeat(1500);
        let result = apply_transcript_limit(exact_1500.clone());
        assert_eq!(result, exact_1500);
        assert_eq!(result.len(), 1500);

        // Test over limit (1501 characters)
        let over_limit = "a".repeat(1501);
        let result = apply_transcript_limit(over_limit);
        assert_eq!(result.len(), 1500); // 1497 + "..." = 1500
        assert!(result.ends_with("..."));

        // Test much longer transcript
        let very_long = "a".repeat(3000);
        let result = apply_transcript_limit(very_long);
        assert_eq!(result.len(), 1500);
        assert!(result.ends_with("..."));
        assert_eq!(&result[0..1497], &"a".repeat(1497));

        // Test with unicode characters (should not be truncated)
        let unicode_text = "🎵".repeat(200); // 200 emoji chars, well under 1500 limit
        let result = apply_transcript_limit(unicode_text.clone());
        assert_eq!(result, unicode_text); // Should fit within 1500 characters
        assert!(result.chars().count() <= 1500);

        // Test unicode that exceeds limit
        let long_unicode = "🎵".repeat(2000); // 2000 emoji chars, over 1500 limit
        let result = apply_transcript_limit(long_unicode);
        assert_eq!(result.chars().count(), 1500); // Should be exactly 1500 chars (1497 + ...)
        assert!(result.ends_with("..."));

        // Test empty transcript
        let empty = String::new();
        let result = apply_transcript_limit(empty);
        assert_eq!(result, "");
    }

    #[tokio::test]
    async fn test_summarize_transcript_mock() {
        use crate::config::OpenRouterConfig;

        // Test that the summarization function handles the request properly
        // Note: This doesn't test the actual API call, just the function structure
        let config = OpenRouterConfig {
            api_key: "test_key".to_string(),
            model: "test-model".to_string(),
            vision_model: "test-vision-model".to_string(),
            text_model: "test-text-model".to_string(),
            base_url: Some("https://test.example.com".to_string()),
            max_tokens: Some(1500),
        };

        let long_transcript = "a".repeat(2000);

        // This will fail because it's a mock config, but we're testing the function exists
        // and handles errors properly
        let result = summarize_transcript(&long_transcript, &config).await;
        assert!(result.is_err());

        // The error should be a MediaError::ProcessingFailed with LLM summarization failure
        match result {
            Err(crate::error::MediaError::ProcessingFailed(msg)) => {
                assert!(msg.contains("LLM summarization failed"));
            }
            _ => panic!("Expected ProcessingFailed error"),
        }
    }

    #[test]
    fn test_summarization_integration() {
        // Test the integration logic in the transcript limiting code
        let short_text = "Short transcript";
        let long_text = "a".repeat(2000);

        // Test that short transcripts are not affected
        assert_eq!(apply_transcript_limit(short_text.to_string()), short_text);

        // Test that long transcripts are truncated when no LLM is available
        let truncated = apply_transcript_limit(long_text);
        assert_eq!(truncated.len(), 1500); // 1497 + "..." = 1500
        assert!(truncated.ends_with("..."));
    }
}
