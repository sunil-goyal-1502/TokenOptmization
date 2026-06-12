//! Integration tests for MACO: multi-agent budget allocation, cross-agent
//! dedup, consumer-contract handoffs, and the orchestrator simulation.

use std::sync::Arc;

use tokenopt_core::{
    allocate_budgets, compile_multi_agent, compress_handoff_for_consumer, effective_weight,
    extract_refs_from_text, extract_text, generate_orchestrator_scenario, rehydrate_messages,
    simulate_orchestrator_loop, AgentContext, AgentFeedback, AgentRole, AllocationStrategy,
    CompileOptions, FoldArtifact, FoldRecord, FoldStatus, MemoryColdStore, MultiAgentOptions,
    OrchestratorSimConfig, RehydrateOptions,
};

fn worker(agent_id: &str, priority: f64, demand_filler_chars: usize) -> AgentContext {
    AgentContext {
        agent_id: agent_id.into(),
        role: AgentRole::Worker,
        priority,
        feedback: AgentFeedback::default(),
        messages: vec![tokenopt_core::TranscriptMessage {
            role: "user".into(),
            content: Some(tokenopt_core::MessageContent::Text(
                "y".repeat(demand_filler_chars),
            )),
            name: None,
            tool_calls: None,
            tool_call_id: None,
        }],
    }
}

#[test]
fn water_filling_respects_global_budget_and_weights() {
    let agents = vec![
        AgentContext {
            role: AgentRole::Supervisor,
            ..worker("supervisor", 1.0, 4)
        },
        worker("worker-a", 1.0, 4),
        worker("worker-b", 1.0, 4),
    ];
    // Every agent demands 10k but only 18k is available.
    let demands = vec![10_000, 10_000, 10_000];
    let ledger = allocate_budgets(
        &agents,
        &demands,
        AllocationStrategy::WaterFilling,
        18_000,
        1_000,
    );

    assert!(
        ledger.total_allocated <= 18_000,
        "allocation {} exceeds global budget",
        ledger.total_allocated
    );
    // The water level should leave little budget unused (rounding only).
    assert!(ledger.total_allocated >= 17_900);
    let supervisor = &ledger.allocations[0];
    let plain_worker = &ledger.allocations[1];
    assert!(
        supervisor.allocated_budget > plain_worker.allocated_budget,
        "supervisor ({}) should out-rank equal-demand worker ({})",
        supervisor.allocated_budget,
        plain_worker.allocated_budget
    );
    // Symmetric workers get symmetric budgets.
    assert_eq!(
        ledger.allocations[1].allocated_budget,
        ledger.allocations[2].allocated_budget
    );
}

#[test]
fn water_filling_grants_full_demand_when_budget_is_loose() {
    let agents = vec![worker("a", 1.0, 4), worker("b", 1.0, 4)];
    let demands = vec![3_000, 5_000];
    let ledger = allocate_budgets(
        &agents,
        &demands,
        AllocationStrategy::WaterFilling,
        100_000,
        1_000,
    );
    assert_eq!(ledger.allocations[0].allocated_budget, 3_000);
    assert_eq!(ledger.allocations[1].allocated_budget, 5_000);
}

#[test]
fn regret_feedback_raises_allocation() {
    let starved = AgentFeedback {
        sufficiency_failures: 2,
        rehydration_requests: 3,
    };
    assert!(
        effective_weight(1.0, AgentRole::Worker, starved)
            > effective_weight(1.0, AgentRole::Worker, AgentFeedback::default())
    );

    let mut agent_a = worker("starved", 1.0, 4);
    agent_a.feedback = starved;
    let agent_b = worker("content", 1.0, 4);
    let ledger = allocate_budgets(
        &[agent_a, agent_b],
        &[10_000, 10_000],
        AllocationStrategy::WaterFilling,
        12_000,
        1_000,
    );
    assert!(
        ledger.allocations[0].allocated_budget > ledger.allocations[1].allocated_budget,
        "agent with regret signals should win budget: {} vs {}",
        ledger.allocations[0].allocated_budget,
        ledger.allocations[1].allocated_budget
    );
}

#[tokio::test]
async fn cross_agent_dedup_masks_duplicates_and_rehydrates() {
    let config = OrchestratorSimConfig {
        workers: 4,
        rounds: 6,
        tool_payload_bytes: 4_000,
        shared_read_fraction: 1.0,
        global_token_budget: 256_000,
        keep_recent_tool_results: 3,
    };
    let agents = generate_orchestrator_scenario(&config);
    let store = Arc::new(MemoryColdStore::new());

    let result = compile_multi_agent(
        &agents,
        MultiAgentOptions {
            global_token_budget: config.global_token_budget,
            base: CompileOptions {
                run_sufficiency_check: false,
                ..Default::default()
            },
            ..Default::default()
        },
        store.clone(),
    )
    .await
    .expect("multi-agent compile");

    assert!(
        result.dedup.duplicates_masked > 0,
        "expected cross-agent duplicates to be masked"
    );
    assert!(result.dedup.tokens_saved > 0);
    assert!(result
        .dedup
        .shared_refs
        .iter()
        .all(|r| r.starts_with("ref://shared/")));

    // A masked duplicate must be recoverable from the shared store.
    let shared_ref = result.dedup.shared_refs[0].clone();
    let probe = vec![tokenopt_core::TranscriptMessage {
        role: "tool".into(),
        content: Some(tokenopt_core::MessageContent::Text(format!(
            "[deduped tool result]\n{shared_ref}"
        ))),
        name: None,
        tool_calls: None,
        tool_call_id: Some("probe".into()),
    }];
    let rehydrated = rehydrate_messages(
        &probe,
        store,
        RehydrateOptions {
            refs: vec![shared_ref],
            ..Default::default()
        },
    )
    .await
    .expect("rehydrate shared ref");
    assert_eq!(rehydrated.refs_expanded, 1);
    assert!(rehydrated.bytes_loaded > 0);
}

