# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.0] - 2025-08-13

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