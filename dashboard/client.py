"""
ghostcacher/client.py
GhostCacher Python SDK — drop-in wrapper for Anthropic and OpenAI clients.

Usage (Anthropic):
    from ghostcacher import GhostCacherClient
    import anthropic

    gc = GhostCacherClient(
        provider="anthropic",
        api_key="sk-ant-...",
        ghostcacher_url="http://localhost:8080",  # sidecar address
    )

    response = gc.messages.create(
        model="claude-sonnet-4-5",
        max_tokens=1024,
        system="You are a legal analysis AI...",         # → cached (SYS block)
        tools=[...],                                       # → cached (TOOLS block)
        documents=["[SOURCE:001] Contract text..."],      # → cached (DOC block)
        messages=[{"role": "user", "content": "..."}],    # → volatile (USER block)
    )

Usage (OpenAI):
    gc = GhostCacherClient(provider="openai", api_key="sk-...", ghostcacher_url="...")
    response = gc.chat.completions.create(model="gpt-4o", messages=[...])
"""

from __future__ import annotations

import hashlib
import json
import os
from dataclasses import dataclass, field
from enum import Enum
from typing import Any, Dict, List, Optional, Union

import httpx


class BlockKind(str, Enum):
    SYSTEM   = "system"
    TOOLS    = "tools"
    DOCUMENT = "document"
    USER     = "user"


@dataclass
class PromptBlock:
    kind:    BlockKind
    content: str
    hash:    Optional[str] = field(default=None, init=False)

    def __post_init__(self):
        self.hash = self._compute_hash()

    def _compute_hash(self) -> str:
        canonical = self._canonicalize()
        tagged    = f"{self.kind.value.upper()}:{canonical}"
        return hashlib.sha256(tagged.encode()).hexdigest()[:16]

    def _canonicalize(self) -> str:
        content = self.content.strip()
        if self.kind == BlockKind.DOCUMENT:
            # Sort [SOURCE:NNN] sections alphabetically for hash stability
            parts    = content.split("[SOURCE:")
            preamble = parts[0]
            sections = sorted(f"[SOURCE:{p}" for p in parts[1:])
            content  = preamble + "\n".join(sections)
        return " ".join(content.split())

    def is_cacheable(self) -> bool:
        return self.kind != BlockKind.USER

    def to_dict(self) -> Dict[str, Any]:
        return {"kind": self.kind.value, "content": self.content, "hash": self.hash}


class GhostCacherMessages:
    """Anthropic-compatible messages API via GhostCacher sidecar."""

    def __init__(self, client: "GhostCacherClient"):
        self._client = client

    def create(
        self,
        *,
        model:      str,
        max_tokens: int,
        system:     Optional[str]               = None,
        tools:      Optional[List[Dict]]         = None,
        documents:  Optional[List[str]]          = None,
        messages:   List[Dict[str, Any]],
        stream:     bool                         = False,
        **kwargs: Any,
    ) -> Dict[str, Any]:
        """
        Structured create() that builds GhostCacher prompt blocks,
        computes prefix hashes, and routes via the sidecar.

        Extra params vs standard Anthropic SDK:
            documents: list of pre-formatted document strings.
                       Each should begin with [SOURCE:NNN] for canonical ordering.
        """
        blocks: List[PromptBlock] = []

        if system:
            blocks.append(PromptBlock(kind=BlockKind.SYSTEM, content=system))

        if tools:
            blocks.append(PromptBlock(kind=BlockKind.TOOLS, content=json.dumps(tools, sort_keys=True)))

        if documents:
            combined = "\n".join(documents)
            blocks.append(PromptBlock(kind=BlockKind.DOCUMENT, content=combined))

        # Last user message is volatile
        if messages:
            all_but_last = messages[:-1]
            last_msg     = messages[-1]
            if all_but_last:
                blocks.append(PromptBlock(
                    kind=BlockKind.DOCUMENT,
                    content=json.dumps(all_but_last, sort_keys=True),
                ))
            blocks.append(PromptBlock(kind=BlockKind.USER, content=json.dumps(last_msg)))

        # Build the GhostCacher-native request body
        prefix_hash = self._compute_prefix_hash(blocks)
        payload = {
            "model":      model,
            "max_tokens": max_tokens,
            "gc_blocks":  [b.to_dict() for b in blocks],
            "gc_meta": {
                "prefix_hash": prefix_hash,
                "provider":    "anthropic",
                "stream":      stream,
            },
            # Also include standard Anthropic fields for upstream passthrough
            "system":   system,
            "messages": messages,
            **kwargs,
        }
        if tools:
            payload["tools"] = tools

        return self._client._post(payload)

    @staticmethod
    def _compute_prefix_hash(blocks: List[PromptBlock]) -> str:
        h = hashlib.sha256()
        sep = b"\x00GC\x00"
        first = True
        for block in blocks:
            if not block.is_cacheable():
                continue
            if not first:
                h.update(sep)
            h.update(block.hash.encode())
            first = False
        return h.hexdigest()[:32]


