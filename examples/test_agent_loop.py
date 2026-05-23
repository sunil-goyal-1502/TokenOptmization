#!/usr/bin/env python3
"""Requires tokenopt-server on TOKENOPT_URL (default http://127.0.0.1:8787)."""

from __future__ import annotations

import os
import sys

from tokenopt import CompileOptions, TokenOptClient, TokenOptConfig

# Reuse same shape as Rust bench: growing messages with fat tool results
def build_messages(turns: int, payload_kb: int) -> list[dict]:
    payload = "y" * (payload_kb * 1024)
    messages: list[dict] = [
        {"role": "system", "content": "Coding agent."},
        {"role": "user", "content": "Fix crates/tokenopt-core/src/lib.rs"},
    ]
    for t in range(turns):
        path = f"crates/mod_{t}/src/lib.rs"
        call_id = f"call_{t}"
        messages.append(
            {
                "role": "assistant",
                "content": f"Reading {path}",
                "tool_calls": [
                    {
                        "id": call_id,
                        "type": "function",
                        "function": {
                            "name": "read_file",
                            "arguments": f'{{"path":"{path}"}}',
                        },
                    }
                ],
            }
        )
        body = f"error\n{payload}" if t % 5 == 4 else f"// {path}\n{payload}"
        messages.append(
            {
                "role": "tool",
                "tool_call_id": call_id,
                "name": "read_file",
                "content": body,
            }
        )
    return messages


def estimate_chars(messages: list[dict]) -> int:
    total = 0
    for m in messages:
        c = m.get("content")
        if isinstance(c, str):
            total += len(c)
        if m.get("tool_calls"):
            total += 200
    return total


def main() -> int:
    url = os.environ.get("TOKENOPT_URL", "http://127.0.0.1:8787")
    turns = int(os.environ.get("TOKENOPT_TURNS", "12"))
    payload_kb = int(os.environ.get("TOKENOPT_PAYLOAD_KB", "8"))

    client = TokenOptClient(TokenOptConfig(base_url=url))
    try:
        client.health()
    except Exception as e:
        print(f"Cannot reach {url}: {e}", file=sys.stderr)
        print("Start server: cargo run -p tokenopt-server --", file=sys.stderr)
        return 1

    messages = build_messages(turns, payload_kb)
    baseline_chars = estimate_chars(messages)

    opts = CompileOptions(
        session_id="py-sim",
        token_budget=128_000,
        keep_recent_tool_results=2,
        soft_sufficiency=True,
        run_sufficiency_check=False,
    )
    result = client.compile(messages, opts)
    compiled_chars = estimate_chars(
        [m.model_dump(exclude_none=True) for m in result.messages]
    )

    print("Python agent-loop test (via HTTP /v1/compile)")
    print(f"  turns:           {turns}")
    print(f"  payload/tool:    {payload_kb} KB")
    print(f"  baseline chars:  {baseline_chars}")
    print(f"  compiled chars:  {compiled_chars}")
    print(f"  SDK tokens_saved: {result.stats.tokens_saved}")
    print(f"  SDK reduction %:  {result.stats.reduction_percent:.1f}")
    print(f"  transforms:       {', '.join(result.stats.transforms_applied)}")

    if result.stats.tokens_saved <= 0 and turns >= 8:
        print("WARN: expected savings with 8+ turns and large payloads", file=sys.stderr)
        return 2
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
