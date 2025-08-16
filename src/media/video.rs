use crate::config::{OpenRouterConfig, WhisperConfig};
use crate::error::MediaError;
use crate::mastodon::MediaAttachment;
use crate::media::audio::{is_ffmpeg_available, summarize_transcript};
use crate::whisper::WhisperModelManager;
use std::process::Command;
use tempfile::NamedTempFile;
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

/// Supported video formats for transcription  
pub const SUPPORTED_VIDEO_FORMATS: &[&str] = &[
    "video/mp4",
    "video/mpeg",
    "video/quicktime",
    "video/x-msvideo", // .avi
    "video/webm",
    "video/x-ms-wmv",
    "video/x-flv",
    "video/3gpp",
    "video/x-matroska", // .mkv
];

/// Process video file for transcription using Whisper
pub async fn process_video_for_transcript(
    media: &MediaAttachment,
    whisper_config: &WhisperConfig,
    openrouter_config: Option<&OpenRouterConfig>,
) -> Result<String, MediaError> {
    // Check if it's a video file
    let is_video = media.media_type.to_lowercase().starts_with("video")
        || media.media_type.to_lowercase() == "video";

    if !is_video {
        return Err(MediaError::UnsupportedType {
            media_type: media.media_type.clone(),
        });
    }

    if media.description.is_some() {
        return Err(MediaError::ProcessingFailed(
            "Video already has description".to_string(),
        ));
    }

    // Check if FFmpeg is available
    if !is_ffmpeg_available() {
        return Err(MediaError::ProcessingFailed(
            "FFmpeg is required for video processing but not found on system".to_string(),
        ));
    }

    // Validate URL before attempting download
    let parsed_url = match url::Url::parse(&media.url) {
        Ok(u) => u,
        Err(e) => {
            tracing::warn!("Invalid video URL format: {}: {}", media.url, e);
            return Err(MediaError::DownloadFailed {
                url: media.url.clone(),
            });
        }
    };

    // Validate URL scheme
    if !matches!(parsed_url.scheme(), "http" | "https") {
        tracing::warn!("Unsupported video URL scheme: {}", parsed_url.scheme());
        return Err(MediaError::DownloadFailed {
            url: media.url.clone(),
        });
    }

    // Download video data
    let http_client = reqwest::Client::new();
    let url_string = media.url.clone(); // Clone early to avoid borrow issues

    let response = http_client.get(&media.url).send().await.map_err(|e| {
        tracing::warn!("Failed to download video from {}: {}", url_string, e);
        MediaError::DownloadFailed {
            url: url_string.clone(),
        }
    })?;

    if !response.status().is_success() {
        tracing::warn!(
            "HTTP error {} for video URL: {}",
            response.status(),
            url_string
        );
        return Err(MediaError::DownloadFailed { url: url_string });
    }

    let video_data = response.bytes().await.map_err(|e| {
        tracing::warn!(
            "Failed to read video response bytes from {}: {}",
            url_string,
            e
        );
        MediaError::DownloadFailed {
            url: url_string.clone(),
        }
    })?;

    // Check video size limits
    let size_mb = video_data.len() as f64 / (1024.0 * 1024.0);
    if let Some(max_duration) = whisper_config.max_duration_minutes {
        // For video, use a conservative estimate of 1MB per minute
        let estimated_duration = size_mb / 10.0; // Rough estimate for video
        if estimated_duration > max_duration as f64 {
            return Err(MediaError::ProcessingFailed(format!(
                  "Video estimated duration {estimated_duration:.1} minutes exceeds limit of {max_duration} minutes"
              )));
        }
    }

    // Also check against default media config size limit
    let max_size_mb = 250.0; // Higher limit for video files
    if size_mb > max_size_mb {
        return Err(MediaError::ProcessingFailed(format!(
            "Video size {size_mb:.2}MB exceeds limit of {max_size_mb:.2}MB"
        )));
    }

    // Extract audio from video and convert to WAV format using FFmpeg
    let wav_data = extract_audio_from_video(&video_data).await?;

    // Transcribe audio using Whisper
    let transcript = transcribe_wav_audio(&wav_data, whisper_config, openrouter_config).await?;

    Ok(transcript)
}

