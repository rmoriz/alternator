use crate::config::WhisperConfig;
use crate::error::AlternatorError;
use indicatif::{ProgressBar, ProgressStyle};
use std::env;
use std::path::{Path, PathBuf};
use tokio::fs;
use tokio::io::AsyncWriteExt;
use tracing::info;

/// Whisper model manager for downloading and managing models
pub struct WhisperModelManager {
    model_dir: PathBuf,
}

impl WhisperModelManager {
    #[allow(clippy::result_large_err)]
    pub fn new(config: WhisperConfig) -> Result<Self, AlternatorError> {
        let model_dir = Self::get_model_directory(&config)?;
        Ok(Self { model_dir })
    }

    /// Get the model directory, using config, environment variable, or default
    #[allow(clippy::result_large_err)]
    fn get_model_directory(config: &WhisperConfig) -> Result<PathBuf, AlternatorError> {
        // Check environment variable first
        if let Ok(env_dir) = env::var("ALTERNATOR_WHISPER_MODEL_DIR") {
            return Ok(PathBuf::from(env_dir));
        }

        // Check config
        if let Some(ref config_dir) = config.model_dir {
            return Ok(PathBuf::from(config_dir));
        }

        // Use default: ~/.alternator/models/
        let home_dir = dirs::home_dir().ok_or_else(|| {
            AlternatorError::InvalidData("Could not determine home directory".to_string())
        })?;

        Ok(home_dir.join(".alternator").join("models"))
    }

    /// Check if the configured model exists locally
    #[allow(dead_code)] // Legacy method, may be used for compatibility
    pub fn model_exists(&self, model_name: &str) -> bool {
        let model_file = self.get_model_path(model_name);
        model_file.exists()
    }

    /// Ensure model is available, download if necessary
    #[allow(dead_code)] // Legacy method, may be used for compatibility
    pub async fn ensure_model_available(
        &self,
        model_name: &str,
    ) -> Result<PathBuf, AlternatorError> {
        let model_file = self.get_model_path(model_name);

        if model_file.exists() {
            info!(
                "Whisper model '{}' found at {}",
                model_name,
                model_file.display()
            );
            return Ok(model_file);
        }

        info!(
            "Whisper model '{}' not found, starting download...",
            model_name
        );
        self.download_model(model_name).await
    }
    /// Download a Whisper model if it doesn't exist or update if newer version available
    pub async fn download_model(&self, model_name: &str) -> Result<PathBuf, AlternatorError> {
        let model_file = self.get_model_path(model_name);

        // Create model directory if it doesn't exist
        if let Some(parent) = model_file.parent() {
            fs::create_dir_all(parent).await.map_err(|e| {
                AlternatorError::InvalidData(format!(
                    "Failed to create model directory {}: {}",
                    parent.display(),
                    e
                ))
            })?;
        }

        // Check if model already exists
        if model_file.exists() {
            info!(
                "Model {} already exists at {}",
                model_name,
                model_file.display()
            );

            // TODO: Add version checking and update logic
            if self.should_update_model(&model_file).await? {
                info!("Updating model {} to newer version", model_name);
                self.download_model_file(model_name, &model_file).await?;
            } else {
                info!("Model {} is up to date", model_name);
            }

            return Ok(model_file);
        }

        info!("Downloading Whisper model: {}", model_name);
        self.download_model_file(model_name, &model_file).await?;

        Ok(model_file)
    }

    /// Get the full path for a model file
    fn get_model_path(&self, model_name: &str) -> PathBuf {
        self.model_dir.join(format!("ggml-{model_name}.bin"))
    }

    /// Check if model should be updated (placeholder for future version checking)
    async fn should_update_model(&self, _model_file: &Path) -> Result<bool, AlternatorError> {
        // For now, we don't update existing models
        // TODO: Implement version checking against remote repository
        Ok(false)
    }

    /// Download the actual model file from OpenAI's repository with progress bar
    async fn download_model_file(
        &self,
        model_name: &str,
        target_path: &Path,
    ) -> Result<(), AlternatorError> {
        let model_url = self.get_model_url(model_name)?;

        info!("Downloading from: {}", model_url);
        info!("Saving to: {}", target_path.display());

        let client = reqwest::Client::new();
        let response = client
            .get(&model_url)
            .send()
            .await
            .map_err(AlternatorError::Network)?;

        if !response.status().is_success() {
            return Err(AlternatorError::InvalidData(format!(
                "Failed to download model: HTTP {}",
                response.status()
            )));
        }

        let total_size = response.content_length();

        // Create progress bar
        let progress_bar = if let Some(size) = total_size {
            info!("Model size: {:.2} MB", size as f64 / 1_048_576.0);
            let pb = ProgressBar::new(size);
            pb.set_style(
                ProgressStyle::default_bar()
                    .template(
                        "{msg} [{bar:40.cyan/blue}] {bytes}/{total_bytes} ({bytes_per_sec}, {eta})",
                    )
                    .unwrap()
                    .progress_chars("#>-"),
            );
            pb.set_message(format!("Downloading {}", model_name));
            Some(pb)
        } else {
            info!("Model size unknown, downloading...");
            let pb = ProgressBar::new_spinner();
            pb.set_style(
                ProgressStyle::default_spinner()
                    .template("{spinner:.green} {msg}")
                    .unwrap(),
            );
            pb.set_message(format!("Downloading {} (size unknown)", model_name));
            Some(pb)
        };

        // Stream download with progress updates
        let mut file = tokio::fs::File::create(target_path).await.map_err(|e| {
            AlternatorError::InvalidData(format!(
                "Failed to create model file {}: {}",
                target_path.display(),
                e
            ))
        })?;

        let mut stream = response.bytes_stream();
        let mut downloaded = 0u64;

        while let Some(chunk) = futures_util::stream::StreamExt::next(&mut stream).await {
            let chunk = chunk.map_err(AlternatorError::Network)?;
            file.write_all(&chunk).await.map_err(|e| {
                AlternatorError::InvalidData(format!(
                    "Failed to write to model file {}: {}",
                    target_path.display(),
                    e
                ))
            })?;

            downloaded += chunk.len() as u64;
            if let Some(ref pb) = progress_bar {
                pb.set_position(downloaded);
            }
        }

        file.flush().await.map_err(|e| {
            AlternatorError::InvalidData(format!(
                "Failed to flush model file {}: {}",
                target_path.display(),
                e
            ))
        })?;

        if let Some(pb) = progress_bar {
            pb.finish_with_message(format!("✓ Downloaded {}", model_name));
        }

        info!("Successfully downloaded model: {}", model_name);
        Ok(())
    }

