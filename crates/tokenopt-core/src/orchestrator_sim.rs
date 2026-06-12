//! Synthetic multi-agent orchestrator benchmark (no live LLM required).
//!
//! Simulates a supervisor + N workers topology where workers overlap on a
//! configurable fraction of tool reads, then compares three strategies per
//! round:
//!
//! - **baseline** — no compilation, full transcripts for every agent
//! - **independent** — per-agent compile with a uniform budget split and no
//!   cross-agent awareness (state of the art before this work)
//! - **maco** — cross-agent dedup + water-filling allocation
//!   ([`crate::orchestrator::compile_multi_agent`])

use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::ir::{MessageContent, ToolCall, ToolFunction, TranscriptMessage};
use crate::orchestrator::{
    compile_multi_agent, AgentContext, AgentRole, AllocationStrategy, MultiAgentOptions,
};
use crate::store::MemoryColdStore;
use crate::tokens::estimate_messages_tokens;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrchestratorSimConfig {
    pub workers: u32,
    pub rounds: u32,
    /// Size of each tool result payload in bytes.
    pub tool_payload_bytes: usize,
    /// Fraction of worker reads per round that hit the same shared artifact
    /// (identical payload across workers), in [0, 1].
    pub shared_read_fraction: f64,
    pub global_token_budget: u64,
    pub keep_recent_tool_results: usize,
}

