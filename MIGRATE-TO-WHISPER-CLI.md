# Migration to OpenAI Whisper CLI

**Project:** Alternator - Mastodon Media Description Bot  
**Migration Date:** August 2025  
**Target:** Replace whisper-rs with OpenAI Whisper Python CLI  

## Executive Summary

**Objective:** Migrate from whisper-rs to OpenAI's official Whisper Python implementation to enable AMD GPU acceleration and improved performance.

**Benefits:**
- ✅ AMD GPU support via PyTorch
- ✅ NVIDIA GPU support (CUDA)
- ✅ Better model variety (turbo, large-v3)
- ✅ Official OpenAI implementation
- ✅ Active development and updates
- ✅ Superior accuracy on latest models

## Current Implementation Analysis

### whisper-rs Dependencies
- **Cargo.toml**: `whisper-rs = "0.14.4"`
- **Core Files**: 
  - `src/whisper.rs` - Model management and downloading
  - `src/media/audio.rs:248-340` - Transcription logic

### Current Architecture
```rust
// Current whisper-rs implementation
let transcript = tokio::task::spawn_blocking(move || -> Result<String, MediaError> {
    let ctx = WhisperContext::new_with_params(&model_path_string, ctx_params)?;
    let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
    // ... configuration
    ctx.full(params, &pcm_data)?;
    // Extract segments and join text
}).await
```

### Current Features
- ✅ Model downloading from HuggingFace
- ✅ Local model caching (`~/.alternator/models/`)
- ✅ Language detection and configuration
- ✅ Progress reporting during downloads
- ✅ Proper async/await integration with `spawn_blocking`

## Target Implementation: OpenAI Whisper CLI

### CLI Command Structure
```bash
# Basic transcription
whisper audio.wav --model medium

# With language specification
whisper audio.wav --model medium --language German

# Translation to English
whisper audio.wav --model medium --language German --task translate

# Output format options
whisper audio.wav --model medium --output_format txt --output_dir /path/to/output
```

### Python Environment Requirements
```bash
# Install Whisper
pip install openai-whisper

# Universal GPU Support (AMD + NVIDIA in same container)
pip install torch torchvision torchaudio --index-url https://download.pytorch.org/whl/cu118
pip install torch torchvision torchaudio --index-url https://download.pytorch.org/whl/rocm5.4.2

# Alternative: Runtime detection approach
pip install openai-whisper
# GPU backends installed at runtime based on hardware detection
```

## Migration Plan

### Phase 1: Environment Setup
1. **Docker Integration - Universal GPU Support**
   - Add Python 3.9+ to Docker images
   - Install PyTorch with **both** AMD ROCm and NVIDIA CUDA support
   - Install OpenAI Whisper package
   - Verify FFmpeg compatibility
   - Runtime GPU detection for optimal backend selection

2. **Configuration Changes**
   ```toml
   [whisper]
   # Remove model_dir (no longer needed)
   # model = "base"  # Remove - will be specified in CLI
   language = "auto"  # Keep language configuration
   enabled = true
   python_executable = "/usr/bin/python3"  # New: Python path
   model = "medium"  # New: Whisper model selection
   device = "auto"    # New: auto, cpu, cuda (works for both AMD/NVIDIA)
   backend = "auto"   # New: auto, cuda, rocm, cpu (optional override)
   ```

### Phase 2: Core Implementation Changes

