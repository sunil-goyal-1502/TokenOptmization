#!/usr/bin/env python3
"""
E2E: real LLM + real filesystem tools + TokenOpt middleware.

Compares cumulative prompt tokens (from API usage when available) for:
  A) baseline — full message history each turn
  B) tokenopt — compile via tokenopt-server before each LLM call

Providers:
  - openai  — OPENAI_API_KEY (native tool calling, gpt-4o-mini)
  - ollama  — OLLAMA_MODEL (default tinyllama), OpenAI-compatible /v1

Usage:
  cargo run -p tokenopt-server -- --bind 127.0.0.1:8787 &
  pip install -r examples/e2e/requirements.txt
  python3 examples/e2e/run_real_agent_e2e.py --provider ollama --turns 6
"""

from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
import time
import urllib.error
import urllib.request
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

# Allow imports from examples/e2e
sys.path.insert(0, str(Path(__file__).resolve().parent))
from tools import execute_tool_call, tools_schema  # noqa: E402

ROOT = Path(__file__).resolve().parents[2]

try:
    from dotenv import load_dotenv

    load_dotenv(ROOT / ".env")
    load_dotenv()
except ImportError:
    pass

sys.path.insert(0, str(ROOT / "bindings" / "python"))
from tokenopt import CompileOptions, TokenOptClient, TokenOptConfig  # noqa: E402


PLANNED_STEPS = [
    ("read_file", {"path": "crates/tokenopt-core/src/lib.rs"}),
    ("read_file", {"path": "crates/tokenopt-core/src/compile.rs"}),
    ("read_file", {"path": "crates/tokenopt-core/src/transform.rs"}),
    ("grep", {"pattern": "compile_context", "path": "crates"}),
    ("list_dir", {"path": "crates"}),
    ("read_file", {"path": "README.md"}),
    ("grep", {"pattern": "Transform", "path": "crates/tokenopt-core"}),
    ("read_file", {"path": "docs/TESTING.md"}),
]


@dataclass
class TurnRecord:
    turn: int
    baseline_prompt_tokens: int
    optimized_prompt_tokens: int
    baseline_messages_chars: int
    optimized_messages_chars: int
    tool: str


@dataclass
class RunMetrics:
    mode: str
    total_prompt_tokens: int = 0
    total_completion_tokens: int = 0
    turns: list[TurnRecord] = field(default_factory=list)


def openai_client():
    from openai import OpenAI

    base = os.environ.get("OPENAI_BASE_URL")
    if os.environ.get("OLLAMA_HOST") or os.environ.get("TOKENOPT_E2E_PROVIDER") == "ollama":
        base = base or "http://127.0.0.1:11434/v1"
        return OpenAI(base_url=base, api_key=os.environ.get("OPENAI_API_KEY", "ollama"))
    key = os.environ.get("OPENAI_API_KEY")
    if not key:
        raise SystemExit(
            "Set OPENAI_API_KEY or run with --provider ollama (and ollama serve + model)."
        )
    return OpenAI(api_key=key)


def chat_completion(
    client: Any,
    model: str,
    messages: list[dict],
    use_tools: bool,
) -> tuple[dict, dict]:
    kwargs: dict[str, Any] = {"model": model, "messages": messages}
    if use_tools and os.environ.get("TOKENOPT_E2E_PROVIDER") != "ollama":
        kwargs["tools"] = tools_schema()
        kwargs["tool_choice"] = "auto"
    resp = client.chat.completions.create(**kwargs)
    msg = resp.choices[0].message
    usage = {
        "prompt_tokens": getattr(resp.usage, "prompt_tokens", 0) or 0,
        "completion_tokens": getattr(resp.usage, "completion_tokens", 0) or 0,
    }
    out: dict[str, Any] = {"role": "assistant", "content": msg.content or ""}
    if msg.tool_calls:
        out["tool_calls"] = [
            {
                "id": tc.id,
                "type": "function",
                "function": {
                    "name": tc.function.name,
                    "arguments": tc.function.arguments,
                },
            }
            for tc in msg.tool_calls
        ]
    return out, usage


