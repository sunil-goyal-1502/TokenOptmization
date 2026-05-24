//! Trainable fold policy hook (JSON rules + optional score weights).
//!
//! Full FoldGRPO training is out of crate scope; this loads exported policies
//! and derives [`FoldRecord`] injections for the compile pipeline.

use std::collections::HashMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::{CompilerError, Result};
use crate::fold::{FoldRecord, FoldStatus};
use crate::ir::ContextBlock;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FoldPolicy {
    #[serde(default)]
    pub rules: Vec<FoldPolicyRule>,
    /// Optional branch scores from offline RL / GRPO export (`branch_id` → score).
    #[serde(default)]
    pub branch_scores: HashMap<String, f64>,
    #[serde(default = "default_threshold")]
    pub collapse_score_threshold: f64,
}

fn default_threshold() -> f64 {
    0.5
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FoldPolicyRule {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub when_subgoal_contains: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub when_block_contains: Option<String>,
    #[serde(default)]
    pub action: FoldPolicyAction,
    #[serde(default)]
    pub priority: u8,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FoldPolicyAction {
    InjectFold,
    CollapseBranch,
    Skip,
}

impl Default for FoldPolicyAction {
    fn default() -> Self {
        FoldPolicyAction::InjectFold
    }
}

impl FoldPolicy {
    pub fn load_from_path(path: impl AsRef<Path>) -> Result<Self> {
        let text = std::fs::read_to_string(path.as_ref())
            .map_err(|e| CompilerError::Other(format!("fold policy: {e}")))?;
        serde_json::from_str(&text).map_err(CompilerError::Serde)
    }

    pub fn default_builtin() -> Self {
        Self {
            rules: vec![FoldPolicyRule {
                when_block_contains: Some("test failed".into()),
                action: FoldPolicyAction::InjectFold,
                priority: 8,
                when_subgoal_contains: None,
            }],
            branch_scores: HashMap::new(),
            collapse_score_threshold: default_threshold(),
        }
    }

    /// Derive fold records to inject before transforms run.
    pub fn derive_fold_records(&self, blocks: &[ContextBlock]) -> Vec<FoldRecord> {
        let mut out = Vec::new();
        let corpus = blocks
            .iter()
            .map(|b| b.content.as_str())
            .collect::<Vec<_>>()
            .join("\n")
            .to_lowercase();

        let mut rules = self.rules.clone();
        rules.sort_by(|a, b| b.priority.cmp(&a.priority));

        for rule in rules {
            let block_match = rule
                .when_block_contains
                .as_ref()
                .map(|p| corpus.contains(&p.to_lowercase()))
                .unwrap_or(true);
            if !block_match {
                continue;
            }
            match rule.action {
                FoldPolicyAction::InjectFold => {
                    let subgoal = rule
                        .when_subgoal_contains
                        .clone()
                        .or(rule.when_block_contains.clone())
                        .unwrap_or_else(|| "policy fold".into());
                    out.push(FoldRecord {
                        subgoal,
                        status: FoldStatus::Partial,
                        artifacts: vec![],
                        preconditions_preserved: vec![],
                        decisions: vec!["fold_policy: inject".into()],
                        open_issues: vec![],
                        token_budget_used: None,
                    });
                }
                FoldPolicyAction::CollapseBranch | FoldPolicyAction::Skip => {}
            }
        }
        out
    }

    /// Branches below score threshold are candidates for collapse (metadata flag).
    pub fn apply_branch_scores(&self, blocks: &mut [ContextBlock]) {
        for block in blocks.iter_mut() {
            if let Some(ref bid) = block.metadata.branch_id {
                if let Some(score) = self.branch_scores.get(bid) {
                    if *score < self.collapse_score_threshold {
                        block.metadata.consumed = true;
                    }
                }
            }
        }
    }
}

/// Apply branch score weights before other fold transforms.
pub struct FoldPolicyTransform {
    policy: FoldPolicy,
}

impl FoldPolicyTransform {
    pub fn new(policy: FoldPolicy) -> Self {
        Self { policy }
    }
}

#[async_trait::async_trait]
impl crate::transform::Transform for FoldPolicyTransform {
    fn name(&self) -> &'static str {
        "fold_policy"
    }

    async fn apply(
        &self,
        mut blocks: Vec<ContextBlock>,
        _ctx: &crate::transform::TransformContext,
        _store: std::sync::Arc<dyn crate::store::ColdStore>,
    ) -> Result<Vec<ContextBlock>> {
        self.policy.apply_branch_scores(&mut blocks);
        Ok(blocks)
    }
}
