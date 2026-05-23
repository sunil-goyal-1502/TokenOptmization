import type {
  AnalyzeReport,
  CompileOptions,
  CompileResult,
  TranscriptMessage,
} from "./types.js";

export interface TokenOptClientConfig {
  baseUrl?: string;
  timeoutMs?: number;
}

/**
 * HTTP client for tokenopt-server — use from Cursor hooks, LangChain.js, Vercel AI SDK, etc.
 */
export class TokenOptClient {
  private readonly baseUrl: string;
  private readonly timeoutMs: number;

  constructor(config: TokenOptClientConfig = {}) {
    this.baseUrl = (config.baseUrl ?? "http://127.0.0.1:8787").replace(/\/$/, "");
    this.timeoutMs = config.timeoutMs ?? 60_000;
  }

  async health(): Promise<{ status: string }> {
    const res = await this.fetch("/health", { method: "GET" });
    return res.json() as Promise<{ status: string }>;
  }

  async analyze(messages: TranscriptMessage[]): Promise<AnalyzeReport> {
    const res = await this.fetch("/v1/analyze", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ messages }),
    });
    return res.json() as Promise<AnalyzeReport>;
  }

  async compile(
    messages: TranscriptMessage[],
    options: CompileOptions = {},
  ): Promise<CompileResult> {
    const res = await this.fetch("/v1/compile", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ messages, options }),
    });
    if (!res.ok) {
      const err = (await res.json()) as { error?: string };
      throw new Error(err.error ?? `compile failed: ${res.status}`);
    }
    return res.json() as Promise<CompileResult>;
  }

  async beforeModel(
    messages: TranscriptMessage[],
    sessionId: string,
    turnIndex = 0,
    options: CompileOptions = {},
  ): Promise<TranscriptMessage[]> {
    const res = await this.fetch("/v1/middleware/before-model", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({
        messages,
        session_id: sessionId,
        turn_index: turnIndex,
        options: { ...options, session_id: sessionId },
      }),
    });
    if (!res.ok) {
      const err = (await res.json()) as { error?: string };
      throw new Error(err.error ?? `middleware failed: ${res.status}`);
    }
    const data = (await res.json()) as { messages: TranscriptMessage[] };
    return data.messages;
  }

  private async fetch(path: string, init: RequestInit): Promise<Response> {
    const controller = new AbortController();
    const timer = setTimeout(() => controller.abort(), this.timeoutMs);
    try {
      return await fetch(`${this.baseUrl}${path}`, {
        ...init,
        signal: controller.signal,
      });
    } finally {
      clearTimeout(timer);
    }
  }
}
