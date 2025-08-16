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

# GPU Support (AMD)
pip install torch torchvision torchaudio --index-url https://download.pytorch.org/whl/rocm5.4.2

# GPU Support (NVIDIA)
pip install torch torchvision torchaudio --index-url https://download.pytorch.org/whl/cu118
```

## Migration Plan

### Phase 1: Environment Setup
1. **Docker Integration**
   - Add Python 3.9+ to Docker images
   - Install PyTorch with AMD ROCm support
   - Install OpenAI Whisper package
   - Verify FFmpeg compatibility

2. **Configuration Changes**
   ```toml
   [whisper]
   # Remove model_dir (no longer needed)
   # model = "base"  # Remove - will be specified in CLI
   language = "auto"  # Keep language configuration
   enabled = true
   python_executable = "/usr/bin/python3"  # New: Python path
   model = "medium"  # New: Whisper model selection
   device = "auto"    # New: auto, cpu, cuda, or mps
   ```

### Phase 2: Core Implementation Changes

#### New Whisper Integration (`src/whisper_cli.rs`)
```rust
pub struct WhisperCli {
    python_executable: String,
    model: String,
    device: String,
    temp_dir: PathBuf,
}

impl WhisperCli {
    pub async fn transcribe_audio(
        &self,
        audio_path: &Path,
        language: Option<&str>,
    ) -> Result<String, MediaError> {
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
        
        // Set device if GPU available
        if self.device != "cpu" {
            cmd.env("TORCH_DEVICE", &self.device);
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
    pub device: Option<String>,  // auto, cpu, cuda, mps
}

impl Default for WhisperConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            model: "medium".to_string(),
            language: Some("auto".to_string()),
            python_executable: Some("python3".to_string()),
            device: Some("auto".to_string()),
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

#### Docker Dependencies
```dockerfile
# Add Python and Whisper installation
RUN apt-get update && apt-get install -y python3 python3-pip
RUN pip3 install openai-whisper

# AMD GPU support (optional)
RUN pip3 install torch torchvision torchaudio --index-url https://download.pytorch.org/whl/rocm5.4.2

# NVIDIA GPU support (optional)
# RUN pip3 install torch torchvision torchaudio --index-url https://download.pytorch.org/whl/cu118
```

## Performance Considerations

### GPU Acceleration
- **AMD GPUs**: ROCm support via PyTorch
- **NVIDIA GPUs**: CUDA support via PyTorch  
- **Apple Silicon**: MPS (Metal Performance Shaders) support
- **CPU Fallback**: Works on any system

### Model Performance Comparison
| Model | whisper-rs | OpenAI Whisper | Speed Improvement | GPU Support |
|-------|------------|----------------|-------------------|-------------|
| tiny  | ✅ | ✅ | ~10x (CPU) | ✅ |
| base  | ✅ | ✅ | ~7x (CPU) | ✅ |
| small | ✅ | ✅ | ~4x (CPU) | ✅ |
| medium| ✅ | ✅ | ~2x (CPU) | ✅ |
| large | ✅ | ✅ | 1x (CPU) | ✅ |
| turbo | ❌ | ✅ | ~8x (CPU) | ✅ |

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
3. **Model Switching**: Test different model sizes
4. **Language Support**: Test transcription accuracy across languages

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
- **Dependency Complexity**: Python environment management in Docker
- **Performance**: Potential overhead from CLI calls vs native library
- **GPU Compatibility**: AMD ROCm setup complexity
- **Model Availability**: Network dependency for initial model downloads

### Mitigation Strategies
- **Docker Multi-stage**: Pre-built images with all dependencies
- **Performance Testing**: Comprehensive benchmarking before deployment
- **Fallback Options**: CPU-only mode always available  
- **Local Caching**: Robust model caching to minimize network dependency

### Success Criteria
- ✅ AMD GPU acceleration working
- ✅ Performance equal or better than whisper-rs
- ✅ All existing audio formats supported
- ✅ Docker build size increase < 500MB
- ✅ Configuration migration seamless for users

---

**Next Steps:**
1. Implement `WhisperCli` struct and basic functionality
2. Update Docker configuration for Python/PyTorch
3. Create integration tests
4. Begin code migration in `src/media/audio.rs`

**Timeline:** Estimated 2-3 weeks for complete migration including testing and documentation.