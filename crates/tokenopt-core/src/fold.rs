use serde::{Deserialize, Serialize};

/// Structured handoff summary for sub-agents and orchestrator workers.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FoldRecord {
    pub subgoal: String,
    pub status: FoldStatus,
    #[serde(default)]
    pub artifacts: Vec<FoldArtifact>,
    #[serde(default)]
    pub preconditions_preserved: Vec<String>,
    #[serde(default)]
    pub decisions: Vec<String>,
    #[serde(default)]
    pub open_issues: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_budget_used: Option<u64>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FoldStatus {
    Success,
    Partial,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FoldArtifact {
    #[serde(rename = "type")]
    pub artifact_type: String,
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hash: Option<String>,
}

impl FoldRecord {
    pub fn to_context_block(&self) -> crate::ir::ContextBlock {
        let content = serde_json::to_string_pretty(self).unwrap_or_else(|_| self.subgoal.clone());
        crate::ir::ContextBlock {
            id: format!("fold-{}", uuid::Uuid::new_v4()),
            kind: crate::ir::BlockKind::Summary,
            content,
            tool_call_id: None,
            tool_name: None,
            metadata: Default::default(),
        }
    }
}