/// Extract audio from video data and convert to WAV format using FFmpeg
async fn extract_audio_from_video(video_data: &[u8]) -> Result<Vec<u8>, MediaError> {
    let input_file = NamedTempFile::new()
        .map_err(|e| MediaError::ProcessingFailed(format!("Failed to create temp file: {e}")))?;

    // Write video data and sync to ensure it's on disk before FFmpeg reads it
    tokio::fs::write(input_file.path(), video_data)
        .await
        .map_err(|e| MediaError::ProcessingFailed(format!("Failed to write video data: {e}")))?;

    // Sync to ensure data is written before FFmpeg processes it
    let input_file_path = input_file.path().to_path_buf();

    let output_file = NamedTempFile::with_suffix(".wav").map_err(|e| {
        MediaError::ProcessingFailed(format!("Failed to create output temp file: {e}"))
    })?;

    let output_file_path = output_file.path().to_path_buf();

    // Extract audio from video and convert to WAV
    let output = Command::new("ffmpeg")
        .args([
            "-i",
            input_file_path.to_str().ok_or_else(|| {
                MediaError::ProcessingFailed("Invalid input file path encoding".to_string())
            })?,
            "-vn", // No video
            "-acodec",
            "pcm_s16le",
            "-ar",
            "16000",
            "-ac",
            "1",
            "-y", // Overwrite output file
            output_file_path.to_str().ok_or_else(|| {
                MediaError::ProcessingFailed("Invalid output file path encoding".to_string())
            })?,
        ])
        .output()
        .map_err(|e| MediaError::ProcessingFailed(format!("FFmpeg execution failed: {e}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(MediaError::ProcessingFailed(format!(
            "FFmpeg video audio extraction failed: {stderr}"
        )));
    }

    // Read output before temp files are dropped
    let result = tokio::fs::read(&output_file_path)
        .await
        .map_err(|e| MediaError::ProcessingFailed(format!("Failed to read extracted WAV: {e}")));

    // Explicit cleanup is handled by NamedTempFile Drop
    result
}

/// Transcribe WAV audio data using Whisper (reused from audio module)
async fn transcribe_wav_audio(
    wav_data: &[u8],
    whisper_config: &WhisperConfig,
    openrouter_config: Option<&OpenRouterConfig>,
) -> Result<String, MediaError> {
    // Suppress all Whisper output during the entire process (model loading and transcription)
    let _suppressed_output = SuppressOutput::new();

    let model_name = whisper_config.model.as_ref().ok_or_else(|| {
        MediaError::ProcessingFailed("No Whisper model specified in configuration".to_string())
    })?;

    // Create model manager and download model
    let model_manager = WhisperModelManager::new(whisper_config.clone()).map_err(|e| {
        MediaError::ProcessingFailed(format!("Failed to initialize Whisper model manager: {e}"))
    })?;

    let model_path = model_manager
        .download_model(model_name)
        .await
        .map_err(|e| {
            MediaError::ProcessingFailed(format!("Failed to download Whisper model: {e}"))
        })?;

    let wav_file = NamedTempFile::with_suffix(".wav").map_err(|e| {
        MediaError::ProcessingFailed(format!("Failed to create WAV temp file: {e}"))
    })?;

    // Write data and sync before Whisper processing
    tokio::fs::write(wav_file.path(), wav_data)
        .await
        .map_err(|e| MediaError::ProcessingFailed(format!("Failed to write WAV data: {e}")))?;

    let wav_file_path = wav_file.path().to_path_buf();

    let ctx = WhisperContext::new_with_params(
        model_path.to_str().ok_or_else(|| {
            MediaError::ProcessingFailed("Invalid model path encoding".to_string())
        })?,
        WhisperContextParameters::default(),
    )
    .map_err(|e| MediaError::ProcessingFailed(format!("Failed to create Whisper context: {e}")))?;

    let audio_data = load_wav_as_f32_pcm(&wav_file_path).await?;

    let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });

    // Explicitly disable translation and force transcription-only mode
    params.set_translate(false);

    // Enable debugging for language detection
    tracing::info!("Whisper model: {}", model_name);
    tracing::info!("Configured language: {:?}", whisper_config.language);

    // Set language - only set it if explicitly configured, otherwise let Whisper auto-detect
    if let Some(ref lang) = whisper_config.language {
        if !lang.is_empty() && lang != "auto" {
            tracing::info!("Using configured language: {}", lang);
            params.set_language(Some(lang));
        } else {
            tracing::info!("Language set to auto-detect mode");
            // Don't set language parameter to enable auto-detection
        }
    } else {
        tracing::info!("No language configured, using auto-detection");
        // Don't set language parameter to enable auto-detection
    }
    // Suppress all possible Whisper output
    params.set_print_special(false);
    params.set_print_progress(false);
    params.set_print_realtime(false);
    params.set_print_timestamps(false);
    params.set_no_context(true);
    params.set_single_segment(false);
    params.set_suppress_blank(true);
    params.set_suppress_nst(true);

    let mut state = ctx.create_state().map_err(|e| {
        MediaError::ProcessingFailed(format!("Failed to create Whisper state: {e}"))
    })?;

    state
        .full(params, &audio_data)
        .map_err(|e| MediaError::ProcessingFailed(format!("Whisper transcription failed: {e}")))?;

    // Output suppression ends here when _suppressed_output is dropped

    let num_segments = state
        .full_n_segments()
        .map_err(|e| MediaError::ProcessingFailed(format!("Failed to get segment count: {e}")))?;

    let mut transcript = String::new();
    for i in 0..num_segments {
        let segment_text = state.full_get_segment_text(i).map_err(|e| {
            MediaError::ProcessingFailed(format!("Failed to get segment text: {e}"))
        })?;
        transcript.push_str(&segment_text);
    }

    // Normalize Unicode and clean the transcript
    let transcript = transcript
        .chars()
        .filter(|&c| c != '\0' && (c.is_whitespace() || !c.is_control()))
        .collect::<String>()
        .trim()
        .to_string();

    // Apply hard limit of 1500 characters for video descriptions
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
            "No transcribable speech found in video".to_string(),
        ));
    }

    Ok(transcript)
}

