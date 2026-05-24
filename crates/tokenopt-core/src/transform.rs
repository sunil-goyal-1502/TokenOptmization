use std::sync::Arc;

use async_trait::async_trait;

use crate::error::Result;
use crate::ir::{BlockKind, ContextBlock};
use crate::store::ColdStore;
use crate::tokens::estimate_tokens;

pub use crate::bacm::BacmTransform;
pub use crate::extended_transforms::{
    AgentOmitTransform, CachePackTransform, ExternalCompressTransform, FoldCollapseTransform,
    FoldInjectTransform, LlmSummarizeTransform, MemoryPruneTransform, RuleSummarizeTransform,
};
pub use crate::fold_policy::FoldPolicyTransform;
pub use crate::guideline::GuidelinePinTransform;

#[derive(Debug, Clone)]
pub struct TransformContext {
    pub session_id: String,
    pub token_budget: u64,
    pub keep_recent_tool_results: usize,
    pub error_compaction_after_turns: u32,
    pub enable_consumed_masking: bool,
    /// When over budget, keep this many trailing blocks before budget_trim.
    pub rolling_tail_blocks: usize,
}

#[async_trait]
pub trait Transform: Send + Sync {
    fn name(&self) -> &'static str;
    async fn apply(
        &self,
        blocks: Vec<ContextBlock>,
        ctx: &TransformContext,
        store: Arc<dyn ColdStore>,
    ) -> Result<Vec<ContextBlock>>;
}

pub struct TransformPipeline {
    transforms: Vec<Arc<dyn Transform>>,
}

impl TransformPipeline {
    pub fn new(transforms: Vec<Arc<dyn Transform>>) -> Self {
        Self { transforms }
    }

    pub fn transform_names(&self) -> Vec<&'static str> {
        self.transforms.iter().map(|t| t.name()).collect()
    }

    pub async fn run(
        &self,
        blocks: Vec<ContextBlock>,
        ctx: &TransformContext,
        store: Arc<dyn ColdStore>,
    ) -> Result<Vec<ContextBlock>> {
        let mut current = blocks;
        for t in &self.transforms {
            current = t.apply(current, ctx, store.clone()).await?;
        }
        Ok(current)
    }
}

pub struct TransformPipelineBuilder {
    transforms: Vec<Arc<dyn Transform>>,
}

impl Default for TransformPipelineBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl TransformPipelineBuilder {
    pub fn new() -> Self {
        Self {
            transforms: Vec::new(),
        }
    }

    pub fn with_defaults() -> Self {
        Self::new()
            .then(ReferentialKeepTransform)
            .then(ErrorCompactionTransform)
            .then(ConsumedResultMaskTransform)
            .then(RollingWindowTransform)
            .then(BudgetTrimTransform)
    }

    /// Full research pipeline (use [`crate::pipeline::build_pipeline`] for option-driven build).
    pub fn with_research_defaults() -> Self {
        Self::with_defaults()
            .then(AgentOmitTransform)
            .then(RuleSummarizeTransform::new(12))
            .then(CachePackTransform)
    }

    pub fn then<T: Transform + 'static>(mut self, transform: T) -> Self {
        self.transforms.push(Arc::new(transform));
        self
    }

    pub fn build(self) -> TransformPipeline {
        TransformPipeline::new(self.transforms)
    }
}

/// Mark referents from recent user/assistant blocks for keep-set.
pub struct ReferentialKeepTransform;

#[async_trait]
impl Transform for ReferentialKeepTransform {
    fn name(&self) -> &'static str {
        "referential_keep"
    }

    async fn apply(
        &self,
        mut blocks: Vec<ContextBlock>,
        _ctx: &TransformContext,
        _store: Arc<dyn ColdStore>,
    ) -> Result<Vec<ContextBlock>> {
        let mut keep: std::collections::HashSet<String> = std::collections::HashSet::new();
        for block in blocks.iter().rev().take(8) {
            for r in &block.metadata.referents {
                keep.insert(r.clone());
            }
        }
        for block in &mut blocks {
            if block.kind == BlockKind::ToolResult {
                for r in &block.metadata.referents {
                    if keep.contains(r) {
                        block.metadata.consumed = false;
                    }
                }
            }
        }
        Ok(blocks)
    }
}

/// Compact failed tool results after N turns.
pub struct ErrorCompactionTransform;

#[async_trait]
impl Transform for ErrorCompactionTransform {
    fn name(&self) -> &'static str {
        "error_compaction"
    }

    async fn apply(
        &self,
        mut blocks: Vec<ContextBlock>,
        ctx: &TransformContext,
        store: Arc<dyn ColdStore>,
    ) -> Result<Vec<ContextBlock>> {
        let max_turn = blocks
            .iter()
            .map(|b| b.metadata.turn_index)
            .max()
            .unwrap_or(0);
        for block in &mut blocks {
            if block.kind != BlockKind::ToolResult || !block.metadata.failed {
                continue;
            }
            if max_turn.saturating_sub(block.metadata.turn_index) < ctx.error_compaction_after_turns {
                continue;
            }
            if block.metadata.cold_ref.is_some() {
                continue;
            }
            let key = format!("err-{}", block.id);
            let payload = block.content.as_bytes();
            let reference = store.put(&ctx.session_id, &key, payload).await?;
            let first_line = block.content.lines().next().unwrap_or("error");
            block.content = format!(
                "[compacted error] {first_line}\nref: {} ({} bytes)",
                reference.uri, reference.byte_length
            );
            block.metadata.cold_ref = Some(reference.uri);
        }
        Ok(blocks)
    }
}

