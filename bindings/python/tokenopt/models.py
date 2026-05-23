from __future__ import annotations

from typing import Any, Literal

from pydantic import BaseModel, Field


class TranscriptMessage(BaseModel):
    role: str
    content: str | list[dict[str, Any]] | None = None
    name: str | None = None
    tool_calls: list[dict[str, Any]] | None = None
    tool_call_id: str | None = None


class Subgoal(BaseModel):
    id: str
    description: str
    required_slots: list[str] = Field(default_factory=list)


class CompileOptions(BaseModel):
    session_id: str = "default"
    token_budget: int = 128_000
    keep_recent_tool_results: int = 3
    error_compaction_after_turns: int = 2
    enable_consumed_masking: bool = True
    run_sufficiency_check: bool = True
    subgoals: list[Subgoal] = Field(default_factory=list)
    infer_subgoals: bool = True
    soft_sufficiency: bool = False


class CompileStats(BaseModel):
    input_tokens: int
    output_tokens: int
    tokens_saved: int
    reduction_percent: float
    blocks_in: int
    blocks_out: int
    transforms_applied: list[str]
    compile_duration_ms: int = 0
    transform_duration_ms: int = 0
    oracle_duration_ms: int = 0
    cold_refs_count: int = 0


class CompileResult(BaseModel):
    blocks: list[dict[str, Any]]
    messages: list[TranscriptMessage]
    stats: CompileStats
    sufficient: bool
    sufficiency_message: str | None = None


class AnalyzeReport(BaseModel):
    total_tokens: int
    block_count: int
    tokens_by_kind: dict[str, int]


class FoldArtifact(BaseModel):
    artifact_type: str = Field(serialization_alias="type", validation_alias="type")
    path: str
    hash: str | None = None


class FoldRecord(BaseModel):
    subgoal: str
    status: Literal["success", "partial", "failed"]
    artifacts: list[FoldArtifact] = Field(default_factory=list)
    preconditions_preserved: list[str] = Field(default_factory=list)
    decisions: list[str] = Field(default_factory=list)
    open_issues: list[str] = Field(default_factory=list)
    token_budget_used: int | None = None
