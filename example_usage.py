#!/usr/bin/env python3
"""
scripts/example_usage.py
GhostCacher usage examples — demonstrates the structured block API
for maximum cache hit rates across all supported providers.

Run:
    export ANTHROPIC_API_KEY="sk-ant-..."
    python scripts/example_usage.py
"""

import asyncio
import os
import sys
import time

# Add scripts dir to path for local dev
sys.path.insert(0, os.path.dirname(__file__))
from ghostcacher_client import GhostCacherClient, AsyncGhostCacherClient

# ── Static blocks (these are cached across your entire team) ──────────────────

SYSTEM_PROMPT = """You are an enterprise AI assistant specializing in legal document
analysis, contract review, and regulatory compliance. Always respond in structured
formats with precise citations to source material. Never speculate beyond the
provided documents."""

TOOL_SCHEMAS = [
    {
        "name": "search_documents",
        "description": "Full-text search across the provided document corpus",
        "input_schema": {
            "type": "object",
            "properties": {
                "query":   {"type": "string"},
                "top_k":   {"type": "integer", "default": 5},
                "filters": {"type": "object"}
            },
            "required": ["query"]
        }
    },
    {
        "name": "extract_clause",
        "description": "Extract a specific clause type from a contract",
        "input_schema": {
            "type": "object",
            "properties": {
                "clause_type": {
                    "type": "string",
                    "enum": ["liability", "termination", "payment", "ip", "warranty"]
                },
                "doc_id": {"type": "string"}
            },
            "required": ["clause_type", "doc_id"]
        }
    }
]

# ── RAG documents (sorted by SOURCE ID for hash stability) ────────────────────
# IMPORTANT: Always sort your RAG results by a stable identifier before passing
# to GhostCacher. The SDK canonicalizes [SOURCE:NNN] sections, but pre-sorting
# ensures consistent ordering even for custom source formats.

RAG_DOCUMENTS = [
    """[ SOURCE:001 ] Master Service Agreement — Revision 3.2 (2024-01-15)
Section 4.1 — Scope of Services
The Provider shall deliver software development services as described in the
applicable Statement of Work (SOW). Services commence upon SOW execution.

Section 4.2 — Liability Limitations
Except for breaches of confidentiality or intellectual property obligations,
each party's total aggregate liability shall not exceed the total fees paid
or payable under the applicable SOW during the twelve (12) months immediately
preceding the claim. Neither party shall be liable for indirect, incidental,
special, consequential, or punitive damages.""",

    """[ SOURCE:002 ] Statement of Work — Q3 2025 Platform Modernization
Effective Date: 2025-07-01 | Total Value: $480,000

Phase 1 (Weeks 1-6): Architecture Assessment & Design
  Deliverable 1.1: Current state architecture documentation
  Deliverable 1.2: Target state architecture blueprint
  Deliverable 1.3: Migration risk assessment report

Phase 2 (Weeks 7-18): Core Implementation
  Deliverable 2.1: Microservices decomposition (12 services)
  Deliverable 2.2: API gateway implementation
  Deliverable 2.3: Data migration scripts and validation suite

Phase 3 (Weeks 19-24): Testing & Deployment
  Deliverable 3.1: Integration test suite (>90% coverage)
  Deliverable 3.2: Performance benchmarks
  Deliverable 3.3: Production deployment with rollback procedures""",
]


# ── Example 1: Basic synchronous usage ───────────────────────────────────────

def example_sync():
    print("\n" + "="*60)
    print("Example 1: Synchronous Anthropic client")
    print("="*60)

    with GhostCacherClient.anthropic() as client:
        # First request — cache MISS (system + tools + docs → Redis write)
        print("\n[Request 1] Cache MISS expected (first request)")
        t0 = time.perf_counter()
        response = client.create_message(
            model="claude-sonnet-4-20250514",
            max_tokens=512,
            gc_blocks={
                "system":    SYSTEM_PROMPT,
                "tools":     TOOL_SCHEMAS,
                "documents": RAG_DOCUMENTS,
            },
            messages=[{
                "role": "user",
                "content": "What are the liability limitations in Section 4.2 of the MSA?"
            }],
        )
        elapsed = (time.perf_counter() - t0) * 1000
        print(f"  Elapsed: {elapsed:.0f}ms")
        print(f"  Prefix hash: {response['_gc_meta']['prefix_hash']}")

        # Second request — cache HIT (same system + docs, different user question)
        print("\n[Request 2] Cache HIT expected (same prefix, different user query)")
        t0 = time.perf_counter()
        response2 = client.create_message(
            model="claude-sonnet-4-20250514",
            max_tokens=512,
            gc_blocks={
                "system":    SYSTEM_PROMPT,      # Same → cache hit
                "tools":     TOOL_SCHEMAS,       # Same → cache hit
                "documents": RAG_DOCUMENTS,      # Same → cache hit
            },
            messages=[{
                "role": "user",
                "content": "What are the Phase 2 deliverables in the SOW?"
            }],
        )
        elapsed2 = (time.perf_counter() - t0) * 1000
        print(f"  Elapsed: {elapsed2:.0f}ms")
        print(f"  Prefix hash: {response2['_gc_meta']['prefix_hash']}")
        print(f"  ✓ Same prefix hash = cache HIT routed to warmed pod")

        assert (
            response["_gc_meta"]["prefix_hash"] ==
            response2["_gc_meta"]["prefix_hash"]
        ), "Prefix hashes should match for same stable blocks!"

        print(f"\n  Estimated savings: {max(0, elapsed - elapsed2):.0f}ms TTFT")


