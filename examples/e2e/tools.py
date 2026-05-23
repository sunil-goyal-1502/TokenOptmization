"""Real tool implementations (filesystem) for E2E agent tests."""

from __future__ import annotations

import json
import os
import subprocess
from pathlib import Path

WORKSPACE = Path(__file__).resolve().parents[2]


def read_file(path: str, padding_kb: int = 0) -> str:
    p = (WORKSPACE / path).resolve()
    if not str(p).startswith(str(WORKSPACE.resolve())):
        return f"error: path outside workspace: {path}"
    if not p.is_file():
        return f"error: file not found: {path}"
    content = p.read_text(encoding="utf-8", errors="replace")
    if padding_kb > 0:
        content += "\n# " + ("x" * (padding_kb * 1024)) + "\n"
    return content


def list_dir(path: str = ".") -> str:
    p = (WORKSPACE / path).resolve()
    if not str(p).startswith(str(WORKSPACE.resolve())):
        return f"error: path outside workspace"
    if not p.is_dir():
        return f"error: not a directory: {path}"
    entries = sorted(os.listdir(p))[:50]
    return "\n".join(entries)


def grep(pattern: str, path: str = "crates") -> str:
    try:
        r = subprocess.run(
            ["rg", "-l", pattern, str(WORKSPACE / path)],
            capture_output=True,
            text=True,
            timeout=30,
        )
        out = r.stdout.strip() or r.stderr.strip() or "(no matches)"
        return out[:8000]
    except FileNotFoundError:
        return "error: rg not installed"
    except subprocess.TimeoutExpired:
        return "error: grep timeout"


def run_tool(name: str, arguments: dict, padding_kb: int = 0) -> str:
    if name == "read_file":
        return read_file(arguments.get("path", ""), padding_kb=padding_kb)
    if name == "list_dir":
        return list_dir(arguments.get("path", "."))
    if name == "grep":
        return grep(arguments.get("pattern", ""), arguments.get("path", "crates"))
    return f"error: unknown tool {name}"


def tools_schema() -> list[dict]:
    return [
        {
            "type": "function",
            "function": {
                "name": "read_file",
                "description": "Read a file from the repo",
                "parameters": {
                    "type": "object",
                    "properties": {"path": {"type": "string"}},
                    "required": ["path"],
                },
            },
        },
        {
            "type": "function",
            "function": {
                "name": "list_dir",
                "description": "List directory entries",
                "parameters": {
                    "type": "object",
                    "properties": {"path": {"type": "string"}},
                },
            },
        },
        {
            "type": "function",
            "function": {
                "name": "grep",
                "description": "Search codebase with ripgrep",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "pattern": {"type": "string"},
                        "path": {"type": "string"},
                    },
                    "required": ["pattern"],
                },
            },
        },
    ]


def execute_tool_call(name: str, raw_args: str, padding_kb: int = 0) -> str:
    try:
        args = json.loads(raw_args) if raw_args.strip() else {}
    except json.JSONDecodeError:
        args = {}
    return run_tool(name, args, padding_kb=padding_kb)