impl Default for OrchestratorSimConfig {
    fn default() -> Self {
        Self {
            workers: 4,
            rounds: 10,
            tool_payload_bytes: 6_000,
            shared_read_fraction: 0.5,
            global_token_budget: 64_000,
            keep_recent_tool_results: 2,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrchestratorRoundMetrics {
    pub round: u32,
    pub baseline_tokens: u64,
    pub independent_tokens: u64,
    pub maco_tokens: u64,
    pub dedup_duplicates_masked: usize,
    pub dedup_tokens_saved: u64,
    pub maco_vs_baseline_percent: f64,
    pub maco_vs_independent_percent: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrchestratorSimReport {
    pub config: OrchestratorSimConfig,
    pub rounds: Vec<OrchestratorRoundMetrics>,
    pub final_baseline_tokens: u64,
    pub final_independent_tokens: u64,
    pub final_maco_tokens: u64,
    pub final_maco_vs_baseline_percent: f64,
    pub final_maco_vs_independent_percent: f64,
    pub cumulative_baseline_tokens: u64,
    pub cumulative_independent_tokens: u64,
    pub cumulative_maco_tokens: u64,
}

fn text_message(role: &str, content: String) -> TranscriptMessage {
    TranscriptMessage {
        role: role.into(),
        content: Some(MessageContent::Text(content)),
        name: None,
        tool_calls: None,
        tool_call_id: None,
    }
}

/// Deterministic supervisor + workers scenario through `rounds` rounds.
pub fn generate_orchestrator_scenario(config: &OrchestratorSimConfig) -> Vec<AgentContext> {
    let payload = "x".repeat(config.tool_payload_bytes.max(64));
    let shared_workers_per_round =
        ((config.workers as f64) * config.shared_read_fraction).round() as u32;

    let mut supervisor_messages = vec![
        text_message(
            "system",
            "You are the supervisor. Decompose the task and delegate to workers.".into(),
        ),
        text_message(
            "user",
            "Refactor the workspace: update crates, fix tests, and write docs.".into(),
        ),
    ];

    let mut workers: Vec<AgentContext> = (0..config.workers)
        .map(|w| AgentContext {
            agent_id: format!("worker-{w}"),
            role: AgentRole::Worker,
            priority: 1.0,
            feedback: Default::default(),
            messages: vec![
                text_message(
                    "system",
                    format!("You are worker-{w}. Complete your assigned subtask using tools."),
                ),
                text_message("user", format!("Subtask {w}: edit module_{w} and verify.")),
            ],
        })
        .collect();

    for round in 0..config.rounds {
        supervisor_messages.push(text_message(
            "assistant",
            format!(
                "Round {round}: delegating subtasks to {} workers; tracking crates/module_{round}/src/lib.rs.",
                config.workers
            ),
        ));

        for (w, worker) in workers.iter_mut().enumerate() {
            let is_shared = (w as u32) < shared_workers_per_round;
            // Shared reads return byte-identical payloads across workers.
            let (file, body) = if is_shared {
                (
                    format!("crates/shared/src/round_{round}.rs"),
                    format!("// shared artifact round {round}\n{payload}"),
                )
            } else {
                (
                    format!("crates/module_{w}/src/round_{round}.rs"),
                    format!("// private artifact worker {w} round {round}\n{payload}"),
                )
            };
            let call_id = format!("call_w{w}_r{round}");
            worker.messages.push(TranscriptMessage {
                role: "assistant".into(),
                content: Some(MessageContent::Text(format!(
                    "Round {round}: reading {file}."
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
            worker.messages.push(TranscriptMessage {
                role: "tool".into(),
                content: Some(MessageContent::Text(body)),
                name: Some("read_file".into()),
                tool_calls: None,
                tool_call_id: Some(call_id),
            });

            supervisor_messages.push(text_message(
                "assistant",
                format!("worker-{w} reported progress on round {round} ({file})."),
            ));
        }
    }

    let mut agents = vec![AgentContext {
        agent_id: "supervisor".into(),
        role: AgentRole::Supervisor,
        priority: 1.0,
        feedback: Default::default(),
        messages: supervisor_messages,
    }];
    agents.extend(workers);
    agents
}

fn truncate_to_round(agents: &[AgentContext], config: &OrchestratorSimConfig, round: u32) -> Vec<AgentContext> {
    // Supervisor grows by 1 + workers messages per round (after 2 seed msgs);
    // each worker grows by 2 messages per round (after 2 seed msgs).
    let supervisor_len = 2 + (round as usize) * (1 + config.workers as usize);
    let worker_len = 2 + (round as usize) * 2;
    agents
        .iter()
        .map(|a| {
            let len = if a.role == AgentRole::Supervisor {
                supervisor_len
            } else {
                worker_len
            };
            AgentContext {
                messages: a.messages[..len.min(a.messages.len())].to_vec(),
                ..a.clone()
            }
        })
        .collect()
}

fn percent_saved(baseline: u64, value: u64) -> f64 {
    if baseline == 0 {
        0.0
    } else {
        (baseline.saturating_sub(value) as f64 / baseline as f64) * 100.0
    }
}

/// Run the orchestrator loop, compiling all agents before each round's model
/// calls, and compare baseline vs independent vs MACO strategies.
pub async fn simulate_orchestrator_loop(
    config: OrchestratorSimConfig,
) -> crate::error::Result<OrchestratorSimReport> {
    let full = generate_orchestrator_scenario(&config);
    let mut rounds = Vec::new();
    let mut cumulative = (0u64, 0u64, 0u64);

    for round in 1..=config.rounds {
        let agents = truncate_to_round(&full, &config, round);
        let baseline_tokens: u64 = agents
            .iter()
            .map(|a| estimate_messages_tokens(&a.messages, "gpt-4o-mini"))
            .sum();

        let mut base = crate::compile::CompileOptions {
            keep_recent_tool_results: config.keep_recent_tool_results,
            run_sufficiency_check: false,
            ..Default::default()
        };
        base.routing_hints = false;

        let independent = compile_multi_agent(
            &agents,
            MultiAgentOptions {
                global_token_budget: config.global_token_budget,
                allocation: AllocationStrategy::Uniform,
                cross_agent_dedup: false,
                base: base.clone(),
                ..Default::default()
            },
            Arc::new(MemoryColdStore::new()),
        )
        .await?;

        let maco = compile_multi_agent(
            &agents,
            MultiAgentOptions {
                global_token_budget: config.global_token_budget,
                allocation: AllocationStrategy::WaterFilling,
                cross_agent_dedup: true,
                base,
                ..Default::default()
            },
            Arc::new(MemoryColdStore::new()),
        )
        .await?;

        let independent_tokens = independent.total_output_tokens;
        let maco_tokens = maco.total_output_tokens;
        cumulative.0 += baseline_tokens;
        cumulative.1 += independent_tokens;
        cumulative.2 += maco_tokens;

        rounds.push(OrchestratorRoundMetrics {
            round,
            baseline_tokens,
            independent_tokens,
            maco_tokens,
            dedup_duplicates_masked: maco.dedup.duplicates_masked,
            dedup_tokens_saved: maco.dedup.tokens_saved,
            maco_vs_baseline_percent: percent_saved(baseline_tokens, maco_tokens),
            maco_vs_independent_percent: percent_saved(independent_tokens, maco_tokens),
        });
    }

    let last = rounds.last().cloned().expect("at least one round");
    Ok(OrchestratorSimReport {
        config,
        final_baseline_tokens: last.baseline_tokens,
        final_independent_tokens: last.independent_tokens,
        final_maco_tokens: last.maco_tokens,
        final_maco_vs_baseline_percent: last.maco_vs_baseline_percent,
        final_maco_vs_independent_percent: last.maco_vs_independent_percent,
        cumulative_baseline_tokens: cumulative.0,
        cumulative_independent_tokens: cumulative.1,
        cumulative_maco_tokens: cumulative.2,
        rounds,
    })
}
