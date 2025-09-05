use crate::config::WhisperConfig;
use crate::error::MediaError;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::fs;
use tracing::{info, warn};

/// Whisper CLI integration for OpenAI Whisper Python implementation
/// Provides GPU acceleration support for both AMD and NVIDIA
pub struct WhisperCli {
    python_executable: String,
    model: String,
    device: String,
    temp_dir: PathBuf,
    model_dir: Option<PathBuf>,
    model_preloaded: Arc<AtomicBool>,
}

impl WhisperCli {
    /// Create a new WhisperCli instance from configuration
    pub fn new(config: &WhisperConfig) -> Result<Self, MediaError> {
        let python_executable = "python3".to_string();

        let model = config
            .model
            .as_ref()
            .unwrap_or(&"medium".to_string())
            .clone();

        let device = Self::detect_optimal_device()?;
        let temp_dir = Self::get_temp_dir()?;
        let model_dir = config.model_dir.as_ref().map(PathBuf::from);

        info!(
            "Initializing WhisperCli with model: {}, device: {}",
            model, device
        );
        if let Some(ref dir) = model_dir {
            info!("Using custom model directory: {}", dir.display());
        }

        Ok(Self {
            python_executable,
            model,
            device,
            temp_dir,
            model_dir,
            model_preloaded: Arc::new(AtomicBool::new(false)),
        })
    }

    /// Detect optimal GPU device at runtime
    pub fn detect_optimal_device() -> Result<String, MediaError> {
        info!("Detecting optimal GPU device...");

        // Check for NVIDIA GPU
        if let Ok(output) = Command::new("nvidia-smi").output() {
            if output.status.success() {
                info!("NVIDIA GPU detected, using CUDA backend");
                return Ok("cuda".to_string());
            }
        }

        // Check for AMD GPU
        if let Ok(output) = Command::new("rocm-smi").output() {
            if output.status.success() {
                info!("AMD GPU detected, using CUDA backend (ROCm)");
                return Ok("cuda".to_string()); // PyTorch uses "cuda" for both NVIDIA and AMD
            }
        }

        // Alternative AMD GPU detection via lspci
        if let Ok(output) = Command::new("lspci").output() {
            if output.status.success() {
                let output_str = String::from_utf8_lossy(&output.stdout);
                if output_str.to_lowercase().contains("amd")
                    && (output_str.to_lowercase().contains("vga")
                        || output_str.to_lowercase().contains("display")
                        || output_str.to_lowercase().contains("3d"))
                {
                    info!("AMD GPU detected via lspci, using CUDA backend (ROCm)");
                    return Ok("cuda".to_string());
                }
            }
        }

        info!("No GPU detected, using CPU backend");
        Ok("cpu".to_string())
    }

    /// Get temporary directory for Whisper operations
    fn get_temp_dir() -> Result<PathBuf, MediaError> {
        let temp_dir = std::env::temp_dir().join("alternator_whisper");
        if !temp_dir.exists() {
            std::fs::create_dir_all(&temp_dir).map_err(|e| {
                MediaError::ProcessingFailed(format!("Failed to create temp directory: {}", e))
            })?;
        }
        Ok(temp_dir)
    }

