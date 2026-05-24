//! BACM-style critical-block retention: keep errors, goals, and recent decisions.

use std::sync::Arc;

use async_trait::async_trait;

use crate::error::Result;
use crate::ir::{BlockKind, ContextBlock};
use crate::store::ColdStore;
use crate::transform::{Transform, TransformContext};

/// Budget-Aware Critical Memory: drop non-critical tool results under pressure.
pub struct BacmTransform;

#[async_trait]
impl Transform for BacmTransform {
    fn name(&self) -> &'static str {
        "bacm"
    }

    async fn apply(
        &self,
        blocks: Vec<ContextBlock>,
        ctx: &TransformContext,
        _store: Arc<dyn ColdStore>,
    ) -> Result<Vec<ContextBlock>> {
        let total: u64 = blocks
            .iter()
            .map(|b| crate::tokens::estimate_tokens(&b.content))
            .sum();
        if total <= ctx.token_budget {
            return Ok(blocks);
        }

        let max_turn = blocks
            .iter()
            .map(|b| b.metadata.turn_index)
            .max()
            .unwrap_or(0);
        let recent_cutoff = max_turn.saturating_sub(6);

        Ok(blocks
            .into_iter()
            .filter(|b| {
                if b.metadata.pinned {
                    return true;
                }
                match b.kind {
                    BlockKind::System | BlockKind::ToolSchema | BlockKind::Summary => true,
                    BlockKind::User => true,
                    BlockKind::Assistant => b.metadata.turn_index >= recent_cutoff,
                    BlockKind::ToolResult => {
                        b.metadata.failed
                            || b.metadata.turn_index >= recent_cutoff
                            || b.content.to_lowercase().contains("error")
                    }
                    _ => b.metadata.turn_index >= recent_cutoff,
                }
            })
            .collect())
    }
}
