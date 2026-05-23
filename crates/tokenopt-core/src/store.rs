use std::path::{Path, PathBuf};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::fs;

use crate::error::{CompilerError, Result};

/// Stable reference into cold storage (`ref://session/...`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StoreRef {
    pub uri: String,
    pub byte_length: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_hash: Option<String>,
}

#[async_trait]
pub trait ColdStore: Send + Sync {
    async fn put(&self, session_id: &str, key: &str, payload: &[u8]) -> Result<StoreRef>;
    async fn get(&self, reference: &StoreRef) -> Result<Vec<u8>>;
    async fn get_text(&self, reference: &StoreRef) -> Result<String> {
        let bytes = self.get(reference).await?;
        String::from_utf8(bytes).map_err(|e| CompilerError::Store(e.to_string()))
    }
}

pub struct MemoryColdStore {
    inner: tokio::sync::RwLock<std::collections::HashMap<String, Vec<u8>>>,
}

impl MemoryColdStore {
    pub fn new() -> Self {
        Self {
            inner: tokio::sync::RwLock::new(std::collections::HashMap::new()),
        }
    }

    fn key(session_id: &str, key: &str) -> String {
        format!("ref://{session_id}/{key}")
    }
}

impl Default for MemoryColdStore {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ColdStore for MemoryColdStore {
    async fn put(&self, session_id: &str, key: &str, payload: &[u8]) -> Result<StoreRef> {
        let uri = Self::key(session_id, key);
        let len = payload.len() as u64;
        self.inner
            .write()
            .await
            .insert(uri.clone(), payload.to_vec());
        Ok(StoreRef {
            uri,
            byte_length: len,
            content_hash: None,
        })
    }

    async fn get(&self, reference: &StoreRef) -> Result<Vec<u8>> {
        self.inner
            .read()
            .await
            .get(&reference.uri)
            .cloned()
            .ok_or_else(|| CompilerError::Store(format!("missing ref {}", reference.uri)))
    }
}

pub struct FileColdStore {
    root: PathBuf,
}

impl FileColdStore {
    pub fn new(root: impl AsRef<Path>) -> Self {
        Self {
            root: root.as_ref().to_path_buf(),
        }
    }

    fn path_for(&self, session_id: &str, key: &str) -> PathBuf {
        self.root.join(session_id).join(format!("{key}.bin"))
    }

    pub fn uri_for(session_id: &str, key: &str) -> String {
        format!("ref://{session_id}/{key}")
    }
}

#[async_trait]
impl ColdStore for FileColdStore {
    async fn put(&self, session_id: &str, key: &str, payload: &[u8]) -> Result<StoreRef> {
        let path = self.path_for(session_id, key);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).await.map_err(|e| {
                CompilerError::Store(format!("create dir {}: {e}", parent.display()))
            })?;
        }
        fs::write(&path, payload).await.map_err(|e| {
            CompilerError::Store(format!("write {}: {e}", path.display()))
        })?;
        Ok(StoreRef {
            uri: Self::uri_for(session_id, key),
            byte_length: payload.len() as u64,
            content_hash: None,
        })
    }

    async fn get(&self, reference: &StoreRef) -> Result<Vec<u8>> {
        let parts: Vec<&str> = reference.uri.strip_prefix("ref://").unwrap_or("").split('/').collect();
        if parts.len() < 2 {
            return Err(CompilerError::Store(format!(
                "invalid ref uri {}",
                reference.uri
            )));
        }
        let session_id = parts[0];
        let key = parts[1..].join("/");
        let path = self.path_for(session_id, &key);
        fs::read(&path)
            .await
            .map_err(|e| CompilerError::Store(format!("read {}: {e}", path.display())))
    }
}
