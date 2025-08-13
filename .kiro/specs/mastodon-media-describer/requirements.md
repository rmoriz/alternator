# Requirements Document

## Introduction

Alternator is a Rust application that automatically adds descriptions to media attachments in Mastodon toots. The application connects to a Mastodon server via WebSocket streaming API and access token, monitors the user's own toots for media without descriptions, and uses OpenRouter's AI services to generate human-readable descriptions for images. The system is designed to run continuously with robust error handling, reconnection logic, and proper API rate limiting.

**Repository:** https://github.com/rmoriz/alternator.git

## Requirements

### Requirement 1

**User Story:** As a Mastodon user, I want my posted images to automatically receive descriptive alt-text, so that my content is more accessible to visually impaired users.

#### Acceptance Criteria

1. WHEN the application starts THEN it SHALL validate connectivity to both Mastodon and OpenRouter APIs
2. WHEN the application connects to Mastodon THEN it SHALL use WebSocket streaming API with provided access token
3. WHEN a new toot is received via the WebSocket stream THEN the system SHALL verify it was posted by the authenticated user's own account
4. WHEN a toot is confirmed to be from the authenticated user's account THEN the system SHALL check for media attachments
5. WHEN media attachments are found THEN the system SHALL verify if descriptions already exist
6. WHEN an image attachment lacks a description THEN the system SHALL detect the language of the toot
7. WHEN the toot language is detected THEN the system SHALL select an appropriate language-dependent prompt for OpenRouter
8. WHEN the prompt is prepared THEN the system SHALL send the image and localized prompt to OpenRouter for analysis
9. WHEN OpenRouter returns a description THEN the system SHALL update the media attachment with the generated description
10. WHEN a media attachment is updated THEN the system SHALL update the associated toot if applicable

### Requirement 2

**User Story:** As a system administrator, I want the application to handle connection failures gracefully, so that the service remains reliable and automatically recovers from network issues.

#### Acceptance Criteria

1. WHEN the websocket connection is lost THEN the system SHALL attempt to reconnect automatically
2. WHEN reconnection attempts fail THEN the system SHALL implement exponential backoff strategy
3. WHEN API rate limits are encountered THEN the system SHALL implement appropriate backoff for both Mastodon and OpenRouter APIs
4. WHEN the application starts THEN it SHALL verify OpenRouter access by retrieving model list and account balance
5. WHEN connection errors occur THEN the system SHALL log appropriate error messages and continue operation
6. WHEN the system recovers from failures THEN it SHALL resume normal monitoring without data loss

### Requirement 3

**User Story:** As a developer, I want comprehensive logging capabilities, so that I can monitor system behavior and troubleshoot issues effectively.

#### Acceptance Criteria

1. WHEN the application runs THEN it SHALL support Error, Info, and Debug log levels
2. WHEN API calls are made THEN the system SHALL log request/response information at Debug level
3. WHEN errors occur THEN the system SHALL log detailed error information at Error level
4. WHEN normal operations occur THEN the system SHALL log status updates at Info level
5. WHEN the log level is configured THEN the system SHALL only output messages at or above the specified level

### Requirement 4

**User Story:** As a content creator, I want only images to be processed initially, so that the system focuses on the most common media type while allowing for future expansion.

#### Acceptance Criteria

1. WHEN media attachments are detected THEN the system SHALL filter for image types only
2. WHEN non-image media is encountered THEN the system SHALL skip processing and log the media type
3. WHEN the system architecture is designed THEN it SHALL allow for future support of other media types
4. WHEN image processing occurs THEN the system SHALL handle common image formats (JPEG, PNG, GIF, WebP)
5. WHEN images are processed THEN the system SHALL implement an abstraction for media transformation including downsizing and still frame creation
6. WHEN media is sent to OpenRouter THEN the system SHALL apply appropriate transformations to optimize for API requirements

### Requirement 5

**User Story:** As a system operator, I want the application to run continuously and maintain state, so that all toots are processed without manual intervention.

#### Acceptance Criteria

1. WHEN the application starts THEN it SHALL enter a continuous monitoring mode
2. WHEN the system is running THEN it SHALL maintain connection state and handle reconnections
3. WHEN processing occurs THEN the system SHALL track processed toots to avoid duplicate work
4. WHEN the application shuts down gracefully THEN it SHALL complete in-progress operations
5. WHEN the system restarts THEN it SHALL resume monitoring from the appropriate point

