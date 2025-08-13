# Implementation Plan

- [x] 1. Set up project structure and core dependencies
  - Initialize git repository and set remote to https://github.com/rmoriz/alternator.git
  - Create Cargo.toml with latest versions of tokio, serde, reqwest, tokio-tungstenite, image, toml, tracing, clap, thiserror, chrono
  - Set up basic project structure with src/main.rs and module organization
  - Add AGPL-3.0 license file and basic README
  - Commit changes: "Initial project setup with dependencies and structure"
  - _Requirements: 6.1, 6.2, 6.7_

- [x] 2. Implement configuration management system
  - Create Config structs for all configuration sections (MastodonConfig, OpenRouterConfig with max_tokens, etc.)
  - Implement TOML file loading with XDG directory support (current dir, then $XDG_CONFIG_HOME/alternator/)
  - Add environment variable override functionality for all config values including token limits
  - Create CLI argument parsing for config file path and help display
  - Write unit tests for configuration loading and validation
  - Commit changes: "Add configuration management with TOML and environment variable support"
  - _Requirements: 8.1, 8.2, 8.3, 8.4, 8.5, 8.6, 8.7, 10.3_

- [x] 3. Create error handling and logging infrastructure
  - Define AlternatorError enum with all error types using thiserror
  - Set up structured logging with tracing crate supporting Error, Info, Debug levels
  - Implement error recovery strategies for different failure scenarios
  - Write unit tests for error handling and logging functionality
  - Commit changes: "Implement error handling and structured logging infrastructure"
  - _Requirements: 3.1, 3.2, 3.3, 3.4, 3.5_

- [x] 4. Implement OpenRouter client with API validation and cost controls
  - Create OpenRouterClient struct with HTTP client and rate limiting
  - Implement get_account_balance() and list_models() methods for startup validation
  - Add describe_image() method with configurable max_tokens parameter
  - Implement token limit response detection and no-retry logic for exceeded limits
  - Add rate limiting and exponential backoff for API calls
  - Write unit tests with mocked HTTP responses including token limit scenarios
  - Commit changes: "Add OpenRouter client with cost controls and API validation"
  - _Requirements: 1.1, 1.4, 2.4, 10.1, 10.2, 10.4, 10.5_

- [x] 5. Build media processing and transformation system
  - Create MediaProcessor with support for image type filtering (JPEG, PNG, GIF, WebP)
  - Implement ImageTransformer for downsizing and format optimization
  - Add media attachment description checking logic
  - Create abstraction layer for future media type support
  - Write unit tests for media filtering and transformation
  - Commit changes: "Implement media processing and image transformation system"
  - _Requirements: 4.1, 4.4, 4.5, 4.6_

- [x] 6. Implement language detection and prompt management
  - Create LanguageDetector with language identification capabilities
  - Build prompt template system with language-specific templates
  - Implement get_prompt_template() method for localized AI prompts
  - Write unit tests for language detection and prompt selection
  - Commit changes: "Add language detection and localized prompt management"
  - _Requirements: 1.6, 1.7_

- [x] 7. Create Mastodon WebSocket streaming client
  - Implement MastodonClient with WebSocket connection management
  - Add toot event parsing and filtering for authenticated user's posts only
  - Implement automatic reconnection with exponential backoff strategy
  - Create get_toot() method to retrieve current toot state for race condition checking
  - Create update_media() method for REST API media description updates
  - Add send_dm() method for direct message notifications
  - Write integration tests with mocked WebSocket server
  - Commit changes: "Implement Mastodon WebSocket client with reconnection and API methods"
  - _Requirements: 1.2, 1.3, 1.4, 2.1, 2.2, 2.5, 11.1_

- [-] 8. Implement balance monitoring system
  - Create BalanceMonitor with configurable daily checking (default noon)
  - Add threshold comparison logic (default $5) with configurable values
  - Implement direct message sending when balance is below threshold
  - Add enable/disable functionality via config and environment variables
  - Write unit tests for balance checking and notification logic
  - Commit changes: "Add OpenRouter balance monitoring with configurable notifications"
  - _Requirements: 9.1, 9.2, 9.3, 9.4, 9.5, 9.6_

- [ ] 9. Build main application orchestration
  - Create main application loop that coordinates all components
  - Implement graceful shutdown handling for WebSocket and ongoing operations
  - Add startup validation for both Mastodon and OpenRouter connectivity
  - Integrate all components into cohesive application flow
  - Write integration tests for complete application startup and shutdown
  - Commit changes: "Implement main application orchestration and startup validation"
  - _Requirements: 1.1, 5.1, 5.4_

- [ ] 10. Implement core toot processing pipeline
  - Create TootStreamHandler to process incoming WebSocket events
  - Add user account verification to ensure only own toots are processed
  - Integrate media filtering, language detection, and AI description generation
  - Implement race condition checking by retrieving current toot state before updates
  - Add logic to skip updates when manual edits are detected
  - Implement media attachment and toot updating after description generation
  - Add duplicate processing prevention and state tracking
  - Write end-to-end tests for complete toot processing workflow including race conditions
  - Commit changes: "Implement core toot processing pipeline with race condition handling"
  - _Requirements: 1.3, 1.4, 1.5, 1.8, 1.9, 1.10, 5.3, 11.1, 11.2, 11.3, 11.4_

- [ ] 11. Add comprehensive error recovery and reconnection logic
  - Implement WebSocket reconnection with exponential backoff (1s to 60s max)
  - Add API rate limit handling with proper backoff strategies
  - Create network timeout handling with retry mechanisms
  - Implement graceful degradation for non-critical failures
  - Write tests for all error recovery scenarios
  - Commit changes: "Add comprehensive error recovery and reconnection strategies"
  - _Requirements: 2.1, 2.2, 2.3, 2.5_

- [ ] 12. Create GitHub Actions CI/CD workflow
  - Set up GitHub workflow for automated testing on push/PR
  - Add build verification for multiple Rust versions
  - Implement cross-compilation for amd64 and arm64 Linux and macOS
  - Configure automatic binary releases on successful CI
  - Add workflow for dependency updates and security scanning
  - Commit changes: "Add GitHub Actions CI/CD with multi-platform binary releases"
  - _Requirements: 7.1, 7.2, 7.3, 7.4, 7.5_

- [ ] 13. Write comprehensive test suite
  - Create unit tests for all core components with >80% coverage
  - Add integration tests for API clients with mocked services
  - Implement end-to-end tests for complete application workflows
  - Add performance tests for media processing and API interactions
  - Create test utilities and fixtures for consistent testing
  - Commit changes: "Add comprehensive test suite with high coverage"
  - _Requirements: 6.4_

- [ ] 14. Create example configuration and documentation
  - Generate example alternator.toml with all configuration options
  - Create comprehensive README with installation and usage instructions
  - Add configuration reference documentation
  - Include troubleshooting guide and FAQ
  - Document environment variable options for container deployment
  - Commit changes: "Add comprehensive documentation and example configuration"
  - _Requirements: 8.7, 7.5_