#### New Whisper Integration (`src/whisper_cli.rs`)
```rust
pub struct WhisperCli {
    python_executable: String,
    model: String,
    device: String,
    temp_dir: PathBuf,
    model_preloaded: Arc<AtomicBool>,
}

impl WhisperCli {
    pub fn new(config: WhisperConfig) -> Result<Self, MediaError> {
        Ok(Self {
            python_executable: config.python_executable.unwrap_or("python3".to_string()),
            model: config.model,
            device: Self::detect_optimal_device(),
            temp_dir: Self::get_temp_dir()?,
            model_preloaded: Arc::new(AtomicBool::new(false)),
        })
    }

    /// Detect optimal GPU device at runtime
    pub fn detect_optimal_device() -> String {
        // Check for NVIDIA GPU
        if std::process::Command::new("nvidia-smi")
            .output()
            .map(|output| output.status.success())
            .unwrap_or(false)
        {
            tracing::info!("NVIDIA GPU detected, using CUDA backend");
            return "cuda".to_string();
        }
        
        // Check for AMD GPU
        if std::process::Command::new("rocm-smi")
            .output()
            .map(|output| output.status.success())
            .unwrap_or(false)
        {
            tracing::info!("AMD GPU detected, using CUDA backend (ROCm)");
            return "cuda".to_string(); // PyTorch uses "cuda" for both NVIDIA and AMD
        }
        
        tracing::info!("No GPU detected, using CPU backend");
        "cpu".to_string()
    }

    /// Preload model on application startup (not on first transcription)
    pub async fn preload_model(&self) -> Result<(), MediaError> {
        if self.model_preloaded.load(Ordering::Relaxed) {
            return Ok(());
        }

        tracing::info!("Preloading Whisper model '{}' on startup...", self.model);
        
        let python_executable = self.python_executable.clone();
        let model = self.model.clone();
        let device = self.device.clone();
        
        let preload_result = tokio::task::spawn_blocking(move || -> Result<(), MediaError> {
            let mut cmd = Command::new(&python_executable);
            cmd.arg("-c")
               .arg(&format!(
                   r#"
import whisper
import torch
import os

# Set device and environment
device = "{device}" if "{device}" != "cpu" and torch.cuda.is_available() else "cpu"
if device != "cpu":
    os.environ["CUDA_VISIBLE_DEVICES"] = "0"

print(f"Preloading Whisper model '{model}' on device: {{device}}")

# Direct model loading (no dummy audio needed)
model = whisper.load_model("{model}", device=device)
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
               ));
            
            let output = cmd.output().map_err(|e| {
                MediaError::ProcessingFailed(format!("Failed to run Python for model preloading: {}", e))
            })?;
            
            if !output.status.success() {
                return Err(MediaError::ProcessingFailed(format!(
                    "Model preloading failed: {}",
                    String::from_utf8_lossy(&output.stderr)
                )));
            }
            
            tracing::info!("Model preloading output: {}", String::from_utf8_lossy(&output.stdout));
            Ok(())
        }).await.map_err(|e| MediaError::ProcessingFailed(format!("Model preloading task failed: {}", e)))??;
        
        self.model_preloaded.store(true, Ordering::Relaxed);
        tracing::info!("✓ Whisper model '{}' preloaded successfully", self.model);
        
        Ok(())
    }

    pub async fn transcribe_audio(
        &self,
        audio_path: &Path,
        language: Option<&str>,
    ) -> Result<String, MediaError> {
        // Ensure model is preloaded (should already be done at startup)
        if !self.model_preloaded.load(Ordering::Relaxed) {
            tracing::warn!("Model not preloaded, loading now (this may cause delay)");
            self.preload_model().await?;
        }

        let output_dir = self.temp_dir.join("whisper_output");
        tokio::fs::create_dir_all(&output_dir).await?;
        
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
           
        if let Some(lang) = language {
            if lang != "auto" && !lang.is_empty() {
                cmd.arg("--language").arg(lang);
            }
        }
        
        // Set GPU device environment
        if self.device != "cpu" {
            cmd.env("CUDA_VISIBLE_DEVICES", "0");
        }
        
        let output = tokio::task::spawn_blocking(move || cmd.output()).await??;
        
        if !output.status.success() {
            return Err(MediaError::ProcessingFailed(
                format!("Whisper CLI failed: {}", String::from_utf8_lossy(&output.stderr))
            ));
        }
        
        // Read transcription from output file
        let transcript_file = output_dir.join(
            audio_path.file_stem().unwrap().to_str().unwrap()
        ).with_extension("txt");
        
        let transcript = tokio::fs::read_to_string(transcript_file).await?;
        Ok(transcript.trim().to_string())
    }
}
```
```

#### Modified Audio Processing (`src/media/audio.rs`)
```rust
// Replace whisper-rs transcription with CLI call
let transcript = if let Some(whisper_cli) = &whisper_cli {
    whisper_cli.transcribe_audio(&temp_audio_path, language.as_deref()).await?
} else {
    return Err(MediaError::ProcessingFailed(
        "Whisper CLI not available".to_string()
    ));
};
```

### Phase 3: Model Management

#### Remove Current Model Downloads
- Delete `src/whisper.rs` model downloading logic
- Remove HuggingFace model dependencies
- Whisper models will be downloaded automatically by the CLI

#### Model Storage
- Models stored in default Whisper cache: `~/.cache/whisper/`
- No manual model management required
- Automatic model downloading on first use

### Phase 4: Configuration Migration

#### Updated Configuration Structure
```rust
#[derive(Debug, Clone, Deserialize)]
pub struct WhisperConfig {
    pub enabled: bool,
    pub model: String,  // tiny, base, small, medium, large, turbo
    pub language: Option<String>,
    pub python_executable: Option<String>,
    pub device: Option<String>,  // auto, cpu, cuda (works for both AMD/NVIDIA)
    pub backend: Option<String>, // auto, cuda, rocm, cpu (optional override)
    pub preload: Option<bool>,   // preload model at startup (default: true)
}

impl Default for WhisperConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            model: "medium".to_string(),
            language: Some("auto".to_string()),
            python_executable: Some("python3".to_string()),
            device: Some("auto".to_string()),
            backend: Some("auto".to_string()),
            preload: Some(true),
        }
    }
}
```

### Phase 5: Dependency Updates

#### Cargo.toml Changes
```toml
# Remove
# whisper-rs = "0.14.4"

