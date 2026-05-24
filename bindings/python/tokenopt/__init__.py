"""TokenOpt — production client for the sufficiency-gated context compiler."""

from tokenopt.client import TokenOptClient, TokenOptConfig
from tokenopt.models import (
    AnalyzeReport,
    CompareReport,
    CompileOptions,
    CompileResult,
    CompileStats,
    FoldRecord,
    RoutingHint,
    TransformToggles,
    TranscriptMessage,
)

try:
    from tokenopt import native as _native_mod
except ImportError:
    _native_mod = None

__version__ = "0.1.0"
__all__ = [
    "TokenOptClient",
    "TokenOptConfig",
    "AnalyzeReport",
    "CompareReport",
    "TransformToggles",
    "RoutingHint",
    "CompileOptions",
    "CompileResult",
    "CompileStats",
    "FoldRecord",
    "TranscriptMessage",
]
