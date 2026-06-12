//! Orchestrator-level context optimization for multi-agent systems (MACO).
//!
//! Single-agent compilation treats every agent's transcript independently. In
//! multi-agent systems (supervisor/worker trees, swarms, critic loops) token
//! waste has *cross-agent* structure that per-agent compilers cannot see:
//!
//! 1. **Shared global budget** — tokens are a shared resource. MACO allocates a
//!    global budget across agents by maximizing total marginal utility
//!    (weighted log-utility water-filling) instead of uniform splits.
//! 2. **Cross-agent redundancy** — agents read the same files and re-run the
//!    same tools. MACO content-addresses tool results and keeps one canonical
//!    copy per orchestrator, replacing duplicates with shared `ref://` previews.
//! 3. **Consumer-contract handoffs** — a handoff (`FoldRecord`) is compressed
//!    against the *receiving* agent's required slots, not the sender's view.
//! 4. **Regret feedback** — sufficiency failures and rehydration requests are
//!    implicit evidence the compiler cut too much for an agent; they raise that
//!    agent's allocation weight on the next round.
//!
//! See `docs/MULTI_AGENT_RESEARCH.md` for the full research design.

use std::collections::HashMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::compile::{compile_context, CompileOptions, CompileStats};
use crate::error::{CompilerError, Result};
use crate::fold::FoldRecord;
use crate::ir::{extract_text, MessageContent, TranscriptMessage};
use crate::store::ColdStore;
use crate::tokens::{estimate_messages_tokens, estimate_tokens};

// ---------------------------------------------------------------------------
// Agent model
// ---------------------------------------------------------------------------

/// Role of an agent inside the orchestrator topology.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AgentRole {
    Supervisor,
    #[default]
    Worker,
    Critic,
    Memory,
    Other,
}

impl AgentRole {
    /// Baseline utility multiplier per role. Supervisors carry routing state
    /// for the whole tree; critics gate quality; memory agents tolerate
    /// aggressive compression because their state lives in external stores.
    pub fn weight_multiplier(self) -> f64 {
        match self {
            AgentRole::Supervisor => 1.5,
            AgentRole::Critic => 1.2,
            AgentRole::Worker => 1.0,
            AgentRole::Other => 1.0,
            AgentRole::Memory => 0.8,
        }
    }
}

/// Implicit regret signals observed since the last orchestrator round.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct AgentFeedback {
    /// Times the sufficiency oracle reported missing slots for this agent.
    #[serde(default)]
    pub sufficiency_failures: u32,
    /// Times the agent had to rehydrate a masked `ref://` payload.
    #[serde(default)]
    pub rehydration_requests: u32,
}

impl AgentFeedback {
    pub fn signal_count(self) -> u32 {
        self.sufficiency_failures + self.rehydration_requests
    }
}

/// One agent's transcript plus allocation inputs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentContext {
    pub agent_id: String,
    #[serde(default)]
    pub role: AgentRole,
    /// Caller-supplied priority; combined with role and feedback into the
    /// allocation weight. Defaults to 1.0.
    #[serde(default = "default_priority")]
    pub priority: f64,
    #[serde(default)]
    pub feedback: AgentFeedback,
    pub messages: Vec<TranscriptMessage>,
}

fn default_priority() -> f64 {
    1.0
}

/// Per-signal weight boost applied by the regret controller.
const FEEDBACK_BOOST_PER_SIGNAL: f64 = 0.25;
/// Cap on the multiplicative feedback boost to keep the ledger stable.
const FEEDBACK_BOOST_MAX: f64 = 3.0;

/// Effective allocation weight: priority x role multiplier x regret boost.
pub fn effective_weight(priority: f64, role: AgentRole, feedback: AgentFeedback) -> f64 {
    let boost =
        (1.0 + FEEDBACK_BOOST_PER_SIGNAL * feedback.signal_count() as f64).min(FEEDBACK_BOOST_MAX);
    priority.max(0.05) * role.weight_multiplier() * boost
}

