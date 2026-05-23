"""TokenOpt — production client for the sufficiency-gated context compiler."""

from tokenopt.client import TokenOptClient, TokenOptConfig
from tokenopt.models import (
    AnalyzeReport,
    CompileOptions,
    CompileResult,
    CompileStats,
    FoldRecord,
    TranscriptMessage,
)

__version__ = "0.1.0"
__all__ = [
    "TokenOptClient",
    "TokenOptConfig",
    "AnalyzeReport",
    "CompileOptions",
    "CompileResult",
    "CompileStats",
    "FoldRecord",
    "TranscriptMessage",
]
