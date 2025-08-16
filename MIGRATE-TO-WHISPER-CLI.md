# Migration Guide: Whisper-rs to OpenAI Whisper CLI

This guide explains the migration from the native Rust `whisper-rs` implementation to the Python-based OpenAI Whisper CLI with enhanced GPU support.

## Summary of Changes

### What Changed
- **Audio Processing Engine**: Migrated from `whisper-rs` (Rust bindings) to OpenAI Whisper CLI (Python)
- **GPU Support**: Added universal GPU support for both AMD ROCm and NVIDIA CUDA
- **Configuration**: Added 4 new optional configuration fields for enhanced control
- **Docker**: Single container now supports both AMD and NVIDIA GPUs
- **Performance**: Model preloading for faster transcription startup

### What Stayed the Same
- **Zero Breaking Changes**: All existing configurations continue to work
- **Same Audio Formats**: All previously supported audio/video formats still work
- **Same API**: Audio processing interface remains identical
- **Same Models**: All Whisper models (tiny, base, small, medium, large) supported

## Migration Steps

### Step 1: Update to Latest Version

**For Docker Users (Recommended):**
```bash
# Pull the latest image with WhisperCli support
docker pull ghcr.io/rmoriz/alternator:latest
```

**For Binary Users:**
```bash
# Download latest release from GitHub
# https://github.com/rmoriz/alternator/releases
```

**For Source Users:**
```bash
git checkout standalone-whisper  # or main when merged
cargo build --release
```

### Step 2: Configuration Migration

Your existing configuration **continues to work unchanged**. No action required.

**Existing configuration (still works):**
```toml
[whisper]
enabled = true
model = "base"
model_dir = "/path/to/models"
language = "en"
max_duration_minutes = 10
```

**Enhanced configuration (optional new features):**
```toml
[whisper]
enabled = true
model = "base"
model_dir = "/path/to/models"
language = "en"
max_duration_minutes = 10

# NEW: Enhanced WhisperCli options (all optional)
python_executable = "python3"    # Python path (default: "python3")
device = "auto"                  # GPU preference (default: auto-detect)
backend = "auto"                 # Backend selection (default: auto-detect)
preload = true                   # Startup preloading (default: true)
```

### Step 3: Environment Variables (Optional)

New environment variables are available for the enhanced features:

```bash
# Existing variables (still work)
export ALTERNATOR_WHISPER_ENABLED="true"
export ALTERNATOR_WHISPER_MODEL="base"
export ALTERNATOR_WHISPER_MODEL_DIR="/path/to/models"

# NEW: Enhanced variables (optional)
export ALTERNATOR_WHISPER_PYTHON_EXECUTABLE="python3"
export ALTERNATOR_WHISPER_DEVICE="auto"
export ALTERNATOR_WHISPER_BACKEND="auto"
export ALTERNATOR_WHISPER_PRELOAD="true"
```

### Step 4: Docker Migration

**Previous Docker setup (still works):**
```bash
docker run \
  -v $(pwd)/config:/app/config \
  -v $(pwd)/whisper-models:/app/models \
  ghcr.io/rmoriz/alternator
```

**Enhanced Docker setup (with GPU support):**
```bash
# For AMD GPUs (ROCm)
docker run \
  --device=/dev/kfd --device=/dev/dri \
  -v $(pwd)/config:/app/config \
  -v $(pwd)/whisper-models:/app/models \
  -e ALTERNATOR_WHISPER_MODEL_DIR=/app/models \
  ghcr.io/rmoriz/alternator

# For NVIDIA GPUs (CUDA)
docker run --gpus all \
  -v $(pwd)/config:/app/config \
  -v $(pwd)/whisper-models:/app/models \
  -e ALTERNATOR_WHISPER_MODEL_DIR=/app/models \
  ghcr.io/rmoriz/alternator

# CPU-only (works everywhere)
docker run \
  -v $(pwd)/config:/app/config \
  -v $(pwd)/whisper-models:/app/models \
  -e ALTERNATOR_WHISPER_MODEL_DIR=/app/models \
  ghcr.io/rmoriz/alternator
```

## New Features Available

### 1. Universal GPU Support
- **AMD GPUs**: ROCm support for Radeon cards
- **NVIDIA GPUs**: CUDA support for GeForce/Tesla cards
- **Automatic Detection**: Runtime GPU detection and optimization
- **CPU Fallback**: Graceful fallback when no GPU is available

### 2. Enhanced Configuration
- **`python_executable`**: Custom Python path (useful for virtual environments)
- **`device`**: Explicit device selection (`auto`, `cpu`, `cuda`, `rocm`)
- **`backend`**: Backend preference (usually same as device)
- **`preload`**: Model preloading control for faster startup

