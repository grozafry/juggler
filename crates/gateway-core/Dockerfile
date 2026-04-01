# ── Stage 1: Build ───────────────────────────────────────────────────────────
FROM rust:1.78-slim-bookworm AS builder

# System deps for sqlx, openssl, etc.
RUN apt-get update && apt-get install -y \
    pkg-config libssl-dev libpq-dev \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Cache dependency layer — copy manifests first
COPY Cargo.toml Cargo.lock ./
COPY crates/gateway-core/Cargo.toml ./crates/gateway-core/

# Dummy build to cache deps
RUN mkdir -p crates/gateway-core/src && \
    echo 'fn main() {}' > crates/gateway-core/src/main.rs && \
    cargo build --release -p gateway-core 2>/dev/null || true && \
    rm -rf crates/gateway-core/src

# Real build
COPY crates ./crates
COPY config.yaml ./
RUN cargo build --release -p gateway-core

# ── Stage 2: Runtime ─────────────────────────────────────────────────────────
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y \
    ca-certificates libssl3 libpq5 curl \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

COPY --from=builder /app/target/release/gateway-core .
COPY --from=builder /app/config.yaml .

EXPOSE 8080

HEALTHCHECK --interval=10s --timeout=5s --retries=5 \
    CMD curl -sf http://localhost:8080/health || exit 1

CMD ["./gateway-core"]
