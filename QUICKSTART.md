# GhostCacher — QUICKSTART

> Get distributed KV prompt caching running in 5 minutes.

---

## Prerequisites

| Tool | Minimum Version | Check |
|------|----------------|-------|
| Docker + Docker Compose | 24.x | `docker --version` |
| Rust + Cargo | 1.78 | `rustc --version` |
| An LLM API key | — | Anthropic or OpenAI |

For Kubernetes deployment: `kubectl` + a cluster (k3s, EKS, GKE, or AKS).

---

## Step 1 — Clone and configure

```bash
git clone https://github.com/david-spies/ghostcacher
cd ghostcacher

# Copy the environment template
cp .env.example .env
```

Open `.env` and set your API key:

```bash
# .env
ANTHROPIC_API_KEY=sk-ant-YOUR_KEY_HERE   # for Anthropic
# or
OPENAI_API_KEY=sk-YOUR_KEY_HERE           # for OpenAI
```

Everything else can stay as the default for local development.

---

## Step 2 — Start the stack

```bash
./scripts/dev.sh up
```
- If Permission Denied - see troubleshooting.md

This starts:

| Service | URL | Description |
|---------|-----|-------------|
| **GhostCacher Sidecar** | `http://localhost:8080` | LLM proxy endpoint |
| **Control Plane** | `http://localhost:7070` | Admin API + pod registry |
| **Redis** | `localhost:6379` | Distributed cache state |
| **RedisInsight** | `http://localhost:8001` | Redis UI |
| **Prometheus** | `http://localhost:9091` | Metrics |
| **Grafana** | `http://localhost:3000` | Dashboard (admin / ghostcacher) |

Wait ~10 seconds for all services to become healthy:

```bash
./scripts/dev.sh status
# Should print: { "status": "ok", "redis": true, "version": "2.1.0" }
```

---

## Step 3 — Point your app at GhostCacher

GhostCacher is a transparent proxy. You only need to change the base URL.

### Option A — Environment variable (zero code changes)

```bash
# Anthropic
export ANTHROPIC_BASE_URL=http://localhost:8080

# OpenAI
export OPENAI_BASE_URL=http://localhost:8080/v1
```

Your existing Anthropic or OpenAI SDK calls now route through GhostCacher
automatically. No code changes required.

### Option B — Python SDK (structured caching)

Install the dependency:

```bash
pip install httpx
```

Use the GhostCacher client directly for maximum cache efficiency:

```python
# my_app.py
import sys
sys.path.insert(0, "./dashboard")  # path to client.py

from client import GhostCacherClient

gc = GhostCacherClient(
    provider="anthropic",
    ghostcacher_url="http://localhost:8080",
)

# This system prompt is cached indefinitely after the first request.
# All subsequent requests with the same system prompt skip the prefill phase.
SYSTEM_PROMPT = """
You are an enterprise AI assistant specializing in legal contract analysis.
You have deep expertise in: contract law, liability clauses, indemnification,
IP ownership, and regulatory compliance. Always cite specific clauses.
"""

# Documents are sorted by source ID for hash stability.
# The same sorted documents = same hash = cache hit every time.
DOCUMENTS = [
    "[SOURCE:001] Master Service Agreement — Section 4.2: Liability is capped at 12 months fees.",
    "[SOURCE:002] SOW-2025-Q3 — Deliverables: Phase 1 architecture, Phase 2 implementation.",
    "[SOURCE:003] DPA-2025 — Data processing terms per GDPR Article 28.",
]

response = gc.messages.create(
    model="claude-sonnet-4-5",
    max_tokens=1024,
    system=SYSTEM_PROMPT,          # → cached ∞  (SYS block)
    documents=DOCUMENTS,            # → cached 4h (DOC block)
    messages=[
        {
            "role": "user",
            "content": "What is the liability cap in the MSA?"   # volatile
        }
    ],
)

print(response["content"][0]["text"])
```

### Option C — TypeScript SDK

