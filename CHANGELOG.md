# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added - OpenAI Whisper CLI Integration (Major Update)
- **Universal GPU Support** - Single Docker container supports both AMD ROCm and NVIDIA CUDA acceleration
- **OpenAI Whisper CLI Integration** - Migrated from whisper-rs to Python-based OpenAI Whisper CLI for better GPU support
- **Enhanced Whisper Configuration** - Added 4 new optional configuration fields:
  - `python_executable` - Custom Python executable path (default: "python3")
  - `device` - GPU/CPU device preference with auto-detection (auto/cpu/cuda/rocm)
  - `backend` - Processing backend selection (auto/cpu/cuda/rocm)
  - `preload` - Model preloading at startup for faster transcription (default: true)
- **Automatic GPU Detection** - Runtime detection and optimization for available hardware
- **Model Preloading** - Whisper models loaded at startup for significantly faster transcription
- **Enhanced Docker Support** - Universal container with automatic GPU detection and fallback
- **Complete Migration Guide** - Added MIGRATE-TO-WHISPER-CLI.md for seamless migration

### Enhanced
- **Audio Transcription Performance** - Significant speed improvements with GPU acceleration:
  - NVIDIA GPUs: ~5-10x faster transcription than CPU
  - AMD GPUs: ~3-7x faster transcription than CPU  
  - Model preloading: ~2-5 seconds faster for first transcription
- **Zero Breaking Changes** - All existing configurations continue to work unchanged
- **Production Ready GPU Support** - Battle-tested with comprehensive error handling and fallback mechanisms

### Technical Improvements
- **Streamlined Audio Pipeline** - Replaced whisper-rs with more reliable OpenAI Whisper CLI
- **Enhanced Configuration Management** - New environment variables for all Whisper options
- **Robust Error Handling** - Improved GPU detection failures and graceful CPU fallback
- **Comprehensive Testing** - All 183+ tests passing with new GPU detection and preloading tests
- **Documentation Updates** - Enhanced README with GPU support and troubleshooting guides

### Docker Enhancements
- **Universal GPU Image** - Single container supports AMD ROCm + NVIDIA CUDA + CPU
- **Automatic Device Detection** - No manual configuration needed for GPU acceleration
- **Optimized Startup** - Model preloading and GPU detection for faster transcription
- **Enhanced Volume Management** - Improved model persistence and sharing across containers

## [0.2.0] - 2025-08-16

### Enhanced
- **Added 5 additional languages** - Finnish, Irish Gaelic, Welsh, Romanian, and Romansh language support for AI attribution
- **Separated Docker builds by PyTorch backend** - Reduced image sizes with specialized builds:
  - `ghcr.io/rmoriz/alternator-cpu:latest` - CPU-only (smallest size)
  - `ghcr.io/rmoriz/alternator-nvidia:latest` - NVIDIA GPU with CUDA
  - `ghcr.io/rmoriz/alternator-amd:latest` - AMD GPU with ROCm

### Technical Improvements  
- **Updated to glibc build system** - Replaced musl static binaries with glibc for better compatibility and performance
- Enhanced Docker optimization for production deployments
- Improved library linking and system integration

## [0.1.2] - 2025-08-13

### Changed
- **Updated prompt templates for better length management** - LLM now manages total response length (description + attribution) within 1500 characters
- **Added localized AI attribution to all descriptions** - Each description now ends with a note about AI generation including the model name in the appropriate language
- **Increased default max_tokens to 1500** - Better alignment with description length limits and improved response quality

### Enhanced
- **Comprehensive multi-language AI attribution support** - Attribution notes now available in 39 languages:
  - **Western European**: English, German, French, Spanish, Italian, Portuguese, Dutch
  - **Germanic variants**: Swiss German (Schweizerdeutsch), Low German (Niederdeutsch)
  - **Nordic**: Danish, Swedish, Norwegian, Icelandic
  - **Celtic**: Scottish Gaelic
  - **Slavic**: Polish, Czech, Slovak, Slovenian, Croatian, Bosnian, Serbian, Russian, Bulgarian, Ukrainian
  - **Baltic**: Lithuanian, Estonian, Latvian
  - **Other European**: Hungarian, Greek, Latin
  - **Semitic**: Hebrew, Yiddish
  - **Asian**: Japanese, Chinese (Simplified), Chinese (Traditional), Hindi, Indonesian
  - **Regional variants**: Brazilian Portuguese

