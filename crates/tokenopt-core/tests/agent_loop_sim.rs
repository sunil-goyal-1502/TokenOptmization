use tokenopt_core::{simulate_agent_loop, AgentLoopSimConfig};

#[tokio::test]
async fn agent_loop_shows_material_token_reduction() {
    let report = simulate_agent_loop(AgentLoopSimConfig {
        turns: 15,
        tool_payload_bytes: 12_000,
        keep_recent_tool_results: 2,
        enable_consumed_masking: true,
        run_sufficiency_check: false,
        ..Default::default()
    })
    .await
    .expect("simulate");

    assert!(
        report.final_reduction_percent > 10.0,
        "expected >10% reduction on final turn, got {:.1}%",
        report.final_reduction_percent
    );
    assert!(
        report.final_baseline_tokens > report.final_compiled_tokens,
        "compiled should be smaller than baseline"
    );
}

#[tokio::test]
async fn masking_off_reduces_savings() {
    let with_mask = simulate_agent_loop(AgentLoopSimConfig {
        turns: 12,
        tool_payload_bytes: 10_000,
        enable_consumed_masking: true,
        ..Default::default()
    })
    .await
    .unwrap();

    let without_mask = simulate_agent_loop(AgentLoopSimConfig {
        turns: 12,
        tool_payload_bytes: 10_000,
        enable_consumed_masking: false,
        ..Default::default()
    })
    .await
    .unwrap();

    assert!(
        with_mask.final_reduction_percent >= without_mask.final_reduction_percent,
        "masking should not reduce savings: with={:.1}% without={:.1}%",
        with_mask.final_reduction_percent,
        without_mask.final_reduction_percent
    );
}