```typescript
// my_app.ts
import { GhostCacherClient } from "./dashboard/client";

const gc = new GhostCacherClient({
  provider: "anthropic",
  ghostcacherUrl: "http://localhost:8080",
});

const SYSTEM_PROMPT = `You are an enterprise legal AI assistant...`;
const DOCUMENTS = [
  "[SOURCE:001] Master Service Agreement...",
  "[SOURCE:002] Statement of Work...",
];

const response = await gc.messages.create({
  model: "claude-sonnet-4-5",
  maxTokens: 1024,
  system: SYSTEM_PROMPT,
  documents: DOCUMENTS,
  messages: [{ role: "user", content: "Summarize the liability clauses." }],
});

console.log(response);
```

---

## Step 4 — Observe the cache working

### Watch cache events in real time

```bash
./scripts/dev.sh logs
```

You'll see log lines like:

```
{"level":"INFO","msg":"Cache HIT — routing to warmed pod","prefix":"3f8a2b...","saved_ms":74}
{"level":"INFO","msg":"Cache MISS — warming prefix","prefix":"9c1d47..."}
{"level":"INFO","msg":"Ghost-Lock acquired","prefix":"9c1d47..."}
```

### Watch Redis keys populate

```bash
./scripts/dev.sh redis-cli
# Inside redis-cli:
KEYS cache_map:*        # list all cached prefix hashes
GET cache_map:<hash>    # inspect a specific entry
TTL cache_map:<hash>    # check remaining TTL
```

### Check Prometheus metrics

```bash
./scripts/dev.sh metrics
```

Key metrics to watch:

```
gc_cache_hit_ratio        → 0.874 (87.4% hit rate)
gc_tokens_cached_total    → 2100000
gc_saved_ttft_ms_total    → 154800
gc_cache_hits_total       → 1204
gc_cache_misses_total     → 174
```

### Open the Grafana dashboard

Navigate to `http://localhost:3000` (admin / ghostcacher) and open the
**GhostCacher** dashboard. You'll see hit ratio, TTFT savings, and cost
reduction charts update in real time.

---

## Step 5 — Run the smoke test

```bash
# Requires a valid ANTHROPIC_API_KEY in your environment
python scripts/smoke_test.py
```

Expected output:

```
GhostCacher Smoke Test
Sidecar: http://localhost:8080

── Health Probes ──────────────────────────────────────────────
  PASS  GET /healthz → 200
  PASS  GET /readyz → 200 or Redis connected
  PASS  readyz.redis = true
  PASS  GET /gc/status → 200
  PASS  status.version = 2.1.0

── Prefix Hash Stability ──────────────────────────────────────
  PASS  Whitespace normalization stable
  PASS  Document source-ordering stable

── Cache Management ───────────────────────────────────────────
  PASS  POST /gc/flush scope=session → 200
  PASS  POST /gc/flush scope=docs → 200
  PASS  POST /gc/flush scope=system → 200
  PASS  POST /gc/flush scope=all → 200

── Observability ──────────────────────────────────────────────
  PASS  gc_cache_hits_total present
  PASS  gc_cache_hit_ratio present
  PASS  gc_tokens_cached_total present

── Live Request (API Key Required) ────────────────────────────
  PASS  First request (cold) → 200
  PASS  Response contains content
  PASS  Second request (warm) → 200
  INFO  Cold: 847ms  Warm: 112ms  Delta: 735ms

────────────────────────────────────────────────────────────────
  Results: 16 passed  0 failed / 16 total

Smoke test PASSED.
```

---

## Common Operations

### Flush the cache

```bash
# Flush only session history (safest — system prompts stay warm)
./scripts/dev.sh flush session

# Flush RAG document cache
./scripts/dev.sh flush docs

# Flush everything (next request will be a cold miss)
./scripts/dev.sh flush all
```

### Stop the stack

```bash
./scripts/dev.sh down
```

### Rebuild after code changes

```bash
./scripts/dev.sh build
./scripts/dev.sh up
```

---

## Prompt Engineering for Maximum Cache Efficiency

GhostCacher achieves maximum cache hit rates when your prompts follow the
**Canonical Prompt Format**:

### Rule 1 — Put stable content first

```
✅ CORRECT order (cache-friendly):
  1. System instructions
  2. Tool schemas
  3. RAG documents
  ── cache_control breakpoint ──
  4. User query (volatile)

❌ WRONG order (cache-busting):
  1. User query first
  2. System instructions after
```

### Rule 2 — Sort RAG documents by Source ID

The GhostCacher SDK does this automatically. If using raw API calls:

