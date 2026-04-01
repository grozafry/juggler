# Juggler — LLM Gateway

> One OpenAI-compatible endpoint. All providers. Full observability. Zero key sprawl.
> **Core engine: Rust** · **Admin API: Go** · **Deploys with a single command**

[![License](https://img.shields.io/badge/license-MIT-blue)](#)
[![Rust](https://img.shields.io/badge/rust-1.78%2B-orange)](https://www.rust-lang.org/)
[![Go](https://img.shields.io/badge/go-1.22%2B-00ACD7)](https://golang.org/)

---

## Quick Start

**Requirements:** Docker + Docker Compose. That's it.

```bash
# 1. Clone
git clone https://github.com/grozafry/juggler && cd juggler

# 2. Configure — add your provider API key(s)
cp .env.example .env
# Edit .env: set GEMINI_API_KEY and/or ANTHROPIC_API_KEY

# 3. Start everything
docker-compose up -d

# 4. Open the dashboard
open http://localhost:8081
```

That's it. The gateway is now live at `http://localhost:8080`.

### Make your first request

A starter virtual key is automatically created when you first run `docker-compose up`:

```bash
curl -N -X POST http://localhost:8080/v1/chat/completions \
  -H "Authorization: Bearer lgw_sk_default1234567890abcdef" \
  -H "Content-Type: application/json" \
  -d '{
    "model": "production-fast",
    "messages": [{"role": "user", "content": "Hello!"}]
  }'
```

> **Note:** Replace the starter key with a proper one from the dashboard → Virtual Keys → Generate Key before sharing with your team.

### Point your existing app at Juggler

Juggler is **OpenAI API-compatible**. If you're using the OpenAI SDK, just change the base URL:

```python
# Python (openai SDK)
client = openai.OpenAI(
    base_url="http://localhost:8080/v1",
    api_key="lgw_sk_<your-virtual-key>"
)
```

```typescript
// TypeScript (openai SDK)
const client = new OpenAI({
  baseURL: "http://localhost:8080/v1",
  apiKey: "lgw_sk_<your-virtual-key>",
});
```

---

## What You Get

| Feature | Detail |
|---|---|
| **Virtual Keys** | Issue `lgw_sk_*` keys per team/app — real provider keys never leave the gateway |
| **Multi-provider routing** | Gemini + Anthropic today, extensible. Weighted load balancing. |
| **Circuit breakers** | Unhealthy providers are bypassed automatically |
| **Auto retry** | Exponential backoff on 429 / 503 — transparent to callers |
| **Cost tracking** | Per-request USD cost from provider pricing tables |
| **Latency split** | TTFB (API processing time) vs. streaming time — know who's slow |
| **Real token counts** | Input + output tokens from provider SSE events, not estimates |
| **Audit trail** | Every request logged to ClickHouse via Kafka, queryable forever |
| **Dashboard** | 7-page live dashboard — overview, latency, errors, providers, keys, logs |

---

## Services & Ports

| Service | Port | Purpose |
|---|---|---|
| Gateway proxy | `8080` | OpenAI-compatible LLM endpoint |
| Admin dashboard | `8081` | Metrics, virtual keys, audit logs |
| Postgres | `5433` | Virtual keys + workspace auth |
| Redpanda (Kafka) | `19092` | Audit log event bus |
| ClickHouse | `9000` / `8123` | Analytics store |

---

## Admin API Auth

Set `ADMIN_API_KEY` in your `.env`. All `/admin/v1/*` endpoints then require:

```
X-Admin-Key: <your-key>
```

If `ADMIN_API_KEY` is empty or `changeme`, auth is disabled (convenient for local dev).

---

## Model Routing

Edit `config.yaml` to add or adjust routes:

```yaml
routes:
  - alias: "production-fast"
    provider: gemini
    model: gemini-2.5-flash
    weight: 80
  - alias: "production-smart"
    provider: anthropic
    model: claude-3-5-sonnet-20241022
    weight: 20
```

No gateway restart required — config is re-read on startup.

---

---

## Why we built this

Every team at the company was talking to LLM providers independently — reinventing retry logic, hardcoding API keys in environment variables, and generating zero visibility into what was being spent, by whom, and on which models. When a provider went down, every team felt it separately.

This gateway centralises all of that. One OpenAI-compatible endpoint. One place for cost governance, guardrails, failover, and audit. Every product team gets the benefits without changing a line of their integration code.

**The problems it solves:**

- No visibility into which team is spending what on which model
- Provider outages propagating directly to end users with no failover
- Every team reimplementing retry, rate-limit handling, and error normalisation
- Raw provider API keys scattered across services and environment configs
- No PII scanning or prompt injection detection on outbound requests
- No semantic deduplication — identical or near-identical prompts hitting providers repeatedly

---

## Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                        Clients                              │
│   App Services · Agent Pipelines · Internal Tools · CI/CD  │
└────────────────────────────┬────────────────────────────────┘
                             │  OpenAI-compatible API
                             ▼
┌─────────────────────────────────────────────────────────────┐
│                  LLM Gateway  (Rust)                        │
│   Auth · Rate Limit · Request ID injection                  │
├──────────────┬──────────────┬──────────────┬────────────────┤
│  Guardrails  │ Semantic     │   Token      │   Request      │
│  (PII, inj,  │ Cache        │   Budgeting  │   Enrichment   │
│   toxicity)  │ (ONNX+Qdrant)│   (Redis)    │   (OTel)       │
├──────────────┴──────────────┴──────────────┴────────────────┤
│              Intelligent Router  (Rust)                     │
│   Circuit Breaker · Weighted Failover · Cost Scoring        │
└──────┬──────────┬──────────┬──────────┬──────────┬──────────┘
       │          │          │          │          │
   OpenAI    Anthropic    Bedrock   Vertex AI   vLLM/OSS
                                              (self-hosted)
```

**Language split — this boundary is firm:**

| Layer | Language | Reason |
|---|---|---|
| Request proxy, SSE streaming | Rust | Zero-copy I/O, no GC pauses, <1ms P99 target |
| Semantic cache hot path | Rust | In-process ONNX embedding, no FFI overhead |
| Guardrails engine | Rust | Regex + NER in tight loop; 10× faster than alternatives |
| Circuit breaker / router | Rust | Lock-free atomics (`std::sync::atomic`) |
| Token budget enforcement | Rust | Atomic Redis Lua — must be consistent under concurrency |
| Admin REST API | Go | CRUD + ORM; rapid iteration; rich ecosystem |
| Config hot-reload service | Go | PostgreSQL LISTEN/NOTIFY; simpler concurrency model |
| Provider health monitor | Go | Background goroutines |
| Audit event pipeline | Go | Kafka producer with batching |
| Observability collector | Go | OTel Go SDK is mature |
| CLI / SDK tooling | Go | Fast compilation, single binaries |

---

## Features

### Core Proxy
- **OpenAI-compatible API surface** — `/v1/chat/completions`, `/v1/completions`, `/v1/embeddings`, `/v1/images/generations`. Zero client-side changes required.
- **Streaming SSE** — end-to-end chunk forwarding with zero buffering. TTFT overhead < 0.5ms.
- **Provider adapters** — OpenAI, Anthropic, AWS Bedrock, Google Vertex AI, Azure OpenAI, Cohere, Mistral, Groq, and a generic OpenAI-compat adapter for vLLM/Ollama.
- **HTTP/2 connection pooling** — persistent per-provider pools, configurable size.
- **Model aliasing** — clients call `model: "production-fast"`, gateway resolves to the current routing policy.

### Reliability
- **Circuit breaker** — per-provider, three-state (Closed → Half-Open → Open), lock-free atomic state machine. Configurable error-rate threshold and rolling window.
- **Weighted failover** — scored by P95 latency (EWMA), cost-per-token, and operator weight. Deterministic and auditable.
- **Retry with jitter** — exponential backoff + full jitter on 429/503/524. Respects `Retry-After`. Never retries 4xx client errors.
- **Multi-key load distribution** — distribute across multiple API keys per provider with per-key rate-limit tracking.

### Semantic Cache
- **In-process embedding** — `all-MiniLM-L6-v2` via ONNX Runtime. No remote call on the hot path. P99 embedding < 5ms.
- **ANN similarity search** — Qdrant vector store. Cosine similarity threshold configurable per workspace (default: 0.93).
- **Cache isolation** — scoped to `(workspace_id, model_name, system_prompt_hash)`. Workspace A never gets Workspace B's cached responses.
- **Cache bypass** — `Cache-Control: no-cache` or `X-LLM-Cache-Bypass: true`.
- **Cache warming** — on startup, pre-populates from the last 7 days of high-frequency prompt embeddings in ClickHouse.

### Guardrails
- **PII detector** — SSNs, card numbers, phone numbers, emails, passport numbers. Configurable action: log / redact / block.
- **Prompt injection detector** — pattern library + semantic similarity to known attack embeddings.
- **Topic/scope guard** — zero-shot classification against a workspace-defined scope description.
- **Toxicity classifier** — in-process binary classifier on completions. P99 < 10ms.
- **Schema validation** — validates JSON-mode completions against registered JSON Schema. Auto-retries with repair prompt.
- **Agent loop breaker** — tracks tool-call chain depth per session. Terminates at configurable limit (default: 20).
- **Post-call scanning** — PII leak detection and toxicity check on every completion before delivery to client.

### Cost Governance
- **4-level budget hierarchy** — Organisation → Workspace → Project → Virtual Key. Limits are cumulative.
- **Budget dimensions** — tokens per minute, tokens per day, tokens per month, USD per day, USD per month.
- **Atomic enforcement** — Redis + Lua script (check-and-increment in a single roundtrip). No double-spending under concurrency.
- **Graceful degradation** — optionally route to a cheaper fallback model on budget exhaustion instead of hard-blocking.
- **Virtual key system** — clients authenticate with `lgw_<env>_<uuid>` keys. Real provider credentials never leave the secrets manager.
- **Zero-downtime key rotation** — add → drain → deactivate. Triggerable via admin API or on a schedule.

### Observability
- **Distributed tracing** — OpenTelemetry spans for every pipeline stage. W3C `traceparent` propagation. Exports to Tempo/Jaeger.
- **Prometheus metrics** — 10 metrics covering requests, latency (P50/P95/P99), TTFT, tokens, cost, cache hit ratio, circuit breaker state, guardrail triggers, and budget utilisation.
- **Structured logging** — JSON-only, zero-allocation (`tracing` crate / `zerolog`). No prompt content in logs — only hash + token count.
- **Audit log** — immutable records in ClickHouse. 13-month retention. Queryable via admin API.

### Admin Plane
- **REST API** — full CRUD for workspaces, virtual keys, budgets, routes, guardrail policies, provider credentials, and audit queries.
- **Config hot-reload** — route configs and budget limits update without restart. Propagation via Redis pub/sub → `ArcSwap` in gateway instances. < 5s end-to-end.
- **RBAC** — 5 roles: Super Admin, Workspace Admin, Developer, Read-Only Analyst, Audit Viewer.

---

## Performance Targets

| Metric | Target |
|---|---|
| Gateway-added latency P50 | < 0.3 ms |
| Gateway-added latency P99 | < 1.0 ms |
| Gateway-added latency P99.9 | < 5.0 ms |
| Semantic cache lookup P99 | < 5 ms |
| Guardrails pipeline P99 | < 15 ms |
| Streaming TTFT overhead | < 0.5 ms |
| Max throughput per pod (4 vCPU) | > 5,000 RPS |
| Availability | 99.99% |
| Cold start time | < 2 seconds |

---

## Tech Stack

| Component | Technology |
|---|---|
| Hot path runtime | Rust (Tokio, Hyper, Axum) |
| Admin services | Go (Gin / Echo) |
| Vector store | Qdrant |
| Cache / budget store | Redis Cluster |
| Config & RBAC database | PostgreSQL |
| Audit & cost store | ClickHouse |
| Metrics | Prometheus + Grafana |
| Tracing | OpenTelemetry → Tempo |
| Secrets | HashiCorp Vault |
| Event bus | Kafka / Redis Streams |
| Embedding runtime | ONNX Runtime (in-process) |
| Infra | Terraform + Helm + ArgoCD |

---

## Repository Structure

```
llm-gateway/
├── crates/                         # Rust workspace
│   ├── gateway-core/               # Hot path: proxy, router, SSE
│   ├── gateway-cache/              # Semantic cache (ONNX + Qdrant)
│   ├── gateway-guardrails/         # Guardrails pipeline
│   ├── gateway-budget/             # Token budget enforcement
│   └── gateway-common/             # Shared types, errors, config
├── services/                       # Go services
│   ├── admin-api/                  # REST admin API
│   ├── config-reloader/            # Hot-reload service
│   ├── health-monitor/             # Provider health checks
│   └── audit-pipeline/             # Kafka → ClickHouse consumer
├── sdk/                            # Client SDKs
│   ├── go/                         # Go SDK
│   └── python/                     # Python SDK (thin wrapper)
├── deploy/
│   ├── helm/                       # Helm chart
│   └── terraform/                  # Cloud infra modules
├── docs/
│   ├── adr/                        # Architecture Decision Records
│   ├── runbooks/                   # Operational runbooks
│   └── api/                        # OpenAPI spec
├── tests/
│   ├── integration/                # Docker Compose integration tests
│   ├── load/                       # k6 load test scripts
│   └── chaos/                      # toxiproxy chaos scenarios
├── docker-compose.dev.yml          # Local dev environment
├── Cargo.toml                      # Rust workspace manifest
└── README.md
```

---

## Getting Started

### Prerequisites

- Rust 1.78+ (`rustup install stable`)
- Go 1.22+
- Docker + Docker Compose
- `kubectl` + `helm` (for Kubernetes deployment)

### Local Development

```bash
# Clone the repo
git clone https://github.com/your-org/llm-gateway
cd llm-gateway

# Start all dependencies (Redis, Postgres, Qdrant, Kafka, mock LLM provider)
docker compose -f docker-compose.dev.yml up -d

# Run database migrations
go run ./services/admin-api/cmd/migrate

# Build and run the Rust gateway
cargo run -p gateway-core

# In a separate terminal, run the Go admin service
go run ./services/admin-api/cmd/server
```

The gateway listens on `:8080` (proxy) and `:8081` (admin API).

### Make your first request

```bash
# Create a workspace and virtual key via admin API
curl -X POST http://localhost:8081/admin/v1/workspaces \
  -H "Authorization: Bearer <admin-jwt>" \
  -d '{"name": "my-team", "budget_usd_monthly": 500}'

curl -X POST http://localhost:8081/admin/v1/virtual-keys \
  -H "Authorization: Bearer <admin-jwt>" \
  -d '{"workspace_id": "<id>", "allowed_models": ["gpt-4o", "claude-3-5-sonnet"]}'

# Use the virtual key to call any supported model
curl -X POST http://localhost:8080/v1/chat/completions \
  -H "Authorization: Bearer lgw_dev_<uuid>" \
  -H "Content-Type: application/json" \
  -d '{
    "model": "gpt-4o",
    "messages": [{"role": "user", "content": "Hello"}]
  }'
```

---

## Configuration

The gateway supports two config modes:

**File mode** (simple, GitOps-friendly):
```yaml
# config.yaml
server:
  port: 8080
  admin_port: 8081

providers:
  openai:
    base_url: https://api.openai.com/v1
    timeout_ttft_ms: 5000
    timeout_stream_ms: 120000
    pool_size: 50

routing:
  default_route:
    primary: openai/gpt-4o
    fallbacks:
      - anthropic/claude-3-5-sonnet
      - openai/gpt-4o-mini

cache:
  enabled: true
  similarity_threshold: 0.93
  ttl_seconds: 86400

guardrails:
  pii_detection:
    enabled: true
    action: redact        # log | redact | block
  injection_detection:
    enabled: true
    action: block
  toxicity:
    enabled: true
    threshold: 0.85
```

**Database mode** — all config lives in Postgres and is hot-reloadable via admin API. Used in production.

---

## Deployment

### Kubernetes (Helm)

```bash
helm repo add llm-gateway https://your-org.github.io/llm-gateway
helm install llm-gateway llm-gateway/llm-gateway \
  --namespace llm-gateway \
  --create-namespace \
  -f values.production.yaml
```

Key Kubernetes resources provisioned:
- `Deployment` — min 3 replicas, anti-affinity across AZs
- `HorizontalPodAutoscaler` — scales on CPU (60%) and RPS (3,000/pod)
- `PodDisruptionBudget` — `minAvailable: 2`
- `NetworkPolicy` — default-deny, explicit egress only to providers + data stores
- Vault Agent sidecar — injects provider secrets at pod startup

### Terraform

```bash
cd deploy/terraform
terraform init
terraform workspace select production
terraform apply -var-file=envs/production.tfvars
```

Provisions: Redis Cluster (ElastiCache), PostgreSQL (RDS), Kafka (MSK), Qdrant, ClickHouse.

---

## Testing

```bash
# Unit tests (Rust)
cargo test --workspace

# Unit tests (Go)
go test ./...

# Integration tests (requires Docker Compose dev stack)
go test ./tests/integration/...

# Load test (k6)
k6 run tests/load/baseline.js --vus 500 --duration 10m

# Chaos test — provider failure scenario
go test ./tests/chaos/... -run TestProviderFailover
```

### Load test acceptance criteria (must pass before every release)

- Sustained 10,000 RPS for 30 minutes with < 0.1% error rate
- P99 added latency (provider mocked at 0ms) remains < 1ms throughout
- Memory footprint does not grow by > 10% over 30 minutes
- Semantic cache hit ratio ≥ 30% on realistic prompt corpus replay

---

## Architecture Decision Records

All major technical decisions are documented in [`docs/adr/`](docs/adr/).

| ADR | Decision |
|---|---|
| [ADR-001](docs/adr/001-rust-go-split.md) | Rust for hot path, Go for admin services |
| [ADR-002](docs/adr/002-in-process-embedding.md) | In-process ONNX embedding vs remote embedding API |
| [ADR-003](docs/adr/003-atomic-budget-enforcement.md) | Redis Lua scripts for atomic budget enforcement |
| [ADR-004](docs/adr/004-qdrant-vs-pgvector.md) | Qdrant for semantic cache vs pgvector |
| [ADR-005](docs/adr/005-arcswap-config-reload.md) | ArcSwap for lock-free config hot-reload in Rust |
| [ADR-006](docs/adr/006-stateless-circuit-breaker.md) | In-process atomic circuit breaker state vs Redis |

---

## Observability

### Grafana Dashboards

Pre-built dashboards in [`deploy/helm/dashboards/`](deploy/helm/dashboards/):

- **Gateway Overview** — RPS, error rate, P50/P95/P99 latency, TTFT
- **Cost Attribution** — daily/monthly spend by workspace, model, and provider
- **Semantic Cache** — hit ratio over time, estimated USD saved, eviction rate
- **Provider Health** — circuit breaker state, per-provider latency and error rate
- **Guardrails** — trigger counts by type and action, false-positive tracking
- **Budget Utilisation** — per-workspace budget burn rate, exhaustion forecasts

### Key metrics

```
llm_gateway_requests_total{workspace, model, provider, status_code, cache_status}
llm_gateway_request_duration_seconds{provider, model, route, phase}
llm_gateway_ttft_seconds{provider, model}
llm_gateway_tokens_total{workspace, model, direction}
llm_gateway_cost_usd_total{workspace, model, provider}
llm_gateway_cache_hit_ratio{workspace, model}
llm_gateway_circuit_breaker_state{provider}   # 0=closed 1=half-open 2=open
llm_gateway_guardrail_triggers_total{stage, action, workspace}
llm_gateway_budget_utilisation_ratio{workspace, dimension}
```

---

## Roadmap

- [x] Phase 0 — Monorepo, CI pipeline, dev environment
- [x] Phase 1 — Core Rust proxy, OpenAI + Anthropic adapters, SSE streaming
- [ ] Phase 2 — Circuit breaker, failover router, retry, health monitor
- [ ] Phase 3 — Semantic cache (ONNX + Qdrant), cache warming
- [ ] Phase 4 — Guardrails engine (PII, injection, toxicity, agent loop)
- [ ] Phase 5 — Virtual key system, RBAC, budget enforcement, secrets manager
- [ ] Phase 6 — Full observability (OTel, Prometheus, ClickHouse audit)
- [ ] Phase 7 — Admin REST API, config hot-reload, Grafana dashboards
- [ ] Phase 8 — Chaos testing, OWASP review, load test CI gate
- [ ] Phase 9 — Multi-region production deployment

---

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md). In particular:

- All hot-path code (anything under `crates/`) must be Rust. No exceptions.
- All changes to `crates/gateway-core` require a benchmark comparison in the PR.
- New provider adapters must include contract tests against the provider's API spec.
- Security-sensitive changes (guardrails, auth, budget enforcement) require two senior engineer approvals.

---

## License

MIT — see [LICENSE](LICENSE).
