use std::sync::Arc;

use tokenopt_core::{
    bench_compile_latency, compare_trace, generate_agent_trace, AgentLoopSimConfig,
    CompileOptions, MemoryColdStore,
};

#[tokio::test]
async fn compare_shows_savings_on_long_trace() {
    let messages = generate_agent_trace(&AgentLoopSimConfig {
        turns: 12,
        tool_payload_bytes: 6_000,
        ..Default::default()
    });
    let store = Arc::new(MemoryColdStore::new());
    let report = compare_trace(
        &messages,
        CompileOptions {
            session_id: "compare-test".into(),
            keep_recent_tool_results: 2,
            run_sufficiency_check: false,
            ..Default::default()
        },
        store,
    )
    .await
    .expect("compare");

    assert!(report.tokens_saved > 0);
    assert!(report.reduction_percent > 5.0);
    assert!(report.compile_duration_ms < 5_000);
}

#[tokio::test]
async fn latency_bench_completes() {
    let messages = generate_agent_trace(&AgentLoopSimConfig {
        turns: 8,
        tool_payload_bytes: 4_000,
        ..Default::default()
    });
    let store = Arc::new(MemoryColdStore::new());
    let report = bench_compile_latency(
        &messages,
        CompileOptions {
            session_id: "lat".into(),
            run_sufficiency_check: false,
            ..Default::default()
        },
        store,
        20,
    )
    .await
    .expect("bench");
    assert_eq!(report.iterations, 20);
    assert!(report.p95_ms < 10_000);
}
