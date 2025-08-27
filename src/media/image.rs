use crate::error::MediaError;
use crate::mastodon::MediaAttachment;
use image::{codecs::jpeg::JpegEncoder, codecs::png::PngEncoder, DynamicImage, GenericImageView};
use std::collections::HashSet;

// Import and re-export ImageFormat for external use
pub use image::ImageFormat;

/// Supported image formats for processing
pub const SUPPORTED_IMAGE_FORMATS: &[&str] = &[
    "image/jpeg",
    "image/jpg",
    "image/png",
    "image/gif",
    "image/webp",
    "image/tiff",
    "image/bmp",
    "image/x-icon",
    "image/avif",
];

/// Maximum dimension for image resizing (width or height)
pub const DEFAULT_MAX_DIMENSION: u32 = 2048;

/// Configuration for image processing
#[derive(Debug, Clone)]
pub struct ImageConfig {
    pub max_size_mb: f64,
    pub max_dimension: u32,
    #[allow(dead_code)]
    // Used in runtime logic but clippy may not detect it in --all-targets mode
    pub supported_formats: HashSet<String>,
}

impl Default for ImageConfig {
    fn default() -> Self {
        let mut supported_formats = HashSet::new();

        // Add image formats
        for format in SUPPORTED_IMAGE_FORMATS {
            supported_formats.insert(format.to_string());
        }

        Self {
            max_size_mb: 10.0, // Default from media.rs
            max_dimension: DEFAULT_MAX_DIMENSION,
            supported_formats,
        }
    }
}

/// Trait for image transformation operations
pub trait ImageTransformer {
    /// Check if a media type is supported for processing
    #[allow(dead_code)]
    // Used in trait implementations but clippy may not detect it in --all-targets mode
    fn is_supported(&self, media_type: &str) -> bool;

    /// Transform image data for analysis (resize, optimize)
    fn transform_for_analysis(&self, image_data: &[u8]) -> Result<Vec<u8>, MediaError>;

    /// Check if media attachment needs a description
    #[allow(dead_code)] // Used in trait implementation, may be needed by external trait users
    fn needs_description(&self, media: &MediaAttachment) -> bool;

    /// Get optimal format for transformed image
    fn get_optimal_format(&self, original_format: ImageFormat) -> ImageFormat;
}

/// Image transformer implementation
pub struct ImageProcessor {
    config: ImageConfig,
}

impl ImageProcessor {
    pub fn new(config: ImageConfig) -> Self {
        Self { config }
    }

    #[allow(dead_code)] // Convenience constructor for tests
    pub fn with_default_config() -> Self {
        Self::new(ImageConfig::default())
    }

    /// Detect image format from raw data
    fn detect_format(&self, data: &[u8]) -> Result<ImageFormat, MediaError> {
        image::guess_format(data)
            .map_err(|e| MediaError::DecodingFailed(format!("Failed to detect image format: {e}")))
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

    /// Public method: Transform image data for analysis (resize, optimize)
    pub fn transform_for_analysis(&self, image_data: &[u8]) -> Result<Vec<u8>, MediaError> {
        <Self as ImageTransformer>::transform_for_analysis(self, image_data)
    }

    /// Public method: Get optimal format for transformed image
    pub fn get_optimal_format(&self, original_format: ImageFormat) -> ImageFormat {
        <Self as ImageTransformer>::get_optimal_format(self, original_format)
    }

    /// Public method: Check if a media type is supported for processing
    #[allow(dead_code)]
    // Used in trait implementation but clippy may not detect it in --all-targets mode
    pub fn is_supported(&self, media_type: &str) -> bool {
        <Self as ImageTransformer>::is_supported(self, media_type)
    }
}

impl ImageTransformer for ImageProcessor {
    fn is_supported(&self, media_type: &str) -> bool {
        let media_type_lower = media_type.to_lowercase();

        // Check if it's already a MIME type that we support
        if self.config.supported_formats.contains(&media_type_lower) {
            return true;
        }

        // Handle Mastodon API format where type is just "image", "video", etc.
        // For "image" type, we support it if we support any image format
        match media_type_lower.as_str() {
            "image" => self
                .config
                .supported_formats
                .iter()
                .any(|f| f.starts_with("image/")),
            _ => false,
        }
    }