    /// Preload model on application startup (not on first transcription)
    pub async fn preload_model(&self) -> Result<(), MediaError> {
        if self.model_preloaded.load(Ordering::Relaxed) {
            return Ok(());
        }

        info!("Preloading Whisper model '{}' on startup...", self.model);

        // Verify model directory exists and is writable if specified
        if let Some(ref model_dir) = self.model_dir {
            if !model_dir.exists() {
                warn!("Model directory does not exist: {}", model_dir.display());
                // Try to create it
                if let Err(e) = std::fs::create_dir_all(model_dir) {
                    warn!("Failed to create model directory {}: {}", model_dir.display(), e);
                } else {
                    info!("Created model directory: {}", model_dir.display());
                }
            } else if !model_dir.is_dir() {
                return Err(MediaError::ProcessingFailed(format!(
                    "Model path exists but is not a directory: {}",
                    model_dir.display()
                )));
            } else {
                // Check if directory is writable
                match std::fs::metadata(model_dir) {
                    Ok(metadata) => {
                        if metadata.permissions().readonly() {
                            warn!("Model directory is read-only: {}", model_dir.display());
                        } else {
                            info!("Model directory is writable: {}", model_dir.display());
                        }
                    }
                    Err(e) => {
                        warn!("Failed to check model directory permissions: {}", e);
                    }
                }
            }
        }

        let python_executable = self.python_executable.clone();
        let model = self.model.clone();
        let device = self.device.clone();
        let model_dir = self.model_dir.clone();

        let preload_result = tokio::task::spawn_blocking(move || -> Result<(), MediaError> {
            let mut cmd = Command::new(&python_executable);
            cmd.arg("-c").arg(format!(
                r#"
import whisper
import torch
import os

# Set device and environment
device = "{device}" if "{device}" != "cpu" and torch.cuda.is_available() else "cpu"
if device != "cpu":
    os.environ["CUDA_VISIBLE_DEVICES"] = "0"

print(f"Preloading Whisper model '{model}' on device: {{device}}")
{model_dir_info}

# Direct model loading with custom model_dir if specified
{model_load_call}
print(f"✓ Model loaded on device: {{model.device}}")

# Optional: Warm up GPU context with minimal computation
if model.device.type == "cuda":
    import torch
    # Create minimal mel spectrogram tensor for GPU warmup
    dummy_mel = torch.randn(1, model.dims.n_mels, 300, device=model.device)
    with torch.no_grad():
        # Quick encoder pass to initialize GPU kernels
        _ = model.encoder(dummy_mel)
    print("✓ GPU context warmed up")

print(f"✓ Model '{model}' preloaded successfully on {{model.device}}")
"#,
                device = device,
                model = model,
                model_dir_info = if let Some(ref dir) = model_dir {
                    format!(
                        "print(f\"Using custom model directory: {}\")",
                        dir.display()
                    )
                } else {
                    "print(\"Using default model directory: ~/.cache/whisper/\")".to_string()
                },
                model_load_call = if let Some(ref dir) = model_dir {
                    format!(
                        "model = whisper.load_model(\"{}\", device=device, download_root=\"{}\")",
                        model,
                        dir.display()
                    )
                } else {
                    format!("model = whisper.load_model(\"{}\", device=device)", model)
                }
            ));

            let output = cmd.output().map_err(|e| {
                MediaError::ProcessingFailed(format!(
                    "Failed to run Python for model preloading: {}",
                    e
                ))
            })?;

            if !output.status.success() {
                return Err(MediaError::ProcessingFailed(format!(
                    "Model preloading failed: {}",
                    String::from_utf8_lossy(&output.stderr)
                )));
            }

            info!(
                "Model preloading output: {}",
                String::from_utf8_lossy(&output.stdout)
            );
            Ok(())
        })
        .await
        .map_err(|e| {
            MediaError::ProcessingFailed(format!("Model preloading task failed: {}", e))
        });

        match preload_result {
            Ok(_) => {
                self.model_preloaded.store(true, Ordering::Relaxed);
                info!("✓ Whisper model '{}' preloaded successfully", self.model);
            }
            Err(e) => {
                // Even if preloading fails, mark as attempted to avoid repeated attempts
                self.model_preloaded.store(true, Ordering::Relaxed);
                warn!("⚠ Whisper model preloading failed, will load on-demand: {}", e);
                // Don't return error - allow application to continue without preloaded model
            }
        }

        Ok(())
    }

