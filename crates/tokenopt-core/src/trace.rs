use serde::{Deserialize, Serialize};

use crate::ir::TranscriptMessage;

/// Standard agent trace format for CLI, HTTP API, and orchestrator adapters.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentTrace {
    #[serde(default)]
    pub trace_id: String,
    #[serde(default)]
    pub session_id: String,
    pub messages: Vec<TranscriptMessage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<TraceMetadata>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TraceMetadata {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub orchestrator: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
}

impl AgentTrace {
    pub fn from_json_slice(bytes: &[u8]) -> crate::error::Result<Self> {
        Ok(serde_json::from_slice(bytes)?)
    }

    pub fn from_json_str(s: &str) -> crate::error::Result<Self> {
        Ok(serde_json::from_str(s)?)
    }
}