    fn transform_for_analysis(&self, image_data: &[u8]) -> Result<Vec<u8>, MediaError> {
        // Check size limits first
        self.check_size_limits(image_data)?;

        // Detect and validate format
        let format = self.detect_format(image_data)?;

        // Load image
        let img = image::load_from_memory(image_data)
            .map_err(|e| MediaError::DecodingFailed(format!("Failed to decode image: {e}")))?;

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
                    MediaError::EncodingFailed(format!("Failed to encode PNG: {e}"))
                })?;
            }
            ImageFormat::Jpeg => {
                // Convert RGBA to RGB if necessary for JPEG compatibility
                let rgb_img = match resized_img.color() {
                    image::ColorType::Rgba8 | image::ColorType::Rgba16 => {
                        // Convert RGBA to RGB by converting the image data
                        DynamicImage::ImageRgb8(resized_img.to_rgb8())
                    }
                    _ => resized_img,
                };

                let encoder = JpegEncoder::new_with_quality(&mut output, 65);
                rgb_img.write_with_encoder(encoder).map_err(|e| {
                    MediaError::EncodingFailed(format!("Failed to encode JPEG: {e}"))
                })?;
            }
            _ => {
                // Fallback to PNG for other formats
                let encoder = PngEncoder::new(&mut output);
                resized_img.write_with_encoder(encoder).map_err(|e| {
                    MediaError::EncodingFailed(format!("Failed to encode fallback PNG: {e}"))
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

    fn get_optimal_format(&self, _original_format: ImageFormat) -> ImageFormat {
        // Convert all formats to JPEG for OpenRouter analysis to reduce file size
        // and ensure maximum compatibility
        ImageFormat::Jpeg
    }
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
    fn test_image_config_default() {
        let config = ImageConfig::default();
        assert_eq!(config.max_size_mb, 10.0);
        assert_eq!(config.max_dimension, DEFAULT_MAX_DIMENSION);
        assert!(config.supported_formats.contains("image/jpeg"));
        assert!(config.supported_formats.contains("image/png"));
        assert!(config.supported_formats.contains("image/gif"));
        assert!(config.supported_formats.contains("image/webp"));
        assert!(config.supported_formats.contains("image/tiff"));
        assert!(config.supported_formats.contains("image/bmp"));
        assert!(config.supported_formats.contains("image/x-icon"));
        assert!(config.supported_formats.contains("image/avif"));
    }

    #[test]
    fn test_image_processor_is_supported() {
        let processor = ImageProcessor::with_default_config();

        // Supported image formats
        assert!(processor.is_supported("image/jpeg"));
        assert!(processor.is_supported("image/png"));
        assert!(processor.is_supported("image/gif"));
        assert!(processor.is_supported("image/webp"));
        assert!(processor.is_supported("image/tiff"));
        assert!(processor.is_supported("image/bmp"));
        assert!(processor.is_supported("image/x-icon"));
        assert!(processor.is_supported("image/avif"));

        // Generic type matching (Mastodon API format)
        assert!(processor.is_supported("image"));
        assert!(processor.is_supported("IMAGE"));
        assert!(processor.is_supported("Image"));

        // Unsupported formats
        assert!(!processor.is_supported("video/mp4"));
        assert!(!processor.is_supported("audio/mp3"));
        assert!(!processor.is_supported("text/plain"));
        assert!(!processor.is_supported("application/pdf"));
        assert!(!processor.is_supported("video"));
        assert!(!processor.is_supported("audio"));
    }

    #[test]
    fn test_image_processor_needs_description() {
        let processor = ImageProcessor::with_default_config();

        // Needs description - no description
        let media1 = create_test_media("1", "image/jpeg", None);
        assert!(processor.needs_description(&media1));

        // Needs description - empty description
        let media2 = create_test_media("2", "image/png", Some("".to_string()));
        assert!(processor.needs_description(&media2));

        // Needs description - whitespace only
        let media3 = create_test_media("3", "image/gif", Some("   \n\t  ".to_string()));
        assert!(processor.needs_description(&media3));

        // Has description
        let media4 = create_test_media("4", "image/webp", Some("A beautiful sunset".to_string()));
        assert!(!processor.needs_description(&media4));

        // Unsupported type
        let media5 = create_test_media("5", "video/mp4", None);
        assert!(!processor.needs_description(&media5));
    }

    #[test]
    fn test_image_processor_get_optimal_format() {
        let processor = ImageProcessor::with_default_config();

        // All formats should convert to JPEG for OpenRouter optimization
        assert!(matches!(
            processor.get_optimal_format(ImageFormat::Png),
            ImageFormat::Jpeg
        ));

        assert!(matches!(
            processor.get_optimal_format(ImageFormat::Gif),
            ImageFormat::Jpeg
        ));

        assert!(matches!(
            processor.get_optimal_format(ImageFormat::WebP),
            ImageFormat::Jpeg
        ));

        assert!(matches!(
            processor.get_optimal_format(ImageFormat::Jpeg),
            ImageFormat::Jpeg
        ));

        assert!(matches!(
            processor.get_optimal_format(ImageFormat::Tiff),
            ImageFormat::Jpeg
        ));

        assert!(matches!(
            processor.get_optimal_format(ImageFormat::Bmp),
            ImageFormat::Jpeg
        ));

        assert!(matches!(
            processor.get_optimal_format(ImageFormat::Ico),
            ImageFormat::Jpeg
        ));

        assert!(matches!(
            processor.get_optimal_format(ImageFormat::Avif),
            ImageFormat::Jpeg
        ));
    }

    #[test]
    fn test_image_processor_check_size_limits() {
        let config = ImageConfig {
            max_size_mb: 1.0, // 1MB limit
            max_dimension: 2048,
            supported_formats: SUPPORTED_IMAGE_FORMATS
                .iter()
                .map(|s| s.to_string())
                .collect(),
        };
        let processor = ImageProcessor::new(config);

        // Small data should pass
        let small_data = vec![0u8; 500_000]; // 500KB
        assert!(processor.check_size_limits(&small_data).is_ok());

        // Large data should fail
        let large_data = vec![0u8; 2_000_000]; // 2MB
        assert!(processor.check_size_limits(&large_data).is_err());
    }

    #[test]
    fn test_image_processor_detect_format_invalid() {
        let processor = ImageProcessor::with_default_config();

        // Invalid image data
        let invalid_data = b"not an image";
        let result = processor.detect_format(invalid_data);
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), MediaError::DecodingFailed(_)));
    }

    #[test]
    fn test_supported_image_formats() {
        assert!(SUPPORTED_IMAGE_FORMATS.contains(&"image/jpeg"));
        assert!(SUPPORTED_IMAGE_FORMATS.contains(&"image/png"));
        assert!(SUPPORTED_IMAGE_FORMATS.contains(&"image/gif"));
        assert!(SUPPORTED_IMAGE_FORMATS.contains(&"image/webp"));
        assert_eq!(SUPPORTED_IMAGE_FORMATS.len(), 9);
    }

    #[test]
    fn test_transform_rgba_to_jpeg() {
        let processor = ImageProcessor::with_default_config();

        // Create a small RGBA PNG image with transparency
        let rgba_img = image::DynamicImage::new_rgba8(4, 4);
        let mut png_data = Vec::new();
        let encoder = PngEncoder::new(&mut png_data);
        rgba_img.write_with_encoder(encoder).unwrap();

        // Transform should succeed and convert RGBA to RGB for JPEG
        let result = processor.transform_for_analysis(&png_data);
        assert!(
            result.is_ok(),
            "RGBA image should be successfully converted to JPEG"
        );

        // Verify the output is valid JPEG data
        let output_data = result.unwrap();
        assert!(!output_data.is_empty());

        // Should be able to load the result as an image
        let loaded_img = image::load_from_memory(&output_data);
        assert!(loaded_img.is_ok(), "Output should be valid image data");

        // The loaded image should be RGB (no alpha channel)
        let loaded_img = loaded_img.unwrap();
        match loaded_img.color() {
            image::ColorType::Rgb8 => {} // Expected
            other => panic!("Expected RGB8, got {:?}", other),
        }
    }

    #[test]
    fn test_transform_rgb_to_jpeg() {
        let processor = ImageProcessor::with_default_config();

        // Create a small RGB image (no alpha)
        let rgb_img = image::DynamicImage::new_rgb8(4, 4);
        let mut jpeg_data = Vec::new();
        let encoder = JpegEncoder::new_with_quality(&mut jpeg_data, 85);
        rgb_img.write_with_encoder(encoder).unwrap();

        // Transform should succeed without needing conversion
        let result = processor.transform_for_analysis(&jpeg_data);
        assert!(
            result.is_ok(),
            "RGB image should be successfully processed as JPEG"
        );

        let output_data = result.unwrap();
        assert!(!output_data.is_empty());

        // Should be able to load the result as an image
        let loaded_img = image::load_from_memory(&output_data);
        assert!(loaded_img.is_ok(), "Output should be valid image data");
    }
}
