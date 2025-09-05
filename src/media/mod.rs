pub mod audio;
pub mod helpers;
pub mod image;
pub mod video;

use crate::error::MediaError;
use crate::mastodon::MediaAttachment;
use std::collections::HashSet;

// Re-export items for backward compatibility
pub use audio::{is_ffmpeg_available, process_audio_for_transcript, SUPPORTED_AUDIO_FORMATS};
pub use helpers::TempFile;
pub use image::{ImageFormat, ImageTransformer, SUPPORTED_IMAGE_FORMATS};
pub use video::{process_video_for_transcript, SUPPORTED_VIDEO_FORMATS};

/// Maximum file size in MB for processing
pub const DEFAULT_MAX_SIZE_MB: f64 = 10.0;

/// Configuration for media processing that supports both images and audio
#[derive(Debug, Clone)]
pub struct MediaConfig {
    pub max_size_mb: f64,
    pub max_dimension: u32,
    pub supported_formats: HashSet<String>,
}

impl Default for MediaConfig {
    fn default() -> Self {
        let mut supported_formats = HashSet::new();

        // Add image formats
        for format in SUPPORTED_IMAGE_FORMATS {
            supported_formats.insert(format.to_string());
        }

        // Add audio formats
        for format in SUPPORTED_AUDIO_FORMATS {
            supported_formats.insert(format.to_string());
        }

        // Add video formats
        for format in SUPPORTED_VIDEO_FORMATS {
            supported_formats.insert(format.to_string());
        }

        Self {
            max_size_mb: DEFAULT_MAX_SIZE_MB,
            max_dimension: image::DEFAULT_MAX_DIMENSION,
            supported_formats,
        }
    }
}

/// Trait for media transformation operations
pub trait MediaTransformer {
    /// Check if a media type is supported for processing
    fn is_supported(&self, media_type: &str) -> bool;

    /// Transform image data for analysis (resize, optimize)
    fn transform_for_analysis(&self, image_data: &[u8]) -> Result<Vec<u8>, MediaError>;

    /// Transform image data for analysis with progress callback
    fn transform_for_analysis_with_progress(
        &self,
        image_data: &[u8],
        progress_callback: Option<Box<dyn FnMut(&str) + Send + Sync>>,
    ) -> Result<Vec<u8>, MediaError> {
        // Default implementation ignores progress callback
        let _ = progress_callback;
        self.transform_for_analysis(image_data)
    }

    /// Check if media attachment needs a description
    fn needs_description(&self, media: &MediaAttachment) -> bool;

    /// Get optimal format for transformed image
    #[allow(dead_code)]
    fn get_optimal_format(&self, original_format: ImageFormat) -> ImageFormat;
}

/// Combined image and audio transformer that supports both media types
pub struct UnifiedMediaTransformer {
    image_processor: image::ImageProcessor,
    config: MediaConfig,
}

impl UnifiedMediaTransformer {
    pub fn new(config: MediaConfig) -> Self {
        // Create image config from media config
        let image_config = image::ImageConfig {
            max_size_mb: config.max_size_mb,
            max_dimension: config.max_dimension,
            supported_formats: config
                .supported_formats
                .iter()
                .filter(|f| f.starts_with("image/"))
                .cloned()
                .collect(),
        };

        Self {
            image_processor: image::ImageProcessor::new(image_config),
            config,
        }
    }

    #[allow(dead_code)] // Convenience constructor for tests
    pub fn with_default_config() -> Self {
        Self::new(MediaConfig::default())
    }
}

