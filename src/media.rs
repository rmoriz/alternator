use crate::error::MediaError;
use crate::mastodon::MediaAttachment;
use image::{
    codecs::jpeg::JpegEncoder, codecs::png::PngEncoder, DynamicImage, GenericImageView, ImageFormat,
};

use std::collections::HashSet;

/// Supported image formats for processing
const SUPPORTED_IMAGE_FORMATS: &[&str] = &[
    "image/jpeg",
    "image/jpg",
    "image/png",
    "image/gif",
    "image/webp",
];

/// Maximum dimension for image resizing (width or height)
const DEFAULT_MAX_DIMENSION: u32 = 1024;

/// Maximum file size in MB for processing
const DEFAULT_MAX_SIZE_MB: f64 = 10.0;

// MediaAttachment is imported from crate::mastodon

// MediaMeta, MediaDimensions are imported from crate::mastodon

/// Configuration for media processing
#[derive(Debug, Clone)]
pub struct MediaConfig {
    pub max_size_mb: f64,
    pub max_dimension: u32,
    pub supported_formats: HashSet<String>,
}

impl Default for MediaConfig {
    fn default() -> Self {
        Self {
            max_size_mb: DEFAULT_MAX_SIZE_MB,
            max_dimension: DEFAULT_MAX_DIMENSION,
            supported_formats: SUPPORTED_IMAGE_FORMATS
                .iter()
                .map(|s| s.to_string())
                .collect(),
        }
    }
}

/// Trait for media transformation operations
pub trait MediaTransformer {
    /// Check if a media type is supported for processing
    fn is_supported(&self, media_type: &str) -> bool;

    /// Transform image data for analysis (resize, optimize)
    fn transform_for_analysis(&self, image_data: &[u8]) -> Result<Vec<u8>, MediaError>;

    /// Check if media attachment needs a description
    fn needs_description(&self, media: &MediaAttachment) -> bool;

    /// Get optimal format for transformed image
    fn get_optimal_format(&self, original_format: ImageFormat) -> ImageFormat;
}

/// Image transformer implementation
pub struct ImageTransformer {
    config: MediaConfig,
}

impl ImageTransformer {
    pub fn new(config: MediaConfig) -> Self {
        Self { config }
    }

    pub fn with_default_config() -> Self {
        Self::new(MediaConfig::default())
    }

    /// Detect image format from raw data
    fn detect_format(&self, data: &[u8]) -> Result<ImageFormat, MediaError> {
        image::guess_format(data).map_err(|e| {
            MediaError::DecodingFailed(format!("Failed to detect image format: {}", e))
        })
    }

    /// Resize image if it exceeds maximum dimensions
    fn resize_if_needed(&self, img: DynamicImage) -> DynamicImage {
        let (width, height) = img.dimensions();
        let max_dim = self.config.max_dimension;

        if width <= max_dim && height <= max_dim {
            return img;
        }

        // Calculate new dimensions maintaining aspect ratio
        let (new_width, new_height) = if width > height {
            let ratio = max_dim as f64 / width as f64;
            (max_dim, (height as f64 * ratio) as u32)
        } else {
            let ratio = max_dim as f64 / height as f64;
            ((width as f64 * ratio) as u32, max_dim)
        };

        img.resize(new_width, new_height, image::imageops::FilterType::Lanczos3)
    }

    /// Check if image data size is within limits
    fn check_size_limits(&self, data: &[u8]) -> Result<(), MediaError> {
        let size_mb = data.len() as f64 / (1024.0 * 1024.0);
        if size_mb > self.config.max_size_mb {
            return Err(MediaError::ProcessingFailed(format!(
                "Image size {:.2}MB exceeds limit of {:.2}MB",
                size_mb, self.config.max_size_mb
            )));
        }
        Ok(())
    }
}

impl MediaTransformer for ImageTransformer {
    fn is_supported(&self, media_type: &str) -> bool {
        self.config
            .supported_formats
            .contains(&media_type.to_lowercase())
    }

    fn transform_for_analysis(&self, image_data: &[u8]) -> Result<Vec<u8>, MediaError> {
        // Check size limits first
        self.check_size_limits(image_data)?;

        // Detect and validate format
        let format = self.detect_format(image_data)?;

        // Load image
        let img = image::load_from_memory(image_data)
            .map_err(|e| MediaError::DecodingFailed(format!("Failed to decode image: {}", e)))?;

        // Resize if needed
        let resized_img = self.resize_if_needed(img);

        // Get optimal output format
        let output_format = self.get_optimal_format(format);

        // Encode to bytes
        let mut output = Vec::new();
        match output_format {
            ImageFormat::Png => {
                let encoder = PngEncoder::new(&mut output);
                resized_img.write_with_encoder(encoder).map_err(|e| {
                    MediaError::EncodingFailed(format!("Failed to encode PNG: {}", e))
                })?;
            }
            ImageFormat::Jpeg => {
                let encoder = JpegEncoder::new_with_quality(&mut output, 85);
                resized_img.write_with_encoder(encoder).map_err(|e| {
                    MediaError::EncodingFailed(format!("Failed to encode JPEG: {}", e))
                })?;
            }
            _ => {
                // Fallback to PNG for other formats
                let encoder = PngEncoder::new(&mut output);
                resized_img.write_with_encoder(encoder).map_err(|e| {
                    MediaError::EncodingFailed(format!("Failed to encode fallback PNG: {}", e))
                })?;
            }
        }

        Ok(output)
    }

