/**
 * Optional @tokenopt/native in-process bindings (requires `npm run build` in bindings/node).
 */

let native: {
  nativeAvailable: () => boolean;
  compileJson: (messagesJson: string, optionsJson?: string) => Promise<string>;
} | null = null;

try {
  // eslint-disable-next-line @typescript-eslint/no-require-imports
  native = require("@tokenopt/native");
} catch {
  native = null;
}

export function nativeAvailable(): boolean {
  return Boolean(native?.nativeAvailable?.());
}

export async function compileNative(
  messages: unknown[],
  options: Record<string, unknown> = {},
): Promise<unknown> {
  if (!native) {
    throw new Error("Install and build @tokenopt/native (bindings/node)");
  }
  const raw = await native.compileJson(JSON.stringify(messages), JSON.stringify(options));
  return JSON.parse(raw) as unknown;
}
