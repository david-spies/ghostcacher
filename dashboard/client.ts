/**
 * ghostcacher/client.ts
 * GhostCacher TypeScript SDK — drop-in wrapper for Anthropic and OpenAI SDKs.
 *
 * Usage (Anthropic):
 *   import { GhostCacherClient } from "./ghostcacher/client";
 *
 *   const gc = new GhostCacherClient({
 *     provider: "anthropic",
 *     apiKey: process.env.ANTHROPIC_API_KEY!,
 *     ghostcacherUrl: "http://localhost:8080",
 *   });
 *
 *   const response = await gc.messages.create({
 *     model: "claude-sonnet-4-5",
 *     maxTokens: 1024,
 *     system: "You are a legal analysis AI...",     // → SYS block (cached ∞)
 *     tools: [...],                                   // → TOOLS block (cached ∞)
 *     documents: ["[SOURCE:001] Contract text..."],  // → DOC block (cached 4h)
 *     messages: [{ role: "user", content: "..." }], // → USER block (volatile)
 *   });
 */

import crypto from "node:crypto";

// ─── Types ───────────────────────────────────────────────────────────────────

type Provider = "anthropic" | "openai" | "bedrock" | "vertex";

interface GhostCacherClientOptions {
  provider?: Provider;
  apiKey?: string;
  ghostcacherUrl?: string;
  timeoutMs?: number;
}

interface PromptBlockRaw {
  kind: "system" | "tools" | "document" | "user";
  content: string;
  hash: string;
}

interface GcMeta {
  prefix_hash: string;
  provider: Provider;
  stream?: boolean;
}

interface AnthropicCreateParams {
  model: string;
  maxTokens: number;
  system?: string;
  tools?: object[];
  documents?: string[];
  messages: Array<{ role: string; content: string | object[] }>;
  stream?: boolean;
  [key: string]: unknown;
}

interface OpenAICreateParams {
  model: string;
  messages: Array<{ role: string; content: string }>;
  [key: string]: unknown;
}

// ─── Block hashing ───────────────────────────────────────────────────────────

const BLOCK_SEP = Buffer.from("\x00GC\x00");

function canonicalize(kind: string, content: string): string {
  const trimmed = content.trim();
  if (kind === "document") {
    const parts = trimmed.split("[SOURCE:");
    const preamble = parts[0];
    const sections = parts
      .slice(1)
      .map((p) => `[SOURCE:${p}`)
      .sort();
    const joined = [preamble, ...sections].join("\n").trim();
    return joined.split(/\s+/).join(" ");
  }
  return trimmed.split(/\s+/).join(" ");
}

function hashBlock(kind: string, content: string): string {
  const canonical = canonicalize(kind, content);
  const tagged = `${kind.toUpperCase()}:${canonical}`;
  return crypto.createHash("sha256").update(tagged).digest("hex").slice(0, 16);
}

function computePrefixHash(blocks: PromptBlockRaw[]): string {
  const h = crypto.createHash("sha256");
  let first = true;
  for (const block of blocks) {
    if (block.kind === "user") continue;
    if (!first) h.update(BLOCK_SEP);
    h.update(block.hash);
    first = false;
  }
  return h.digest("hex").slice(0, 32);
}

function makeBlock(
  kind: "system" | "tools" | "document" | "user",
  content: string
): PromptBlockRaw {
  return { kind, content, hash: hashBlock(kind, content) };
}

// ─── Sub-clients ─────────────────────────────────────────────────────────────

class GhostCacherMessages {
  constructor(private readonly client: GhostCacherClient) {}

  async create(params: AnthropicCreateParams): Promise<unknown> {
    const { model, maxTokens, system, tools, documents, messages, stream, ...rest } = params;
    const blocks: PromptBlockRaw[] = [];

    if (system)    blocks.push(makeBlock("system", system));
    if (tools)     blocks.push(makeBlock("tools", JSON.stringify(tools)));
    if (documents) blocks.push(makeBlock("document", documents.join("\n")));

    const allButLast = messages.slice(0, -1);
    const lastMsg    = messages[messages.length - 1];

    if (allButLast.length > 0) {
      blocks.push(makeBlock("document", JSON.stringify(allButLast)));
    }
    if (lastMsg) {
      blocks.push(makeBlock("user", JSON.stringify(lastMsg)));
    }

    const prefixHash = computePrefixHash(blocks);

    const payload = {
      model,
      max_tokens: maxTokens,
      system,
      tools,
      messages,
      gc_blocks: blocks,
      gc_meta: { prefix_hash: prefixHash, provider: this.client.provider, stream } satisfies GcMeta,
      ...rest,
    };

    return this.client["_post"]("/v1/messages", payload);
  }
}