impl MediaTransformer for UnifiedMediaTransformer {
    fn is_supported(&self, media_type: &str) -> bool {
        let media_type_lower = media_type.trim().to_lowercase();

        tracing::debug!(
            "Checking if media type '{}' (trimmed: '{}') is supported",
            media_type,
            media_type_lower
        );
        tracing::debug!("Supported formats: {:?}", self.config.supported_formats);

        // Check if it's already a MIME type that we support
        if self.config.supported_formats.contains(&media_type_lower) {
            tracing::debug!(
                "Media type '{}' directly found in supported formats",
                media_type
            );
            return true;
        }

        // Handle Mastodon API format where type is just "image", "video", etc.
        let result = match media_type_lower.as_str() {
            "image" => {
                let has_image = self
                    .config
                    .supported_formats
                    .iter()
                    .any(|f| f.starts_with("image/"));
                tracing::debug!("Generic 'image' type: has_image_formats = {}", has_image);
                has_image
            }
            "audio" => {
                let has_audio = self
                    .config
                    .supported_formats
                    .iter()
                    .any(|f| f.starts_with("audio/"));
                tracing::debug!("Generic 'audio' type: has_audio_formats = {}", has_audio);
                has_audio
            }
            "video" => {
                let has_video = self
                    .config
                    .supported_formats
                    .iter()
                    .any(|f| f.starts_with("video/"));
                tracing::debug!("Generic 'video' type: has_video_formats = {}", has_video);
                tracing::debug!(
                    "Video formats found: {:?}",
                    self.config
                        .supported_formats
                        .iter()
                        .filter(|f| f.starts_with("video/"))
                        .collect::<Vec<_>>()
                );
                has_video
            }
            _ => {
                tracing::debug!("Unknown media type: '{}'", media_type);
                false
            }
        };

        tracing::debug!("Media type '{}' support result: {}", media_type, result);
        result
    }

    fn transform_for_analysis(&self, image_data: &[u8]) -> Result<Vec<u8>, MediaError> {
        // Delegate to image processor for image transformation
        self.image_processor.transform_for_analysis(image_data)
    }

    fn transform_for_analysis_with_progress(
        &self,
        image_data: &[u8],
        progress_callback: Option<Box<dyn FnMut(&str) + Send + Sync>>,
    ) -> Result<Vec<u8>, MediaError> {
        // Delegate to image processor for streaming image transformation
        self.image_processor.transform_for_analysis_with_progress(image_data, progress_callback)
    }

    fn needs_description(&self, media: &MediaAttachment) -> bool {
        // Check if it's a supported media type
        if !self.is_supported(&media.media_type) {
            return false;
        }

        // Additional safety: check for valid media ID and URL
        if media.id.trim().is_empty() || media.url.trim().is_empty() {
            tracing::warn!("Media attachment has empty ID or URL: {:?}", media);
            return false;
        }

        // Check if description is missing or empty
        match &media.description {
            None => true,
            Some(desc) => desc.trim().is_empty(),
        }
    }

    fn get_optimal_format(&self, original_format: ImageFormat) -> ImageFormat {
        self.image_processor.get_optimal_format(original_format)
    }
}

/// Main media processor that coordinates filtering and transformation
pub struct MediaProcessor {
    transformer: Box<dyn MediaTransformer + Send + Sync>,
    http_client: reqwest::Client,
}

impl MediaProcessor {
    pub fn new(transformer: Box<dyn MediaTransformer + Send + Sync>) -> Self {
        Self {
            transformer,
            http_client: reqwest::Client::new(),
        }
    }

    /// Create processor with unified transformer (supports both images and audio)
    pub fn with_unified_transformer(config: MediaConfig) -> Self {
        Self::new(Box::new(UnifiedMediaTransformer::new(config)))
    }

    /// Backward compatibility: create processor with image transformer
    pub fn with_image_transformer(config: MediaConfig) -> Self {
        Self::with_unified_transformer(config)
    }

    #[allow(dead_code)] // Convenience constructor for tests
    pub fn with_default_config() -> Self {
        Self::with_unified_transformer(MediaConfig::default())
    }