```python
# WRONG — different orderings produce different hashes
docs = retrieve_documents(query)  # order is non-deterministic

# CORRECT — sort by source ID before inserting
docs = sorted(retrieve_documents(query), key=lambda d: d["source_id"])
```

### Rule 3 — Don't embed volatile data in the system prompt

```python
# WRONG — current time in system prompt = cache miss every minute
system = f"You are a helpful AI. Current time: {datetime.now()}"

# CORRECT — put volatile context in the user message
system = "You are a helpful AI."
user   = f"Current time: {datetime.now()}. Answer my question: ..."
```

### Rule 4 — Use consistent tool schema serialization

```python
# WRONG — dict ordering may differ across Python versions / runs
tools_json = json.dumps(tools)

# CORRECT — sort keys for deterministic serialization
tools_json = json.dumps(tools, sort_keys=True)
# (The GhostCacher SDK does this automatically)
```

---

## Kubernetes Quick Deploy

```bash
# 1. Build and push images
docker build -f sidecar/Dockerfile -t ghcr.io/your-org/ghostcacher-sidecar:2.1.0 .
docker build -f control-plane/Dockerfile -t ghcr.io/your-org/ghostcacher-control-plane:2.1.0 .
docker push ghcr.io/your-org/ghostcacher-sidecar:2.1.0
docker push ghcr.io/your-org/ghostcacher-control-plane:2.1.0

# 2. Create the API key secret
kubectl create secret generic llm-api-keys \
  --from-literal=anthropic-api-key="$ANTHROPIC_API_KEY" \
  -n ghostcacher

# 3. Apply all manifests
./scripts/dev.sh k8s-apply

# 4. Verify
kubectl -n ghostcacher get pods
# NAME                                        READY   STATUS    RESTARTS
# ghostcacher-redis-0                         1/1     Running   0
# ghostcacher-control-plane-xxx               1/1     Running   0
# ghostcacher-kv-relay-xxx (per node)         1/1     Running   0
# my-llm-app-xxx (with sidecar injected)      2/2     Running   0

# 5. Run smoke test against the cluster
GC_SIDECAR_URL=http://$(kubectl -n ghostcacher get svc my-llm-app -o jsonpath='{.status.loadBalancer.ingress[0].ip}') \
  python scripts/smoke_test.py
```

---

## Troubleshooting

### "Redis connection refused"

```bash
# Check Redis is running
docker compose ps redis
# Check the Redis URL matches your setup
echo $GC_REDIS_URL
# Test connectivity
docker compose exec sidecar redis-cli -u $GC_REDIS_URL ping
```

### "Cache hit ratio is 0%"

This usually means prompts are not structured correctly. Verify:

1. Your system prompt doesn't contain volatile data (timestamps, request IDs)
2. RAG documents are sorted by source ID before sending
3. The user query is always the **last** message
4. Run `./scripts/dev.sh logs` and look for `[MISS]` lines with the reason

### "Sidecar overhead is too high (> 5ms)"

```bash
# Check Redis latency
docker compose exec redis redis-cli --latency-history -i 1
# Should be < 1ms for local Redis

# Check sidecar p99 latency
curl -s http://localhost:9090/metrics | grep gc_request_latency_ms
```

If Redis latency is high, ensure the sidecar and Redis are in the same
availability zone / network segment.

### "Ghost-Lock is timing out frequently"

Increase the lock timeout or reduce the number of concurrent requests hitting
the same cold prefix simultaneously:

```bash
# In .env
GC_GHOST_LOCK_TIMEOUT_SECS=60
```

---

## Next Steps

- **[README.md](./README.md)** — Full architecture documentation, configuration reference, and API docs
- **Add more providers** — Set `GC_UPSTREAM_URL` to any OpenAI-compatible endpoint
- **Enable RDMA** — Set `GC_RDMA_ENABLED=true` and `GC_RELAY_RDMA_AVAILABLE=true` on SmartNIC-equipped nodes
- **Auto-inject sidecar** — Deploy `k8s/02-webhook.yaml` (MutatingWebhookConfiguration) to inject the sidecar into any pod with the `ghostcacher.io/inject: "true"` label
- **Tune eviction** — Adjust `GC_CP_MAX_CACHE_ENTRIES` and `GC_CP_EVICTION_INTERVAL_SECS` based on your workload profile
