"""LangGraph / custom orchestrator helpers."""

from __future__ import annotations

from typing import Any, Callable

from tokenopt.client import TokenOptClient
from tokenopt.models import CompileOptions, TranscriptMessage


def make_before_model_hook(
    client: TokenOptClient,
    session_id: str,
    options: CompileOptions | None = None,
) -> Callable[[list[dict[str, Any]]], list[dict[str, Any]]]:
    """Wrap agent state messages before LLM invocation."""

    def hook(messages: list[dict[str, Any]]) -> list[dict[str, Any]]:
        compiled = client.before_model(
            messages=[TranscriptMessage.model_validate(m) for m in messages],
            session_id=session_id,
            options=options,
        )
        return [m.model_dump(exclude_none=True) for m in compiled]

    return hook
