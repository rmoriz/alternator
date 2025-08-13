# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- Core toot processing pipeline with TootStreamHandler
- Comprehensive error recovery and reconnection logic
- GitHub Actions CI/CD with multi-platform binary releases
- Docker support with multi-stage builds
- Automated security scanning and dependency updates
- Comprehensive test suite with >80% coverage goals
- Example configuration with detailed documentation
- Multi-language support for generated descriptions
- Race condition protection for manual edits
- Balance monitoring with configurable notifications
- OpenRouter integration with cost controls
- Mastodon WebSocket streaming with automatic reconnection
- Configuration management with TOML and environment variables
- Cross-platform support (Linux/macOS, x86_64/ARM64)

### Features
- Real-time monitoring of Mastodon toots via WebSocket
- AI-powered image description generation using OpenRouter API
- Support for multiple AI models (Claude, GPT, Gemini)
- Automatic language detection with localized prompts
- Configurable image filtering and processing
- Comprehensive logging with multiple levels
- Graceful shutdown handling
- Container-ready deployment
- Systemd service integration

### Security
- Token-based authentication for all APIs
- No storage of sensitive credentials in logs
- Secure Docker image with non-root user
- Regular security audits via GitHub Actions
- AGPL-3.0 licensing for transparency

## [0.1.0] - Initial Development

### Added
- Project structure and dependencies
- Basic configuration system
- Error handling infrastructure
- Mastodon client implementation
- OpenRouter client implementation
- Media processing system
- Language detection system
- Balance monitoring system
- Main application orchestration
- Initial test suite
- Documentation and examples