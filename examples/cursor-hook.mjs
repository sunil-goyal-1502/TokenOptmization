/**
 * Example Cursor hook pattern: call tokenopt-server before model (pseudo-hook).
 * Wire into .cursor/hooks per Cursor hooks documentation.
 */
import { TokenOptClient } from "@tokenopt/client";

const client = new TokenOptClient({ baseUrl: process.env.TOKENOPT_URL ?? "http://127.0.0.1:8787" });

export async function beforeModel({ sessionId, messages }) {
  const compiled = await client.beforeModel(messages, sessionId, 0, {
    token_budget: Number(process.env.TOKENOPT_BUDGET ?? 128000),
    soft_sufficiency: true,
  });
  return { messages: compiled };
}
