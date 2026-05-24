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
use crate::fold::FoldRecord;
use crate::llm::{LlmConfig, LlmSufficiencyOracle};
use crate::metrics::{record_compile_error, record_compile_success};
use crate::pipeline::build_pipeline;
use crate::rehydrate::{extract_refs_from_text, rehydrate_messages, RehydrateOptions};
use crate::routing::{compute_routing_hint, RoutingHint};
use crate::store::ColdStore;
use crate::tokens::{estimate_blocks_tokens, estimate_tokens};
use crate::transform::TransformContext;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransformToggles {
    #[serde(default = "default_true")]
    pub referential_keep: bool,
    #[serde(default = "default_true")]
    pub error_compaction: bool,
    #[serde(default = "default_true")]
    pub consumed_result_mask: bool,
    #[serde(default = "default_true")]
    pub rolling_window: bool,
    #[serde(default = "default_true")]
    pub budget_trim: bool,
    #[serde(default)]
    pub guideline_bank: bool,
    #[serde(default = "default_true")]
    pub fold_collapse: bool,
    #[serde(default = "default_true")]
    pub agent_omit: bool,
    #[serde(default)]
    pub memory_prune: bool,
    #[serde(default = "default_true")]
    pub summarization: bool,
    #[serde(default = "default_true")]
    pub external_compress: bool,
    #[serde(default = "default_true")]
    pub cache_packer: bool,
    #[serde(default)]
    pub llm_summarization: bool,
    #[serde(default)]
    pub bacm: bool,
    #[serde(default)]
    pub fold_policy: bool,
}

impl Default for TransformToggles {
    fn default() -> Self {
        Self {
            referential_keep: true,
            error_compaction: true,
            consumed_result_mask: true,
            rolling_window: true,
            budget_trim: true,
            guideline_bank: false,
            fold_collapse: true,
            agent_omit: true,
            memory_prune: false,
            summarization: true,
            external_compress: true,
            cache_packer: true,
            llm_summarization: false,
            bacm: false,
            fold_policy: false,
        }
    }
}

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
    /// Model name for accurate token counting when `accurate-tokens` feature is enabled.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_count_model: Option<String>,
    #[serde(default)]
    pub transforms: TransformToggles,
    #[serde(default)]
    pub fold_records: Vec<FoldRecord>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guideline_bank_path: Option<String>,
    #[serde(default = "default_rolling_tail")]
    pub rolling_tail_blocks: usize,
    #[serde(default = "default_summarize_keep")]
    pub summarize_keep_recent_blocks: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external_compress_url: Option<String>,
    #[serde(default)]
    pub llm_oracle: LlmConfig,
    #[serde(default)]
    pub auto_rehydrate_refs: bool,
    #[serde(default = "default_true")]
    pub routing_hints: bool,
    #[serde(default)]
    pub turn_index: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fold_policy_path: Option<String>,
    /// Shared LLM config for oracle + optional `llm_summarize` transform.
    #[serde(default)]
    pub llm_summarize: LlmConfig,
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
fn default_rolling_tail() -> usize {
    16
}
fn default_summarize_keep() -> usize {
    12
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
            token_count_model: None,
            transforms: TransformToggles::default(),
            fold_records: vec![],
            guideline_bank_path: None,
            rolling_tail_blocks: default_rolling_tail(),
            summarize_keep_recent_blocks: default_summarize_keep(),
            external_compress_url: None,
            llm_oracle: LlmConfig::default(),
            auto_rehydrate_refs: false,
            routing_hints: true,
            turn_index: 0,
            fold_policy_path: None,
            llm_summarize: LlmConfig::default(),
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
    pub token_count_method: crate::tokens::TokenCountMethod,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompileResult {
    pub blocks: Vec<ContextBlock>,
    pub messages: Vec<TranscriptMessage>,
    pub stats: CompileStats,
    pub sufficient: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sufficiency_message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub routing_hint: Option<RoutingHint>,
}

fn duration_ms(d: std::time::Duration) -> u64 {
    let micros = d.as_micros();
    if micros == 0 {
        0
    } else {
        ((micros + 999) / 1000).max(1) as u64
    }
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

    let mut working_messages = messages.to_vec();
    if options.auto_rehydrate_refs {
        let mut refs = Vec::new();
        for msg in messages {
            refs.extend(extract_refs_from_text(&crate::ir::extract_text(msg)));
        }
        refs.sort();
        refs.dedup();
        if !refs.is_empty() {
            working_messages = rehydrate_messages(
                &working_messages,
                store.clone(),
                RehydrateOptions {
                    refs,
                    ..Default::default()
                },
            )
            .await?
            .messages;
        }
    }

    let blocks_in = parse_transcript(&working_messages)?;
    validate_blocks(&blocks_in)?;
    let input_tokens = estimate_blocks_tokens(&blocks_in);

    let mut pipeline_options = options.clone();
    if pipeline_options.transforms.fold_policy {
        let policy = pipeline_options
            .fold_policy_path
            .as_ref()
            .and_then(|p| crate::fold_policy::FoldPolicy::load_from_path(p).ok())
            .unwrap_or_else(crate::fold_policy::FoldPolicy::default_builtin);
        pipeline_options
            .fold_records
            .extend(policy.derive_fold_records(&blocks_in));
    }
    if !pipeline_options.enable_consumed_masking {
        pipeline_options.transforms.consumed_result_mask = false;
    }

    let ctx = TransformContext {
        session_id: options.session_id.clone(),
        token_budget: options.token_budget,
        keep_recent_tool_results: options.keep_recent_tool_results,
        error_compaction_after_turns: options.error_compaction_after_turns,
        enable_consumed_masking: options.enable_consumed_masking,
        rolling_tail_blocks: options.rolling_tail_blocks,
    };

    let pipeline = build_pipeline(&pipeline_options);
    let transforms_applied: Vec<String> = pipeline
        .transform_names()
        .into_iter()
        .map(|s| s.to_string())
        .collect();

    let blocks_in_len = blocks_in.len();
    let transform_start = std::time::Instant::now();
    let blocks_out = pipeline.run(blocks_in.clone(), &ctx, store.clone()).await?;
    let transform_ms = duration_ms(transform_start.elapsed());

    let subgoals = if options.subgoals.is_empty() && options.infer_subgoals {
        infer_subgoals_from_blocks(&blocks_in)
    } else {
        options.subgoals.clone()
    };

    let oracle_impl: Arc<dyn SufficiencyOracle> = oracle.unwrap_or_else(|| {
        let mut oracles: Vec<Arc<dyn SufficiencyOracle>> = vec![Arc::new(RuleOracle::default())];
        if options.llm_oracle.enabled {
            oracles.push(Arc::new(LlmSufficiencyOracle::new(options.llm_oracle.clone())));
        }
        Arc::new(CompositeOracle { oracles })
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
    let oracle_ms = duration_ms(oracle_start.elapsed());

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
    let compile_ms = duration_ms(compile_start.elapsed());

    let routing_hint = if options.routing_hints {
        Some(compute_routing_hint(
            final_output_tokens,
            options.token_budget,
            &sufficiency,
            options.turn_index,
        ))
    } else {
        None
    };

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
            token_count_method: crate::tokens::token_count_method(),
        },
        sufficient: sufficiency.sufficient,
        sufficiency_message: sufficiency.message,
        routing_hint,
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
