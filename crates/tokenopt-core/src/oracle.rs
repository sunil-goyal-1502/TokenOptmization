use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::ir::{BlockKind, ContextBlock};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Subgoal {
    pub id: String,
    pub description: String,
    #[serde(default)]
    pub required_slots: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SufficiencyResult {
    pub sufficient: bool,
    #[serde(default)]
    pub missing_slots: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

#[async_trait]
pub trait SufficiencyOracle: Send + Sync {
    async fn check(
        &self,
        compressed: &[ContextBlock],
        subgoals: &[Subgoal],
    ) -> Result<SufficiencyResult>;
}

/// Rule-based oracle: verifies required slots appear in compressed context.
pub struct RuleOracle {
    pub strict: bool,
}

impl Default for RuleOracle {
    fn default() -> Self {
        Self { strict: true }
    }
}

#[async_trait]
impl SufficiencyOracle for RuleOracle {
    async fn check(
        &self,
        compressed: &[ContextBlock],
        subgoals: &[Subgoal],
    ) -> Result<SufficiencyResult> {
        let corpus = compressed
            .iter()
            .map(|b| b.content.as_str())
            .collect::<Vec<_>>()
            .join("\n")
            .to_lowercase();

        let mut missing = Vec::new();
        for sg in subgoals {
            for slot in &sg.required_slots {
                if !corpus.contains(&slot.to_lowercase()) {
                    missing.push(format!("{}:{}", sg.id, slot));
                }
            }
        }

        let sufficient = missing.is_empty() || !self.strict;
        Ok(SufficiencyResult {
            sufficient,
            missing_slots: missing.clone(),
            message: if sufficient {
                None
            } else {
                Some(format!("missing required slots: {}", missing.join(", ")))
            },
        })
    }
}

pub struct CompositeOracle {
    pub oracles: Vec<std::sync::Arc<dyn SufficiencyOracle>>,
}

#[async_trait]
impl SufficiencyOracle for CompositeOracle {
    async fn check(
        &self,
        compressed: &[ContextBlock],
        subgoals: &[Subgoal],
    ) -> Result<SufficiencyResult> {
        let mut all_missing = Vec::new();
        for oracle in &self.oracles {
            let res = oracle.check(compressed, subgoals).await?;
            if !res.sufficient {
                all_missing.extend(res.missing_slots);
            }
        }
        all_missing.sort();
        all_missing.dedup();
        Ok(SufficiencyResult {
            sufficient: all_missing.is_empty(),
            missing_slots: all_missing.clone(),
            message: if all_missing.is_empty() {
                None
            } else {
                Some(format!("composite check failed: {}", all_missing.join(", ")))
            },
        })
    }
}

/// Infer subgoals from plan/summary blocks and referents in recent assistant turns.
pub fn infer_subgoals_from_blocks(blocks: &[ContextBlock]) -> Vec<Subgoal> {
    let mut slots = Vec::new();
    for block in blocks.iter().rev().take(12) {
        if matches!(
            block.kind,
            BlockKind::Assistant | BlockKind::Summary | BlockKind::User
        ) {
            slots.extend(
                block
                    .metadata
                    .referents
                    .iter()
                    .cloned()
                    .map(|r| r.to_lowercase()),
            );
        }
    }
    slots.sort();
    slots.dedup();
    if slots.is_empty() {
        return vec![Subgoal {
            id: "default".into(),
            description: "complete agent task".into(),
            required_slots: vec![],
        }];
    }
    vec![Subgoal {
        id: "inferred".into(),
        description: "preserve referents from recent turns".into(),
        required_slots: slots,
    }]
}
