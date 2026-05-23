//! Synthetic multi-turn agent traces and loop benchmarks (no live LLM required).

use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::compile::{compile_context, CompileOptions};
use crate::ir::{MessageContent, ToolCall, ToolFunction, TranscriptMessage};
use crate::store::MemoryColdStore;
use crate::tokens::estimate_tokens;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentLoopSimConfig {
    pub turns: u32,
    /// Size of each tool result payload in bytes (approximates large read_file / shell output).
    pub tool_payload_bytes: usize,
    pub keep_recent_tool_results: usize,
    pub token_budget: u64,
    pub enable_consumed_masking: bool,
    pub run_sufficiency_check: bool,
}

impl Default for AgentLoopSimConfig {
    fn default() -> Self {
        Self {
            turns: 20,
            tool_payload_bytes: 8_000,
            keep_recent_tool_results: 2,
            token_budget: 128_000,
            enable_consumed_masking: true,
            run_sufficiency_check: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnMetrics {
    pub turn: u32,
    pub message_count: u32,
    pub baseline_tokens: u64,
    pub compiled_tokens: u64,
    pub tokens_saved: u64,
    pub reduction_percent: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentLoopReport {
    pub config: AgentLoopSimConfig,
    pub turns: Vec<TurnMetrics>,
    pub final_baseline_tokens: u64,
    pub final_compiled_tokens: u64,
    pub final_reduction_percent: f64,
    pub total_tokens_saved_across_turns: u64,
    pub optimizations_active: Vec<String>,
}

/// Build a realistic coding-agent style transcript with growing tool results.
pub fn generate_agent_trace(config: &AgentLoopSimConfig) -> Vec<TranscriptMessage> {
    let mut messages = vec![
        TranscriptMessage {
            role: "system".into(),
            content: Some(MessageContent::Text(
                "You are a coding agent. Use tools to read files and run tests.".into(),
            )),
            name: None,
            tool_calls: None,
            tool_call_id: None,
        },
        TranscriptMessage {
            role: "user".into(),
            content: Some(MessageContent::Text(
                "Fix bugs across crates/tokenopt-core/src/lib.rs and run cargo test.".into(),
            )),
            name: None,
            tool_calls: None,
            tool_call_id: None,
        },
    ];

    let payload = "x".repeat(config.tool_payload_bytes.max(64));
    for turn in 0..config.turns {
        let file = format!("crates/module_{turn}/src/lib.rs");
        let call_id = format!("call_{turn}");
        messages.push(TranscriptMessage {
            role: "assistant".into(),
            content: Some(MessageContent::Text(format!(
                "Turn {turn}: reading {file} and running tests."
            ))),
            name: None,
            tool_calls: Some(vec![ToolCall {
                id: call_id.clone(),
                kind: "function".into(),
                function: ToolFunction {
                    name: "read_file".into(),
                    arguments: format!("{{\"path\":\"{file}\"}}"),
                },
            }]),
            tool_call_id: None,
        });
        let tool_body = if turn % 5 == 4 {
            format!("error: command failed\n{payload}")
        } else {
            format!("// contents of {file}\n{payload}")
        };
        messages.push(TranscriptMessage {
            role: "tool".into(),
            content: Some(MessageContent::Text(tool_body)),
            name: Some("read_file".into()),
            tool_calls: None,
            tool_call_id: Some(call_id),
        });
    }
    messages
}

fn estimate_messages_tokens(messages: &[TranscriptMessage]) -> u64 {
    messages
        .iter()
        .map(|m| {
            let text = crate::ir::extract_text(m);
            estimate_tokens(&text)
                + m
                    .tool_calls
                    .as_ref()
                    .map(|c| {
                        c.iter()
                            .map(|tc| estimate_tokens(&tc.function.arguments) + 20)
                            .sum::<u64>()
                    })
                    .unwrap_or(0)
        })
        .sum()
}

/// Simulate an agent loop: before each model call, compile context and measure tokens.
pub async fn simulate_agent_loop(config: AgentLoopSimConfig) -> crate::error::Result<AgentLoopReport> {
    let messages = generate_agent_trace(&config);
    let store = Arc::new(MemoryColdStore::new());
    let mut turn_metrics = Vec::new();
    let mut total_saved = 0u64;

    for turn in 1..=config.turns {
        let end = 2 + (turn as usize) * 2;
        let slice = &messages[..end.min(messages.len())];
        let baseline_tokens = estimate_messages_tokens(slice);

        let opts = CompileOptions {
            session_id: "sim-agent".into(),
            token_budget: config.token_budget,
            keep_recent_tool_results: config.keep_recent_tool_results,
            enable_consumed_masking: config.enable_consumed_masking,
            run_sufficiency_check: config.run_sufficiency_check,
            infer_subgoals: true,
            ..Default::default()
        };

        let compiled = compile_context(slice, opts, store.clone(), None).await?;
        let compiled_tokens = estimate_messages_tokens(&compiled.messages);
        let saved = baseline_tokens.saturating_sub(compiled_tokens);
        total_saved += saved;
        let reduction_percent = if baseline_tokens == 0 {
            0.0
        } else {
            (saved as f64 / baseline_tokens as f64) * 100.0
        };

        turn_metrics.push(TurnMetrics {
            turn,
            message_count: slice.len() as u32,
            baseline_tokens,
            compiled_tokens,
            tokens_saved: saved,
            reduction_percent,
        });
    }

    let last_turn = turn_metrics.last().cloned().unwrap_or(TurnMetrics {
        turn: 0,
        message_count: 0,
        baseline_tokens: 0,
        compiled_tokens: 0,
        tokens_saved: 0,
        reduction_percent: 0.0,
    });

    let optimizations_active = vec![
        "referential_keep".into(),
        "error_compaction".into(),
        if config.enable_consumed_masking {
            "consumed_result_mask".into()
        } else {
            "consumed_result_mask (disabled)".into()
        },
        "budget_trim".into(),
    ];

    Ok(AgentLoopReport {
        config,
        turns: turn_metrics,
        final_baseline_tokens: last_turn.baseline_tokens,
        final_compiled_tokens: last_turn.compiled_tokens,
        final_reduction_percent: last_turn.reduction_percent,
        total_tokens_saved_across_turns: total_saved,
        optimizations_active,
    })
}
