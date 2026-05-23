//! Research backlog transforms: summarize, fold, cache pack, omit, compress hook.

use std::sync::Arc;

use async_trait::async_trait;

use crate::error::Result;
use crate::fold::FoldRecord;
use crate::ir::{BlockKind, ContextBlock};
use crate::store::ColdStore;
use crate::transform::{Transform, TransformContext};

/// Inject structured fold handoff blocks after system prefix.
pub struct FoldInjectTransform {
    records: Vec<FoldRecord>,
}

impl FoldInjectTransform {
    pub fn new(records: Vec<FoldRecord>) -> Self {
        Self { records }
    }
}

#[async_trait]
impl Transform for FoldInjectTransform {
    fn name(&self) -> &'static str {
        "fold_inject"
    }

    async fn apply(
        &self,
        blocks: Vec<ContextBlock>,
        _ctx: &TransformContext,
        _store: Arc<dyn ColdStore>,
    ) -> Result<Vec<ContextBlock>> {
        if self.records.is_empty() {
            return Ok(blocks);
        }
        let mut prefix = 0usize;
        while prefix < blocks.len()
            && matches!(
                blocks[prefix].kind,
                BlockKind::System | BlockKind::ToolSchema
            )
        {
            prefix += 1;
        }
        let mut out = blocks[..prefix].to_vec();
        for record in &self.records {
            out.push(record.to_context_block());
        }
        out.extend(blocks[prefix..].iter().cloned());
        Ok(out)
    }
}

/// Collapse duplicate branch summaries — keep latest per `branch_id`.
pub struct FoldCollapseTransform;

#[async_trait]
impl Transform for FoldCollapseTransform {
    fn name(&self) -> &'static str {
        "fold_collapse"
    }

    async fn apply(
        &self,
        blocks: Vec<ContextBlock>,
        _ctx: &TransformContext,
        _store: Arc<dyn ColdStore>,
    ) -> Result<Vec<ContextBlock>> {
        use std::collections::HashMap;
        let mut latest_fold: HashMap<String, usize> = HashMap::new();
        for (i, b) in blocks.iter().enumerate() {
            if let Some(ref bid) = b.metadata.branch_id {
                latest_fold.insert(bid.clone(), i);
            }
        }
        if latest_fold.is_empty() {
            return Ok(blocks);
        }
        let mut out = Vec::new();
        for (i, b) in blocks.into_iter().enumerate() {
            if let Some(ref bid) = b.metadata.branch_id {
                if latest_fold.get(bid) == Some(&i) {
                    out.push(b);
                }
                continue;
            }
            out.push(b);
        }
        Ok(out)
    }
}

/// Rule-based summarization of old middle blocks (no LLM required).
pub struct RuleSummarizeTransform {
    keep_recent: usize,
}

impl RuleSummarizeTransform {
    pub fn new(keep_recent: usize) -> Self {
        Self { keep_recent }
    }
}

#[async_trait]
impl Transform for RuleSummarizeTransform {
    fn name(&self) -> &'static str {
        "rule_summarize"
    }

    async fn apply(
        &self,
        blocks: Vec<ContextBlock>,
        ctx: &TransformContext,
        store: Arc<dyn ColdStore>,
    ) -> Result<Vec<ContextBlock>> {
        let protected = [
            BlockKind::System,
            BlockKind::ToolSchema,
            BlockKind::Summary,
        ];
        let tail = self.keep_recent.max(ctx.keep_recent_tool_results + 4);
        if blocks.len() <= tail + 3 {
            return Ok(blocks);
        }

        let mut prefix_end = 0usize;
        for (i, b) in blocks.iter().enumerate() {
            if protected.contains(&b.kind) {
                prefix_end = i + 1;
            } else {
                break;
            }
        }
        let tail_start = blocks.len().saturating_sub(tail);
        if tail_start <= prefix_end + 1 {
            return Ok(blocks);
        }

        let mut summary_parts = Vec::new();
        for b in &blocks[prefix_end..tail_start] {
            if b.metadata.pinned {
                continue;
            }
            let line = b.content.lines().next().unwrap_or("").chars().take(80).collect::<String>();
            summary_parts.push(format!("{:?}: {line}", b.kind));
        }
        if summary_parts.is_empty() {
            return Ok(blocks);
        }

        let middle_count = tail_start - prefix_end;
        let summary_body = summary_parts.join("\n");
        let key = format!("summary-{}", uuid::Uuid::new_v4());
        let reference = store
            .put(
                &ctx.session_id,
                &key,
                summary_body.as_bytes(),
            )
            .await?;

        let mut out = blocks[..prefix_end].to_vec();
        out.push(ContextBlock {
            id: format!("sum-{}", uuid::Uuid::new_v4()),
            kind: BlockKind::Summary,
            content: format!(
                "[rule_summarize] collapsed {middle_count} blocks\n{}\nref: {}",
                summary_parts
                    .iter()
                    .take(8)
                    .cloned()
                    .collect::<Vec<_>>()
                    .join("\n"),
                reference.uri
            ),
            tool_call_id: None,
            tool_name: None,
            metadata: Default::default(),
        });
        out.extend(blocks[tail_start..].iter().cloned());
        Ok(out)
    }
}

/// Prompt-cache-friendly ordering: system → tool schema → summaries → chronological tail.
pub struct CachePackTransform;

