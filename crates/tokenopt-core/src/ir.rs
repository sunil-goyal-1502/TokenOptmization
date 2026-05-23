use serde::{Deserialize, Serialize};

use crate::error::{CompilerError, Result};

/// OpenAI-style / Anthropic-style chat message for ingestion.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct TranscriptMessage {
    pub role: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<MessageContent>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum MessageContent {
    Text(String),
    Parts(Vec<ContentPart>),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentPart {
    Text { text: String },
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub function: ToolFunction,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolFunction {
    pub name: String,
    pub arguments: String,
}

/// Typed block in the compiler IR.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextBlock {
    pub id: String,
    pub kind: BlockKind,
    pub content: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
    #[serde(default)]
    pub metadata: BlockMetadata,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BlockKind {
    System,
    ToolSchema,
    User,
    Assistant,
    ToolCall,
    ToolResult,
    Reasoning,
    Summary,
    Other,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BlockMetadata {
    pub turn_index: u32,
    pub consumed: bool,
    pub failed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cold_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub referents: Vec<String>,
    /// ACON / guideline bank: never mask or drop this block.
    #[serde(default)]
    pub pinned: bool,
    /// Context-folding branch id for collapse merge.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch_id: Option<String>,
}

/// Parse a transcript into compiler IR blocks.
pub fn parse_transcript(messages: &[TranscriptMessage]) -> Result<Vec<ContextBlock>> {
    let mut blocks = Vec::with_capacity(messages.len() * 2);
    for (turn, msg) in messages.iter().enumerate() {
        let turn_index = turn as u32;
        match msg.role.as_str() {
            "system" => blocks.push(ContextBlock {
                id: format!("sys-{turn_index}"),
                kind: BlockKind::System,
                content: extract_text(msg),
                tool_call_id: None,
                tool_name: None,
                metadata: BlockMetadata {
                    turn_index,
                    ..Default::default()
                },
            }),
            "user" => blocks.push(ContextBlock {
                id: format!("user-{turn_index}"),
                kind: BlockKind::User,
                content: extract_text(msg),
                tool_call_id: None,
                tool_name: None,
                metadata: BlockMetadata {
                    turn_index,
                    referents: extract_referents(&extract_text(msg)),
                    ..Default::default()
                },
            }),
            "assistant" => {
                let text = extract_text(msg);
                if !text.is_empty() {
                    blocks.push(ContextBlock {
                        id: format!("asst-{turn_index}"),
                        kind: BlockKind::Assistant,
                        content: text.clone(),
                        tool_call_id: None,
                        tool_name: None,
                        metadata: BlockMetadata {
                            turn_index,
                            referents: extract_referents(&text),
                            ..Default::default()
                        },
                    });
                }
                if let Some(calls) = &msg.tool_calls {
                    for call in calls {
                        blocks.push(ContextBlock {
                            id: format!("tc-{}", call.id),
                            kind: BlockKind::ToolCall,
                            content: format!(
                                "{}({})",
                                call.function.name, call.function.arguments
                            ),
                            tool_call_id: Some(call.id.clone()),
                            tool_name: Some(call.function.name.clone()),
                            metadata: BlockMetadata {
                                turn_index,
                                ..Default::default()
                            },
                        });
                    }
                }
            }
            "fold" => {
                let content = extract_text(msg);
                let branch_id = msg.name.clone();
                blocks.push(ContextBlock {
                    id: format!("fold-{turn_index}"),
                    kind: BlockKind::Summary,
                    content,
                    tool_call_id: None,
                    tool_name: None,
                    metadata: BlockMetadata {
                        turn_index,
                        branch_id,
                        ..Default::default()
                    },
                });
            }
            "tool" => {
                let content = extract_text(msg);
                blocks.push(ContextBlock {
                    id: format!(
                        "tr-{}",
                        msg.tool_call_id
                            .clone()
                            .unwrap_or_else(|| turn_index.to_string())
                    ),
                    kind: BlockKind::ToolResult,
                    content,
                    tool_call_id: msg.tool_call_id.clone(),
                    tool_name: msg.name.clone(),
                    metadata: BlockMetadata {
                        turn_index,
                        failed: false,
                        ..Default::default()
                    },
                });
            }
            _ => blocks.push(ContextBlock {
                id: format!("other-{turn_index}"),
                kind: BlockKind::Other,
                content: extract_text(msg),
                tool_call_id: None,
                tool_name: None,
                metadata: BlockMetadata {
                    turn_index,
                    ..Default::default()
                },
            }),
        }
    }
    mark_failed_tool_results(&mut blocks);
    Ok(blocks)
}

fn mark_failed_tool_results(blocks: &mut [ContextBlock]) {
    for block in blocks.iter_mut() {
        if block.kind == BlockKind::ToolResult {
            let lower = block.content.to_lowercase();
            block.metadata.failed = lower.contains("error")
                || lower.contains("exception")
                || lower.contains("exit code")
                || lower.starts_with("failed");
        }
    }
}

pub fn extract_text(msg: &TranscriptMessage) -> String {
    match &msg.content {
        Some(MessageContent::Text(t)) => t.clone(),
        Some(MessageContent::Parts(parts)) => parts
            .iter()
            .filter_map(|p| match p {
                ContentPart::Text { text } => Some(text.as_str()),
                ContentPart::Unknown => None,
            })
            .collect::<Vec<_>>()
            .join("\n"),
        None => String::new(),
    }
}

/// Extract file paths, symbols, and identifiers likely needed for task continuity.
pub fn extract_referents(text: &str) -> Vec<String> {
    let mut refs = Vec::new();
    let path_re =
        regex::Regex::new(r"([\w./-]+\.(?:rs|tsx?|js|py|go|java|json|ya?ml|md|toml))")
            .expect("path regex");
    for cap in path_re.captures_iter(text) {
        if let Some(m) = cap.get(1) {
            refs.push(m.as_str().to_string());
        }
    }
    let branch_re = regex::Regex::new(r"(?i)branch[:\s]+[`']?([\w./-]+)").expect("branch regex");
    for cap in branch_re.captures_iter(text) {
        if let Some(m) = cap.get(1) {
            refs.push(format!("branch:{}", m.as_str()));
        }
    }
    refs.sort();
    refs.dedup();
    refs
}

pub fn blocks_to_messages(blocks: &[ContextBlock]) -> Result<Vec<TranscriptMessage>> {
    let mut messages = Vec::new();
    for block in blocks {
        let role = match block.kind {
            BlockKind::System | BlockKind::ToolSchema => "system",
            BlockKind::User => "user",
            BlockKind::Assistant | BlockKind::ToolCall | BlockKind::Reasoning | BlockKind::Summary => {
                "assistant"
            }
            BlockKind::ToolResult => "tool",
            BlockKind::Other => "user",
        };
        if block.kind == BlockKind::ToolResult {
            messages.push(TranscriptMessage {
                role: role.to_string(),
                content: Some(MessageContent::Text(block.content.clone())),
                name: block.tool_name.clone(),
                tool_calls: None,
                tool_call_id: block.tool_call_id.clone(),
            });
        } else {
            messages.push(TranscriptMessage {
                role: role.to_string(),
                content: Some(MessageContent::Text(block.content.clone())),
                name: None,
                tool_calls: None,
                tool_call_id: None,
            });
        }
    }
    Ok(messages)
}

pub fn validate_blocks(blocks: &[ContextBlock]) -> Result<()> {
    if blocks.is_empty() {
        return Err(CompilerError::InvalidTrace(
            "transcript produced zero blocks".into(),
        ));
    }
    Ok(())
}