class GhostCacherChatCompletions {
  constructor(private readonly client: GhostCacherClient) {}

  async create(params: OpenAICreateParams): Promise<unknown> {
    const { messages, ...rest } = params;
    const blocks: PromptBlockRaw[] = [];

    const sysMsgs    = messages.filter((m) => m.role === "system");
    const history    = messages.filter((m) => m.role !== "system");
    const allButLast = history.slice(0, -1);
    const lastMsg    = history[history.length - 1];

    if (sysMsgs.length > 0) {
      blocks.push(makeBlock("system", sysMsgs.map((m) => m.content).join("\n")));
    }
    if (allButLast.length > 0) {
      blocks.push(makeBlock("document", JSON.stringify(allButLast)));
    }
    if (lastMsg) {
      blocks.push(makeBlock("user", JSON.stringify(lastMsg)));
    }

    const prefixHash = computePrefixHash(blocks);

    const payload = {
      messages,
      gc_blocks: blocks,
      gc_meta: { prefix_hash: prefixHash, provider: this.client.provider } satisfies GcMeta,
      store: true,
      ...rest,
    };

    return this.client["_post"]("/v1/chat/completions", payload);
  }
}

// ─── Main Client ─────────────────────────────────────────────────────────────

export class GhostCacherClient {
  readonly provider: Provider;
  private readonly apiKey: string;
  private readonly ghostcacherUrl: string;
  private readonly timeoutMs: number;

  readonly messages: GhostCacherMessages;
  readonly chat: { completions: GhostCacherChatCompletions };

  constructor(options: GhostCacherClientOptions = {}) {
    this.provider        = options.provider ?? "anthropic";
    this.apiKey          = options.apiKey ?? process.env.ANTHROPIC_API_KEY ?? process.env.OPENAI_API_KEY ?? "";
    this.ghostcacherUrl  = (options.ghostcacherUrl ?? "http://localhost:8080").replace(/\/$/, "");
    this.timeoutMs       = options.timeoutMs ?? 120_000;

    this.messages = new GhostCacherMessages(this);
    this.chat     = { completions: new GhostCacherChatCompletions(this) };
  }

  private async _post(endpoint: string, payload: object): Promise<unknown> {
    const headers: Record<string, string> = {
      "Content-Type": "application/json",
    };

    if (this.provider === "anthropic") {
      headers["x-api-key"]          = this.apiKey;
      headers["anthropic-version"]   = "2023-06-01";
      headers["anthropic-beta"]      = "prompt-caching-2024-07-31";
    } else if (this.provider === "openai") {
      headers["Authorization"] = `Bearer ${this.apiKey}`;
    }

    const controller = new AbortController();
    const timer      = setTimeout(() => controller.abort(), this.timeoutMs);

    try {
      const resp = await fetch(`${this.ghostcacherUrl}${endpoint}`, {
        method:  "POST",
        headers,
        body:    JSON.stringify(payload),
        signal:  controller.signal,
      });

      if (!resp.ok) {
        const text = await resp.text();
        throw new Error(`GhostCacher upstream error ${resp.status}: ${text}`);
      }

      return resp.json();
    } finally {
      clearTimeout(timer);
    }
  }

  /** Flush cached entries. scope: 'session' | 'docs' | 'system' | 'all' */
  async flushCache(scope: "session" | "docs" | "system" | "all" = "session"): Promise<unknown> {
    const resp = await fetch(`${this.ghostcacherUrl}/gc/flush`, {
      method:  "POST",
      headers: { "Content-Type": "application/json" },
      body:    JSON.stringify({ scope }),
    });
    return resp.json();
  }

  /** Return sidecar status and current configuration. */
  async status(): Promise<unknown> {
    const resp = await fetch(`${this.ghostcacherUrl}/gc/status`);
    return resp.json();
  }

  /** Return Prometheus metrics text. */
  async metrics(): Promise<string> {
    const resp = await fetch(`${this.ghostcacherUrl}/metrics`);
    return resp.text();
  }
}

// ─── Convenience factory ─────────────────────────────────────────────────────

export function createGhostCacherClient(opts?: GhostCacherClientOptions): GhostCacherClient {
  return new GhostCacherClient(opts);
}
