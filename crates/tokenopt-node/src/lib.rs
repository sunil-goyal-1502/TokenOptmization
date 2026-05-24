#![deny(clippy::all)]

use std::sync::Arc;

use napi::bindgen_prelude::*;
use napi_derive::napi;
use tokenopt_core::{
    analyze_trace, compare_trace, compile_context, CompileOptions, MemoryColdStore,
    TranscriptMessage,
};

fn parse_messages(json: &str) -> Result<Vec<TranscriptMessage>> {
    serde_json::from_str(json).map_err(|e| Error::from_reason(e.to_string()))
}

fn parse_options(json: &str) -> Result<CompileOptions> {
    if json.is_empty() || json == "{}" {
        return Ok(CompileOptions::default());
    }
    serde_json::from_str(json).map_err(|e| Error::from_reason(e.to_string()))
}

#[napi]
pub async fn compile_json(messages_json: String, options_json: String) -> Result<String> {
    let messages = parse_messages(&messages_json)?;
    let options = parse_options(&options_json)?;
    let store = Arc::new(MemoryColdStore::new());
    let result = compile_context(&messages, options, store, None)
        .await
        .map_err(|e| Error::from_reason(e.to_string()))?;
    serde_json::to_string(&result).map_err(|e| Error::from_reason(e.to_string()))
}

#[napi]
pub async fn compare_json(messages_json: String, options_json: String) -> Result<String> {
    let messages = parse_messages(&messages_json)?;
    let options = parse_options(&options_json)?;
    let store = Arc::new(MemoryColdStore::new());
    let report = compare_trace(&messages, options, store)
        .await
        .map_err(|e| Error::from_reason(e.to_string()))?;
    serde_json::to_string(&report).map_err(|e| Error::from_reason(e.to_string()))
}

#[napi]
pub fn analyze_json(messages_json: String) -> Result<String> {
    let messages = parse_messages(&messages_json)?;
    let report = analyze_trace(&messages).map_err(|e| Error::from_reason(e.to_string()))?;
    serde_json::to_string(&report).map_err(|e| Error::from_reason(e.to_string()))
}