# Keep existing audio processing dependencies
# No new Rust dependencies needed for CLI integration
```

#### Docker Dependencies - Universal GPU Support
```dockerfile
# Multi-stage build for optimal image size
FROM python:3.11-slim as python-base

# Install system dependencies
RUN apt-get update && apt-get install -y \
    python3 python3-pip \
    ffmpeg \
    curl \
    && rm -rf /var/lib/apt/lists/*

# Install Whisper and universal GPU support
RUN pip3 install openai-whisper

# Install both AMD and NVIDIA GPU support in same container
# Option 1: Install both backends (larger image ~3GB additional)
RUN pip3 install torch torchvision torchaudio --index-url https://download.pytorch.org/whl/cu118
RUN pip3 install torch torchvision torchaudio --index-url https://download.pytorch.org/whl/rocm5.4.2

# Option 2: Runtime detection script approach (smaller base image)
# COPY scripts/setup-gpu.sh /usr/local/bin/
# RUN chmod +x /usr/local/bin/setup-gpu.sh
# RUN /usr/local/bin/setup-gpu.sh

# Runtime GPU detection script (scripts/setup-gpu.sh)
# #!/bin/bash
# if command -v nvidia-smi &> /dev/null; then
#     echo "NVIDIA GPU detected, installing CUDA PyTorch..."
#     pip3 install torch torchvision torchaudio --index-url https://download.pytorch.org/whl/cu118
# elif command -v rocm-smi &> /dev/null; then
#     echo "AMD GPU detected, installing ROCm PyTorch..."
#     pip3 install torch torchvision torchaudio --index-url https://download.pytorch.org/whl/rocm5.4.2
# else
#     echo "No GPU detected, using CPU-only PyTorch..."
#     pip3 install torch torchvision torchaudio --index-url https://download.pytorch.org/whl/cpu
# fi

# Continue with Rust build...
FROM rust:1.70 as rust-build
# ... existing Rust build steps
```

## Performance Considerations

### GPU Acceleration - Universal Support
- **AMD GPUs**: ROCm support via PyTorch (same "cuda" device in PyTorch)
- **NVIDIA GPUs**: CUDA support via PyTorch
- **Apple Silicon**: MPS (Metal Performance Shaders) support  
- **CPU Fallback**: Works on any system
- **Runtime Detection**: Automatic optimal backend selection
- **Single Container**: Both AMD and NVIDIA support in same Docker image

### Model Performance Comparison
| Model | whisper-rs | OpenAI Whisper | GPU Speed | AMD GPU | NVIDIA GPU |
|-------|------------|----------------|-----------|---------|------------|
| tiny  | ✅ | ✅ | ~25x | ✅ | ✅ |
| base  | ✅ | ✅ | ~20x | ✅ | ✅ |
| small | ✅ | ✅ | ~15x | ✅ | ✅ |
| medium| ✅ | ✅ | ~10x | ✅ | ✅ |
| large | ✅ | ✅ | ~5x  | ✅ | ✅ |
| turbo | ❌ | ✅ | ~40x | ✅ | ✅ |

### Memory Requirements
- **Reduced**: No need to link Whisper models into Rust binary
- **Dynamic**: Models loaded only when needed
- **Shared**: Multiple processes can share model cache

## Testing Strategy

### Unit Tests
```rust
#[cfg(test)]
mod tests {
    use super::*;
    
    #[tokio::test]
    async fn test_whisper_cli_transcription() {
        let whisper_cli = WhisperCli::new(WhisperConfig::default()).unwrap();
        
        // Test with sample audio file
        let result = whisper_cli.transcribe_audio(
            Path::new("tests/fixtures/sample.wav"),
            Some("en")
        ).await;
        
        assert!(result.is_ok());
        assert!(!result.unwrap().is_empty());
    }
    
    #[tokio::test]
    async fn test_gpu_detection() {
        let device = WhisperCli::detect_optimal_device();
        
        // Should return either "cuda" or "cpu"
        assert!(device == "cuda" || device == "cpu");
        
        // Log which GPU type was detected
        if device == "cuda" {
            println!("GPU detected and available for acceleration");
        } else {
            println!("No GPU detected, using CPU");
        }
    }
    
    #[tokio::test]
    async fn test_model_preloading() {
        let whisper_cli = WhisperCli::new(WhisperConfig::default()).unwrap();
        
        // Test model preloading
        let result = whisper_cli.preload_model().await;
        assert!(result.is_ok());
        
        // Verify model is marked as preloaded
        assert!(whisper_cli.model_preloaded.load(Ordering::Relaxed));
    }
    
    #[tokio::test]
    async fn test_auto_language_detection() {
        let whisper_cli = WhisperCli::new(WhisperConfig::default()).unwrap();
        
        let result = whisper_cli.transcribe_audio(
            Path::new("tests/fixtures/multilingual.wav"),
            None  // Auto-detect language
        ).await;
        
        assert!(result.is_ok());
    }
}
```

### Integration Tests
1. **Docker Environment**: Test full pipeline with Python/Whisper installation
2. **GPU Detection**: Verify AMD/NVIDIA GPU usage when available  
3. **Universal GPU Support**: Test both AMD and NVIDIA GPUs in same container
4. **Model Switching**: Test different model sizes (tiny through turbo)
5. **Language Support**: Test transcription accuracy across languages
6. **Performance Benchmarking**: Compare CPU vs GPU performance
7. **Container Size**: Verify Docker image size with dual GPU support

### GPU Testing Matrix
| Test Case | AMD GPU | NVIDIA GPU | Expected Result |
|-----------|---------|------------|-----------------|
| ROCm + CUDA Container | ✅ | ❌ | Uses ROCm backend |
| ROCm + CUDA Container | ❌ | ✅ | Uses CUDA backend |
| ROCm + CUDA Container | ✅ | ✅ | Uses detected GPU (priority: NVIDIA) |
| ROCm + CUDA Container | ❌ | ❌ | Falls back to CPU |

## Migration Steps

### Step 1: Create Feature Branch
```bash
git checkout -b migrate-to-whisper-cli
```

### Step 2: Update Dependencies
1. Remove `whisper-rs` from `Cargo.toml`
2. Update Docker files for Python/Whisper support
3. Create new `WhisperCli` implementation

### Step 3: Code Migration
1. Replace whisper-rs calls in `src/media/audio.rs`
2. Update configuration structure
3. Remove `src/whisper.rs` model management
4. Add CLI integration logic

### Step 4: Testing
1. Unit tests for CLI integration
2. Integration tests with Docker
3. Performance benchmarking
4. GPU acceleration testing

### Step 5: Documentation
1. Update README with new requirements
2. Update configuration examples
3. Add GPU setup instructions
4. Create migration guide for users

## Rollback Plan

### Rollback Triggers
- Performance degradation > 50%
- GPU acceleration not working
- Compatibility issues with audio formats
- Installation complexity too high

### Rollback Process
1. **Keep whisper-rs branch**: Maintain `main` branch with current implementation
2. **Revert Docker changes**: Roll back to whisper-rs Docker configuration
3. **Configuration compatibility**: Ensure old configs still work
4. **Graceful fallback**: Allow runtime switching between implementations

## Future Enhancements

### Advanced Features
1. **Batch Processing**: Process multiple audio files simultaneously
2. **Streaming**: Real-time transcription for live audio
3. **Custom Models**: Support for fine-tuned Whisper models
4. **Quality Metrics**: Confidence scores for transcriptions

### Performance Optimizations
1. **Model Caching**: Keep models loaded in memory for faster subsequent calls
2. **Preprocessing**: Optimize audio preprocessing pipeline
3. **Hardware Detection**: Automatic optimal device selection
4. **Parallel Processing**: Multiple worker processes for concurrent transcriptions

## Risk Assessment

### Technical Risks
- **Dependency Complexity**: Python environment management in Docker with dual GPU support
- **Performance**: Potential overhead from CLI calls vs native library
- **GPU Compatibility**: AMD ROCm + NVIDIA CUDA coexistence
- **Model Availability**: Network dependency for initial model downloads
- **Container Size**: Dual GPU support increases Docker image size significantly (~3GB)

### Mitigation Strategies
- **Docker Multi-stage**: Pre-built images with all dependencies optimized
- **Performance Testing**: Comprehensive benchmarking before deployment
- **Universal GPU Fallback**: CPU-only mode always available regardless of GPU type
- **Local Caching**: Robust model caching to minimize network dependency
- **Container Optimization**: Optional runtime GPU detection for smaller base images

### Success Criteria
- ✅ Both AMD and NVIDIA GPU acceleration working in same container
- ✅ Performance equal or better than whisper-rs (5-40x speedup with GPU)
- ✅ All existing audio formats supported
- ✅ Docker build size increase < 3GB (acceptable for universal GPU support)
- ✅ Configuration migration seamless for users
- ✅ Runtime GPU detection working correctly
- ✅ Graceful degradation to CPU when no GPU available

---

**Next Steps:**
1. Implement `WhisperCli` struct with universal GPU detection
2. Update Docker configuration for dual GPU support (AMD + NVIDIA)
3. Create comprehensive GPU testing matrix
4. Begin code migration in `src/media/audio.rs`
5. Add model preloading for faster subsequent transcriptions

**Timeline:** Estimated 3-4 weeks for complete migration including dual GPU testing and documentation.