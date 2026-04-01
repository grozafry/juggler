# ── Stage 1: Build ───────────────────────────────────────────────────────────
FROM golang:1.22-bookworm AS builder

WORKDIR /app

COPY services/admin-api/go.mod services/admin-api/go.sum ./
RUN go mod download

COPY services/admin-api/ .
RUN CGO_ENABLED=0 GOOS=linux go build -o admin-api .

# ── Stage 2: Runtime ─────────────────────────────────────────────────────────
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y ca-certificates curl \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

COPY --from=builder /app/admin-api .
COPY --from=builder /app/dashboard.html .

EXPOSE 8081

HEALTHCHECK --interval=10s --timeout=5s --retries=5 \
    CMD curl -sf http://localhost:8081/ || exit 1

CMD ["./admin-api"]
