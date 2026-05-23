//! Context-folding runtime: merge branch transcripts into fold records.

use crate::error::Result;
use crate::fold::{FoldRecord, FoldStatus};
use crate::ir::{parse_transcript, TranscriptMessage};

/// Collapse messages tagged with `role: fold` and `name: branch_id` into a single handoff record.
pub fn collapse_branch_messages(messages: &[TranscriptMessage]) -> Result<(Vec<TranscriptMessage>, Vec<FoldRecord>)> {
    let blocks = parse_transcript(messages)?;
    let mut fold_msgs = Vec::new();
    let mut other = Vec::new();
    let mut records = Vec::new();

    for msg in messages {
        if msg.role == "fold" {
            fold_msgs.push(msg.clone());
        } else {
            other.push(msg.clone());
        }
    }

    if !fold_msgs.is_empty() {
        let subgoal = fold_msgs
            .first()
            .map(|m| crate::ir::extract_text(m))
            .unwrap_or_else(|| "branch handoff".into());
        records.push(FoldRecord {
            subgoal: subgoal.chars().take(200).collect(),
            status: FoldStatus::Success,
            artifacts: vec![],
            preconditions_preserved: vec![],
            decisions: fold_msgs
                .iter()
                .map(|m| crate::ir::extract_text(m).chars().take(120).collect())
                .collect(),
            open_issues: vec![],
            token_budget_used: None,
        });
    }

    let _ = blocks;
    Ok((other, records))
}