def messages_char_count(messages: list[dict]) -> int:
    return len(json.dumps(messages, ensure_ascii=False))


def compile_messages(
    client: TokenOptClient,
    messages: list[dict],
    session_id: str,
    budget: int,
) -> list[dict]:
    opts = CompileOptions(
        session_id=session_id,
        token_budget=budget,
        keep_recent_tool_results=2,
        soft_sufficiency=True,
        run_sufficiency_check=False,
    )
    result = client.compile(messages, opts)
    return [m.model_dump(exclude_none=True) for m in result.messages]


def run_single_trajectory(
    llm_client: Any,
    model: str,
    tokenopt: TokenOptClient,
    turns: int,
    padding_kb: int,
    budget: int,
) -> tuple[list[dict], list[TurnRecord]]:
    """One real agent run: each turn logs baseline vs compiled size before the LLM call."""
    records: list[TurnRecord] = []
    messages: list[dict] = [
        {
            "role": "system",
            "content": (
                "You are a codebase exploration agent. Be brief (1-2 sentences) "
                "before each tool use. After tools run, summarize findings."
            ),
        },
        {
            "role": "user",
            "content": (
                "Explore the TokenOpt repository: core compiler, transforms, and tests. "
                "Use tools each turn."
            ),
        },
    ]
    session_id = f"e2e-single-{int(time.time())}"
    total_prompt = 0
    total_completion = 0

    for turn in range(1, turns + 1):
        tool_name, tool_args = PLANNED_STEPS[(turn - 1) % len(PLANNED_STEPS)]
        baseline_chars = messages_char_count(messages)
        compiled = compile_messages(tokenopt, messages, session_id, budget)
        optimized_chars = messages_char_count(compiled)

        assistant, usage = chat_completion(llm_client, model, compiled, use_tools=False)
        total_prompt += usage["prompt_tokens"]
        total_completion += usage["completion_tokens"]
        if not assistant.get("content"):
            assistant["content"] = f"Calling {tool_name}."
        messages.append(assistant)

        tool_output = execute_tool_call(tool_name, json.dumps(tool_args), padding_kb=padding_kb)
        messages.append(
            {
                "role": "tool",
                "tool_call_id": f"call_{turn}",
                "name": tool_name,
                "content": tool_output,
            }
        )
        records.append(
            TurnRecord(
                turn=turn,
                baseline_prompt_tokens=usage["prompt_tokens"],
                optimized_prompt_tokens=int(
                    usage["prompt_tokens"] * optimized_chars / baseline_chars
                )
                if baseline_chars
                else usage["prompt_tokens"],
                baseline_messages_chars=baseline_chars,
                optimized_messages_chars=optimized_chars,
                tool=tool_name,
            )
        )

    return messages, records


