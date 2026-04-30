# control-plane/Dockerfile
FROM docker.io/library/rust:1.86-slim-bookworm AS builder

# 1. ADD THIS SECTION HERE:
RUN apt-get update && apt-get install -y \
    pkg-config \
    libssl-dev \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /build

COPY Cargo.toml Cargo.lock ./
COPY sidecar/Cargo.toml ./sidecar/
COPY control-plane/Cargo.toml ./control-plane/
COPY kv-relay/Cargo.toml ./kv-relay/

RUN mkdir -p sidecar/src control-plane/src kv-relay/src \
 && echo "fn main() {}" > sidecar/src/main.rs \
 && echo "fn main() {}" > control-plane/src/main.rs \
 && echo "fn main() {}" > kv-relay/src/main.rs

RUN cargo build --release -p ghostcacher-control-plane 2>/dev/null || true

COPY control-plane/src ./control-plane/src
RUN touch control-plane/src/main.rs

RUN cargo build --release -p ghostcacher-control-plane
RUN strip /build/target/release/ghostcacher-control-plane

FROM gcr.io/distroless/cc-debian12:nonroot

LABEL org.opencontainers.image.title="GhostCacher Control Plane"
LABEL org.opencontainers.image.version="2.1.0"

COPY --from=builder /build/target/release/ghostcacher-control-plane /usr/local/bin/ghostcacher-control-plane

USER nonroot

EXPOSE 7070

ENTRYPOINT ["/usr/local/bin/ghostcacher-control-plane"]
