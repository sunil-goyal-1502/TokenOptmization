#!/usr/bin/env python3
"""
Quality A/B: same planned tool trajectory with vs without TokenOpt before each LLM call.

Logs per turn:
  - wall-clock (LLM + optional compile)
  - compile_duration_ms from POST /v1/compare (TokenOpt arm only)
  - OpenAI usage.prompt_tokens / completion_tokens when available
  - task success (tool outputs contain expected substrings)

Requires: tokenopt-server, OPENAI_API_KEY (or --provider ollama).

Usage:
  cargo run -p tokenopt-server -- --bind 127.0.0.1:8787 &
  python3 examples/e2e/run_quality_ab.py --provider openai --turns-per-task 3
"""

from __future__ import annotations

import argparse
import json
import os
import sys
import time
from dataclasses import asdict, dataclass, field
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parents[2]
E2E_DIR = Path(__file__).resolve().parent
sys.path.insert(0, str(E2E_DIR))

try:
    from dotenv import load_dotenv

    load_dotenv(ROOT / ".env")
    load_dotenv()
except ImportError:
    pass

sys.path.insert(0, str(ROOT / "bindings" / "python"))
from tokenopt import CompileOptions, TokenOptClient, TokenOptConfig  # noqa: E402

from run_real_agent_e2e import (  # noqa: E402
    chat_completion,
    ensure_tokenopt_server,
    messages_char_count,
    openai_client,
    start_server_if_requested,
)
from tools import execute_tool_call  # noqa: E402


@dataclass
class TurnMetrics:
    turn: int
    tool: str
    wall_ms: float
    compile_ms: int
    prompt_tokens: int
    completion_tokens: int
    baseline_chars: int
    optimized_chars: int
    tool_ok: bool


@dataclass
class TaskRun:
    task_id: str
    mode: str
    success: bool
    total_wall_ms: float
    total_prompt_tokens: int
    total_compile_ms: int
    char_reduction_percent: float
    turns: list[TurnMetrics] = field(default_factory=list)


def load_tasks(path: Path) -> list[dict[str, Any]]:
    data = json.loads(path.read_text())
    return data["tasks"]


def run_task_arm(
    *,
    task: dict[str, Any],
    mode: str,
    llm: Any,
    model: str,
    tokenopt: TokenOptClient | None,
    padding_kb: int,
    budget: int,
    max_turns: int,
) -> TaskRun:
    steps = task["steps"][:max_turns]
    success_needles = task.get("success_contains", [])
    session_id = f"quality-{task['id']}-{mode}-{int(time.time())}"
    messages: list[dict] = [
        {
            "role": "system",
            "content": "You are a coding agent. Reply briefly before tools run.",
        },
        {"role": "user", "content": task["description"]},
    ]
    tool_outputs: list[str] = []
    turns: list[TurnMetrics] = []
    total_wall = 0.0
    total_prompt = 0
    total_compile = 0
    b_chars_sum = 0
    o_chars_sum = 0

    opts = CompileOptions(
        session_id=session_id,
        token_budget=budget,
        keep_recent_tool_results=2,
        soft_sufficiency=True,
        run_sufficiency_check=False,
    )

    for turn, (tool_name, tool_args) in enumerate(steps, start=1):
        baseline = list(messages)
        b_chars = messages_char_count(baseline)
        compile_ms = 0
        send = baseline

        t0 = time.perf_counter()
        if tokenopt is not None:
            cmp_start = time.perf_counter()
            report = tokenopt.compare(baseline, opts)
            compile_ms = report.compile_duration_ms or int(
                (time.perf_counter() - cmp_start) * 1000
            )
            compiled = tokenopt.compile(baseline, opts)
            send = [m.model_dump(exclude_none=True) for m in compiled.messages]
        o_chars = messages_char_count(send)

        assistant, usage = chat_completion(llm, model, send, use_tools=False)
        wall_ms = (time.perf_counter() - t0) * 1000
        total_wall += wall_ms
        total_prompt += usage.get("prompt_tokens", 0)
        total_compile += compile_ms
        b_chars_sum += b_chars
        o_chars_sum += o_chars

        if not assistant.get("content"):
            assistant["content"] = f"Using {tool_name}."
        messages.append(assistant)

        out = execute_tool_call(tool_name, json.dumps(tool_args), padding_kb=padding_kb)
        tool_outputs.append(out)
        messages.append(
            {
                "role": "tool",
                "tool_call_id": f"call_{turn}",
                "name": tool_name,
                "content": out,
            }
        )

        tool_ok = bool(out.strip())
        turns.append(
            TurnMetrics(
                turn=turn,
                tool=tool_name,
                wall_ms=wall_ms,
                compile_ms=compile_ms,
                prompt_tokens=usage.get("prompt_tokens", 0),
                completion_tokens=usage.get("completion_tokens", 0),
                baseline_chars=b_chars,
                optimized_chars=o_chars,
                tool_ok=tool_ok,
            )
        )

    combined = "\n".join(tool_outputs).lower()
    success = all(needle.lower() in combined for needle in success_needles)
    char_red = (1 - o_chars_sum / b_chars_sum) * 100 if b_chars_sum else 0.0

    return TaskRun(
        task_id=task["id"],
        mode=mode,
        success=success,
        total_wall_ms=total_wall,
        total_prompt_tokens=total_prompt,
        total_compile_ms=total_compile,
        char_reduction_percent=char_red,
        turns=turns,
    )