def run_agent_loop(
    *,
    label: str,
    llm_client: Any,
    model: str,
    tokenopt: TokenOptClient | None,
    turns: int,
    padding_kb: int,
    budget: int,
) -> RunMetrics:
    metrics = RunMetrics(mode=label)
    messages: list[dict] = [
        {
            "role": "system",
            "content": (
                "You are a codebase exploration agent. Be brief (1-2 sentences) "
                "before each tool use. After tools run, summarize findings."
            ),
        },
        {
            "role": "user",
            "content": (
                "Explore the TokenOpt repository: core compiler, transforms, and tests. "
                "Use tools each turn."
            ),
        },
    ]
    session_id = f"e2e-{label}-{int(time.time())}"
    use_native_tools = tokenopt is None or os.environ.get("TOKENOPT_E2E_PROVIDER") != "ollama"

    for turn in range(1, turns + 1):
        tool_name, tool_args = PLANNED_STEPS[(turn - 1) % len(PLANNED_STEPS)]

        baseline_messages = list(messages)
        baseline_chars = messages_char_count(baseline_messages)

        if tokenopt is not None:
            send_messages = compile_messages(tokenopt, baseline_messages, session_id, budget)
        else:
            send_messages = baseline_messages

        optimized_chars = messages_char_count(send_messages)

        assistant, usage = chat_completion(llm_client, model, send_messages, use_tools=False)
        metrics.total_prompt_tokens += usage["prompt_tokens"]
        metrics.total_completion_tokens += usage["completion_tokens"]

        # Append assistant (may mention next action)
        if not assistant.get("content"):
            assistant["content"] = f"I will call {tool_name} to continue exploration."
        messages.append(assistant)

        # Execute REAL tool with optional padding to simulate large outputs
        raw_args = json.dumps(tool_args)
        tool_output = execute_tool_call(tool_name, raw_args, padding_kb=padding_kb)
        messages.append(
            {
                "role": "tool",
                "tool_call_id": f"call_{turn}",
                "name": tool_name,
                "content": tool_output,
            }
        )

        # Estimate optimized prompt tokens if API doesn't report separately
        opt_prompt = usage["prompt_tokens"]
        if tokenopt is not None and baseline_chars > 0:
            ratio = optimized_chars / baseline_chars
            est_opt = int(usage["prompt_tokens"] / ratio) if ratio > 0 else usage["prompt_tokens"]
            baseline_est = usage["prompt_tokens"]
            opt_prompt = int(baseline_est * ratio) if ratio < 1 else usage["prompt_tokens"]
        else:
            baseline_est = usage["prompt_tokens"]

        metrics.turns.append(
            TurnRecord(
                turn=turn,
                baseline_prompt_tokens=baseline_est if tokenopt else usage["prompt_tokens"],
                optimized_prompt_tokens=opt_prompt if tokenopt else usage["prompt_tokens"],
                baseline_messages_chars=baseline_chars,
                optimized_messages_chars=optimized_chars,
                tool=tool_name,
            )
        )

    return metrics


def ensure_tokenopt_server(url: str) -> None:
    try:
        req = urllib.request.Request(f"{url.rstrip('/')}/health")
        with urllib.request.urlopen(req, timeout=2) as r:
            if r.status != 200:
                raise RuntimeError(f"health status {r.status}")
    except (urllib.error.URLError, TimeoutError, RuntimeError) as e:
        raise SystemExit(
            f"tokenopt-server not reachable at {url}: {e}\n"
            "Start: cargo run -p tokenopt-server -- --bind 127.0.0.1:8787"
        ) from e


def start_server_if_requested() -> subprocess.Popen | None:
    if os.environ.get("TOKENOPT_START_SERVER") != "1":
        return None
    proc = subprocess.Popen(
        ["cargo", "run", "-p", "tokenopt-server", "--", "--bind", "127.0.0.1:8787"],
        cwd=ROOT,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )
    for _ in range(30):
        try:
            ensure_tokenopt_server("http://127.0.0.1:8787")
            return proc
        except SystemExit:
            time.sleep(0.5)
    proc.kill()
    raise SystemExit("failed to start tokenopt-server")