## [0.1.1] - 2025-08-13

### Fixed
- **Improved media deletion retry strategy** - Increased initial delay from 5s to 10s and implemented exponential backoff (10s, 20s, 40s)
- Fixed GitHub workflow failures in CI/CD pipeline
- Resolved test failures in OpenRouter client and language detection
- Fixed formatting issues and clippy warnings
- Updated release workflow to use modern GitHub CLI instead of deprecated actions

### Technical Improvements
- Replaced deprecated `actions/create-release@v1` and `actions/upload-release-asset@v1` with `gh` CLI
- Added proper permissions for GitHub Actions workflows
- Implemented custom Debug trait for OpenRouterClient to hide sensitive API keys
- Enhanced media cleanup process to reduce race conditions with Mastodon API
- More conservative approach to media deletion reduces API conflicts

### CI/CD Enhancements
- Added native macOS binary builds (Intel and Apple Silicon)
- Fixed cross-compilation issues for Apple Darwin targets
- Modernized release workflow with GitHub CLI
- All 163 unit tests and 14 integration tests now pass consistently
- Complete multi-platform support: Linux (AMD64/ARM64), macOS (Intel/Apple Silicon)

### Changed
- **Upgraded Docker base image to Debian Trixie (13)** - Updated from `debian:bookworm-slim` to `debian:trixie-slim` for latest security updates and improvements
- **Removed libssl3 dependency from Docker images** - No longer needed since we use rustls instead of OpenSSL for TLS

## [0.1.0] - 2025-08-13

### Changed (from Unreleased)
- Migrated from OpenSSL/native-tls to rustls for improved security and static linking
- **Simplified to musl-only Linux builds** - statically linked binaries work on all distributions (Alpine, Debian, Ubuntu, RHEL, etc.)
- Optimized binary size with LTO and strip configuration
- Replaced reqwest native-tls backend with rustls-tls
- Replaced tokio-tungstenite native-tls with rustls-tls-webpki-roots
- Eliminated separate glibc builds in favor of universal musl static binaries
- Enhanced CI/CD workflows for simplified cross-platform deployment
- Improved rate limiter implementation to avoid borrowing issues

### Added
- Core toot processing pipeline with TootStreamHandler
- Comprehensive error recovery and reconnection logic
- GitHub Actions CI/CD with multi-platform binary releases
- Docker support with optimized multi-stage builds
- Automated security scanning and dependency updates
- Comprehensive test suite with >90% coverage
- Example configuration with detailed documentation
- Multi-language support for generated descriptions
- Race condition protection for manual edits
- Balance monitoring with configurable notifications
- OpenRouter integration with cost controls
- Mastodon WebSocket streaming with automatic reconnection
- Configuration management with TOML and environment variables
- Cross-platform support (Linux/macOS, x86_64/ARM64)
- Real-time monitoring of Mastodon toots via WebSocket
- AI-powered image description generation using OpenRouter API
- Support for multiple AI models (Claude, GPT, Gemini)
- Automatic language detection with localized prompts
- Configurable image filtering and processing
- Comprehensive logging with multiple levels
- Graceful shutdown handling
- Container-ready deployment
- Systemd service integration

### Changed
- **BREAKING**: Updated CI/CD workflow to use optimized Docker builds
- Switched Docker images to GitHub Container Registry (ghcr.io/rmoriz/alternator)
- Optimized Docker build process to reuse pre-built binaries instead of rebuilding
- Updated model recommendations: removed tngtech/deepseek-r1t2-chimera:free
- Simplified paid model recommendations to focus on google/gemini-2.5-flash-lite
- Reduced default image resize dimension from 1024px to 512px
- Increased image resize limit to 2048px and reduced JPEG quality to 75%

### Fixed
- Japanese text UTF-8 character boundary panic in text processing
- Media description updates for posts with empty content
- Improved error handling in Mastodon media upload and processing
- ARM64 cross-compilation failures in CI/CD workflows using cross tool
- OpenSSL dependency issues for aarch64-unknown-linux-gnu target

### Performance
- Eliminated duplicate Rust compilation in CI/CD workflows
- Faster Docker image builds using pre-compiled binaries
- Reduced CI/CD resource usage and build times

### Security
- Token-based authentication for all APIs
- No storage of sensitive credentials in logs
- Secure Docker image with non-root user
- Regular security audits via GitHub Actions
- AGPL-3.0 licensing for transparency