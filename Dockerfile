# Multi-stage build for optimal image size
FROM rust:1.83-slim AS builder

# Install build dependencies
RUN apt-get update && apt-get install -y \
    pkg-config \
    libssl-dev \
    && rm -rf /var/lib/apt/lists/*

# Create app user for security
RUN useradd -m -u 1001 alternator

# Set working directory
WORKDIR /app

# Copy dependency manifests
COPY Cargo.toml Cargo.lock ./

# Copy source code
COPY src/ ./src/

# Build the application in release mode
RUN cargo build --release

# Runtime stage with Python and Whisper support
FROM python:3.11-slim

# Install system dependencies including FFmpeg and Python tools
RUN apt-get update && apt-get install -y \
    ca-certificates \
    ffmpeg \
    curl \
    unzip \
    && rm -rf /var/lib/apt/lists/*

# Install Deno JavaScript runtime (required for upcoming yt-dlp changes)
RUN curl -fsSL https://deno.land/install.sh | DENO_INSTALL=/usr/local sh \
    && chmod +x /usr/local/bin/deno

# Install OpenAI Whisper
RUN pip3 install --no-cache-dir openai-whisper

# Install Universal GPU Support (AMD + NVIDIA)
# Install both CUDA and ROCm PyTorch wheels for maximum compatibility
RUN pip3 install --no-cache-dir torch torchvision torchaudio --index-url https://download.pytorch.org/whl/cu118
RUN pip3 install --no-cache-dir torch torchvision torchaudio --index-url https://download.pytorch.org/whl/rocm5.4.2

# Create app user with same UID as builder stage
RUN useradd -m -u 1001 alternator

# Create directories for configuration and models
RUN mkdir -p /app/config /app/models && chown -R alternator:alternator /app

# Copy binary from builder stage
COPY --from=builder /app/target/release/alternator /usr/local/bin/alternator
RUN chmod +x /usr/local/bin/alternator

# Switch to non-root user
USER alternator

# Set working directory
WORKDIR /app

# Create volumes for configuration and models
VOLUME ["/app/config", "/app/models"]

# Environment variables for container deployment
ENV ALTERNATOR_CONFIG_PATH=/app/config/alternator.toml
ENV RUST_LOG=info
# Default Whisper model cache directory (can be overridden by model_dir config)
ENV XDG_CACHE_HOME=/app/models

# Health check
HEALTHCHECK --interval=30s --timeout=10s --start-period=5s --retries=3 \
  CMD pgrep alternator || exit 1

# Run the application
ENTRYPOINT ["alternator"]
CMD ["--config", "/app/config/alternator.toml"]