    /// Filter media attachments to only include supported types that need descriptions
    pub fn filter_processable_media<'a>(
        &self,
        media_attachments: &'a [MediaAttachment],
    ) -> Vec<&'a MediaAttachment> {
        media_attachments
            .iter()
            .filter(|media| {
                self.transformer.is_supported(&media.media_type)
                    && self.transformer.needs_description(media)
            })
            .collect()
    }

    /// Filter media attachments to include image, audio, and video types when enabled
    pub fn filter_processable_media_with_audio<'a>(
        &self,
        media_attachments: &'a [MediaAttachment],
        audio_enabled: bool,
    ) -> Vec<&'a MediaAttachment> {
        media_attachments
            .iter()
            .filter(|media| {
                // Check for image support via transformer
                let image_supported = self.transformer.is_supported(&media.media_type)
                    && self.transformer.needs_description(media);

                // Check for audio support if enabled
                let audio_supported = if audio_enabled {
                    let media_type_lower = media.media_type.to_lowercase();
                    let is_audio = SUPPORTED_AUDIO_FORMATS.contains(&media_type_lower.as_str())
                        || media_type_lower.starts_with("audio")
                        || media_type_lower == "audio";
                    is_audio
                        && media
                            .description
                            .as_ref()
                            .map_or(true, |desc| desc.trim().is_empty())
                } else {
                    false
                };

                // Check for video support if audio is enabled (since video processing uses audio extraction)
                let video_supported = if audio_enabled {
                    let media_type_lower = media.media_type.to_lowercase();
                    let is_video = SUPPORTED_VIDEO_FORMATS.contains(&media_type_lower.as_str())
                        || media_type_lower.starts_with("video")
                        || media_type_lower == "video";
                    is_video
                        && media
                            .description
                            .as_ref()
                            .map_or(true, |desc| desc.trim().is_empty())
                } else {
                    false
                };

                image_supported || audio_supported || video_supported
            })
            .collect()
    }

    /// Download media from URL with streaming support
    pub async fn download_media(&self, url: &str) -> Result<Vec<u8>, MediaError> {
        self.download_media_with_callback(url, None).await
    }

    /// Download media from URL with optional streaming callback for processing chunks
    pub async fn download_media_with_callback(
        &self,
        url: &str,
        mut callback: Option<Box<dyn FnMut(&[u8]) -> Result<(), MediaError> + Send + Sync>>,
    ) -> Result<Vec<u8>, MediaError> {
        // Validate URL format before attempting download
        let parsed_url = match url::Url::parse(url) {
            Ok(u) => u,
            Err(e) => {
                tracing::warn!("Invalid URL format: {}: {}", url, e);
                return Err(MediaError::DownloadFailed {
                    url: url.to_string(),
                });
            }
        };

        // Validate URL scheme
        if !matches!(parsed_url.scheme(), "http" | "https") {
            tracing::warn!("Unsupported URL scheme: {}", parsed_url.scheme());
            return Err(MediaError::DownloadFailed {
                url: url.to_string(),
            });
        }

        // Clone URL early to avoid borrow issues in error handling
        let url_string = url.to_string();

        let response = self.http_client.get(url).send().await.map_err(|e| {
            tracing::warn!("Failed to send request to {}: {}", url_string, e);
            MediaError::DownloadFailed {
                url: url_string.clone(),
            }
        })?;

        if !response.status().is_success() {
            tracing::warn!("HTTP error {} for URL: {}", response.status(), url_string);
            return Err(MediaError::DownloadFailed { url: url_string });
        }

        // Use streaming download to reduce memory usage for large files
        let mut stream = response.bytes_stream();
        let mut data = Vec::new();
        let mut total_size = 0usize;

        use futures_util::StreamExt;
        while let Some(chunk_result) = stream.next().await {
            let chunk = chunk_result.map_err(|e| {
                tracing::warn!("Failed to read chunk from {}: {}", url_string, e);
                MediaError::DownloadFailed {
                    url: url_string.clone(),
                }
            })?;

            // Check for reasonable size limits to prevent memory exhaustion
            total_size += chunk.len();
            if total_size > 100 * 1024 * 1024 { // 100MB limit
                return Err(MediaError::ProcessingFailed(
                    "Media file too large (>100MB)".to_string(),
                ));
            }

            // Call callback if provided for streaming processing
            if let Some(ref mut cb) = callback {
                cb(&chunk)?;
            }

            data.extend_from_slice(&chunk);
        }

        Ok(data)
    }

    /// Process media attachment: download, transform, and prepare for analysis
    pub async fn process_media_for_analysis(
        &self,
        media: &MediaAttachment,
    ) -> Result<Vec<u8>, MediaError> {
        self.process_media_for_analysis_with_progress(media, None).await
    }

    /// Process media attachment with optional progress callback for streaming processing
    pub async fn process_media_for_analysis_with_progress(
        &self,
        media: &MediaAttachment,
        progress_callback: Option<Box<dyn FnMut(&str) + Send + Sync>>,
    ) -> Result<Vec<u8>, MediaError> {
        // Check if media is supported and needs processing
        if !self.transformer.is_supported(&media.media_type) {
            return Err(MediaError::UnsupportedType {
                media_type: media.media_type.clone(),
            });
        }

        if !self.transformer.needs_description(media) {
            return Err(MediaError::ProcessingFailed(
                "Media already has description".to_string(),
            ));
        }

        // Download media data with streaming support
        let media_data = self.download_media(&media.url).await?;

        // Transform for analysis with progress callback
        self.transformer.transform_for_analysis_with_progress(&media_data, progress_callback)
    }

    /// Download media from an attachment and return the raw bytes for re-upload
    pub async fn download_media_for_recreation(
        &self,
        media: &MediaAttachment,
    ) -> Result<Vec<u8>, MediaError> {
        // Check if media is supported
        if !self.transformer.is_supported(&media.media_type) {
            return Err(MediaError::UnsupportedType {
                media_type: media.media_type.clone(),
            });
        }

        // Download the original media data (not transformed for analysis)
        self.download_media(&media.url).await
    }

    /// Get statistics about media attachments
    #[allow(dead_code)] // Public API method, may be used in future
    pub fn get_media_stats(&self, media_attachments: &[MediaAttachment]) -> MediaStats {
        let total = media_attachments.len();
        let supported = media_attachments
            .iter()
            .filter(|m| self.transformer.is_supported(&m.media_type))
            .count();
        let needs_description = media_attachments
            .iter()
            .filter(|m| {
                self.transformer.is_supported(&m.media_type)
                    && self.transformer.needs_description(m)
            })
            .count();
        let processable = self.filter_processable_media(media_attachments).len();

        MediaStats {
            total,
            supported,
            needs_description,
            processable,
        }
    }
}

