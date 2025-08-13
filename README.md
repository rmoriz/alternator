# Alternator

Automatically adds descriptions to media attachments in Mastodon toots using AI-powered image analysis.

## Features

- Real-time monitoring of Mastodon public timeline via WebSocket streaming
- AI-powered image description generation using OpenRouter API
- Language detection for appropriate description language
- Configurable image filtering (size, format, content type)
- Balance monitoring for API usage
- Robust error handling and retry mechanisms

## Installation

```bash
cargo build --release
```

## Configuration

Create a `config.toml` file with your settings:

```toml
# TODO: Add configuration example
```

## Usage

```bash
./target/release/alternator --config config.toml
```

## License

This project is licensed under the AGPL-3.0 License - see the [LICENSE](LICENSE) file for details.