def main() -> int:
    parser = argparse.ArgumentParser(description="TokenOpt quality A/B harness")
    parser.add_argument("--provider", choices=["openai", "ollama"], default=None)
    parser.add_argument("--model", default=None)
    parser.add_argument("--tasks-file", type=Path, default=E2E_DIR / "quality_tasks.json")
    parser.add_argument("--turns-per-task", type=int, default=None, help="Cap steps per task")
    parser.add_argument("--padding-kb", type=int, default=4)
    parser.add_argument("--budget", type=int, default=128_000)
    parser.add_argument("--tokenopt-url", default="http://127.0.0.1:8787")
    parser.add_argument("--output", type=Path, default=E2E_DIR / "quality_ab_report.json")
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

    tasks = load_tasks(args.tasks_file)
    llm = openai_client()
    tok = TokenOptClient(TokenOptConfig(base_url=args.tokenopt_url))

    print(f"Quality A/B — provider={provider} model={model} tasks={len(tasks)}")
    print()

    all_runs: list[dict[str, Any]] = []
    try:
        for task in tasks:
            max_turns = args.turns_per_task or len(task["steps"])
            print(f"=== Task: {task['id']} ===")
            baseline = run_task_arm(
                task=task,
                mode="baseline",
                llm=llm,
                model=model,
                tokenopt=None,
                padding_kb=args.padding_kb,
                budget=args.budget,
                max_turns=max_turns,
            )
            optimized = run_task_arm(
                task=task,
                mode="tokenopt",
                llm=llm,
                model=model,
                tokenopt=tok,
                padding_kb=args.padding_kb,
                budget=args.budget,
                max_turns=max_turns,
            )
            print(
                f"  baseline: success={baseline.success} "
                f"prompt_tokens={baseline.total_prompt_tokens} "
                f"wall_ms={baseline.total_wall_ms:.0f}"
            )
            print(
                f"  tokenopt: success={optimized.success} "
                f"prompt_tokens={optimized.total_prompt_tokens} "
                f"compile_ms={optimized.total_compile_ms} "
                f"char_reduction={optimized.char_reduction_percent:.1f}% "
                f"wall_ms={optimized.total_wall_ms:.0f}"
            )
            if baseline.success and not optimized.success:
                print("  WARN: TokenOpt run failed success checks", file=sys.stderr)
            all_runs.extend([asdict(baseline), asdict(optimized)])
            print()

        report = {
            "provider": provider,
            "model": model,
            "tasks_file": str(args.tasks_file),
            "runs": all_runs,
        }
        args.output.write_text(json.dumps(report, indent=2))
        print(f"Wrote {args.output}")

        baseline_ok = sum(1 for r in all_runs if r["mode"] == "baseline" and r["success"])
        tokenopt_ok = sum(1 for r in all_runs if r["mode"] == "tokenopt" and r["success"])
        if tokenopt_ok < baseline_ok:
            return 2
        return 0
    finally:
        tok.close()
        if server_proc:
            server_proc.terminate()


if __name__ == "__main__":
    raise SystemExit(main())
