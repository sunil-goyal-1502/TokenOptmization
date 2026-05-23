import { TokenOptClient } from "./client.js";
import type { CompileOptions, TranscriptMessage } from "./types.js";

/**
 * Drop-in hook: compile messages before sending to your LLM provider.
 */
export function createBeforeModelMiddleware(
  client: TokenOptClient,
  sessionId: string,
  options?: CompileOptions,
): (messages: TranscriptMessage[]) => Promise<TranscriptMessage[]> {
  return (messages) => client.beforeModel(messages, sessionId, 0, options);
}
