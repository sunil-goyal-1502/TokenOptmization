"""In-process TokenOpt via pyo3 (`tokenopt._native`)."""

from __future__ import annotations

import json
from typing import Any

from tokenopt.models import CompileOptions, TranscriptMessage


def native_available() -> bool:
    try:
        import tokenopt._native  # noqa: F401

        return True
    except ImportError:
        return False


def _serialize_messages(messages: list[TranscriptMessage | dict[str, Any]]) -> str:
    out = []
    for m in messages:
        if isinstance(m, TranscriptMessage):
            out.append(m.model_dump(exclude_none=True))
        else:
            out.append(m)
    return json.dumps(out)


def compile_native(
    messages: list[TranscriptMessage | dict[str, Any]],
    options: CompileOptions | None = None,
) -> dict[str, Any]:
    import tokenopt._native as native

    opts = options or CompileOptions()
    raw = native.compile_json(
        _serialize_messages(messages),
        opts.model_dump_json(exclude_none=True),
    )
    return json.loads(raw)


def compare_native(
    messages: list[TranscriptMessage | dict[str, Any]],
    options: CompileOptions | None = None,
) -> dict[str, Any]:
    import tokenopt._native as native

    opts = options or CompileOptions()
    raw = native.compare_json(
        _serialize_messages(messages),
        opts.model_dump_json(exclude_none=True),
    )
    return json.loads(raw)
