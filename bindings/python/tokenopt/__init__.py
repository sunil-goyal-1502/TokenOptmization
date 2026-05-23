"""TokenOpt — production client for the sufficiency-gated context compiler."""

from tokenopt.client import TokenOptClient, TokenOptConfig
from tokenopt.models import (
    AnalyzeReport,
    CompareReport,
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
    "CompareReport",
    "CompileOptions",
    "CompileResult",
    "CompileStats",
    "FoldRecord",
    "TranscriptMessage",
]
