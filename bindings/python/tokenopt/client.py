from __future__ import annotations

from dataclasses import dataclass
from typing import Any

import httpx

from tokenopt.models import AnalyzeReport, CompileOptions, CompileResult, TranscriptMessage


@dataclass
class TokenOptConfig:
    base_url: str = "http://127.0.0.1:8787"
    timeout_seconds: float = 60.0


class TokenOptClient:
    """HTTP client for `tokenopt-server` — works with any Python orchestrator."""

    def __init__(self, config: TokenOptConfig | None = None) -> None:
        self._config = config or TokenOptConfig()
        self._client = httpx.Client(
            base_url=self._config.base_url.rstrip("/"),
            timeout=self._config.timeout_seconds,
        )

    def close(self) -> None:
        self._client.close()

    def __enter__(self) -> TokenOptClient:
        return self

    def __exit__(self, *args: object) -> None:
        self.close()

    def health(self) -> dict[str, Any]:
        r = self._client.get("/health")
        r.raise_for_status()
        return r.json()

    def analyze(self, messages: list[TranscriptMessage | dict[str, Any]]) -> AnalyzeReport:
        payload = {"messages": [_serialize(m) for m in messages]}
        r = self._client.post("/v1/analyze", json=payload)
        r.raise_for_status()
        return AnalyzeReport.model_validate(r.json())

    def compile(
        self,
        messages: list[TranscriptMessage | dict[str, Any]],
        options: CompileOptions | None = None,
    ) -> CompileResult:
        opts = options or CompileOptions()
        payload = {
            "messages": [_serialize(m) for m in messages],
            "options": opts.model_dump(),
        }
        r = self._client.post("/v1/compile", json=payload)
        r.raise_for_status()
        return CompileResult.model_validate(r.json())

    def compare(
        self,
        messages: list[TranscriptMessage | dict[str, Any]],
        options: CompileOptions | None = None,
    ) -> dict[str, Any]:
        opts = options or CompileOptions()
        payload = {
            "messages": [_serialize(m) for m in messages],
            "options": opts.model_dump(),
        }
        # Compare is CLI-only today; use compile stats + manual baseline via analyze
        analyze = self.analyze(messages)
        compiled = self.compile(messages, opts)
        return {
            "baseline_tokens": analyze.total_tokens,
            "compiled_tokens": compiled.stats.output_tokens,
            "tokens_saved": compiled.stats.tokens_saved,
            "reduction_percent": compiled.stats.reduction_percent,
            "compile_duration_ms": compiled.stats.compile_duration_ms,
            "transform_duration_ms": compiled.stats.transform_duration_ms,
            "oracle_duration_ms": compiled.stats.oracle_duration_ms,
            "cold_refs_count": compiled.stats.cold_refs_count,
            "sufficient": compiled.sufficient,
        }

    def metrics(self) -> dict[str, Any]:
        r = self._client.get("/v1/metrics")
        r.raise_for_status()
        return r.json()

    def rehydrate(
        self,
        messages: list[TranscriptMessage | dict[str, Any]],
        options: dict[str, Any] | None = None,
    ) -> dict[str, Any]:
        payload = {
            "messages": [_serialize(m) for m in messages],
            "options": options or {},
        }
        r = self._client.post("/v1/rehydrate", json=payload)
        r.raise_for_status()
        return r.json()

    def before_model(
        self,
        messages: list[TranscriptMessage | dict[str, Any]],
        session_id: str,
        turn_index: int = 0,
        options: CompileOptions | None = None,
    ) -> list[TranscriptMessage]:
        opts = options or CompileOptions(session_id=session_id)
        opts.session_id = session_id
        payload = {
            "messages": [_serialize(m) for m in messages],
            "session_id": session_id,
            "turn_index": turn_index,
            "options": opts.model_dump(),
        }
        r = self._client.post("/v1/middleware/before-model", json=payload)
        r.raise_for_status()
        data = r.json()
        return [TranscriptMessage.model_validate(m) for m in data["messages"]]


def _serialize(m: TranscriptMessage | dict[str, Any]) -> dict[str, Any]:
    if isinstance(m, TranscriptMessage):
        return m.model_dump(exclude_none=True)
    return m
