//! In-process Python bindings for TokenOpt (no HTTP hop).

use std::sync::Arc;

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use tokenopt_core::{
    analyze_trace, compare_trace, compile_context, CompileOptions, MemoryColdStore,
    TranscriptMessage,
};

fn parse_messages(json: &str) -> PyResult<Vec<TranscriptMessage>> {
    serde_json::from_str(json).map_err(|e| PyValueError::new_err(e.to_string()))
}

fn parse_options(json: &str) -> PyResult<CompileOptions> {
    if json.is_empty() || json == "{}" {
        return Ok(CompileOptions::default());
    }
    serde_json::from_str(json).map_err(|e| PyValueError::new_err(e.to_string()))
}

#[pyfunction]
fn compile_json(messages_json: &str, options_json: &str) -> PyResult<String> {
    let messages = parse_messages(messages_json)?;
    let options = parse_options(options_json)?;
    let rt = tokio::runtime::Runtime::new().map_err(|e| PyValueError::new_err(e.to_string()))?;
    let store = Arc::new(MemoryColdStore::new());
    let result = rt
        .block_on(compile_context(&messages, options, store, None))
        .map_err(|e| PyValueError::new_err(e.to_string()))?;
    serde_json::to_string(&result).map_err(|e| PyValueError::new_err(e.to_string()))
}

#[pyfunction]
fn compare_json(messages_json: &str, options_json: &str) -> PyResult<String> {
    let messages = parse_messages(messages_json)?;
    let options = parse_options(options_json)?;
    let rt = tokio::runtime::Runtime::new().map_err(|e| PyValueError::new_err(e.to_string()))?;
    let store = Arc::new(MemoryColdStore::new());
    let report = rt
        .block_on(compare_trace(&messages, options, store))
        .map_err(|e| PyValueError::new_err(e.to_string()))?;
    serde_json::to_string(&report).map_err(|e| PyValueError::new_err(e.to_string()))
}

#[pyfunction]
fn analyze_json(messages_json: &str) -> PyResult<String> {
    let messages = parse_messages(messages_json)?;
    let report = analyze_trace(&messages).map_err(|e| PyValueError::new_err(e.to_string()))?;
    serde_json::to_string(&report).map_err(|e| PyValueError::new_err(e.to_string()))
}

#[pymodule]
#[pyo3(name = "_native")]
fn _native(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(compile_json, m)?)?;
    m.add_function(wrap_pyfunction!(compare_json, m)?)?;
    m.add_function(wrap_pyfunction!(analyze_json, m)?)?;
    Ok(())
}