// ---------------------------------------------------------------------------
// Global budget allocation (water-filling)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AllocationStrategy {
    /// Equal share of the global budget per agent.
    Uniform,
    /// Weighted log-utility water-filling: maximize sum of w_i ln(1 + b_i)
    /// subject to sum b_i <= B and floor_i <= b_i <= demand_i.
    #[default]
    WaterFilling,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BudgetAllocation {
    pub agent_id: String,
    pub weight: f64,
    /// Tokens the agent's (post-dedup) transcript currently needs.
    pub demand_tokens: u64,
    pub allocated_budget: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BudgetLedger {
    pub strategy: AllocationStrategy,
    pub global_budget: u64,
    pub total_demand: u64,
    pub total_allocated: u64,
    pub allocations: Vec<BudgetAllocation>,
}

struct AllocationInput {
    agent_id: String,
    weight: f64,
    demand: u64,
    floor: u64,
}

/// Solve max sum w_i ln(1 + b_i) s.t. sum b_i <= budget, floor_i <= b_i <= demand_i
/// via bisection on the KKT multiplier: b_i(lambda) = clamp(w_i/lambda - 1).
fn water_fill(inputs: &[AllocationInput], global_budget: u64) -> Vec<u64> {
    let total_demand: u64 = inputs.iter().map(|a| a.demand).sum();
    if total_demand <= global_budget {
        return inputs.iter().map(|a| a.demand).collect();
    }
    let total_floor: u64 = inputs.iter().map(|a| a.floor).sum();
    if total_floor >= global_budget {
        // Even floors exceed the budget; floors win (sufficiency over budget).
        return inputs.iter().map(|a| a.floor).collect();
    }

    let alloc_at = |lambda: f64| -> Vec<f64> {
        inputs
            .iter()
            .map(|a| {
                let raw = a.weight / lambda - 1.0;
                raw.clamp(a.floor as f64, a.demand as f64)
            })
            .collect()
    };

    let mut lo = 1e-12_f64;
    let mut hi = inputs.iter().map(|a| a.weight).fold(1e-12, f64::max) * 2.0 + 1.0;
    for _ in 0..200 {
        let mid = 0.5 * (lo + hi);
        let total: f64 = alloc_at(mid).iter().sum();
        if total > global_budget as f64 {
            lo = mid; // allocating too much -> raise the water line
        } else {
            hi = mid;
        }
        if (hi - lo) / hi < 1e-9 {
            break;
        }
    }
    alloc_at(hi).into_iter().map(|b| b.floor() as u64).collect()
}

/// Allocate the global token budget across agents.
pub fn allocate_budgets(
    agents: &[AgentContext],
    demands: &[u64],
    strategy: AllocationStrategy,
    global_budget: u64,
    min_agent_budget: u64,
) -> BudgetLedger {
    let allocations: Vec<BudgetAllocation> = match strategy {
        AllocationStrategy::Uniform => {
            let share = global_budget / agents.len().max(1) as u64;
            agents
                .iter()
                .zip(demands)
                .map(|(a, &demand)| BudgetAllocation {
                    agent_id: a.agent_id.clone(),
                    weight: effective_weight(a.priority, a.role, a.feedback),
                    demand_tokens: demand,
                    allocated_budget: share,
                })
                .collect()
        }
        AllocationStrategy::WaterFilling => {
            let inputs: Vec<AllocationInput> = agents
                .iter()
                .zip(demands)
                .map(|(a, &demand)| AllocationInput {
                    agent_id: a.agent_id.clone(),
                    weight: effective_weight(a.priority, a.role, a.feedback),
                    demand,
                    floor: min_agent_budget.min(demand),
                })
                .collect();
            let budgets = water_fill(&inputs, global_budget);
            inputs
                .into_iter()
                .zip(budgets)
                .map(|(input, budget)| BudgetAllocation {
                    agent_id: input.agent_id,
                    weight: input.weight,
                    demand_tokens: input.demand,
                    allocated_budget: budget,
                })
                .collect()
        }
    };

    BudgetLedger {
        strategy,
        global_budget,
        total_demand: demands.iter().sum(),
        total_allocated: allocations.iter().map(|a| a.allocated_budget).sum(),
        allocations,
    }
}

// ---------------------------------------------------------------------------
// Cross-agent content-addressed dedup
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DedupReport {
    pub unique_payloads: usize,
    pub duplicates_masked: usize,
    pub chars_saved: u64,
    pub tokens_saved: u64,
    pub shared_refs: Vec<String>,
}

/// FNV-1a 128-bit content hash (no extra deps; 128 bits make accidental
/// collisions across distinct tool payloads negligible for this use).
fn fnv1a_128(bytes: &[u8]) -> u128 {
    const OFFSET: u128 = 0x6c62272e07bb014262b821756295c58d;
    const PRIME: u128 = 0x0000000001000000000000000000013B;
    let mut hash = OFFSET;
    for &b in bytes {
        hash ^= b as u128;
        hash = hash.wrapping_mul(PRIME);
    }
    hash
}

fn dedup_preview(text: &str) -> String {
    let preview: String = text.chars().take(160).collect();
    preview.replace('\n', " ")
}

/// Replace cross-agent duplicate tool results with shared-store references.
///
/// The first sighting of a payload (scanning agents in order, oldest message
/// first) stays intact and becomes the canonical copy; every later sighting in
/// any agent is replaced by a preview plus `ref://{shared_session}/{hash}`.
/// Each agent's most recent `keep_recent` tool results are never masked so the
/// active working set is untouched.
async fn dedup_across_agents(
    agents: &mut [AgentContext],
    store: Arc<dyn ColdStore>,
    shared_session: &str,
    min_chars: usize,
    keep_recent: usize,
) -> Result<DedupReport> {
    let mut seen: HashMap<u128, String> = HashMap::new();
    let mut report = DedupReport::default();

    // First pass: register canonical copies (in protected windows too, so a
    // recent read in agent A still covers an old read in agent B).
    for agent in agents.iter() {
        for msg in &agent.messages {
            if msg.role != "tool" {
                continue;
            }
            let text = extract_text(msg);
            if text.chars().count() < min_chars {
                continue;
            }
            let hash = fnv1a_128(text.as_bytes());
            seen.entry(hash).or_insert_with(|| agent.agent_id.clone());
        }
    }
    report.unique_payloads = seen.len();

    // Second pass: mask non-canonical duplicates outside protected windows.
    let mut stored: HashMap<u128, String> = HashMap::new();
    for agent in agents.iter_mut() {
        let tool_indices: Vec<usize> = agent
            .messages
            .iter()
            .enumerate()
            .filter(|(_, m)| m.role == "tool")
            .map(|(i, _)| i)
            .collect();
        let protected_from = tool_indices.len().saturating_sub(keep_recent);
        let mut canonical_seen_here: HashMap<u128, bool> = HashMap::new();

        for (tool_pos, &msg_idx) in tool_indices.iter().enumerate() {
            let text = extract_text(&agent.messages[msg_idx]);
            if text.chars().count() < min_chars {
                continue;
            }
            let hash = fnv1a_128(text.as_bytes());
            let canonical_owner = seen.get(&hash).cloned().unwrap_or_default();
            let is_first_local = !canonical_seen_here.contains_key(&hash);
            canonical_seen_here.insert(hash, true);

            // The canonical owner keeps its first occurrence; recent results
            // of every agent are protected.
            if (canonical_owner == agent.agent_id && is_first_local)
                || tool_pos >= protected_from
            {
                continue;
            }

            let uri = if let Some(existing) = stored.get(&hash) {
                existing.clone()
            } else {
                let key = format!("{hash:032x}");
                let store_ref = store.put(shared_session, &key, text.as_bytes()).await?;
                stored.insert(hash, store_ref.uri.clone());
                store_ref.uri
            };

            let masked = format!(
                "[deduped tool result: {} chars shared across agents]\npreview: {}\n{}",
                text.chars().count(),
                dedup_preview(&text),
                uri
            );
            report.chars_saved += text.chars().count().saturating_sub(masked.chars().count()) as u64;
            report.tokens_saved +=
                estimate_tokens(&text).saturating_sub(estimate_tokens(&masked));
            report.duplicates_masked += 1;
            if !report.shared_refs.contains(&uri) {
                report.shared_refs.push(uri.clone());
            }
            agent.messages[msg_idx].content = Some(MessageContent::Text(masked));
        }
    }

    Ok(report)
}

// ---------------------------------------------------------------------------
// Consumer-contract handoff compression
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HandoffCompression {
    pub record: FoldRecord,
    pub tokens_before: u64,
    pub tokens_after: u64,
}

fn matches_any_slot(text: &str, slots: &[String]) -> bool {
    let lower = text.to_lowercase();
    slots.iter().any(|s| {
        let slot = s.to_lowercase();
        lower.contains(&slot) || slot.contains(&lower)
    })
}

/// Compress a handoff record against the *consumer's* required slots.
///
/// Status, subgoal, and budget accounting always survive; decisions, open
/// issues, artifacts, and preserved preconditions survive only if they match a
/// slot the receiving agent declared it needs. Empty slots = no contract =
/// record passes through unchanged.
pub fn compress_handoff_for_consumer(
    record: &FoldRecord,
    consumer_slots: &[String],
) -> HandoffCompression {
    let tokens_before = estimate_tokens(
        &serde_json::to_string(record).unwrap_or_else(|_| record.subgoal.clone()),
    );
    if consumer_slots.is_empty() {
        return HandoffCompression {
            record: record.clone(),
            tokens_before,
            tokens_after: tokens_before,
        };
    }
    let compressed = FoldRecord {
        subgoal: record.subgoal.clone(),
        status: record.status,
        artifacts: record
            .artifacts
            .iter()
            .filter(|a| matches_any_slot(&a.path, consumer_slots))
            .cloned()
            .collect(),
        preconditions_preserved: record
            .preconditions_preserved
            .iter()
            .filter(|p| matches_any_slot(p, consumer_slots))
            .cloned()
            .collect(),
        decisions: record
            .decisions
            .iter()
            .filter(|d| matches_any_slot(d, consumer_slots))
            .cloned()
            .collect(),
        open_issues: record
            .open_issues
            .iter()
            .filter(|o| matches_any_slot(o, consumer_slots))
            .cloned()
            .collect(),
        token_budget_used: record.token_budget_used,
    };
    let tokens_after = estimate_tokens(
        &serde_json::to_string(&compressed).unwrap_or_else(|_| compressed.subgoal.clone()),
    );
    HandoffCompression {
        record: compressed,
        tokens_before,
        tokens_after,
    }
}

// ---------------------------------------------------------------------------
// Multi-agent compile
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MultiAgentOptions {
    pub global_token_budget: u64,
    #[serde(default)]
    pub allocation: AllocationStrategy,
    #[serde(default = "default_true")]
    pub cross_agent_dedup: bool,
    /// Tool results shorter than this are never dedup-masked.
    #[serde(default = "default_min_dedup_chars")]
    pub min_dedup_chars: usize,
    /// Each agent's newest N tool results are never dedup-masked (the active
    /// working set). Smaller than per-agent `keep_recent_tool_results`
    /// because duplicates are rehydratable from the shared store.
    #[serde(default = "default_dedup_keep_recent")]
    pub dedup_keep_recent: usize,
    /// Floor allocation per agent (water-filling never starves an agent).
    #[serde(default = "default_min_agent_budget")]
    pub min_agent_budget: u64,
    /// Cold-store session that holds canonical shared payloads.
    #[serde(default = "default_shared_session")]
    pub shared_session_id: String,
    /// Per-agent compile options (session_id and token_budget are overridden).
    #[serde(default)]
    pub base: CompileOptions,
}

fn default_true() -> bool {
    true
}
fn default_min_dedup_chars() -> usize {
    256
}
fn default_dedup_keep_recent() -> usize {
    1
}
fn default_min_agent_budget() -> u64 {
    2048
}
fn default_shared_session() -> String {
    "shared".into()
}

impl Default for MultiAgentOptions {
    fn default() -> Self {
        Self {
            global_token_budget: 128_000,
            allocation: AllocationStrategy::default(),
            cross_agent_dedup: true,
            min_dedup_chars: default_min_dedup_chars(),
            dedup_keep_recent: default_dedup_keep_recent(),
            min_agent_budget: default_min_agent_budget(),
            shared_session_id: default_shared_session(),
            base: CompileOptions::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentCompileReport {
    pub agent_id: String,
    pub role: AgentRole,
    pub allocated_budget: u64,
    pub baseline_tokens: u64,
    pub compiled_tokens: u64,
    pub sufficient: bool,
    pub stats: CompileStats,
    pub messages: Vec<TranscriptMessage>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MultiAgentCompileResult {
    pub ledger: BudgetLedger,
    pub dedup: DedupReport,
    pub agents: Vec<AgentCompileReport>,
    pub total_input_tokens: u64,
    pub total_output_tokens: u64,
    pub total_tokens_saved: u64,
    pub total_reduction_percent: f64,
}

fn sanitize_session(raw: &str) -> String {
    let cleaned: String = raw
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') {
                c
            } else {
                '-'
            }
        })
        .collect();
    if cleaned.is_empty() {
        "agent".into()
    } else {
        cleaned
    }
}

/// Compile every agent's context under a shared global budget.
///
/// Pipeline: cross-agent dedup -> demand estimation -> budget allocation ->
/// per-agent [`compile_context`] against the shared cold store. Per-agent
/// sufficiency never hard-fails the orchestrator round; agents that fail the
/// oracle keep their original messages and report `sufficient: false`.
pub async fn compile_multi_agent(
    agents: &[AgentContext],
    options: MultiAgentOptions,
    store: Arc<dyn ColdStore>,
) -> Result<MultiAgentCompileResult> {
    if agents.is_empty() {
        return Err(CompilerError::InvalidTrace("no agents provided".into()));
    }

    let model = options
        .base
        .token_count_model
        .clone()
        .unwrap_or_else(|| "gpt-4o-mini".into());
    let baseline: Vec<u64> = agents
        .iter()
        .map(|a| estimate_messages_tokens(&a.messages, &model))
        .collect();
    let total_input_tokens: u64 = baseline.iter().sum();

    let mut working: Vec<AgentContext> = agents.to_vec();
    let dedup = if options.cross_agent_dedup {
        dedup_across_agents(
            &mut working,
            store.clone(),
            &sanitize_session(&options.shared_session_id),
            options.min_dedup_chars,
            options.dedup_keep_recent,
        )
        .await?
    } else {
        DedupReport::default()
    };

    let demands: Vec<u64> = working
        .iter()
        .map(|a| estimate_messages_tokens(&a.messages, &model))
        .collect();
    let ledger = allocate_budgets(
        &working,
        &demands,
        options.allocation,
        options.global_token_budget,
        options.min_agent_budget,
    );

    let mut reports = Vec::with_capacity(working.len());
    for (idx, agent) in working.iter().enumerate() {
        let mut opts = options.base.clone();
        opts.session_id = sanitize_session(&agent.agent_id);
        opts.token_budget = ledger.allocations[idx].allocated_budget.max(1);
        // One failing agent must not abort the whole orchestrator round.
        opts.soft_sufficiency = true;

        let compiled = compile_context(&agent.messages, opts, store.clone(), None).await?;
        let compiled_tokens = estimate_messages_tokens(&compiled.messages, &model);
        reports.push(AgentCompileReport {
            agent_id: agent.agent_id.clone(),
            role: agent.role,
            allocated_budget: ledger.allocations[idx].allocated_budget,
            baseline_tokens: baseline[idx],
            compiled_tokens,
            sufficient: compiled.sufficient,
            stats: compiled.stats,
            messages: compiled.messages,
        });
    }

    let total_output_tokens: u64 = reports.iter().map(|r| r.compiled_tokens).sum();
    let total_tokens_saved = total_input_tokens.saturating_sub(total_output_tokens);
    let total_reduction_percent = if total_input_tokens == 0 {
        0.0
    } else {
        (total_tokens_saved as f64 / total_input_tokens as f64) * 100.0
    };

    Ok(MultiAgentCompileResult {
        ledger,
        dedup,
        agents: reports,
        total_input_tokens,
        total_output_tokens,
        total_tokens_saved,
        total_reduction_percent,
    })
}