/// Statistics about media processing
#[allow(dead_code)] // Stats struct for API completeness
#[derive(Debug, Clone)]
pub struct MediaStats {
    pub total: usize,
    pub supported: usize,
    pub needs_description: usize,
    pub processable: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn test_media_config_default() {
        let config = MediaConfig::default();
        assert_eq!(config.max_size_mb, DEFAULT_MAX_SIZE_MB);
        assert_eq!(config.max_dimension, image::DEFAULT_MAX_DIMENSION);
        assert!(config.supported_formats.contains("image/jpeg"));
        assert!(config.supported_formats.contains("image/png"));
        assert!(config.supported_formats.contains("image/gif"));
        assert!(config.supported_formats.contains("image/webp"));
        // Should also contain audio formats
        assert!(config.supported_formats.contains("audio/mp3"));
        assert!(config.supported_formats.contains("audio/wav"));
        // Should also contain video formats
        assert!(config.supported_formats.contains("video/mp4"));
        assert!(config.supported_formats.contains("video/webm"));
    }

    #[test]
    fn test_unified_transformer_is_supported() {
        let transformer = UnifiedMediaTransformer::with_default_config();

        // Supported image formats
        assert!(transformer.is_supported("image/jpeg"));
        assert!(transformer.is_supported("image/png"));
        assert!(transformer.is_supported("image/gif"));
        assert!(transformer.is_supported("image/webp"));

        // Supported audio formats
        assert!(transformer.is_supported("audio/mp3"));
        assert!(transformer.is_supported("audio/wav"));
        assert!(transformer.is_supported("audio/flac"));

        // Supported video formats
        assert!(transformer.is_supported("video/mp4"));
        assert!(transformer.is_supported("video/webm"));
        assert!(transformer.is_supported("video/quicktime"));

        // Generic type matching (Mastodon API format)
        assert!(transformer.is_supported("image"));
        assert!(transformer.is_supported("audio"));
        assert!(transformer.is_supported("video"));

        // Unsupported formats
        assert!(!transformer.is_supported("text/plain"));
        assert!(!transformer.is_supported("application/pdf"));
    }

