//! Side-by-side baseline vs compiled comparison on the same trace.

use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::compile::{compile_context, CompileOptions, CompileResult};
use crate::error::Result;
use crate::ir::TranscriptMessage;
use crate::store::ColdStore;
use crate::tokens::estimate_blocks_tokens;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompareReport {
    pub baseline_tokens: u64,
    pub compiled_tokens: u64,
    pub tokens_saved: u64,
    pub reduction_percent: f64,
    pub baseline_message_chars: usize,
    pub compiled_message_chars: usize,
    pub char_reduction_percent: f64,
    pub compile_duration_ms: u64,
    pub transform_duration_ms: u64,
    pub oracle_duration_ms: u64,
    pub cold_refs_count: usize,
    pub sufficient: bool,
    pub transforms_applied: Vec<String>,
}

pub async fn compare_trace(
    messages: &[TranscriptMessage],
    options: CompileOptions,
    store: Arc<dyn ColdStore>,
) -> Result<CompareReport> {
    let baseline_tokens = estimate_blocks_tokens(
        &crate::ir::parse_transcript(messages)?,
    );
    let baseline_chars = message_json_chars(messages);

    let compiled: CompileResult = compile_context(messages, options, store, None).await?;
    let compiled_chars = message_json_chars(&compiled.messages);

    let cold_refs_count = compiled
        .blocks
        .iter()
        .filter(|b| b.metadata.cold_ref.is_some())
        .count();

    Ok(CompareReport {
        baseline_tokens,
        compiled_tokens: compiled.stats.output_tokens,
        tokens_saved: compiled.stats.tokens_saved,
        reduction_percent: compiled.stats.reduction_percent,
        baseline_message_chars: baseline_chars,
        compiled_message_chars: compiled_chars,
        char_reduction_percent: if baseline_chars == 0 {
            0.0
        } else {
            (1.0 - compiled_chars as f64 / baseline_chars as f64) * 100.0
        },
        compile_duration_ms: compiled.stats.compile_duration_ms,
        transform_duration_ms: compiled.stats.transform_duration_ms,
        oracle_duration_ms: compiled.stats.oracle_duration_ms,
        cold_refs_count,
        sufficient: compiled.sufficient,
        transforms_applied: compiled.stats.transforms_applied,
    })
}

fn message_json_chars(messages: &[TranscriptMessage]) -> usize {
    serde_json::to_string(messages)
        .map(|s| s.len())
        .unwrap_or(0)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LatencyBenchReport {
    pub iterations: u32,
    pub trace_messages: usize,
    pub p50_ms: u64,
    pub p95_ms: u64,
    pub p99_ms: u64,
    pub min_ms: u64,
    pub max_ms: u64,
    pub mean_ms: f64,
}

pub async fn bench_compile_latency(
    messages: &[TranscriptMessage],
    options: CompileOptions,
    store: Arc<dyn ColdStore>,
    iterations: u32,
) -> Result<LatencyBenchReport> {
    let mut durations = Vec::with_capacity(iterations as usize);
    for i in 0..iterations {
        let mut opts = options.clone();
        opts.session_id = format!("{}-bench-{i}", opts.session_id);
        let start = std::time::Instant::now();
        let _ = compile_context(messages, opts, store.clone(), None).await?;
        durations.push(start.elapsed().as_millis() as u64);
    }
    durations.sort_unstable();
    let n = durations.len();
    let mean_ms = durations.iter().map(|d| *d as f64).sum::<f64>() / n as f64;
    Ok(LatencyBenchReport {
        iterations,
        trace_messages: messages.len(),
        p50_ms: durations[n * 50 / 100],
        p95_ms: durations[n * 95 / 100],
        p99_ms: durations[n * 99 / 100],
        min_ms: durations[0],
        max_ms: durations[n - 1],
        mean_ms,
    })
}
