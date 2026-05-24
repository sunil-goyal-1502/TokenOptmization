#!/usr/bin/env python3
"""
SWE-bench-lite harness: compile each task trace with TokenOpt and check reduction.

No Docker / patch application — validates context compiler on SWE-style long traces.
"""

from __future__ import annotations

import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "bindings" / "python"))

from tokenopt import CompileOptions, TransformToggles  # noqa: E402

try:
    from tokenopt.native import compile_native, native_available
except ImportError:
    native_available = lambda: False  # type: ignore
    compile_native = None  # type: ignore


def load_trace(path: Path) -> list[dict]:
    data = json.loads(path.read_text())
    if "messages" in data:
        return data["messages"]
    return data


def compile_trace(messages: list[dict], use_native: bool) -> dict:
    opts = CompileOptions(
        session_id="swe-bench",
        keep_recent_tool_results=2,
        run_sufficiency_check=False,
        soft_sufficiency=True,
        transforms=TransformToggles(
            guideline_bank=True,
            summarization=True,
            memory_prune=True,
        ),
        guideline_bank_path=str(ROOT / "fixtures/guidelines/default.json"),
        fold_policy_path=str(ROOT / "fixtures/fold_policies/default.json"),
    )
    if use_native and native_available() and compile_native:
        return compile_native(messages, opts)
    from tokenopt import TokenOptClient, TokenOptConfig

    with TokenOptClient(TokenOptConfig(base_url="http://127.0.0.1:8787")) as client:
        result = client.compile(messages, opts)
        return result.model_dump()


def main() -> int:
    tasks_path = ROOT / "fixtures/swe_bench/lite_tasks.json"
    tasks = json.loads(tasks_path.read_text())["tasks"]
    use_native = "--native" in sys.argv
    failed = 0
    print(f"SWE-bench-lite harness — tasks={len(tasks)} native={use_native}")
    for task in tasks:
        trace_path = ROOT / task["trace_file"]
        if not trace_path.exists():
            print(f"  SKIP {task['instance_id']}: missing {trace_path}", file=sys.stderr)
            failed += 1
            continue
        messages = load_trace(trace_path)
        try:
            out = compile_trace(messages, use_native)
        except Exception as e:
            print(f"  FAIL {task['instance_id']}: {e}", file=sys.stderr)
            failed += 1
            continue
        stats = out.get("stats", out)
        reduction = float(stats.get("reduction_percent", 0))
        min_red = float(task.get("min_reduction_percent", 0))
        ok = reduction >= min_red
        status = "PASS" if ok else "FAIL"
        print(f"  {status} {task['instance_id']}: reduction={reduction:.1f}% (min {min_red}%)")
        if not ok:
            failed += 1
    return 1 if failed else 0


if __name__ == "__main__":
    raise SystemExit(main())