    #[test]
    fn test_unified_transformer_needs_description() {
        let transformer = UnifiedMediaTransformer::with_default_config();

        // Image needs description
        let media1 = create_test_media("1", "image/jpeg", None);
        assert!(transformer.needs_description(&media1));

        // Audio needs description
        let media2 = create_test_media("2", "audio/mp3", None);
        assert!(transformer.needs_description(&media2));

        // Video needs description
        let media3 = create_test_media("3", "video/mp4", None);
        assert!(transformer.needs_description(&media3));

        // Has description
        let media4 = create_test_media("4", "image/png", Some("A beautiful sunset".to_string()));
        assert!(!transformer.needs_description(&media4));

        // Unsupported type
        let media5 = create_test_media("5", "text/plain", None);
        assert!(!transformer.needs_description(&media5));
    }

    #[test]
    fn test_media_processor_filter_processable_media() {
        let processor = MediaProcessor::with_default_config();

        let media_attachments = vec![
            create_test_media("1", "image/jpeg", None), // Should be included
            create_test_media("2", "image/png", Some("Has description".to_string())), // Should be excluded
            create_test_media("3", "text/plain", None), // Should be excluded (unsupported)
            create_test_media("4", "image/gif", Some("".to_string())), // Should be included (empty description)
            create_test_media("5", "audio/mp3", None),                 // Should be included
            create_test_media("6", "video/mp4", None),                 // Should be included
        ];

        let processable = processor.filter_processable_media(&media_attachments);

        assert_eq!(processable.len(), 4);
        assert_eq!(processable[0].id, "1");
        assert_eq!(processable[1].id, "4");
        assert_eq!(processable[2].id, "5");
        assert_eq!(processable[3].id, "6");
    }

    #[test]
    fn test_media_processor_get_media_stats() {
        let processor = MediaProcessor::with_default_config();

        let media_attachments = vec![
            create_test_media("1", "image/jpeg", None), // Supported, needs description, processable
            create_test_media("2", "image/png", Some("Has description".to_string())), // Supported, has description
            create_test_media("3", "video/mp4", None), // Supported, needs description, processable
            create_test_media("4", "image/gif", Some("".to_string())), // Supported, needs description, processable
            create_test_media("5", "audio/mp3", None), // Supported, needs description, processable
        ];

        let stats = processor.get_media_stats(&media_attachments);

        assert_eq!(stats.total, 5);
        assert_eq!(stats.supported, 5); // JPEG, PNG, GIF, MP3, MP4
        assert_eq!(stats.needs_description, 4); // JPEG (none), GIF (empty), MP3 (none), MP4 (none)
        assert_eq!(stats.processable, 4); // JPEG, GIF, MP3, and MP4
    }

    #[test]
    fn test_media_stats_display() {
        let stats = MediaStats {
            total: 10,
            supported: 8,
            needs_description: 5,
            processable: 3,
        };

        // Test that stats can be formatted
        let debug_str = format!("{stats:?}");
        assert!(debug_str.contains("total: 10"));
        assert!(debug_str.contains("supported: 8"));
        assert!(debug_str.contains("needs_description: 5"));
        assert!(debug_str.contains("processable: 3"));
    }

    #[test]
    fn test_video_support_debug() {
        let config = MediaConfig::default();
        let transformer = UnifiedMediaTransformer::new(config.clone());

        // Print supported formats for debugging
        println!("All supported formats:");
        let mut video_formats = Vec::new();
        for format in &config.supported_formats {
            if format.starts_with("video/") {
                video_formats.push(format.clone());
                println!("  {format}");
            }
        }

        println!("Found {} video formats", video_formats.len());
        assert!(!video_formats.is_empty(), "Should have video formats");

        // Test generic video type with debugging
        println!("Testing generic 'video' type support...");
        let video_supported = transformer.is_supported("video");
        println!("Result: {video_supported}");
        assert!(video_supported, "Generic 'video' type should be supported");

        // Test case variations
        assert!(transformer.is_supported("Video"));
        assert!(transformer.is_supported("VIDEO"));
        assert!(transformer.is_supported("video"));
    }
}