    fn needs_description(&self, media: &MediaAttachment) -> bool {
        // Check if it's a supported media type
        if !self.is_supported(&media.media_type) {
            return false;
        }

        // Check if description is missing or empty
        match &media.description {
            None => true,
            Some(desc) => desc.trim().is_empty(),
        }
    }

    fn get_optimal_format(&self, original_format: ImageFormat) -> ImageFormat {
        match original_format {
            ImageFormat::Png => ImageFormat::Png,
            ImageFormat::Gif => ImageFormat::Png, // Convert GIF to PNG for analysis
            ImageFormat::WebP => ImageFormat::Png, // Convert WebP to PNG for better compatibility
            _ => ImageFormat::Jpeg,               // Use JPEG for other formats
        }
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

    pub fn with_image_transformer(config: MediaConfig) -> Self {
        Self::new(Box::new(ImageTransformer::new(config)))
    }

    pub fn with_default_config() -> Self {
        Self::with_image_transformer(MediaConfig::default())
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

    /// Download media from URL
    pub async fn download_media(&self, url: &str) -> Result<Vec<u8>, MediaError> {
        let response =
            self.http_client
                .get(url)
                .send()
                .await
                .map_err(|_e| MediaError::DownloadFailed {
                    url: url.to_string(),
                })?;

        if !response.status().is_success() {
            return Err(MediaError::DownloadFailed {
                url: url.to_string(),
            });
        }

        let bytes = response
            .bytes()
            .await
            .map_err(|_| MediaError::DownloadFailed {
                url: url.to_string(),
            })?;

        Ok(bytes.to_vec())
    }

    /// Process media attachment: download, transform, and prepare for analysis
    pub async fn process_media_for_analysis(
        &self,
        media: &MediaAttachment,
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

        // Download media data
        let media_data = self.download_media(&media.url).await?;

        // Transform for analysis
        self.transformer.transform_for_analysis(&media_data)
    }

    /// Get statistics about media attachments
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
            url: format!("https://example.com/media/{}", id),
            preview_url: None,
            description,
            meta: None,
        }
    }

    #[test]
    fn test_media_config_default() {
        let config = MediaConfig::default();
        assert_eq!(config.max_size_mb, DEFAULT_MAX_SIZE_MB);
        assert_eq!(config.max_dimension, DEFAULT_MAX_DIMENSION);
        assert!(config.supported_formats.contains("image/jpeg"));
        assert!(config.supported_formats.contains("image/png"));
        assert!(config.supported_formats.contains("image/gif"));
        assert!(config.supported_formats.contains("image/webp"));
    }

    #[test]
    fn test_image_transformer_is_supported() {
        let transformer = ImageTransformer::with_default_config();

        // Supported formats
        assert!(transformer.is_supported("image/jpeg"));
        assert!(transformer.is_supported("image/jpg"));
        assert!(transformer.is_supported("image/png"));
        assert!(transformer.is_supported("image/gif"));
        assert!(transformer.is_supported("image/webp"));

        // Case insensitive
        assert!(transformer.is_supported("IMAGE/JPEG"));
        assert!(transformer.is_supported("Image/PNG"));

        // Unsupported formats
        assert!(!transformer.is_supported("video/mp4"));
        assert!(!transformer.is_supported("audio/mp3"));
        assert!(!transformer.is_supported("text/plain"));
        assert!(!transformer.is_supported("application/pdf"));
    }

    #[test]
    fn test_image_transformer_needs_description() {
        let transformer = ImageTransformer::with_default_config();

        // Needs description - no description
        let media1 = create_test_media("1", "image/jpeg", None);
        assert!(transformer.needs_description(&media1));

        // Needs description - empty description
        let media2 = create_test_media("2", "image/png", Some("".to_string()));
        assert!(transformer.needs_description(&media2));

        // Needs description - whitespace only
        let media3 = create_test_media("3", "image/gif", Some("   \n\t  ".to_string()));
        assert!(transformer.needs_description(&media3));

        // Has description
        let media4 = create_test_media("4", "image/webp", Some("A beautiful sunset".to_string()));
        assert!(!transformer.needs_description(&media4));

        // Unsupported type
        let media5 = create_test_media("5", "video/mp4", None);
        assert!(!transformer.needs_description(&media5));
    }

