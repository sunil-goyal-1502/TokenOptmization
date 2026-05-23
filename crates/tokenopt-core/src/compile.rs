use std::collections::HashMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::error::{CompilerError, Result};
use crate::ir::{
    blocks_to_messages, parse_transcript, validate_blocks, ContextBlock, TranscriptMessage,
};
use crate::oracle::{
    infer_subgoals_from_blocks, CompositeOracle, RuleOracle, Subgoal, SufficiencyOracle,
    SufficiencyResult,
};
use crate::metrics::{record_compile_error, record_compile_success};
use crate::store::ColdStore;
use crate::tokens::{estimate_blocks_tokens, estimate_tokens};
use crate::transform::{TransformContext, TransformPipelineBuilder};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompileOptions {
    pub session_id: String,
    pub token_budget: u64,
    #[serde(default = "default_keep_recent")]
    pub keep_recent_tool_results: usize,
    #[serde(default = "default_error_turns")]
    pub error_compaction_after_turns: u32,
    #[serde(default = "default_true")]
    pub enable_consumed_masking: bool,
    #[serde(default = "default_true")]
    pub run_sufficiency_check: bool,
    #[serde(default)]
    pub subgoals: Vec<Subgoal>,
    #[serde(default = "default_true")]
    pub infer_subgoals: bool,
    /// When true, sufficiency failure returns original messages with `sufficient: false` instead of error.
    #[serde(default)]
    pub soft_sufficiency: bool,
}

fn default_keep_recent() -> usize {
    3
}
fn default_error_turns() -> u32 {
    2
}
fn default_true() -> bool {
    true
}

impl Default for CompileOptions {
    fn default() -> Self {
        Self {
            session_id: "default".into(),
            token_budget: 128_000,
            keep_recent_tool_results: default_keep_recent(),
            error_compaction_after_turns: default_error_turns(),
            enable_consumed_masking: true,
            run_sufficiency_check: true,
            subgoals: vec![],
            infer_subgoals: true,
            soft_sufficiency: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompileStats {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub tokens_saved: u64,
    pub reduction_percent: f64,
    pub blocks_in: usize,
    pub blocks_out: usize,
    pub transforms_applied: Vec<String>,
    pub compile_duration_ms: u64,
    pub transform_duration_ms: u64,
    pub oracle_duration_ms: u64,
    pub cold_refs_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompileResult {
    pub blocks: Vec<ContextBlock>,
    pub messages: Vec<TranscriptMessage>,
    pub stats: CompileStats,
    pub sufficient: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sufficiency_message: Option<String>,
}

pub async fn compile_context(
    messages: &[TranscriptMessage],
    options: CompileOptions,
    store: Arc<dyn ColdStore>,
    oracle: Option<Arc<dyn SufficiencyOracle>>,
) -> Result<CompileResult> {
    let result = compile_context_inner(messages, options, store, oracle).await;
    match &result {
        Ok(r) => record_compile_success(r.stats.compile_duration_ms),
        Err(_) => record_compile_error(),
    }
    result
}

async fn compile_context_inner(
    messages: &[TranscriptMessage],
    options: CompileOptions,
    store: Arc<dyn ColdStore>,
    oracle: Option<Arc<dyn SufficiencyOracle>>,
) -> Result<CompileResult> {
    let compile_start = std::time::Instant::now();
    let blocks_in = parse_transcript(messages)?;
    validate_blocks(&blocks_in)?;
    let input_tokens = estimate_blocks_tokens(&blocks_in);

    let ctx = TransformContext {
        session_id: options.session_id.clone(),
        token_budget: options.token_budget,
        keep_recent_tool_results: options.keep_recent_tool_results,
        error_compaction_after_turns: options.error_compaction_after_turns,
        enable_consumed_masking: options.enable_consumed_masking,
    };

    let pipeline = TransformPipelineBuilder::with_defaults().build();
    let transforms_applied: Vec<String> = pipeline
        .transform_names()
        .into_iter()
        .map(|s| s.to_string())
        .collect();

    let blocks_in_len = blocks_in.len();
    let transform_start = std::time::Instant::now();
    let blocks_out = pipeline.run(blocks_in.clone(), &ctx, store.clone()).await?;
    let transform_ms = transform_start.elapsed().as_millis() as u64;

    let subgoals = if options.subgoals.is_empty() && options.infer_subgoals {
        infer_subgoals_from_blocks(&blocks_in)
    } else {
        options.subgoals.clone()
    };

    let oracle_impl: Arc<dyn SufficiencyOracle> = oracle.unwrap_or_else(|| {
        Arc::new(CompositeOracle {
            oracles: vec![Arc::new(RuleOracle::default())],
        })
    });

    let oracle_start = std::time::Instant::now();
    let sufficiency: SufficiencyResult = if options.run_sufficiency_check {
        oracle_impl.check(&blocks_out, &subgoals).await?
    } else {
        SufficiencyResult {
            sufficient: true,
            missing_slots: vec![],
            message: None,
        }
    };
    let oracle_ms = oracle_start.elapsed().as_millis() as u64;

    if options.run_sufficiency_check && !sufficiency.sufficient && !options.soft_sufficiency {
        return Err(CompilerError::SufficiencyFailed(
            sufficiency
                .message
                .clone()
                .unwrap_or_else(|| "sufficiency check failed".into()),
        ));
    }

    let (final_blocks, final_messages) = if sufficiency.sufficient {
        let msgs = blocks_to_messages(&blocks_out)?;
        (blocks_out, msgs)
    } else if options.soft_sufficiency {
        let msgs = blocks_to_messages(&blocks_in)?;
        (blocks_in, msgs)
    } else {
        let msgs = blocks_to_messages(&blocks_out)?;
        (blocks_out, msgs)
    };

    let final_output_tokens = estimate_blocks_tokens(&final_blocks);
    let final_blocks_len = final_blocks.len();
    let tokens_saved = input_tokens.saturating_sub(final_output_tokens);
    let reduction_percent = if input_tokens == 0 {
        0.0
    } else {
        (tokens_saved as f64 / input_tokens as f64) * 100.0
    };

    let cold_refs_count = final_blocks
        .iter()
        .filter(|b| b.metadata.cold_ref.is_some())
        .count();
    let compile_ms = compile_start.elapsed().as_millis() as u64;

    Ok(CompileResult {
        blocks: final_blocks,
        messages: final_messages,
        stats: CompileStats {
            input_tokens,
            output_tokens: final_output_tokens,
            tokens_saved,
            reduction_percent,
            blocks_in: blocks_in_len,
            blocks_out: final_blocks_len,
            transforms_applied,
            compile_duration_ms: compile_ms,
            transform_duration_ms: transform_ms,
            oracle_duration_ms: oracle_ms,
            cold_refs_count,
        },
        sufficient: sufficiency.sufficient,
        sufficiency_message: sufficiency.message,
    })
}

pub fn analyze_trace(messages: &[TranscriptMessage]) -> Result<AnalyzeReport> {
    let blocks = parse_transcript(messages)?;
    validate_blocks(&blocks)?;
    let total = estimate_blocks_tokens(&blocks);
    let mut by_kind: HashMap<String, u64> = HashMap::new();
    for block in &blocks {
        let kind = format!("{:?}", block.kind);
        *by_kind.entry(kind).or_insert(0) += estimate_tokens(&block.content);
    }
    Ok(AnalyzeReport {
        total_tokens: total,
        block_count: blocks.len(),
        tokens_by_kind: by_kind,
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnalyzeReport {
    pub total_tokens: u64,
    pub block_count: usize,
    pub tokens_by_kind: HashMap<String, u64>,
}
