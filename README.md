# Alternator

Automatically adds descriptions to media attachments in Mastodon toots using AI-powered image analysis.

[![CI](https://github.com/rmoriz/alternator/workflows/CI/badge.svg)](https://github.com/rmoriz/alternator/actions)
[![License: AGPL v3](https://img.shields.io/badge/License-AGPL%20v3-blue.svg)](https://www.gnu.org/licenses/agpl-3.0)

## Features

- **Real-time monitoring** of your Mastodon toots via WebSocket streaming
- **AI-powered image descriptions** using OpenRouter API with multiple model options
- **Multi-language support** with automatic language detection and localized prompts
- **Race condition protection** to avoid overwriting manual edits
- **Cost controls** with configurable token limits and balance monitoring
- **Comprehensive error recovery** with automatic reconnection and backoff strategies
- **Container-ready** with Docker support and environment variable configuration
- **Cross-platform** binaries for Linux and macOS (x86_64 and ARM64)

## Quick Start

### 1. Download Binary

Download the latest release for your platform from the [GitHub Releases](https://github.com/rmoriz/alternator/releases) page.

### 2. Configuration

Copy the example configuration and customize it:

```bash
cp alternator.toml.example alternator.toml
```

Edit `alternator.toml` with your credentials:

```toml
[mastodon]
instance_url = "https://your.mastodon.instance"
access_token = "your_mastodon_access_token"

[openrouter]
api_key = "your_openrouter_api_key"
model = "anthropic/claude-3-haiku"
```

### 3. Run

```bash
./alternator --config alternator.toml
```

## Installation

### From Releases (Recommended)

Download pre-built binaries from [GitHub Releases](https://github.com/rmoriz/alternator/releases):

- `alternator-linux-amd64.tar.gz` - Linux x86_64
- `alternator-linux-arm64.tar.gz` - Linux ARM64
- `alternator-macos-amd64.tar.gz` - macOS x86_64  
- `alternator-macos-arm64.tar.gz` - macOS ARM64 (Apple Silicon)

### From Source

Requires Rust 1.70.0 or later:

```bash
git clone https://github.com/rmoriz/alternator.git
cd alternator
cargo build --release
```

### Docker

```bash
# Using Docker Hub (when available)
docker run -v $(pwd)/config:/app/config alternator/alternator

# Build locally
docker build -t alternator .
docker run -v $(pwd)/config:/app/config alternator
```

## Configuration

### Configuration File

Create `alternator.toml` based on the provided example:

```toml
[mastodon]
instance_url = "https://mastodon.social"
access_token = "your_token_here"

[openrouter]
api_key = "your_api_key_here"
model = "anthropic/claude-3-haiku"
max_tokens = 150

[balance]
enabled = true
threshold = 5.0
check_time = "12:00"

[logging]
level = "info"
```

### Environment Variables

All configuration options can be overridden with environment variables:

```bash
export ALTERNATOR_MASTODON_INSTANCE_URL="https://your.instance.com"
export ALTERNATOR_MASTODON_ACCESS_TOKEN="your_token"
export ALTERNATOR_OPENROUTER_API_KEY="your_key"
export ALTERNATOR_OPENROUTER_MODEL="anthropic/claude-3-sonnet"
export ALTERNATOR_LOG_LEVEL="debug"
```

### Getting Credentials

#### Mastodon Access Token

1. Go to your Mastodon instance
2. Navigate to **Settings** → **Development** → **New Application**
3. Create an application with these scopes:
   - `read` - Read your account information and toots
   - `write` - Update media descriptions
   - `push` - Receive notifications (for balance alerts)
4. Copy the **access token**

#### OpenRouter API Key

1. Sign up at [OpenRouter](https://openrouter.ai)
2. Add credits to your account
3. Go to [API Keys](https://openrouter.ai/keys) and create a new key
4. Copy the API key

## Supported AI Models

Popular model options for the `model` configuration:

- `anthropic/claude-3-haiku` - Fast and cost-effective (recommended)
- `anthropic/claude-3-sonnet` - Balanced performance and cost
- `openai/gpt-4o-mini` - OpenAI's efficient model
- `google/gemini-pro-vision` - Google's vision model

See [OpenRouter Models](https://openrouter.ai/models) for the complete list.

## Usage

### Basic Usage

```bash
# Use config file in current directory
./alternator

# Specify config file location
./alternator --config /path/to/alternator.toml

# Set log level
./alternator --log-level debug

# Verbose mode (equivalent to --log-level debug)
./alternator --verbose
```

### Docker Usage

```bash
# Create config directory
mkdir -p config
cp alternator.toml.example config/alternator.toml
# Edit config/alternator.toml with your credentials

# Run with Docker
docker run -v $(pwd)/config:/app/config alternator/alternator
```

### Systemd Service

Create `/etc/systemd/system/alternator.service`:

```ini
[Unit]
Description=Alternator - Mastodon Media Describer
After=network.target

[Service]
Type=simple
User=alternator
WorkingDirectory=/opt/alternator
ExecStart=/opt/alternator/alternator --config /etc/alternator/alternator.toml
Restart=always
RestartSec=10

[Install]
WantedBy=multi-user.target
```

Enable and start:

```bash
sudo systemctl enable alternator
sudo systemctl start alternator
```

## How It Works

1. **Stream Monitoring**: Connects to your Mastodon instance's WebSocket stream
2. **Toot Filtering**: Identifies your own toots with media attachments
3. **Media Processing**: Filters for supported image types without descriptions
4. **Language Detection**: Determines the toot's language for appropriate prompts
5. **AI Description**: Sends images to OpenRouter for description generation
6. **Race Condition Check**: Verifies the toot hasn't been manually edited
7. **Update**: Adds the generated description to the media attachment

## Features in Detail

### Race Condition Protection

Alternator checks the current state of toots before updating to avoid overwriting manual edits:

- Fetches current toot state before processing
- Skips updates if descriptions were manually added
- Logs race condition detections for monitoring

### Balance Monitoring

Automatic monitoring of your OpenRouter account balance:

- Daily balance checks at configurable times
- Direct message notifications when balance is low
- Configurable threshold amounts
- Can be disabled if not needed

### Error Recovery

Robust error handling with automatic recovery:

- WebSocket reconnection with exponential backoff (1s to 60s max)
- API rate limit handling with proper delays
- Network timeout handling with retry mechanisms
- Graceful degradation for non-critical failures

### Multi-language Support

Generates descriptions in the detected language of your toot:

- Automatic language detection from toot content
- Language-specific prompt templates
- Fallback to English for unsupported languages
- Support for 8+ languages including English, German, French, Spanish, Italian, Dutch, Portuguese, Japanese

## Configuration Reference

### `[mastodon]` Section

| Option | Type | Required | Default | Description |
|--------|------|----------|---------|-------------|
| `instance_url` | String | Yes | - | Your Mastodon instance URL |
| `access_token` | String | Yes | - | Your Mastodon access token |
| `user_stream` | Boolean | No | `true` | Use user stream vs public timeline |

### `[openrouter]` Section

| Option | Type | Required | Default | Description |
|--------|------|----------|---------|-------------|
| `api_key` | String | Yes | - | Your OpenRouter API key |
| `model` | String | No | `"anthropic/claude-3-haiku"` | AI model to use |
| `base_url` | String | No | `"https://openrouter.ai/api/v1"` | OpenRouter API base URL |
| `max_tokens` | Integer | No | `150` | Maximum tokens per request |

### `[media]` Section

| Option | Type | Required | Default | Description |
|--------|------|----------|---------|-------------|
| `max_size_mb` | Float | No | `10.0` | Maximum file size to process (MB) |
| `supported_formats` | Array | No | `["image/jpeg", ...]` | Supported image formats |
| `resize_max_dimension` | Integer | No | `1024` | Maximum dimension for resizing |

### `[balance]` Section

| Option | Type | Required | Default | Description |
|--------|------|----------|---------|-------------|
| `enabled` | Boolean | No | `true` | Enable balance monitoring |
| `threshold` | Float | No | `5.0` | Balance threshold for notifications |
| `check_time` | String | No | `"12:00"` | Daily check time (24-hour format) |

### `[logging]` Section

| Option | Type | Required | Default | Description |
|--------|------|----------|---------|-------------|
| `level` | String | No | `"info"` | Log level: `error`, `warn`, `info`, `debug`, `trace` |

## Troubleshooting

### Common Issues

**Connection Failed**
```
Error: Failed to connect to Mastodon WebSocket
```
- Check your `instance_url` is correct
- Verify your `access_token` has the required scopes
- Ensure your Mastodon instance supports WebSocket streaming

**Authentication Failed**
```
Error: Invalid access token
```
- Regenerate your Mastodon access token
- Check the token has `read`, `write`, and `push` scopes

**OpenRouter API Error**
```
Error: OpenRouter API request failed
```
- Verify your OpenRouter API key is correct
- Check your account has sufficient credits
- Ensure the selected model is available

**Balance Too Low**
```
Warning: OpenRouter balance is low ($1.23)
```
- Add credits to your OpenRouter account
- Adjust the `threshold` setting if needed

### Debug Mode

Enable debug logging for troubleshooting:

```bash
./alternator --log-level debug
```

Or set in configuration:

```toml
[logging]
level = "debug"
```

### Container Troubleshooting

Check container logs:
```bash
docker logs <container_id>
```

Verify configuration mounting:
```bash
docker run -v $(pwd)/config:/app/config alternator/alternator ls -la /app/config
```

## FAQ

**Q: How much does it cost to run?**
A: Costs depend on your usage and chosen AI model. Claude-3-Haiku typically costs $0.002-0.01 per image description. Monitor your usage via OpenRouter's dashboard.

**Q: Can I use this with multiple Mastodon accounts?**
A: Currently, each Alternator instance supports one Mastodon account. Run multiple instances with different configurations for multiple accounts.

**Q: Will it overwrite descriptions I add manually?**
A: No. Alternator checks for existing descriptions and skips processing if descriptions are already present.

**Q: What image formats are supported?**
A: By default: JPEG, PNG, GIF, and WebP. You can customize this in the configuration.

**Q: Can I run this on a server?**
A: Yes. Alternator is designed to run as a service. See the systemd service example above.

**Q: Is there a rate limit?**
A: Alternator respects both Mastodon and OpenRouter rate limits with automatic backoff. Default limits are usually sufficient for personal use.

## Contributing

1. Fork the repository
2. Create a feature branch
3. Make your changes
4. Add tests if applicable
5. Run the test suite: `cargo test`
6. Submit a pull request

## License

This project is licensed under the AGPL-3.0 License - see the [LICENSE](LICENSE) file for details.

## Support

- 📖 **Documentation**: This README and inline code documentation
- 🐛 **Bug Reports**: [GitHub Issues](https://github.com/rmoriz/alternator/issues)
- 💬 **Discussions**: [GitHub Discussions](https://github.com/rmoriz/alternator/discussions)
- 📧 **Security Issues**: Email security@example.com

---

**Made with ❤️ for the Mastodon community**