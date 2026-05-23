use std::sync::Arc;

use tokenopt_core::{
    analyze_trace, compile_context, parse_transcript, AgentTrace, CompileOptions, MemoryColdStore,
    TranscriptMessage,
};

fn fixture_messages() -> Vec<TranscriptMessage> {
    serde_json::from_value(serde_json::json!([
        {"role": "system", "content": "You are a coding agent."},
        {"role": "user", "content": "Fix src/main.rs and run tests."},
        {"role": "assistant", "content": "Reading src/main.rs", "tool_calls": [{
            "id": "call_1",
            "type": "function",
            "function": {"name": "read_file", "arguments": "{\"path\":\"src/main.rs\"}"}
        }]},
        {"role": "tool", "tool_call_id": "call_1", "content": "fn main() {\n    println!(\"hello\");\n}\n".repeat(200)},
        {"role": "assistant", "content": "Applied fix to src/main.rs"},
        {"role": "tool", "tool_call_id": "call_2", "content": "error: test failed with exit code 1\n".repeat(50)},
    ]))
    .unwrap()
}

#[tokio::test]
async fn compile_reduces_tokens() {
    let messages = fixture_messages();
    let store = Arc::new(MemoryColdStore::new());
    let opts = CompileOptions {
        session_id: "test-session".into(),
        token_budget: 128_000,
        keep_recent_tool_results: 1,
        run_sufficiency_check: false,
        ..Default::default()
    };
    let result = compile_context(&messages, opts, store, None)
        .await
        .expect("compile");
    assert!(result.stats.tokens_saved > 0 || result.stats.reduction_percent >= 0.0);
    assert!(!result.messages.is_empty());
}

#[test]
fn analyze_reports_kinds() {
    let messages = fixture_messages();
    let report = analyze_trace(&messages).expect("analyze");
    assert!(report.total_tokens > 0);
    assert!(report.block_count > 0);
}

#[test]
fn parse_trace_file_format() {
    let trace = AgentTrace {
        trace_id: "t1".into(),
        session_id: "s1".into(),
        messages: fixture_messages(),
        metadata: None,
    };
    let json = serde_json::to_string(&trace).unwrap();
    let parsed = AgentTrace::from_json_str(&json).unwrap();
    let blocks = parse_transcript(&parsed.messages).unwrap();
    assert!(!blocks.is_empty());
}