### Requirement 6

**User Story:** As a developer, I want a simple and maintainable codebase, so that the application is easy to understand, test, and modify without unnecessary complexity.

#### Acceptance Criteria

1. WHEN the application is built THEN it SHALL be implemented in Rust with minimal dependencies
2. WHEN dependencies are chosen THEN they SHALL be well-established, necessary for core functionality, and use the latest available crate versions
3. WHEN the architecture is designed THEN it SHALL be compact and avoid over-engineering
4. WHEN code is written THEN it SHALL include reasonable test coverage for core functionality
5. WHEN design decisions are made THEN they SHALL prioritize simplicity over enterprise-style abstractions
6. WHEN the codebase is structured THEN it SHALL be straightforward and avoid unnecessary complexity
7. WHEN the project is distributed THEN it SHALL be licensed under AGPL-3.0

### Requirement 7

**User Story:** As a user, I want pre-built binaries available for my platform, so that I can easily install and run Alternator without compiling from source.

#### Acceptance Criteria

1. WHEN code is pushed to the repository THEN GitHub Workflows SHALL run CI/CD pipelines
2. WHEN CI/CD runs THEN it SHALL execute tests and build verification
3. WHEN CI/CD is successful THEN it SHALL build release binaries for amd64 and arm64 architectures
4. WHEN binaries are built THEN they SHALL target Linux and macOS platforms
5. WHEN releases are created THEN binaries SHALL be automatically attached to GitHub releases

### Requirement 8

**User Story:** As a user, I want flexible configuration options, so that I can easily configure Alternator for different environments and deployment scenarios.

#### Acceptance Criteria

1. WHEN the application starts THEN it SHALL use TOML format for configuration files
2. WHEN looking for configuration THEN it SHALL search in order: current directory, then $XDG_CONFIG_HOME/alternator/alternator.toml
3. WHEN CLI is invoked THEN it SHALL support options to specify full config file path and display help
4. WHEN configuration is needed THEN access tokens, OpenRouter secrets, and model selection SHALL be configurable via environment variables
5. WHEN environment variables are not set THEN the system SHALL fall back to values in alternator.toml
6. WHEN the application runs in containers THEN all configuration SHALL be available via environment variables
7. WHEN default configuration is needed THEN alternator.toml SHALL contain sensible defaults

### Requirement 9

**User Story:** As a user, I want to be notified when my OpenRouter account balance is low, so that I can top up my account before the service stops working.

#### Acceptance Criteria

1. WHEN the application runs THEN it SHALL check OpenRouter account balance daily at a configurable time (default: noon)
2. WHEN account balance is checked THEN it SHALL compare against a configurable threshold (default: $5)
3. WHEN balance falls below the threshold THEN the system SHALL send a direct message to the authenticated user
4. WHEN the direct message is sent THEN it SHALL inform the user to increase their OpenRouter account balance
5. WHEN balance checking is configured THEN both the check time and threshold SHALL be configurable in the TOML file
6. WHEN balance monitoring is enabled THEN it SHALL be on by default but SHALL be configurable to disable via environment variables or config file

### Requirement 10

**User Story:** As a cost-conscious user, I want to control OpenRouter token usage, so that I can prevent unexpectedly high API costs.

#### Acceptance Criteria

1. WHEN OpenRouter requests are made THEN the system SHALL respect a configurable maximum token limit
2. WHEN OpenRouter responses indicate token limit was reached THEN the system SHALL not retry that specific media
3. WHEN token limits are configured THEN they SHALL be specified in the TOML configuration file
4. WHEN token limit responses are received THEN the system SHALL log the occurrence and skip the media
5. WHEN cost control is implemented THEN it SHALL prevent runaway token usage scenarios

### Requirement 11

**User Story:** As a user, I want the system to handle race conditions gracefully, so that my manual edits to toots are not overwritten by automated updates.

#### Acceptance Criteria

1. WHEN updating a toot with enriched media descriptions THEN the system SHALL retrieve the current toot state first
2. WHEN the current toot state is retrieved THEN the system SHALL verify the toot has not been manually edited
3. WHEN manual edits are detected THEN the system SHALL skip the automated update to prevent overwriting user changes
4. WHEN race conditions are handled THEN the system SHALL log the occurrence for monitoring purposes