#[async_trait]
impl Transform for CachePackTransform {
    fn name(&self) -> &'static str {
        "cache_packer"
    }

    async fn apply(
        &self,
        blocks: Vec<ContextBlock>,
        _ctx: &TransformContext,
        _store: Arc<dyn ColdStore>,
    ) -> Result<Vec<ContextBlock>> {
        let mut system = Vec::new();
        let mut tool_schema = Vec::new();
        let mut summaries = Vec::new();
        let mut rest = Vec::new();

        for b in blocks {
            match b.kind {
                BlockKind::System => system.push(b),
                BlockKind::ToolSchema => tool_schema.push(b),
                BlockKind::Summary => summaries.push(b),
                _ => rest.push(b),
            }
        }
        system.extend(tool_schema);
        system.extend(summaries);
        system.extend(rest);
        Ok(system)
    }
}

/// Agent-Omit: drop redundant short assistant filler before tool results.
pub struct AgentOmitTransform;

#[async_trait]
impl Transform for AgentOmitTransform {
    fn name(&self) -> &'static str {
        "agent_omit"
    }

    async fn apply(
        &self,
        blocks: Vec<ContextBlock>,
        _ctx: &TransformContext,
        _store: Arc<dyn ColdStore>,
    ) -> Result<Vec<ContextBlock>> {
        let filler = [
            "ok", "okay", "sure", "i will", "let me", "calling", "using",
        ];
        let mut out = Vec::with_capacity(blocks.len());
        for (i, b) in blocks.iter().enumerate() {
            if b.kind != BlockKind::Assistant || b.metadata.pinned {
                out.push(b.clone());
                continue;
            }
            let next_is_tool = blocks
                .get(i + 1)
                .map(|n| n.kind == BlockKind::ToolResult)
                .unwrap_or(false);
            let short = b.content.chars().count() < 120;
            let low_value = filler.iter().any(|f| b.content.to_lowercase().contains(f));
            if next_is_tool && short && low_value {
                continue;
            }
            out.push(b.clone());
        }
        Ok(out)
    }
}

/// MEM1-style: drop very old tool results not pinned and not in recent window.
pub struct MemoryPruneTransform;

#[async_trait]
impl Transform for MemoryPruneTransform {
    fn name(&self) -> &'static str {
        "memory_prune"
    }

    async fn apply(
        &self,
        blocks: Vec<ContextBlock>,
        ctx: &TransformContext,
        _store: Arc<dyn ColdStore>,
    ) -> Result<Vec<ContextBlock>> {
        let max_turn = blocks
            .iter()
            .map(|b| b.metadata.turn_index)
            .max()
            .unwrap_or(0);
        let keep_turns = (ctx.keep_recent_tool_results as u32).saturating_add(4);
        Ok(blocks
            .into_iter()
            .filter(|b| {
                if b.kind != BlockKind::ToolResult {
                    return true;
                }
                if b.metadata.pinned || b.metadata.cold_ref.is_some() {
                    return true;
                }
                max_turn.saturating_sub(b.metadata.turn_index) <= keep_turns
            })
            .collect())
    }
}

/// Hook for external compressors (e.g. LLMLingua-2 HTTP sidecar).
pub struct ExternalCompressTransform {
    url: Option<String>,
}

impl ExternalCompressTransform {
    pub fn new(url: Option<String>) -> Self {
        Self { url }
    }
}

#[async_trait]
impl Transform for ExternalCompressTransform {
    fn name(&self) -> &'static str {
        "external_compress"
    }

    async fn apply(
        &self,
        blocks: Vec<ContextBlock>,
        _ctx: &TransformContext,
        _store: Arc<dyn ColdStore>,
    ) -> Result<Vec<ContextBlock>> {
        let Some(url) = &self.url else {
            return Ok(blocks);
        };
        #[cfg(feature = "llm-http")]
        {
            return compress_via_http(url, blocks).await;
        }
        #[cfg(not(feature = "llm-http"))]
        {
            let _ = url;
            Ok(blocks)
        }
    }
}

#[cfg(feature = "llm-http")]
async fn compress_via_http(url: &str, blocks: Vec<ContextBlock>) -> Result<Vec<ContextBlock>> {
    let payload = serde_json::json!({
        "blocks": blocks.iter().map(|b| {
            serde_json::json!({
                "id": b.id,
                "kind": format!("{:?}", b.kind),
                "content": b.content,
            })
        }).collect::<Vec<_>>(),
    });
    let client = reqwest::Client::new();
    let resp = client
        .post(url)
        .json(&payload)
        .timeout(std::time::Duration::from_secs(30))
        .send()
        .await
        .map_err(|e| CompilerError::Other(format!("external_compress: {e}")))?;
    if !resp.status().is_success() {
        return Ok(blocks);
    }
    let body: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| CompilerError::Other(format!("external_compress json: {e}")))?;
    let Some(arr) = body.get("blocks").and_then(|v| v.as_array()) else {
        return Ok(blocks);
    };
    let mut out = blocks;
    for item in arr {
        let id = item.get("id").and_then(|v| v.as_str());
        let content = item.get("content").and_then(|v| v.as_str());
        if let (Some(id), Some(content)) = (id, content) {
            if let Some(b) = out.iter_mut().find(|b| b.id == id) {
                b.content = content.to_string();
            }
        }
    }
    Ok(out)
}