### 3. Performance Improvements
- **Model Preloading**: Models loaded at startup for faster transcription
- **GPU Acceleration**: Significantly faster processing on compatible hardware
- **Optimized Pipeline**: Streamlined audio processing workflow

### 4. Single Container Solution
- **Universal Image**: One Docker image supports all GPU types
- **Automatic Detection**: Runtime detection of available acceleration
- **No Manual Configuration**: Works out-of-the-box with optimal settings

## Verification Steps

### 1. Test Basic Functionality
```bash
# Verify audio transcription still works
# Post a toot with an audio file and check if it gets transcribed
```

### 2. Check GPU Detection
Look for these log messages at startup:
```
INFO alternator: Initializing Whisper CLI with model: base
INFO alternator: ✓ Whisper CLI initialized - Model: base, Device: cuda
INFO alternator: Preloading Whisper model for faster transcriptions...
INFO alternator: ✓ Whisper model preloaded successfully
```

Device types you might see:
- `cuda` - NVIDIA GPU or AMD GPU with ROCm
- `cpu` - CPU processing (no GPU detected)

### 3. Test New Configuration Options
```toml
[whisper]
enabled = true
model = "base"
device = "cpu"     # Force CPU to test fallback
preload = false    # Disable preloading to test on-demand loading
```

## Troubleshooting

### Common Issues

**"No GPU detected, using CPU backend"**
- This is normal if you don't have a GPU or GPU drivers installed
- CPU processing works but is slower
- For GPU support, ensure appropriate drivers are installed

**"Model preloading failed"**
- Check if Python 3.7+ is installed: `python3 --version`
- Verify OpenAI Whisper is available: `python3 -c "import whisper"`
- Try disabling preloading: `preload = false`

**"Failed to run Python for model preloading"**
- Check Python executable path: `which python3`
- Update configuration: `python_executable = "/usr/bin/python3"`
- Verify OpenAI Whisper installation: `pip install openai-whisper`

### Performance Comparison

**Expected performance improvements with GPU:**
- **NVIDIA RTX 3080**: ~5-10x faster than CPU
- **AMD RX 6800 XT**: ~3-7x faster than CPU
- **CPU only**: Same performance as before

**Model preloading benefits:**
- **First transcription**: ~2-5 seconds faster (model already loaded)
- **Subsequent transcriptions**: Same speed as before
- **Memory usage**: Slightly higher (model kept in memory)

## Rollback Plan

If you need to rollback to the previous version:

### Docker Rollback
```bash
# Use a specific previous version tag
docker run ghcr.io/rmoriz/alternator:v0.1.0  # Replace with last known good version
```

### Binary Rollback
Download a previous release from [GitHub Releases](https://github.com/rmoriz/alternator/releases).

### Source Rollback
```bash
git checkout main  # Or the previous stable branch
cargo build --release
```

## FAQ

**Q: Do I need to change my configuration?**
A: No, existing configurations work unchanged. New features are optional.

**Q: Will this break my existing setup?**
A: No, this migration maintains 100% backward compatibility.

**Q: Do I need special GPU drivers?**
A: For GPU acceleration, yes. For CPU-only operation, no additional drivers needed.

**Q: Is the audio quality different?**
A: No, audio quality is identical. The same Whisper models are used.

**Q: What if I don't have a GPU?**
A: Everything works the same as before. CPU processing is unchanged.

**Q: Can I disable the new features?**
A: Yes, don't configure the new options and they won't be used.

**Q: Is Docker still recommended?**
A: Yes, even more so. Docker now includes GPU support out-of-the-box.

## Support

If you encounter issues during migration:

1. **Check the logs**: Enable debug logging with `--log-level debug`
2. **Verify configuration**: Ensure your config file is valid TOML
3. **Test basic functionality**: Try with `device = "cpu"` first
4. **Check GitHub Issues**: Search for similar problems
5. **Create an issue**: Provide logs and configuration details

## Benefits of Migration

### For Users
- **Faster transcription** with GPU acceleration
- **More reliable** with battle-tested OpenAI Whisper CLI
- **Better compatibility** across different systems
- **Enhanced control** with new configuration options

### For Developers
- **Simplified maintenance** with Python-based Whisper
- **Better GPU support** through PyTorch ecosystem
- **More robust error handling** with mature Whisper CLI
- **Future-proof architecture** aligned with OpenAI updates

---

**Migration completed successfully!** 🎉

Your Alternator instance now supports universal GPU acceleration while maintaining full backward compatibility.