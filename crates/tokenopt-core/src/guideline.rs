//! ACON-style guideline bank: pin, summarize, or prefer-mask patterns.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::{CompilerError, Result};
use crate::ir::{BlockKind, ContextBlock};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GuidelineBank {
    #[serde(default)]
    pub rules: Vec<GuidelineRule>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GuidelineRule {
    pub pattern: String,
    pub action: GuidelineAction,
    #[serde(default)]
    pub priority: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GuidelineAction {
    Keep,
    Summarize,
    MaskLast,
}

impl GuidelineBank {
    pub fn load_from_path(path: impl AsRef<Path>) -> Result<Self> {
        let path_ref = path.as_ref();
        if let Some(s) = path_ref.to_str() {
            crate::path_util::validate_config_path(s)?;
        }
        let text = std::fs::read_to_string(path_ref)
            .map_err(|e| CompilerError::Other(format!("guideline bank: {e}")))?;
        serde_json::from_str(&text).map_err(CompilerError::Serde)
    }

    pub fn default_builtin() -> Self {
        Self {
            rules: vec![
                GuidelineRule {
                    pattern: "README".into(),
                    action: GuidelineAction::Keep,
                    priority: 10,
                },
                GuidelineRule {
                    pattern: "Cargo.toml".into(),
                    action: GuidelineAction::Keep,
                    priority: 9,
                },
                GuidelineRule {
                    pattern: "error:".into(),
                    action: GuidelineAction::Summarize,
                    priority: 5,
                },
            ],
        }
    }

    pub fn matching_action(&self, content: &str) -> Option<GuidelineAction> {
        let mut best: Option<(u8, GuidelineAction)> = None;
        for rule in &self.rules {
            if content.contains(&rule.pattern) {
                let replace = best.as_ref().map_or(true, |(p, _)| rule.priority > *p);
                if replace {
                    best = Some((rule.priority, rule.action));
                }
            }
        }
        best.map(|(_, a)| a)
    }
}

/// Apply guideline pins before masking transforms.
pub struct GuidelinePinTransform {
    bank: GuidelineBank,
}

impl GuidelinePinTransform {
    pub fn new(bank: GuidelineBank) -> Self {
        Self { bank }
    }
}

#[async_trait::async_trait]
impl crate::transform::Transform for GuidelinePinTransform {
    fn name(&self) -> &'static str {
        "guideline_pin"
    }

    async fn apply(
        &self,
        mut blocks: Vec<ContextBlock>,
        _ctx: &crate::transform::TransformContext,
        _store: std::sync::Arc<dyn crate::store::ColdStore>,
    ) -> Result<Vec<ContextBlock>> {
        for block in &mut blocks {
            match self.bank.matching_action(&block.content) {
                Some(GuidelineAction::Keep) => {
                    block.metadata.pinned = true;
                    block.metadata.consumed = false;
                }
                Some(GuidelineAction::Summarize) => {
                    if block.kind == BlockKind::ToolResult && block.content.len() > 400 {
                        let first = block.content.lines().take(3).collect::<Vec<_>>().join("\n");
                        block.content = format!("[guideline summarize] {first}\n…");
                    }
                }
                Some(GuidelineAction::MaskLast) => {}
                None => {}
            }
        }
        Ok(blocks)
    }
}
