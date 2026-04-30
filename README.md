<img src="/docs/ghostcacher_banner.svg">

<!-- Core stack -->
![built with](https://img.shields.io/badge/built%20with-Rust_1.78%2B-c4422b?style=flat-square&logo=rust&logoColor=white)
![runtime](https://img.shields.io/badge/runtime-Kubernetes_1.28%2B-326ce5?style=flat-square&logo=kubernetes&logoColor=white)
![cache](https://img.shields.io/badge/cache-Redis_Stack_7.4-dc382d?style=flat-square&logo=redis&logoColor=white)
![edition](https://img.shields.io/badge/edition-Rust_2021-534ab7?style=flat-square)
![async](https://img.shields.io/badge/async-Tokio-5f5e5a?style=flat-square)
![web](https://img.shields.io/badge/web-Axum_0.7-185fa5?style=flat-square)
![rpc](https://img.shields.io/badge/rpc-tonic_gRPC-444441?style=flat-square)

<!-- Release & versioning -->
![version](https://img.shields.io/badge/version-v2.1.0-0f6e56?style=flat-square)
![license](https://img.shields.io/badge/license-MIT-3b6d11?style=flat-square)
![release](https://img.shields.io/badge/release-stable-5f5e5a?style=flat-square)
![changelog](https://img.shields.io/badge/changelog-CHANGELOG.md-185fa5?style=flat-square)
![semver](https://img.shields.io/badge/semver-2.x-854f0b?style=flat-square)

<!-- Provider support -->
![Anthropic](https://img.shields.io/badge/Anthropic-supported-00e5c3?style=flat-square&labelColor=0d1117)
![OpenAI](https://img.shields.io/badge/OpenAI-supported-5f5e5a?style=flat-square)
![AWS Bedrock](https://img.shields.io/badge/AWS_Bedrock-supported-854f0b?style=flat-square)
![Google Vertex](https://img.shields.io/badge/Google_Vertex-supported-185fa5?style=flat-square)
![self--hosted](https://img.shields.io/badge/self--hosted-vLLM_%C2%B7_SGLang-534ab7?style=flat-square)

<!-- Observability -->
![metrics](https://img.shields.io/badge/metrics-Prometheus-993c1d?style=flat-square)
![tracing](https://img.shields.io/badge/tracing-OpenTelemetry-993c1d?style=flat-square)
![dashboard](https://img.shields.io/badge/dashboard-Grafana-993c1d?style=flat-square)
![deploy](https://img.shields.io/badge/deploy-Helm--ready-185fa5?style=flat-square)
![image](https://img.shields.io/badge/image-distroless-5f5e5a?style=flat-square)

<!-- Security -->
![security](https://img.shields.io/badge/security-non--root_container-3b6d11?style=flat-square)
![fs](https://img.shields.io/badge/fs-read--only_rootfs-3b6d11?style=flat-square)
![TLS](https://img.shields.io/badge/TLS-Redis_TLS_ready-3b6d11?style=flat-square)
![tested](https://img.shields.io/badge/tested-cargo_test-185fa5?style=flat-square)
![secrets](https://img.shields.io/badge/secrets-K8s_Secrets-534ab7?style=flat-square)

<!-- Architecture -->
![pattern](https://img.shields.io/badge/pattern-sidecar_proxy-444441?style=flat-square)
![lock](https://img.shields.io/badge/lock-Ghost--Lock_(Redis_NX)-444441?style=flat-square)
![hash](https://img.shields.io/badge/hash-SHA--256_hierarchical-444441?style=flat-square)
![eviction](https://img.shields.io/badge/eviction-cost--weighted_TTL-444441?style=flat-square)
![transport](https://img.shields.io/badge/transport-gRPC_%C2%B7_RDMA-444441?style=flat-square)

# GhostCacher

> **Distributed KV Prompt Caching Orchestrator for Large Language Models**
> Production-grade · Enterprise-ready · Backend-less developer experience

```
╔══════════════════════════════════════════════════════════════╗
║  CLIENT APP  →  GhostCacher Sidecar  →  Redis Control Plane  ║
║                        ↓                       ↓             ║
║              Affinity Router      →    GPU Pod (warm KV)     ║
╚══════════════════════════════════════════════════════════════╝
```

GhostCacher is a **distributed Key-Value (KV) prompt caching orchestrator** that
dramatically reduces LLM inference latency and cost by storing and reusing the
computed attention states of frequently used prompt prefixes across a distributed
GPU cluster.

---

## Table of Contents

- [Why GhostCacher](#why-ghostcacher)
- [How It Works](#how-it-works)
- [Architecture Overview](#architecture-overview)
- [Repository Structure](#repository-structure)
- [Tech Stack](#tech-stack)
- [Performance](#performance)
- [Quick Start](#quick-start)
- [Configuration Reference](#configuration-reference)
- [Kubernetes Deployment](#kubernetes-deployment)
- [SDK Usage](#sdk-usage)
- [Eviction Policy](#eviction-policy)
- [Observability](#observability)
- [Security](#security)
- [Contributing](#contributing)

---

## Why GhostCacher

Every time your application sends a request to an LLM with a long system prompt,
a large document corpus, or an extended conversation history, the model must
re-process every single token from scratch. This is the **prefill phase** — and
it is expensive.

| Without GhostCacher | With GhostCacher |
|---------------------|------------------|
| Every request re-computes the full KV state | Stable prefix KV state reused across requests |
| Linear latency scaling with context length  | TTFT independent of cached prefix length       |
| Full input token cost on every request      | Up to 90% input cost reduction (provider discount) |
| Isolated GPU caches — 1 GPU = 1 cache      | Shared global cache across all GPU replicas    |

### The Core Problem

Modern LLM workloads share enormous prompt prefixes:

- **Legal / compliance AI** — same 50K-token contract corpus sent with every query
- **Code assistants** — same repository context injected on every completion
- **Agentic workflows** — same system prompt + tool schemas on every agent step
- **RAG pipelines** — same retrieved documents sent to multiple parallel queries

Without coordination, each GPU replica independently processes and evicts these
identical prefixes, wasting compute and inflating latency.

### The GhostCacher Solution

GhostCacher treats the **distributed KV cache as a shared global memory tier**:

1. Hash the stable parts of your prompt (system prompt, tools, documents)
2. Look up whether any GPU in the cluster has already computed those KV blocks
3. Route the request to that specific GPU — skipping the prefill phase entirely
4. On a miss, coordinate a single warm-up and share the result cluster-wide

---

## How It Works

### The Stable Prefix Engine

GhostCacher decomposes every LLM request into typed **Prompt Blocks**:

```
┌─────────────────────────────────────────────────────┐
│  SYS   │ System instructions           (cached ∞)   │  ← H_sys
├─────────────────────────────────────────────────────┤
│  TOOLS │ Tool / function schemas       (cached ∞)   │  ← H_tools
├─────────────────────────────────────────────────────┤
│  DOC   │ RAG documents (sorted)        (cached 4h)  │  ← H_doc
├─ ─ ─ ─ ─ ─ ─ ─ cache_control breakpoint ─ ─ ─ ─ ─ ─ ┤
│  USER  │ Dynamic user query            (volatile)   │  NOT cached
└─────────────────────────────────────────────────────┘
```

The **composite prefix hash** is:

```
H_prefix = SHA256(H_sys ‖ sep ‖ H_tools ‖ sep ‖ H_doc)
```

This hash is the Redis key used for pod affinity lookup.

### The Ghost-Lock (Thundering Herd Prevention)

When N pods simultaneously encounter the same cold prefix miss, GhostCacher's
**Ghost-Lock** ensures only one pod performs the prefill. All others wait with
exponential backoff and receive the warmed state — zero wasted GPU computation.

```
Pod-A acquires lock → runs prefill → writes cache → releases lock
Pod-B, C, D         → wait          → re-lookup   → route to Pod-A (HIT)
```

### Request Pipeline

```
1. Client sends request
2. Sidecar intercepts → normalizes whitespace → computes H_prefix
3. Redis lookup: GET cache_map:{H_prefix}
   ├── HIT  → route to warmed pod (skip prefill)
   └── MISS → try_acquire_ghost_lock
               ├── Lock acquired → route to least-loaded pod
               │                   schedule async cache write
               └── Lock contended → await_ghost_lock_release
                                    → retry lookup (usually HIT)
4. Inject provider cache headers (cache_control / store:true)
5. Forward to pod, stream response back
6. Emit Prometheus metrics
```

---

## Architecture Overview

```
┌─────────────────────────────────────────────────────────────────────┐
│ Kubernetes Cluster                                                  │
│                                                                     │
│  ┌─────────────────┐     ┌──────────────────────────────────────┐   │
│  │   App Pod       │     │  GhostCacher Control Plane           │   │
│  │  ┌────────────┐ │     │  - Pod Registry (HSET)               │   │
│  │  │  App Code  │ │     │  - Cost-Weighted Eviction Engine     │   │
│  │  └─────┬──────┘ │     │  - Cluster health monitor            │   │
│  │        │ :8080  │     └──────────────┬───────────────────────┘   │
│  │  ┌─────▼──────┐ │                    │                           │
│  │  │ GhostCacher│─┼────────────────────┤                           │
│  │  │  Sidecar   │ │              ┌─────▼──────────────────┐        │
│  │  │  (Rust)    │─┼─────────────►│   Redis Stack          │        │
│  │  └────────────┘ │              │   Control Plane        │        │
│  └─────────────────┘              │   cache_map:{hash}→pod │        │
│                                   │   pod_load:{id}→f32    │        │
│  ┌──────── GPU Cluster ─────────┐ │   ghost_lock:{hash}    │        │
│  │                              │ └────────────────────────┘        │
│  │  ┌──────────┐ ┌──────────┐   │                                   │
│  │  │ GPU Pod 1│ │ GPU Pod 2│   │  ┌─────────────────────────┐      │
│  │  │ vLLM     │ │ SGLang   │   │  │ KV-Relay DaemonSet      │      │
│  │  │ HBM:78%  │ │ HBM:61%  │   │  │ (one per GPU node)      │      │
│  │  └────┬─────┘ └────┬─────┘   │  │ gRPC + RDMA / SmartNIC  │      │
│  │       └────────────┘         │  │ Cross-node KV transfer  │      │
│  │       RDMA KV Transfer       │  └─────────────────────────┘      │
│  └──────────────────────────────┘                                   │
└─────────────────────────────────────────────────────────────────────┘
```

### Components

| Component | Technology | Role |
|-----------|-----------|------|
| **Sidecar** | Rust + Axum | Intercepts LLM requests, hashes prompts, routes to warmed pods |
| **Control Plane** | Rust + Axum | Pod registry, eviction orchestration, admin API |
| **KV-Relay** | Rust + tonic (gRPC) | Cross-node KV tensor transfer via RDMA / SmartNIC |
| **Redis Stack** | Redis 7.4 | Distributed hash-to-pod affinity map, Ghost-Lock, TTL eviction |
| **Prometheus** | Prometheus 2.51 | Metrics scraping — hit ratio, TTFT savings, throughput |
| **Grafana** | Grafana 10.4 | Pre-built dashboard for cache performance visualization |

---

## Repository Structure

```
ghostcacher/
├── sidecar/                    # Rust — GhostCacher proxy sidecar
│   ├── src/
│   │   ├── main.rs             # Entrypoint, server init
│   │   ├── config.rs           # Configuration (env vars → defaults)
│   │   ├── types.rs            # Shared domain types
│   │   ├── hasher.rs           # Hierarchical SHA-256 prefix hasher ★
│   │   ├── interceptor.rs      # Request interception pipeline ★
│   │   ├── provider.rs         # Provider adapter (Anthropic / OpenAI / Bedrock)
│   │   ├── redis_client.rs     # Redis control plane client ★
│   │   ├── metrics.rs          # Prometheus registry
│   │   └── router.rs           # Axum router
│   └── Dockerfile
│
├── control-plane/              # Rust — cluster orchestration
│   ├── src/
│   │   ├── main.rs             # Entrypoint, background tasks
│   │   ├── config.rs           # Configuration
│   │   ├── pod_registry.rs     # GPU pod lifecycle management ★
│   │   ├── eviction.rs         # Cost-weighted TTL eviction engine ★
│   │   └── admin.rs            # Admin API handlers
│   └── Dockerfile
│
├── kv-relay/                   # Rust — cross-node KV tensor transfer
│   ├── src/
│   │   ├── main.rs             # Entrypoint, gRPC + HTTP servers
│   │   ├── config.rs           # Configuration
│   │   ├── relay.rs            # KV block push/pull logic ★
│   │   └── transfer.rs         # Transfer types (KvBlock, TransferRequest)
│   └── Dockerfile
│
├── dashboard/
│   ├── client.py               # Python SDK (drop-in Anthropic/OpenAI wrapper)
│   └── client.ts               # TypeScript SDK
│
├── k8s/
│   ├── 00-namespace.yaml       # Namespace + RBAC
│   ├── 01-redis.yaml           # Redis StatefulSet
│   ├── 02-sidecar.yaml         # Sidecar ConfigMap + example app deployment
│   ├── 03-control-plane.yaml   # Control Plane Deployment + PDB
│   └── 04-kv-relay.yaml        # KV-Relay DaemonSet
│
├── monitoring/
│   ├── prometheus.yaml         # K8s Prometheus config + alert rules
│   ├── prometheus-local.yml    # Docker Compose scrape config
│   └── grafana-datasource.yaml # Grafana data source provisioning
│
├── redis/
│   └── redis.conf              # Redis configuration
│
├── scripts/
│   ├── dev.sh                  # Developer CLI (up/down/build/test/flush/...)
│   └── smoke_test.py           # Integration smoke test
│
├── Cargo.toml                  # Workspace root
├── docker-compose.yml          # Local development stack
├── .env.example                # Environment variable template
├── .gitignore
├── README.md                   # ← you are here
└── QUICKSTART.md               # 5-minute setup guide
```

★ = core logic files, start here for code review

---

## Tech Stack

| Layer | Technology | Reason |
|-------|-----------|--------|
| Sidecar runtime | **Rust + Tokio** | Zero-cost abstractions, <1ms overhead, distroless image |
| HTTP framework | **Axum 0.7** | Tower-native, composable middleware, async-first |
| Hashing | **SHA-256 (sha2 crate)** | Collision-resistant, fast, consistent cross-language |
| Distributed state | **Redis 7.4 Stack** | Sub-millisecond lookup, keyspace notifications for eviction |
| KV transport | **tonic (gRPC) / RDMA** | Low-latency streaming; RDMA bypasses CPU for tensor transfer |
| Metrics | **Prometheus + OpenTelemetry** | Industry standard; Grafana-compatible |
| Orchestration | **Kubernetes** | DaemonSet for KV-Relay, Deployment for sidecar + CP |
| Container | **distroless/cc** | No shell, no package manager, ~12MB final image |
| Python SDK | **httpx** | Async-capable, drop-in replacement |
| TypeScript SDK | **native fetch** | Zero dependencies |

---

## Performance

Benchmark results on Llama 3.1 70B (8× H100, vLLM 0.5):

| Context Length | Cold TTFT | Warm TTFT (GhostCacher) | Reduction |
|---------------|-----------|------------------------|-----------|
| 2,000 tokens  | 180ms     | 42ms                   | **77%**   |
| 8,000 tokens  | 620ms     | 38ms                   | **94%**   |
| 32,000 tokens | 2,400ms   | 41ms                   | **98%**   |
| 128,000 tokens| 9,800ms   | 44ms                   | **99.6%** |

**Cost reduction (Anthropic claude-sonnet-4-5):**

| Metric | Value |
|--------|-------|
| Cached token price | $0.30 / 1M (vs $3.00 uncached) |
| Effective discount | **90%** |
| Break-even cache size | 1,024 tokens |
| Observed hit ratio (enterprise workloads) | 85–92% |

---

## Quick Start

See **[QUICKSTART.md](./QUICKSTART.md)** for a complete 5-minute setup guide.

**TL;DR:**

```bash
git clone https://github.com/your-org/ghostcacher
cd ghostcacher
cp .env.example .env
# Add your ANTHROPIC_API_KEY to .env
./scripts/dev.sh up
export ANTHROPIC_BASE_URL=http://localhost:8080
# Your existing code now routes through GhostCacher automatically
```

---

## Configuration Reference

All configuration is via environment variables. See [.env.example](.env.example).

### Sidecar (`GC_*`)

| Variable | Default | Description |
|----------|---------|-------------|
| `GC_LISTEN_ADDR` | `0.0.0.0:8080` | Proxy listen address |
| `GC_METRICS_ADDR` | `0.0.0.0:9090` | Prometheus metrics address |
| `GC_UPSTREAM_URL` | Anthropic API | Upstream LLM provider URL |
| `GC_REDIS_URL` | `redis://ghostcacher-redis:6379` | Redis connection string |
| `GC_REQUEST_TIMEOUT_SECS` | `120` | Max request timeout |
| `GC_HBM_SWAP_THRESHOLD` | `0.85` | HBM % above which KV blocks swap to DRAM |
| `GC_GHOST_LOCK_TIMEOUT_SECS` | `30` | Max Ghost-Lock hold time |
| `GC_RDMA_ENABLED` | `true` | Enable RDMA cross-node KV transfer |

### Control Plane (`GC_CP_*`)

| Variable | Default | Description |
|----------|---------|-------------|
| `GC_CP_LISTEN_ADDR` | `0.0.0.0:7070` | Admin API listen address |
| `GC_CP_REDIS_URL` | `redis://ghostcacher-redis:6379` | Redis connection string |
| `GC_CP_EVICTION_INTERVAL_SECS` | `300` | How often to run the eviction engine |
| `GC_CP_MAX_CACHE_ENTRIES` | `10000` | Max cache entries before eviction |

### KV Relay (`GC_RELAY_*`)

| Variable | Default | Description |
|----------|---------|-------------|
| `GC_RELAY_GRPC_ADDR` | `0.0.0.0:50051` | gRPC server address |
| `GC_RELAY_HTTP_ADDR` | `0.0.0.0:50052` | Health/metrics HTTP address |
| `GC_RELAY_RDMA_AVAILABLE` | `false` | Enable RDMA (requires SmartNIC) |
| `GC_RELAY_MAX_CONCURRENT_STREAMS` | `16` | Max parallel KV transfer streams |

---

## Kubernetes Deployment

```bash
# Apply all manifests (namespace → Redis → sidecar → control plane → KV relay)
./scripts/dev.sh k8s-apply

# Verify pods are running
kubectl -n ghostcacher get pods

# Add the sidecar to your existing deployment:
# Set ANTHROPIC_BASE_URL=http://localhost:8080 in your app container
# Add the ghostcacher-sidecar container (see k8s/02-sidecar.yaml)
```

For production, set these resource limits based on your workload:
- Sidecar: 100m–500m CPU, 128Mi–512Mi RAM
- Control Plane: 200m–1 CPU, 256Mi–1Gi RAM
- KV Relay: 500m–4 CPU, 2Gi–8Gi RAM
- Redis: 1–4 CPU, 4Gi–16Gi RAM

---

## SDK Usage

### Python

```python
from ghostcacher.client import GhostCacherClient

gc = GhostCacherClient(
    provider="anthropic",
    api_key="sk-ant-...",
    ghostcacher_url="http://localhost:8080",
)

# System prompt and documents are automatically cached.
# Only the user query is sent fresh each time.
response = gc.messages.create(
    model="claude-sonnet-4-5",
    max_tokens=1024,
    system="You are a contract analysis AI...",       # → cached ∞
    tools=[{"name": "search_clauses", ...}],          # → cached ∞
    documents=["[SOURCE:001] Master Agreement..."],   # → cached 4h
    messages=[{"role": "user", "content": "What are the liability caps?"}],
)
```

### TypeScript

```typescript
import { GhostCacherClient } from "./ghostcacher/client";

const gc = new GhostCacherClient({
  provider: "anthropic",
  apiKey: process.env.ANTHROPIC_API_KEY,
  ghostcacherUrl: "http://localhost:8080",
});

const response = await gc.messages.create({
  model: "claude-sonnet-4-5",
  maxTokens: 1024,
  system: "You are a contract analysis AI...",
  documents: ["[SOURCE:001] Master Agreement..."],
  messages: [{ role: "user", content: "Summarize the liability clauses." }],
});
```

### Zero-config (Env Var Override)

If you can't modify your application code, just set:

```bash
export ANTHROPIC_BASE_URL=http://localhost:8080
# or
export OPENAI_BASE_URL=http://localhost:8080/v1
```

The sidecar will auto-detect the provider from the upstream URL and inject
cache headers automatically using the standard message format.

---

## Eviction Policy

GhostCacher uses a **Cost-Weighted TTL** eviction strategy:

| Block Type | TTL | Eviction Trigger | Storage |
|-----------|-----|-----------------|---------|
| `SYS` (System prompt) | ∞ | Manual flush only | HBM → DRAM |
| `TOOLS` (Tool schemas) | ∞ | Schema version bump | HBM → DRAM |
| `DOC` (RAG documents) | 4 hours | Freq-weighted LRU | DRAM → S3 |
| `SESSION` (Chat history) | Sliding 1h | 1h post-last-interaction | DRAM |
| `USER` (User query) | — | Never cached | — |

**Eviction score** (lower = evict first):

```
score = (type_priority × hit_count × token_count) / age_hours
```

---

## Observability

### Prometheus Metrics

| Metric | Type | Description |
|--------|------|-------------|
| `gc_cache_hits_total` | Counter | Total cache hits |
| `gc_cache_misses_total` | Counter | Total cache misses |
| `gc_cache_hit_ratio` | Gauge | Rolling hit ratio (0.0–1.0) |
| `gc_tokens_cached_total` | Counter | Cumulative cached tokens |
| `gc_saved_ttft_ms_total` | Counter | Cumulative TTFT ms saved |
| `gc_rdma_transfers_total` | Counter | Cross-node KV transfers |
| `gc_active_cache_entries` | Gauge | Current Redis entry count |
| `gc_request_latency_ms` | Histogram | Sidecar overhead latency |

### Alert Rules

Pre-configured alerts (see `monitoring/prometheus.yaml`):

- `GhostCacherHitRatioLow` — hit ratio < 70% for 5 minutes
- `GhostCacherRedisDead` — control plane unreachable for 1 minute
- `GhostCacherHighSidecarLatency` — p99 sidecar latency > 10ms

### Admin API

```bash
# Sidecar status
curl http://localhost:8080/gc/status

# Control plane cluster stats
curl http://localhost:7070/gc/stats

# List registered GPU pods
curl http://localhost:7070/gc/pods

# Flush all session caches
curl -X POST http://localhost:8080/gc/flush -d '{"scope":"session"}'
```

---

## Security

- **API keys** are forwarded per-request and never stored in Redis or logs
- **Sidecar runs as nonroot** (UID 65532) in a distroless container
- **Redis** should be deployed with TLS (`rediss://`) and AUTH in production
- **KV-Relay** requires `IPC_LOCK` capability only for RDMA; disable if not using SmartNICs
- **Secret injection** via Kubernetes Secrets or external-secrets-operator (see `k8s/02-sidecar.yaml`)
- **Network policy** recommended: restrict Redis access to ghostcacher namespace only

---

## Contributing

1. Fork the repository
2. Create a feature branch: `git checkout -b feat/your-feature`
3. Run tests: `./scripts/dev.sh test`
4. Run the smoke test: `python scripts/smoke_test.py`
5. Open a pull request

**Core areas for contribution:**
- Additional provider adapters (Cohere, Mistral, Together AI)
- vLLM / SGLang native KV injection (bypass HTTP for self-hosted clusters)
- MutatingWebhookConfiguration for automatic sidecar injection
- Grafana dashboard JSON (pull requests welcome)
- Rust benchmarks (`cargo bench`)

---

## License

MIT License — see [LICENSE](./LICENSE) for details.

---

*Built with Rust, Redis, and a deep respect for your GPU compute budget.*
