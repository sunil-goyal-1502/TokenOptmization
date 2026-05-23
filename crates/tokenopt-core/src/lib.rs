//! # tokenopt-core
//!
//! Production context compiler for agent orchestrators: parse transcripts into typed IR,
//! apply sufficiency-gated transforms, and emit budgeted context with cold-store references.
//!
//! ## Integration
//! - Embed directly in Rust orchestrators
//! - Run [`tokenopt-server`] and call from any language
//! - Use Python/TypeScript clients over HTTP

pub mod compare;
pub mod compile;
pub mod error;
pub mod fold;
pub mod ir;
pub mod metrics;
pub mod middleware;
pub mod oracle;
pub mod rehydrate;
pub mod simulate;
pub mod store;
pub mod tokens;
pub mod trace;
pub mod transform;

pub use compare::{bench_compile_latency, compare_trace, CompareReport, LatencyBenchReport};
pub use compile::{
    analyze_trace, compile_context, AnalyzeReport, CompileOptions, CompileResult, CompileStats,
};
pub use metrics::{prometheus_text, snapshot as metrics_snapshot, MetricsSnapshot};
pub use rehydrate::{
    extract_refs_from_text, rehydrate_messages, RehydrateOptions, RehydrateResult,
};
pub use error::{CompilerError, Result};
pub use fold::{FoldArtifact, FoldRecord, FoldStatus};
pub use ir::{
    blocks_to_messages, extract_referents, extract_text, parse_transcript, validate_blocks,
    BlockKind, BlockMetadata, ContextBlock, MessageContent, TranscriptMessage,
};
pub use middleware::{
    AgentMiddleware, MiddlewareAction, MiddlewareContext, OrchestratorAdapter,
};
pub use oracle::{
    infer_subgoals_from_blocks, CompositeOracle, RuleOracle, Subgoal, SufficiencyOracle,
    SufficiencyResult,
};
pub use store::{ColdStore, FileColdStore, MemoryColdStore, StoreRef};
pub use tokens::{estimate_blocks_tokens, estimate_tokens};
pub use simulate::{
    generate_agent_trace, simulate_agent_loop, AgentLoopReport, AgentLoopSimConfig, TurnMetrics,
};
pub use trace::{AgentTrace, TraceMetadata};
pub use transform::{Transform, TransformContext, TransformPipeline, TransformPipelineBuilder};
