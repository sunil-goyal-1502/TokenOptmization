//! Safe path helpers for optional config file loads.

use std::path::Path;

use crate::error::{CompilerError, Result};

/// Reject obvious path traversal in user-supplied config paths (HTTP `CompileOptions`).
pub fn validate_config_path(path: &str) -> Result<()> {
    if path.is_empty() {
        return Err(CompilerError::InvalidTrace("empty config path".into()));
    }
    if path.contains("..") {
        return Err(CompilerError::InvalidTrace(format!(
            "path traversal not allowed: {path}"
        )));
    }
    Ok(())
}

/// Resolve path and ensure it stays under `base` (best-effort; use for server-side policy files).
pub fn resolve_under_base(base: &Path, path: &str) -> Result<std::path::PathBuf> {
    validate_config_path(path)?;
    let joined = base.join(path);
    let canonical_base = base
        .canonicalize()
        .unwrap_or_else(|_| base.to_path_buf());
    let canonical_joined = joined
        .canonicalize()
        .map_err(|e| CompilerError::InvalidTrace(format!("config path not found: {e}")))?;
    if !canonical_joined.starts_with(&canonical_base) {
        return Err(CompilerError::InvalidTrace(format!(
            "config path escapes base directory: {path}"
        )));
    }
    Ok(canonical_joined)
}