#[tokio::test]
async fn dedup_never_masks_each_agents_newest_tool_result() {
    let config = OrchestratorSimConfig {
        workers: 3,
        rounds: 4,
        tool_payload_bytes: 4_000,
        shared_read_fraction: 1.0,
        global_token_budget: 256_000,
        keep_recent_tool_results: 2,
    };
    let agents = generate_orchestrator_scenario(&config);
    let result = compile_multi_agent(
        &agents,
        MultiAgentOptions {
            global_token_budget: config.global_token_budget,
            base: CompileOptions {
                run_sufficiency_check: false,
                ..Default::default()
            },
            ..Default::default()
        },
        Arc::new(MemoryColdStore::new()),
    )
    .await
    .expect("multi-agent compile");

    for agent in &result.agents {
        if agent.role == AgentRole::Supervisor {
            continue;
        }
        let last_tool_text = agent
            .messages
            .iter()
            .rev()
            .find(|m| m.role == "tool")
            .map(extract_text)
            .unwrap_or_default();
        assert!(
            !last_tool_text.contains("[deduped tool result"),
            "{}'s newest tool result must stay live",
            agent.agent_id
        );
    }
}

#[tokio::test]
async fn maco_beats_independent_compile_and_baseline() {
    let report = simulate_orchestrator_loop(OrchestratorSimConfig {
        workers: 4,
        rounds: 10,
        tool_payload_bytes: 6_000,
        shared_read_fraction: 0.5,
        global_token_budget: 64_000,
        keep_recent_tool_results: 2,
    })
    .await
    .expect("orchestrator sim");

    assert!(
        report.cumulative_maco_tokens < report.cumulative_independent_tokens,
        "MACO ({}) must beat independent per-agent compile ({})",
        report.cumulative_maco_tokens,
        report.cumulative_independent_tokens
    );
    assert!(
        report.cumulative_independent_tokens < report.cumulative_baseline_tokens,
        "independent compile must beat no compile"
    );
    assert!(
        report.final_maco_vs_baseline_percent > 50.0,
        "expected >50% end-state savings, got {:.1}%",
        report.final_maco_vs_baseline_percent
    );
    assert!(
        report.final_maco_vs_independent_percent >= 5.0,
        "expected >=5% savings over independent compile, got {:.1}%",
        report.final_maco_vs_independent_percent
    );
}

#[test]
fn handoff_compression_respects_consumer_contract() {
    let record = FoldRecord {
        subgoal: "Implement auth middleware".into(),
        status: FoldStatus::Success,
        artifacts: vec![
            FoldArtifact {
                artifact_type: "file".into(),
                path: "src/auth/middleware.rs".into(),
                hash: None,
            },
            FoldArtifact {
                artifact_type: "file".into(),
                path: "docs/diagram.svg".into(),
                hash: None,
            },
        ],
        preconditions_preserved: vec!["database migrations applied".into()],
        decisions: vec![
            "Used JWT for auth tokens".into(),
            "Renamed CI workflow file".into(),
        ],
        open_issues: vec!["auth token expiry untested".into()],
        token_budget_used: Some(900),
    };

    // No contract: pass-through.
    let unchanged = compress_handoff_for_consumer(&record, &[]);
    assert_eq!(unchanged.tokens_before, unchanged.tokens_after);
    assert_eq!(unchanged.record.decisions.len(), 2);

    // Consumer only cares about auth: CI/docs details dropped.
    let compressed =
        compress_handoff_for_consumer(&record, &["auth".into(), "src/auth/middleware.rs".into()]);
    assert!(compressed.tokens_after < compressed.tokens_before);
    assert_eq!(compressed.record.artifacts.len(), 1);
    assert_eq!(compressed.record.decisions.len(), 1);
    assert_eq!(compressed.record.open_issues.len(), 1);
    assert!(compressed.record.decisions[0].contains("JWT"));
}

#[tokio::test]
async fn shared_refs_inside_messages_are_extractable() {
    let config = OrchestratorSimConfig {
        workers: 3,
        rounds: 5,
        tool_payload_bytes: 4_000,
        shared_read_fraction: 1.0,
        global_token_budget: 256_000,
        keep_recent_tool_results: 2,
    };
    let agents = generate_orchestrator_scenario(&config);
    let result = compile_multi_agent(
        &agents,
        MultiAgentOptions {
            global_token_budget: config.global_token_budget,
            base: CompileOptions {
                run_sufficiency_check: false,
                ..Default::default()
            },
            ..Default::default()
        },
        Arc::new(MemoryColdStore::new()),
    )
    .await
    .expect("multi-agent compile");

    let mut found_shared_ref = false;
    for agent in &result.agents {
        for msg in &agent.messages {
            for r in extract_refs_from_text(&extract_text(msg)) {
                if r.starts_with("ref://shared/") {
                    found_shared_ref = true;
                }
            }
        }
    }
    assert!(
        found_shared_ref,
        "compiled output should carry shared refs for rehydration"
    );
}