    #[test]
    fn test_image_transformer_get_optimal_format() {
        let transformer = ImageTransformer::with_default_config();

        // PNG should stay PNG
        assert!(matches!(
            transformer.get_optimal_format(ImageFormat::Png),
            ImageFormat::Png
        ));

        // GIF should convert to PNG
        assert!(matches!(
            transformer.get_optimal_format(ImageFormat::Gif),
            ImageFormat::Png
        ));

        // WebP should convert to PNG
        assert!(matches!(
            transformer.get_optimal_format(ImageFormat::WebP),
            ImageFormat::Png
        ));

        // JPEG should stay JPEG
        assert!(matches!(
            transformer.get_optimal_format(ImageFormat::Jpeg),
            ImageFormat::Jpeg
        ));
    }

    #[test]
    fn test_media_processor_filter_processable_media() {
        let processor = MediaProcessor::with_default_config();

        let media_attachments = vec![
            create_test_media("1", "image/jpeg", None), // Should be included
            create_test_media("2", "image/png", Some("Has description".to_string())), // Should be excluded
            create_test_media("3", "video/mp4", None), // Should be excluded (unsupported)
            create_test_media("4", "image/gif", Some("".to_string())), // Should be included (empty description)
            create_test_media("5", "image/webp", None),                // Should be included
        ];

        let processable = processor.filter_processable_media(&media_attachments);

        assert_eq!(processable.len(), 3);
        assert_eq!(processable[0].id, "1");
        assert_eq!(processable[1].id, "4");
        assert_eq!(processable[2].id, "5");
    }

    #[test]
    fn test_media_processor_get_media_stats() {
        let processor = MediaProcessor::with_default_config();

        let media_attachments = vec![
            create_test_media("1", "image/jpeg", None), // Supported, needs description, processable
            create_test_media("2", "image/png", Some("Has description".to_string())), // Supported, has description
            create_test_media("3", "video/mp4", None),                                // Unsupported
            create_test_media("4", "image/gif", Some("".to_string())), // Supported, needs description, processable
            create_test_media("5", "audio/mp3", None),                 // Unsupported
        ];

        let stats = processor.get_media_stats(&media_attachments);

        assert_eq!(stats.total, 5);
        assert_eq!(stats.supported, 3); // JPEG, PNG, GIF
        assert_eq!(stats.needs_description, 2); // JPEG (none), GIF (empty) - only supported ones that need descriptions
        assert_eq!(stats.processable, 2); // JPEG and GIF
    }

    #[test]
    fn test_image_transformer_check_size_limits() {
        let config = MediaConfig {
            max_size_mb: 1.0, // 1MB limit
            max_dimension: 1024,
            supported_formats: SUPPORTED_IMAGE_FORMATS
                .iter()
                .map(|s| s.to_string())
                .collect(),
        };
        let transformer = ImageTransformer::new(config);

        // Small data should pass
        let small_data = vec![0u8; 500_000]; // 500KB
        assert!(transformer.check_size_limits(&small_data).is_ok());

        // Large data should fail
        let large_data = vec![0u8; 2_000_000]; // 2MB
        assert!(transformer.check_size_limits(&large_data).is_err());
    }

    #[test]
    fn test_image_transformer_detect_format_invalid() {
        let transformer = ImageTransformer::with_default_config();

        // Invalid image data
        let invalid_data = b"not an image";
        let result = transformer.detect_format(invalid_data);
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), MediaError::DecodingFailed(_)));
    }

    #[test]
    fn test_media_attachment_serialization() {
        let media = MediaAttachment {
            id: "123".to_string(),
            media_type: "image/jpeg".to_string(),
            url: "https://example.com/image.jpg".to_string(),
            preview_url: Some("https://example.com/preview.jpg".to_string()),
            description: Some("A test image".to_string()),
            meta: Some(crate::mastodon::MediaMeta {
                original: Some(crate::mastodon::MediaDimensions {
                    width: Some(1920),
                    height: Some(1080),
                    size: Some("1920x1080".to_string()),
                    aspect: Some(1.777777777777778),
                }),
                small: None,
            }),
        };

        // Test serialization
        let json = serde_json::to_string(&media).unwrap();
        assert!(json.contains("\"id\":\"123\""));
        assert!(json.contains("\"type\":\"image/jpeg\""));

        // Test deserialization
        let deserialized: MediaAttachment = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.id, "123");
        assert_eq!(deserialized.media_type, "image/jpeg");
        assert_eq!(deserialized.description, Some("A test image".to_string()));
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
        let debug_str = format!("{:?}", stats);
        assert!(debug_str.contains("total: 10"));
        assert!(debug_str.contains("supported: 8"));
        assert!(debug_str.contains("needs_description: 5"));
        assert!(debug_str.contains("processable: 3"));
    }
}
