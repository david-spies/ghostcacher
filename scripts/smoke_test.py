#!/usr/bin/env python3
"""
scripts/smoke_test.py
GhostCacher integration smoke test — verifies the full sidecar pipeline
against a running local stack.

Usage:
    export ANTHROPIC_API_KEY=sk-ant-...
    python scripts/smoke_test.py

    # Run against a remote sidecar:
    GC_SIDECAR_URL=http://my-cluster:8080 python scripts/smoke_test.py
"""

import json
import os
import sys
import time
import hashlib

import httpx

SIDECAR_URL = os.environ.get("GC_SIDECAR_URL", "http://localhost:8080")
API_KEY     = os.environ.get("ANTHROPIC_API_KEY", "")

TEAL   = "\033[36m"
GREEN  = "\033[32m"
RED    = "\033[31m"
YELLOW = "\033[33m"
BOLD   = "\033[1m"
RESET  = "\033[0m"

PASS = f"{GREEN}{BOLD}PASS{RESET}"
FAIL = f"{RED}{BOLD}FAIL{RESET}"
INFO = f"{TEAL}{BOLD}INFO{RESET}"

passed = 0
failed = 0


def check(name: str, condition: bool, detail: str = ""):
    global passed, failed
    if condition:
        print(f"  {PASS}  {name}")
        passed += 1
    else:
        print(f"  {FAIL}  {name}" + (f" — {detail}" if detail else ""))
        failed += 1


def section(title: str):
    print(f"\n{BOLD}{TEAL}── {title} {'─' * (50 - len(title))}{RESET}")


def main():
    print(f"\n{BOLD}GhostCacher Smoke Test{RESET}")
    print(f"Sidecar: {SIDECAR_URL}\n")

    client = httpx.Client(base_url=SIDECAR_URL, timeout=30.0)

    # ── 1. Health checks ────────────────────────────────────────────────────
    section("Health Probes")

    r = client.get("/healthz")
    check("GET /healthz → 200", r.status_code == 200)

    r = client.get("/readyz")
    check("GET /readyz → 200 or Redis connected", r.status_code == 200)
    data = r.json()
    check("readyz.redis = true", data.get("redis") is True, f"got: {data}")

    r = client.get("/gc/status")
    check("GET /gc/status → 200", r.status_code == 200)
    status = r.json()
    check("status.version = 2.1.0", status.get("version") == "2.1.0", f"got: {status.get('version')}")

    # ── 2. Hash stability ────────────────────────────────────────────────────
    section("Prefix Hash Stability")

    SYSTEM_PROMPT = "You are a legal analysis AI with expertise in contract review."
    SYSTEM_PROMPT_PADDED = "  You   are  a legal analysis AI with expertise in contract review.  "
    DOCS_UNORDERED = "[SOURCE:002] Second clause.\n[SOURCE:001] First clause."
    DOCS_ORDERED   = "[SOURCE:001] First clause.\n[SOURCE:002] Second clause."

    def block_hash(kind: str, content: str) -> str:
        canonical = " ".join(content.strip().split())
        if kind == "document":
            parts    = content.strip().split("[SOURCE:")
            preamble = parts[0]
            sections = sorted(f"[SOURCE:{p}" for p in parts[1:])
            canonical = " ".join((" ".join([preamble] + sections)).split())
        tagged = f"{kind.upper()}:{canonical}"
        return hashlib.sha256(tagged.encode()).hexdigest()[:16]

    h1 = block_hash("system", SYSTEM_PROMPT)
    h2 = block_hash("system", SYSTEM_PROMPT_PADDED)
    check("Whitespace normalization stable", h1 == h2, f"h1={h1}, h2={h2}")

    hd1 = block_hash("document", DOCS_UNORDERED)
    hd2 = block_hash("document", DOCS_ORDERED)
    check("Document source-ordering stable", hd1 == hd2, f"hd1={hd1}, hd2={hd2}")

    # ── 3. Cache flush endpoint ───────────────────────────────────────────────
    section("Cache Management")

    for scope in ["session", "docs", "system", "all"]:
        r = client.post("/gc/flush", json={"scope": scope})
        check(f"POST /gc/flush scope={scope} → 200", r.status_code == 200)

    # ── 4. Prometheus metrics endpoint ───────────────────────────────────────
    section("Observability")

    r = client.get("http://localhost:9090/metrics")
    if r.status_code == 200:
        metrics_text = r.text
        check("gc_cache_hits_total present",   "gc_cache_hits_total"   in metrics_text)
        check("gc_cache_misses_total present",  "gc_cache_misses_total"  in metrics_text)
        check("gc_cache_hit_ratio present",     "gc_cache_hit_ratio"     in metrics_text)
        check("gc_tokens_cached_total present", "gc_tokens_cached_total" in metrics_text)
    else:
        print(f"  {YELLOW}SKIP{RESET}  Metrics endpoint not reachable (port 9090 not exposed?)")

    # ── 5. Live request (requires valid API key) ──────────────────────────────
    section("Live Request (API Key Required)")

    if not API_KEY:
        print(f"  {YELLOW}SKIP{RESET}  ANTHROPIC_API_KEY not set — skipping live request test")
    else:
        sys_prompt  = "You are a helpful AI assistant for smoke testing."
        user_msg    = "Say exactly: GhostCacher test successful."

        payload = {
            "model":      "claude-haiku-4-5-20251001",
            "max_tokens": 64,
            "system":     sys_prompt,
            "messages":   [{"role": "user", "content": user_msg}],
        }
        headers = {
            "x-api-key":          API_KEY,
            "anthropic-version":  "2023-06-01",
            "anthropic-beta":     "prompt-caching-2024-07-31",
            "Content-Type":       "application/json",
        }

        # First request — cold miss
        t0 = time.perf_counter()
        r1 = client.post("/v1/messages", json=payload, headers=headers)
        t1 = time.perf_counter()
        cold_ms = (t1 - t0) * 1000

        check("First request (cold) → 200", r1.status_code == 200, r1.text[:200])

        if r1.status_code == 200:
            data1 = r1.json()
            check(
                "Response contains content",
                bool(data1.get("content")),
                str(data1)[:200],
            )

            # Second identical request — should be a cache hit
            t0 = time.perf_counter()
            r2 = client.post("/v1/messages", json=payload, headers=headers)
            t1 = time.perf_counter()
            warm_ms = (t1 - t0) * 1000

            check("Second request (warm) → 200", r2.status_code == 200)
            print(f"  {INFO}  Cold: {cold_ms:.0f}ms  Warm: {warm_ms:.0f}ms  "
                  f"Delta: {cold_ms - warm_ms:.0f}ms")

    # ── Summary ───────────────────────────────────────────────────────────────
    total = passed + failed
    print(f"\n{'─' * 56}")
    print(f"  {BOLD}Results:{RESET} {GREEN}{passed} passed{RESET}  "
          f"{(RED + str(failed) + ' failed' + RESET) if failed else '0 failed'}  "
          f"/ {total} total")

    if failed > 0:
        print(f"\n{RED}{BOLD}Smoke test FAILED.{RESET}")
        sys.exit(1)
    else:
        print(f"\n{GREEN}{BOLD}Smoke test PASSED.{RESET}")
        sys.exit(0)


if __name__ == "__main__":
    main()