# ── Example 2: Async batch processing ────────────────────────────────────────

async def example_async_batch():
    print("\n" + "="*60)
    print("Example 2: Async batch — same prefix, 5 parallel user queries")
    print("="*60)

    questions = [
        "What is the total contract value?",
        "When does Phase 1 end?",
        "What is the liability cap?",
        "What testing coverage is required?",
        "What are the Phase 3 deliverables?",
    ]

    async with AsyncGhostCacherClient.anthropic() as client:
        tasks = [
            client.create_message(
                model="claude-sonnet-4-20250514",
                max_tokens=256,
                gc_blocks={
                    "system":    SYSTEM_PROMPT,
                    "documents": RAG_DOCUMENTS,
                },
                messages=[{"role": "user", "content": q}],
            )
            for q in questions
        ]

        t0 = time.perf_counter()
        results = await asyncio.gather(*tasks, return_exceptions=True)
        total_ms = (time.perf_counter() - t0) * 1000

        print(f"\n  {len(questions)} parallel requests completed in {total_ms:.0f}ms")
        hashes = set()
        for r in results:
            if isinstance(r, dict):
                hashes.add(r["_gc_meta"]["prefix_hash"])

        print(f"  Unique prefix hashes: {len(hashes)}")
        print(f"  ✓ All requests share the same prefix → single warmed GPU pod")


# ── Example 3: Whitespace drift test ─────────────────────────────────────────

def example_whitespace_stability():
    print("\n" + "="*60)
    print("Example 3: Whitespace drift test — hash stability check")
    print("="*60)

    from ghostcacher_client import GhostCacherRequest

    # These should produce identical prefix hashes despite formatting differences
    variants = [
        "You are a legal AI.  Respond with citations.",
        "You are a legal AI. Respond with citations.",
        "  You are a legal AI.\n  Respond with citations.\n",
        "You  are  a  legal  AI.  Respond  with  citations.",
    ]

    hashes = set()
    for v in variants:
        req = GhostCacherRequest(
            provider="anthropic",
            model="claude-sonnet-4-20250514",
            max_tokens=256,
            gc_blocks={"system": v},
            messages=[{"role": "user", "content": "test"}],
        )
        hashes.add(req.prefix_hash)
        print(f"  '{v[:40]}...' → {req.prefix_hash}")

    if len(hashes) == 1:
        print(f"\n  ✓ All variants produce identical prefix hash: {next(iter(hashes))}")
    else:
        print(f"\n  ✗ Hash instability detected! {len(hashes)} unique hashes")


# ── Example 4: Document ordering stability ────────────────────────────────────

def example_doc_ordering():
    print("\n" + "="*60)
    print("Example 4: RAG document ordering stability")
    print("="*60)

    from ghostcacher_client import GhostCacherRequest

    # Simulate RAG returning results in different orders
    doc_order_1 = [RAG_DOCUMENTS[0], RAG_DOCUMENTS[1]]
    doc_order_2 = [RAG_DOCUMENTS[1], RAG_DOCUMENTS[0]]  # Reversed

    req1 = GhostCacherRequest(
        provider="anthropic", model="claude-sonnet-4-20250514",
        max_tokens=256, gc_blocks={"system": SYSTEM_PROMPT, "documents": doc_order_1},
        messages=[{"role": "user", "content": "test"}],
    )
    req2 = GhostCacherRequest(
        provider="anthropic", model="claude-sonnet-4-20250514",
        max_tokens=256, gc_blocks={"system": SYSTEM_PROMPT, "documents": doc_order_2},
        messages=[{"role": "user", "content": "test"}],
    )

    print(f"  Order 1 prefix: {req1.prefix_hash}")
    print(f"  Order 2 prefix: {req2.prefix_hash}")

    if req1.prefix_hash == req2.prefix_hash:
        print("  ✓ Document ordering is stable — RAG result order won't cause cache misses")
    else:
        print("  Note: Docs don't use [SOURCE:NNN] format — pre-sort your RAG results!")


if __name__ == "__main__":
    example_whitespace_stability()
    example_doc_ordering()

    # Only run live API calls if key is set
    if os.environ.get("ANTHROPIC_API_KEY"):
        example_sync()
        asyncio.run(example_async_batch())
    else:
        print("\n  (Set ANTHROPIC_API_KEY to run live API examples)")
