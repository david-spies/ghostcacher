# sidecar/Dockerfile
# Multi-stage build — final image is ~12MB (distroless)

# ── Stage 1: Build ───────────────────────────────────────────────────────────
FROM docker.io/library/rust:1.86-slim-bookworm AS builder

# 1. ADD THIS SECTION HERE:
RUN apt-get update && apt-get install -y \
    pkg-config \
    libssl-dev \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /build

# Cache dependencies before copying source
COPY Cargo.toml Cargo.lock ./
COPY sidecar/Cargo.toml ./sidecar/
COPY control-plane/Cargo.toml ./control-plane/
COPY kv-relay/Cargo.toml ./kv-relay/

# Stub lib files so Cargo can resolve the workspace graph
RUN mkdir -p sidecar/src control-plane/src kv-relay/src \
 && echo "fn main() {}" > sidecar/src/main.rs \
 && echo "fn main() {}" > control-plane/src/main.rs \
 && echo "fn main() {}" > kv-relay/src/main.rs

# Pre-build dependencies (cached layer — only invalidated when Cargo.toml changes)
RUN cargo build --release -p ghostcacher-sidecar 2>/dev/null || true

# Copy real source
COPY sidecar/src ./sidecar/src

# Touch main.rs to force rebuild after stub
RUN touch sidecar/src/main.rs

# Build the real binary
RUN cargo build --release -p ghostcacher-sidecar

# Strip debug symbols (~40% size reduction)
RUN strip /build/target/release/ghostcacher-sidecar

# ── Stage 2: Runtime (distroless — no shell, no package manager) ─────────────
FROM gcr.io/distroless/cc-debian12:nonroot

LABEL org.opencontainers.image.title="GhostCacher Sidecar"
LABEL org.opencontainers.image.version="2.1.0"
LABEL org.opencontainers.image.description="GhostCacher distributed KV-cache sidecar proxy"
LABEL org.opencontainers.image.source="https://github.com/your-org/ghostcacher"

COPY --from=builder /build/target/release/ghostcacher-sidecar /usr/local/bin/ghostcacher-sidecar

USER nonroot

EXPOSE 8080 9090

ENTRYPOINT ["/usr/local/bin/ghostcacher-sidecar"]