/// Replace consumed tool results with cold-store references.
pub struct ConsumedResultMaskTransform;

#[async_trait]
impl Transform for ConsumedResultMaskTransform {
    fn name(&self) -> &'static str {
        "consumed_result_mask"
    }

    async fn apply(
        &self,
        mut blocks: Vec<ContextBlock>,
        ctx: &TransformContext,
        store: Arc<dyn ColdStore>,
    ) -> Result<Vec<ContextBlock>> {
        if !ctx.enable_consumed_masking {
            return Ok(blocks);
        }

        let tool_result_indices: Vec<usize> = blocks
            .iter()
            .enumerate()
            .filter(|(_, b)| b.kind == BlockKind::ToolResult)
            .map(|(i, _)| i)
            .collect();

        let keep_count = ctx.keep_recent_tool_results;
        for (idx, block_idx) in tool_result_indices.iter().enumerate() {
            let is_recent = idx + keep_count >= tool_result_indices.len();
            if is_recent {
                continue;
            }
            let block = &mut blocks[*block_idx];
            if block.metadata.cold_ref.is_some() || block.metadata.pinned {
                continue;
            }
            let key = format!("tool-{}", block.id);
            let reference = store
                .put(&ctx.session_id, &key, block.content.as_bytes())
                .await?;
            let preview: String = block.content.chars().take(120).collect();
            block.content = format!(
                "[masked tool result] preview: {preview}...\nref: {} ({} bytes)",
                reference.uri, reference.byte_length
            );
            block.metadata.cold_ref = Some(reference.uri);
            block.metadata.consumed = true;
        }
        Ok(blocks)
    }
}

/// Drop oldest non-system blocks when over budget (respects referential keep).
pub struct RollingWindowTransform;

#[async_trait]
impl Transform for RollingWindowTransform {
    fn name(&self) -> &'static str {
        "rolling_window"
    }

    async fn apply(
        &self,
        blocks: Vec<ContextBlock>,
        ctx: &TransformContext,
        _store: Arc<dyn ColdStore>,
    ) -> Result<Vec<ContextBlock>> {
        let total: u64 = blocks.iter().map(|b| estimate_tokens(&b.content)).sum();
        if total <= ctx.token_budget {
            return Ok(blocks);
        }

        let tail = ctx.rolling_tail_blocks.max(4);
        if blocks.len() <= tail + 2 {
            return Ok(blocks);
        }

        let protected = [
            BlockKind::System,
            BlockKind::ToolSchema,
            BlockKind::Summary,
        ];
        let mut prefix_end = 0usize;
        for (i, b) in blocks.iter().enumerate() {
            if protected.contains(&b.kind) {
                prefix_end = i + 1;
            } else {
                break;
            }
        }

        let tail_start = blocks.len().saturating_sub(tail);
        if tail_start <= prefix_end {
            return Ok(blocks);
        }

        let mut out = Vec::new();
        out.extend(blocks[..prefix_end].iter().cloned());
        out.push(ContextBlock {
            id: format!("rolled-{}", uuid::Uuid::new_v4()),
            kind: BlockKind::Summary,
            content: format!(
                "[rolling_window] omitted {} middle blocks ({} tokens) — use cold-store refs to recover",
                tail_start - prefix_end,
                total.saturating_sub(ctx.token_budget)
            ),
            tool_call_id: None,
            tool_name: None,
            metadata: Default::default(),
        });
        out.extend(blocks[tail_start..].iter().cloned());
        Ok(out)
    }
}

/// Trim from the middle: keep system + recent tail until under budget.
pub struct BudgetTrimTransform;

#[async_trait]
impl Transform for BudgetTrimTransform {
    fn name(&self) -> &'static str {
        "budget_trim"
    }

    async fn apply(
        &self,
        blocks: Vec<ContextBlock>,
        ctx: &TransformContext,
        _store: Arc<dyn ColdStore>,
    ) -> Result<Vec<ContextBlock>> {
        let mut tokens: u64 = blocks.iter().map(|b| estimate_tokens(&b.content)).sum();
        if tokens <= ctx.token_budget {
            return Ok(blocks);
        }

        let mut result = blocks.clone();
        let protected_kinds = [
            BlockKind::System,
            BlockKind::ToolSchema,
            BlockKind::Summary,
        ];

        let mut i = 0;
        while tokens > ctx.token_budget && i < result.len() {
            if protected_kinds.contains(&result[i].kind) {
                i += 1;
                continue;
            }
            if result[i].metadata.cold_ref.is_some() {
                i += 1;
                continue;
            }
            let removed = estimate_tokens(&result[i].content);
            result.remove(i);
            tokens = tokens.saturating_sub(removed);
        }
        Ok(result)
    }
}