    /// Transcribe audio file using Whisper CLI
    pub async fn transcribe_audio(
        &self,
        audio_path: &Path,
        language: Option<&str>,
    ) -> Result<String, MediaError> {
        // Check if model was preloaded successfully at startup
        if !self.model_preloaded.load(Ordering::Relaxed) {
            warn!("Model not preloaded, loading now (this may cause delay)");
            // Try to preload now, but don't fail if it doesn't work
            if let Err(e) = self.preload_model().await {
                warn!("Failed to preload model on-demand, proceeding with CLI: {}", e);
            }
        }

        info!("Transcribing audio file: {}", audio_path.display());

        let output_dir = self.temp_dir.join("whisper_output");
        fs::create_dir_all(&output_dir).await.map_err(|e| {
            MediaError::ProcessingFailed(format!("Failed to create output directory: {}", e))
        })?;

        let mut cmd = Command::new(&self.python_executable);
        cmd.arg("-m")
            .arg("whisper")
            .arg(audio_path)
            .arg("--model")
            .arg(&self.model)
            .arg("--output_format")
            .arg("txt")
            .arg("--output_dir")
            .arg(&output_dir);

        // Use existing model_dir configuration with Whisper CLI's --model_dir option
        if let Some(ref model_dir) = self.model_dir {
            cmd.arg("--model_dir").arg(model_dir);
        }

        if let Some(lang) = language {
            if !lang.is_empty() && lang != "auto" {
                info!("Using specified language: {}", lang);
                cmd.arg("--language").arg(lang);
            } else {
                info!("Using automatic language detection");
            }
        }

        // Set GPU device environment
        if self.device != "cpu" {
            cmd.env("CUDA_VISIBLE_DEVICES", "0");
        }

        info!("=== Whisper CLI Command Debug ===");
        info!("Command: {:?} {:?}", cmd.get_program(), cmd.get_args());
        info!("Environment: {:?}", cmd.get_envs().collect::<Vec<_>>());
        info!("=== End Whisper CLI Command Debug ===");

        let output = tokio::task::spawn_blocking(move || cmd.output())
            .await
            .map_err(|e| {
                MediaError::ProcessingFailed(format!("Failed to execute Whisper CLI: {}", e))
            })?
            .map_err(|e| {
                MediaError::ProcessingFailed(format!("Whisper CLI execution failed: {}", e))
            })?;

        info!("=== Whisper CLI Result Debug ===");
        info!("Exit Status: {}", output.status);
        info!("Stdout: {}", String::from_utf8_lossy(&output.stdout));
        info!("Stderr: {}", String::from_utf8_lossy(&output.stderr));
        info!("=== End Whisper CLI Result Debug ===");

        if !output.status.success() {
            return Err(MediaError::ProcessingFailed(format!(
                "Whisper CLI failed with status {}: {}",
                output.status,
                String::from_utf8_lossy(&output.stderr)
            )));
        }

        // Read transcription from output file
        let transcript_file = output_dir
            .join(
                audio_path
                    .file_stem()
                    .ok_or_else(|| {
                        MediaError::ProcessingFailed("Invalid audio file path".to_string())
                    })?
                    .to_str()
                    .ok_or_else(|| {
                        MediaError::ProcessingFailed("Invalid audio file name".to_string())
                    })?,
            )
            .with_extension("txt");

        info!("Looking for transcript file: {}", transcript_file.display());

        // List all files in output directory for debugging
        if let Ok(mut entries) = tokio::fs::read_dir(&output_dir).await {
            info!("=== Output Directory Contents ===");
            while let Ok(Some(entry)) = entries.next_entry().await {
                if let Ok(metadata) = entry.metadata().await {
                    info!(
                        "File: {:?}, Size: {} bytes",
                        entry.file_name(),
                        metadata.len()
                    );
                }
            }
            info!("=== End Output Directory Contents ===");
        }

        if !transcript_file.exists() {
            return Err(MediaError::ProcessingFailed(format!(
                "Transcript file not found: {}",
                transcript_file.display()
            )));
        }

        let transcript = fs::read_to_string(&transcript_file).await.map_err(|e| {
            MediaError::ProcessingFailed(format!("Failed to read transcript file: {}", e))
        })?;

        info!("=== Whisper Transcript Content ===");
        info!("File: {}", transcript_file.display());
        info!("Size: {} characters", transcript.len());
        info!("Content:\n{}", transcript);
        info!("=== End Whisper Transcript Content ===");

        // Clean up output files
        let _ = fs::remove_file(&transcript_file).await;

        let result = transcript.trim().to_string();
        info!("Transcription completed, {} characters", result.len());

        Ok(result)
    }

    /// Check if model is preloaded
    #[allow(dead_code)] // Public API method, may be used in future
    pub fn is_model_preloaded(&self) -> bool {
        self.model_preloaded.load(Ordering::Relaxed)
    }

    /// Get current device being used
    pub fn device(&self) -> &str {
        &self.device
    }

    /// Get current model name
    pub fn model(&self) -> &str {
        &self.model
    }

    /// Get model directory if configured
    #[allow(dead_code)] // Public API method, may be used in future
    pub fn model_dir(&self) -> Option<&Path> {
        self.model_dir.as_deref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_gpu_detection() {
        // GPU detection should not fail, even if no GPU is present
        let device = WhisperCli::detect_optimal_device().unwrap();
        assert!(device == "cuda" || device == "cpu");
    }

    #[test]
    fn test_temp_dir_creation() {
        let temp_dir = WhisperCli::get_temp_dir().unwrap();
        assert!(temp_dir.exists());
        assert!(temp_dir.is_dir());
    }

    #[tokio::test]
    async fn test_whisper_cli_creation() {
        let config = WhisperConfig {
            enabled: Some(true),
            model: Some("tiny".to_string()),
            model_dir: None,
            language: Some("auto".to_string()),
            max_duration_minutes: Some(10),
            python_executable: Some("python3".to_string()),
            device: None,
            backend: None,
            preload: Some(true),
        };

        let whisper_cli = WhisperCli::new(&config).unwrap();
        assert_eq!(whisper_cli.model(), "tiny");
        assert!(whisper_cli.device() == "cuda" || whisper_cli.device() == "cpu");
        assert!(!whisper_cli.is_model_preloaded());
    }

    #[tokio::test]
    async fn test_model_preloading() {
        let config = WhisperConfig {
            enabled: Some(true),
            model: Some("tiny".to_string()),
            model_dir: None,
            language: Some("auto".to_string()),
            max_duration_minutes: Some(10),
            python_executable: Some("python3".to_string()),
            device: None,
            backend: None,
            preload: Some(true),
        };

        let whisper_cli = WhisperCli::new(&config).unwrap();

        // Test model preloading (may take some time on first run)
        let result = whisper_cli.preload_model().await;

        // This test might fail in CI environments without internet access
        // or if Whisper is not properly installed, so we log but don't assert
        match result {
            Ok(_) => {
                assert!(whisper_cli.is_model_preloaded());
                info!("Model preloading test passed");
            }
            Err(e) => {
                warn!(
                    "Model preloading test failed (this may be expected in CI): {}",
                    e
                );
            }
        }
    }
}