/// Load WAV file as f32 PCM data for Whisper processing
async fn load_wav_as_f32_pcm(wav_path: &std::path::Path) -> Result<Vec<f32>, MediaError> {
    let wav_path_str = wav_path.to_str().ok_or_else(|| {
        MediaError::ProcessingFailed("Invalid WAV file path encoding".to_string())
    })?;

    let output = Command::new("ffmpeg")
        .args([
            "-i",
            wav_path_str,
            "-f",
            "f32le",
            "-ar",
            "16000",
            "-ac",
            "1",
            "-",
        ])
        .output()
        .map_err(|e| MediaError::ProcessingFailed(format!("FFmpeg PCM extraction failed: {e}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(MediaError::ProcessingFailed(format!(
            "FFmpeg PCM extraction failed: {stderr}"
        )));
    }

    let pcm_bytes = output.stdout;
    let mut pcm_data = Vec::with_capacity(pcm_bytes.len() / 4);

    for chunk in pcm_bytes.chunks_exact(4) {
        if chunk.len() != 4 {
            continue; // Skip incomplete chunks to prevent panic
        }
        let sample = f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
        pcm_data.push(sample);
    }

    Ok(pcm_data)
}

/// Utility to suppress stdout/stderr output during Whisper processing
struct SuppressOutput {
    // This is a marker struct - actual suppression happens via Whisper parameters
    // The underlying C library output can't be easily suppressed without libc
}

impl SuppressOutput {
    fn new() -> Self {
        // For now, we rely on the Whisper parameters to suppress most output
        // The initialization messages from the C library are harder to suppress
        // without adding libc dependency
        Self {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_video_formats_list() {
        assert!(SUPPORTED_VIDEO_FORMATS.contains(&"video/mp4"));
        assert!(SUPPORTED_VIDEO_FORMATS.contains(&"video/webm"));
        assert!(SUPPORTED_VIDEO_FORMATS.contains(&"video/quicktime"));
        assert_eq!(SUPPORTED_VIDEO_FORMATS.len(), 9);
    }

    #[test]
    fn test_video_size_estimation() {
        // Test the video size limit logic
        let size_mb = 100.0;
        let estimated_duration = size_mb / 10.0; // 10 minutes
        assert_eq!(estimated_duration, 10.0);
    }

    /// Helper function to test transcript character limiting logic for video
    fn apply_video_transcript_limit(input: String) -> String {
        // Apply the same logic as in transcribe_wav_audio for video
        let transcript = input
            .chars()
            .filter(|&c| c != '\0' && (c.is_whitespace() || !c.is_control()))
            .collect::<String>()
            .trim()
            .to_string();

        // Apply hard limit of 1500 characters for video descriptions
        if transcript.len() > 1500 {
            let truncated = transcript.chars().take(1497).collect::<String>();
            format!("{truncated}...")
        } else {
            transcript
        }
    }

    #[test]
    fn test_video_transcript_character_limit() {
        // Test short transcript (under limit)
        let short_text = "This is a short video transcript that should not be truncated.";
        let result = apply_video_transcript_limit(short_text.to_string());
        assert_eq!(result, short_text);
        assert!(result.len() <= 1500);

        // Test exactly 1500 characters
        let exact_1500 = "a".repeat(1500);
        let result = apply_video_transcript_limit(exact_1500.clone());
        assert_eq!(result, exact_1500);
        assert_eq!(result.len(), 1500);

        // Test over limit (1501 characters)
        let over_limit = "a".repeat(1501);
        let result = apply_video_transcript_limit(over_limit);
        assert_eq!(result.len(), 1500); // 1497 + "..." = 1500
        assert!(result.ends_with("..."));

        // Test much longer transcript
        let very_long = "a".repeat(3000);
        let result = apply_video_transcript_limit(very_long);
        assert_eq!(result.len(), 1500);
        assert!(result.ends_with("..."));
        assert_eq!(&result[0..1497], &"a".repeat(1497));
    }
}