def main() -> int:
    parser = argparse.ArgumentParser(description="Real LLM agent E2E vs TokenOpt")
    parser.add_argument("--provider", choices=["openai", "ollama"], default=None)
    parser.add_argument("--model", default=None)
    parser.add_argument("--turns", type=int, default=6)
    parser.add_argument("--padding-kb", type=int, default=4, help="Extra KB per tool result")
    parser.add_argument("--budget", type=int, default=128_000)
    parser.add_argument("--tokenopt-url", default="http://127.0.0.1:8787")
    parser.add_argument(
        "--dual-run",
        action="store_true",
        help="Run two full agent loops (baseline vs tokenopt); costly, non-deterministic",
    )
    args = parser.parse_args()

    provider = args.provider or os.environ.get("TOKENOPT_E2E_PROVIDER", "openai")
    os.environ["TOKENOPT_E2E_PROVIDER"] = provider

    if provider == "ollama":
        model = args.model or os.environ.get("OLLAMA_MODEL", "tinyllama")
        os.environ.setdefault("OPENAI_BASE_URL", "http://127.0.0.1:11434/v1")
    else:
        model = args.model or os.environ.get("OPENAI_MODEL", "gpt-4o-mini")

    server_proc = start_server_if_requested()
    ensure_tokenopt_server(args.tokenopt_url)

    llm = openai_client()
    tok = TokenOptClient(TokenOptConfig(base_url=args.tokenopt_url))

    print(f"Real agent E2E — provider={provider} model={model} turns={args.turns}")
    print(f"Tool padding: {args.padding_kb} KB/result | tokenopt={args.tokenopt_url}")
    print()

    try:
        if args.dual_run:
            print("=== Run A: baseline (no TokenOpt) ===")
            baseline_metrics = run_agent_loop(
                label="baseline",
                llm_client=llm,
                model=model,
                tokenopt=None,
                turns=args.turns,
                padding_kb=args.padding_kb,
                budget=args.budget,
            )
            print(f"  API prompt tokens (sum): {baseline_metrics.total_prompt_tokens}\n")
            print("=== Run B: with TokenOpt ===")
            optimized_metrics = run_agent_loop(
                label="tokenopt",
                llm_client=llm,
                model=model,
                tokenopt=tok,
                turns=args.turns,
                padding_kb=args.padding_kb,
                budget=args.budget,
            )
            print(f"  API prompt tokens (sum): {optimized_metrics.total_prompt_tokens}\n")
            turns_data = optimized_metrics.turns
        else:
            print("=== Single trajectory (real LLM + real tools + TokenOpt each turn) ===")
            _messages, turns_data = run_single_trajectory(
                llm, model, tok, args.turns, args.padding_kb, args.budget
            )

        b_chars = sum(t.baseline_messages_chars for t in turns_data)
        o_chars = sum(t.optimized_messages_chars for t in turns_data)
        char_saved_pct = (1 - o_chars / b_chars) * 100 if b_chars else 0

        print("=== What would be sent to the LLM each turn ===")
        print(f"  without TokenOpt (chars): {b_chars}")
        print(f"  with TokenOpt (chars):    {o_chars}")
        print(f"  reduction:                {char_saved_pct:.1f}%")
        print()
        print(f"{'turn':>4} {'tool':>12} {'base_chars':>12} {'opt_chars':>12} {'saved%':>8}")
        for t in turns_data:
            saved = (
                (1 - t.optimized_messages_chars / t.baseline_messages_chars) * 100
                if t.baseline_messages_chars
                else 0
            )
            print(
                f"{t.turn:>4} {t.tool:>12} {t.baseline_messages_chars:>12} "
                f"{t.optimized_messages_chars:>12} {saved:>7.1f}%"
            )

        report = {
            "provider": provider,
            "model": model,
            "turns": args.turns,
            "padding_kb": args.padding_kb,
            "char_reduction_percent": char_saved_pct,
            "turns_detail": [
                {
                    "turn": t.turn,
                    "tool": t.tool,
                    "baseline_chars": t.baseline_messages_chars,
                    "optimized_chars": t.optimized_messages_chars,
                }
                for t in turns_data
            ],
        }
        out_path = ROOT / "examples" / "e2e" / "last_report.json"
        out_path.write_text(json.dumps(report, indent=2))
        print(f"\nWrote {out_path}")

        if char_saved_pct < 5 and args.turns >= 4:
            print("\nWARN: low char reduction; increase --turns or --padding-kb", file=sys.stderr)
            return 2
        return 0
    finally:
        tok.close()
        if server_proc:
            server_proc.terminate()


if __name__ == "__main__":
    raise SystemExit(main())
