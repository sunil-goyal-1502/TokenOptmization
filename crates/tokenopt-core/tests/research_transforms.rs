use std::sync::Arc;

use tokenopt_core::{
    compile_context, extract_text, CompileOptions, FoldRecord, FoldStatus, MemoryColdStore,
    TransformToggles,
};

#[tokio::test]
async fn guideline_pin_keeps_readme_content() {
    let messages: Vec<tokenopt_core::TranscriptMessage> = serde_json::from_value(serde_json::json!([
        {"role": "user", "content": "read README"},
        {"role": "tool", "tool_call_id": "c1", "content": format!("README.md content\n{}", "x".repeat(300))}
    ]))
    .unwrap();
    let store = Arc::new(MemoryColdStore::new());
    let result = compile_context(
        &messages,
        CompileOptions {
            session_id: "g".into(),
            keep_recent_tool_results: 0,
            run_sufficiency_check: false,
            transforms: TransformToggles {
                consumed_result_mask: true,
                guideline_bank: true,
                summarization: false,
                agent_omit: false,
                ..Default::default()
            },
            ..Default::default()
        },
        store,
        None,
    )
    .await
    .expect("compile");
    let pinned = result.blocks.iter().any(|b| b.metadata.pinned);
    assert!(pinned, "README tool result should be pinned");
}

#[tokio::test]
async fn fold_inject_adds_summary_blocks() {
    let messages = vec![
        serde_json::from_value(serde_json::json!({"role": "user", "content": "go"})).unwrap(),
    ];
    let store = Arc::new(MemoryColdStore::new());
    let result = compile_context(
        &messages,
        CompileOptions {
            session_id: "f".into(),
            run_sufficiency_check: false,
            fold_records: vec![FoldRecord {
                subgoal: "explore lib".into(),
                status: FoldStatus::Success,
                artifacts: vec![],
                preconditions_preserved: vec![],
                decisions: vec!["read lib.rs".into()],
                open_issues: vec![],
                token_budget_used: None,
            }],
            transforms: TransformToggles {
                consumed_result_mask: false,
                summarization: false,
                ..Default::default()
            },
            ..Default::default()
        },
        store,
        None,
    )
    .await
    .expect("compile");
    assert!(result
        .messages
        .iter()
        .any(|m| extract_text(m).contains("explore lib")));
}

#[tokio::test]
async fn routing_hint_present_when_enabled() {
    let messages = tokenopt_core::generate_agent_trace(&tokenopt_core::AgentLoopSimConfig {
        turns: 10,
        tool_payload_bytes: 4000,
        ..Default::default()
    });
    let store = Arc::new(MemoryColdStore::new());
    let result = compile_context(
        &messages,
        CompileOptions {
            session_id: "r".into(),
            run_sufficiency_check: false,
            routing_hints: true,
            turn_index: 20,
            ..Default::default()
        },
        store,
        None,
    )
    .await
    .expect("compile");
    assert!(result.routing_hint.is_some());
}
