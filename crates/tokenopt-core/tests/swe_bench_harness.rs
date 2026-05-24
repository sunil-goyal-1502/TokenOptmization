use std::path::PathBuf;
use std::sync::Arc;

use tokenopt_core::{
    compare_trace, AgentTrace, CompileOptions, MemoryColdStore, TransformToggles,
};

fn manifest_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

#[tokio::test]
async fn swe_bench_lite_tasks_meet_reduction_targets() {
    let root = manifest_dir().join("../..");
    let tasks: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(root.join("fixtures/swe_bench/lite_tasks.json")).unwrap())
            .unwrap();
    let store = Arc::new(MemoryColdStore::new());

    for task in tasks["tasks"].as_array().unwrap() {
        let trace_path = root.join(task["trace_file"].as_str().unwrap());
        let min_red = task["min_reduction_percent"].as_f64().unwrap_or(0.0);
        let bytes = std::fs::read(&trace_path).unwrap();
        let trace = AgentTrace::from_json_slice(&bytes).unwrap();
        let report = compare_trace(
            &trace.messages,
            CompileOptions {
                session_id: "swe".into(),
                keep_recent_tool_results: 2,
                run_sufficiency_check: false,
                transforms: TransformToggles {
                    guideline_bank: true,
                    summarization: true,
                    fold_policy: true,
                    ..Default::default()
                },
                guideline_bank_path: Some(root.join("fixtures/guidelines/default.json").display().to_string()),
                fold_policy_path: Some(root.join("fixtures/fold_policies/default.json").display().to_string()),
                ..Default::default()
            },
            store.clone(),
        )
        .await
        .unwrap_or_else(|e| panic!("{}: {e}", task["instance_id"]));
        assert!(
            report.reduction_percent >= min_red,
            "{} reduction {:.1}% < min {:.1}%",
            task["instance_id"],
            report.reduction_percent,
            min_red
        );
    }
}