    /// Get the download URL for a specific model
    #[allow(clippy::result_large_err)]
    fn get_model_url(&self, model_name: &str) -> Result<String, AlternatorError> {
        let base_url = "https://huggingface.co/ggerganov/whisper.cpp/resolve/main";

        match model_name {
            "tiny" => Ok(format!("{base_url}/ggml-tiny.bin")),
            "tiny.en" => Ok(format!("{base_url}/ggml-tiny.en.bin")),
            "base" => Ok(format!("{base_url}/ggml-base.bin")),
            "base.en" => Ok(format!("{base_url}/ggml-base.en.bin")),
            "small" => Ok(format!("{base_url}/ggml-small.bin")),
            "small.en" => Ok(format!("{base_url}/ggml-small.en.bin")),
            "medium" => Ok(format!("{base_url}/ggml-medium.bin")),
            "medium.en" => Ok(format!("{base_url}/ggml-medium.en.bin")),
            "large" => Ok(format!("{base_url}/ggml-large-v1.bin")),
            "large-v1" => Ok(format!("{base_url}/ggml-large-v1.bin")),
            "large-v2" => Ok(format!("{base_url}/ggml-large-v2.bin")),
            "large-v3" => Ok(format!("{base_url}/ggml-large-v3.bin")),
            _ => Err(AlternatorError::InvalidData(format!(
                "Unknown Whisper model: {model_name}. Available models: tiny, tiny.en, base, base.en, small, small.en, medium, medium.en, large, large-v1, large-v2, large-v3"
            ))),
        }
    }

    /// List available models
    pub fn list_available_models() -> Vec<&'static str> {
        vec![
            "tiny",
            "tiny.en",
            "base",
            "base.en",
            "small",
            "small.en",
            "medium",
            "medium.en",
            "large",
            "large-v1",
            "large-v2",
            "large-v3",
        ]
    }

    /// Validate that a model name is supported
    #[allow(clippy::result_large_err)]
    pub fn validate_model_name(model_name: &str) -> Result<(), AlternatorError> {
        if Self::list_available_models().contains(&model_name) {
            Ok(())
        } else {
            Err(AlternatorError::InvalidData(format!(
                "Invalid Whisper model '{}'. Available models: {}",
                model_name,
                Self::list_available_models().join(", ")
            )))
        }
    }

    /// Get model directory path
    pub fn model_directory(&self) -> &Path {
        &self.model_dir
    }
}

/// Download whisper model from CLI command
pub async fn download_whisper_model_cli(
    model_name: String,
    config: WhisperConfig,
) -> Result<(), AlternatorError> {
    // Validate model name
    WhisperModelManager::validate_model_name(&model_name)?;

    info!("Starting Whisper model download: {}", model_name);

    let manager = WhisperModelManager::new(config)?;

    info!("Model directory: {}", manager.model_directory().display());

    let model_path = manager.download_model(&model_name).await?;

    info!(
        "✓ Whisper model '{}' ready at: {}",
        model_name,
        model_path.display()
    );
    info!("You can now enable Whisper in your configuration and restart Alternator");

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_model_validation() {
        assert!(WhisperModelManager::validate_model_name("base").is_ok());
        assert!(WhisperModelManager::validate_model_name("small").is_ok());
        assert!(WhisperModelManager::validate_model_name("large").is_ok());
        assert!(WhisperModelManager::validate_model_name("invalid").is_err());
    }

    #[test]
    fn test_model_url_generation() {
        let config = WhisperConfig::default();
        let manager = WhisperModelManager::new(config).unwrap();

        assert!(manager
            .get_model_url("base")
            .unwrap()
            .contains("ggml-base.bin"));
        assert!(manager
            .get_model_url("small")
            .unwrap()
            .contains("ggml-small.bin"));
        assert!(manager.get_model_url("invalid").is_err());
    }

    #[test]
    fn test_list_available_models() {
        let models = WhisperModelManager::list_available_models();
        assert!(models.contains(&"base"));
        assert!(models.contains(&"small"));
        assert!(models.contains(&"large"));
        assert!(!models.is_empty());
    }
}
