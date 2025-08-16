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

# Runtime stage with minimal base image
FROM debian:trixie-slim

# Install runtime dependencies including FFmpeg for audio/video processing
RUN apt-get update && apt-get install -y \
    ca-certificates \
    ffmpeg \
    && rm -rf /var/lib/apt/lists/*

# Create app user with same UID as builder stage
RUN useradd -m -u 1001 alternator

# Create directory for configuration
RUN mkdir -p /app/config && chown -R alternator:alternator /app

# Copy binary from builder stage
COPY --from=builder /app/target/release/alternator /usr/local/bin/alternator
RUN chmod +x /usr/local/bin/alternator

# Switch to non-root user
USER alternator

# Set working directory
WORKDIR /app

# Create volume for configuration
VOLUME ["/app/config"]

# Environment variables for container deployment
ENV ALTERNATOR_CONFIG_PATH=/app/config/alternator.toml
ENV RUST_LOG=info

# Health check
HEALTHCHECK --interval=30s --timeout=10s --start-period=5s --retries=3 \
  CMD pgrep alternator || exit 1

# Run the application
ENTRYPOINT ["alternator"]
CMD ["--config", "/app/config/alternator.toml"]