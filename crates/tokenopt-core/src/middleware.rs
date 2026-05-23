use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::compile::{compile_context, CompileOptions, CompileResult};
use crate::error::Result;
use crate::ir::TranscriptMessage;
use crate::oracle::SufficiencyOracle;
use crate::store::ColdStore;

/// Hook surface for any orchestrator (LangGraph, Temporal, custom harness, Cursor hooks via HTTP).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MiddlewareContext {
    pub session_id: String,
    pub turn_index: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub orchestrator: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MiddlewareAction {
    /// Pass messages through unchanged.
    Passthrough,
    /// Replace transcript with compiled version.
    Compiled(CompileResult),
}

/// Production middleware: call before each model invocation.
pub struct AgentMiddleware {
    pub options: CompileOptions,
    pub store: Arc<dyn ColdStore>,
    pub oracle: Option<Arc<dyn SufficiencyOracle>>,
}

impl AgentMiddleware {
    pub fn new(options: CompileOptions, store: Arc<dyn ColdStore>) -> Self {
        Self {
            options,
            store,
            oracle: None,
        }
    }

    pub fn with_oracle(mut self, oracle: Arc<dyn SufficiencyOracle>) -> Self {
        self.oracle = Some(oracle);
        self
    }

    pub async fn before_model(
        &self,
        messages: &[TranscriptMessage],
        ctx: &MiddlewareContext,
    ) -> Result<MiddlewareAction> {
        let mut opts = self.options.clone();
        opts.session_id = ctx.session_id.clone();
        let result = compile_context(messages, opts, self.store.clone(), self.oracle.clone()).await?;
        Ok(MiddlewareAction::Compiled(result))
    }

    /// After tool execution: optionally append digest only (orchestrator-specific).
    pub fn tool_result_digest(tool_name: &str, preview: &str, cold_ref: Option<&str>) -> String {
        match cold_ref {
            Some(r) => format!("[{tool_name}] {preview}... (full: {r})"),
            None => format!("[{tool_name}] {preview}"),
        }
    }
}

#[async_trait]
pub trait OrchestratorAdapter: Send + Sync {
    async fn on_before_model(
        &self,
        messages: &[TranscriptMessage],
        ctx: &MiddlewareContext,
    ) -> Result<Vec<TranscriptMessage>>;
}

#[async_trait]
impl OrchestratorAdapter for AgentMiddleware {
    async fn on_before_model(
        &self,
        messages: &[TranscriptMessage],
        ctx: &MiddlewareContext,
    ) -> Result<Vec<TranscriptMessage>> {
        match self.before_model(messages, ctx).await? {
            MiddlewareAction::Passthrough => Ok(messages.to_vec()),
            MiddlewareAction::Compiled(r) => Ok(r.messages),
        }
    }
}
