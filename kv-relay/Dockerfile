# kv-relay/Dockerfile
FROM rust:1.78-slim-bookworm AS builder

WORKDIR /build

# Install protobuf compiler (required for tonic/prost)
RUN apt-get update && apt-get install -y protobuf-compiler && rm -rf /var/lib/apt/lists/*

COPY Cargo.toml Cargo.lock ./
COPY sidecar/Cargo.toml ./sidecar/
COPY control-plane/Cargo.toml ./control-plane/
COPY kv-relay/Cargo.toml ./kv-relay/

RUN mkdir -p sidecar/src control-plane/src kv-relay/src \
 && echo "fn main() {}" > sidecar/src/main.rs \
 && echo "fn main() {}" > control-plane/src/main.rs \
 && echo "fn main() {}" > kv-relay/src/main.rs

RUN cargo build --release -p ghostcacher-kv-relay 2>/dev/null || true

COPY kv-relay/src ./kv-relay/src
RUN touch kv-relay/src/main.rs

RUN cargo build --release -p ghostcacher-kv-relay
RUN strip /build/target/release/ghostcacher-kv-relay

FROM gcr.io/distroless/cc-debian12:nonroot

LABEL org.opencontainers.image.title="GhostCacher KV-Relay"
LABEL org.opencontainers.image.version="2.1.0"

COPY --from=builder /build/target/release/ghostcacher-kv-relay /usr/local/bin/ghostcacher-kv-relay

USER nonroot

EXPOSE 50051 50052

ENTRYPOINT ["/usr/local/bin/ghostcacher-kv-relay"]