class GhostCacherChatCompletions:
    """OpenAI-compatible chat.completions API via GhostCacher sidecar."""

    def __init__(self, client: "GhostCacherClient"):
        self._client = client

    def create(self, *, model: str, messages: List[Dict], **kwargs: Any) -> Dict[str, Any]:
        blocks: List[PromptBlock] = []
        history  = []
        sys_msgs = []

        for msg in messages:
            role = msg.get("role", "")
            if role == "system":
                sys_msgs.append(msg.get("content", ""))
            elif role in ("user", "assistant"):
                history.append(msg)

        if sys_msgs:
            blocks.append(PromptBlock(kind=BlockKind.SYSTEM, content="\n".join(sys_msgs)))

        if len(history) > 1:
            blocks.append(PromptBlock(
                kind=BlockKind.DOCUMENT,
                content=json.dumps(history[:-1], sort_keys=True),
            ))
        if history:
            blocks.append(PromptBlock(kind=BlockKind.USER, content=json.dumps(history[-1])))

        prefix_hash = GhostCacherMessages._compute_prefix_hash(blocks)
        payload = {
            "model":    model,
            "messages": messages,
            "gc_blocks": [b.to_dict() for b in blocks],
            "gc_meta": {
                "prefix_hash": prefix_hash,
                "provider":    "openai",
            },
            "store": True,  # OpenAI Responses API persistent caching
            **kwargs,
        }
        return self._client._post(payload)


class GhostCacherClient:
    """
    Top-level GhostCacher client.

    Wraps the sidecar HTTP proxy and provides provider-specific
    sub-clients (messages for Anthropic, chat for OpenAI).
    """

    def __init__(
        self,
        *,
        provider:        str   = "anthropic",
        api_key:         Optional[str] = None,
        ghostcacher_url: str   = "http://localhost:8080",
        timeout:         float = 120.0,
    ):
        self.provider        = provider
        self.api_key         = api_key or os.environ.get("ANTHROPIC_API_KEY") or os.environ.get("OPENAI_API_KEY", "")
        self.ghostcacher_url = ghostcacher_url.rstrip("/")
        self.timeout         = timeout

        self._http = httpx.Client(timeout=timeout)

        # Provider sub-clients
        self.messages = GhostCacherMessages(self)
        self.chat     = type("Chat", (), {"completions": GhostCacherChatCompletions(self)})()

    def _post(self, payload: Dict[str, Any]) -> Dict[str, Any]:
        endpoint = {
            "anthropic": "/v1/messages",
            "openai":    "/v1/chat/completions",
        }.get(self.provider, "/v1/messages")

        headers = {
            "Content-Type": "application/json",
            "x-api-key":    self.api_key,
        }
        if self.provider == "anthropic":
            headers["anthropic-version"]  = "2023-06-01"
            headers["anthropic-beta"]     = "prompt-caching-2024-07-31"
        elif self.provider == "openai":
            headers["Authorization"] = f"Bearer {self.api_key}"

        resp = self._http.post(
            f"{self.ghostcacher_url}{endpoint}",
            json=payload,
            headers=headers,
        )
        resp.raise_for_status()
        return resp.json()

    def flush_cache(self, scope: str = "session") -> Dict[str, Any]:
        """Flush cached entries. scope: 'session' | 'docs' | 'system' | 'all'"""
        resp = self._http.post(
            f"{self.ghostcacher_url}/gc/flush",
            json={"scope": scope},
        )
        resp.raise_for_status()
        return resp.json()

    def status(self) -> Dict[str, Any]:
        """Return sidecar status and current configuration."""
        return self._http.get(f"{self.ghostcacher_url}/gc/status").json()

    def close(self):
        self._http.close()

    def __enter__(self):
        return self

    def __exit__(self, *args):
        self.close()
