# ===========================
# Build Stage
# ===========================
FROM rust:1.90-slim-bookworm AS builder

# Install build deps
RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config \
    libssl-dev \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Cache dependencies
COPY Cargo.toml Cargo.lock ./
# Copy real source
COPY src ./src
RUN cargo build --release

# ===========================
# Runtime Stage
# ===========================
FROM debian:bookworm-slim

# Install only runtime libs actually required
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    libssl3 \
    libgcc-s1 \
    libstdc++6 \
    libc6 \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Copy compiled binary from builder
COPY --from=builder /app/target/release/m87-server /app/m87-server


# Default entrypoint
ENTRYPOINT ["/app/m87-server"]
