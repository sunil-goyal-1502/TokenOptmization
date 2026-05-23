//! Expand cold-store references back into message content.

use std::sync::Arc;

use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::error::{CompilerError, Result};
use crate::ir::{extract_text, MessageContent, TranscriptMessage};
use crate::metrics::record_rehydrate;
use crate::store::{ColdStore, StoreRef};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RehydrateOptions {
    /// Only expand these refs; if empty, expand all `ref://` found in messages.
    #[serde(default)]
    pub refs: Vec<String>,
    /// Cap bytes loaded per reference (default 64KB).
    #[serde(default = "default_max_bytes")]
    pub max_bytes_per_ref: usize,
}

fn default_max_bytes() -> usize {
    65_536
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RehydrateResult {
    pub messages: Vec<TranscriptMessage>,
    pub refs_expanded: usize,
    pub bytes_loaded: u64,
}

pub async fn rehydrate_messages(
    messages: &[TranscriptMessage],
    store: Arc<dyn ColdStore>,
    options: RehydrateOptions,
) -> Result<RehydrateResult> {
    record_rehydrate();
    let ref_re = Regex::new(r"ref://[\w./-]+").expect("ref regex");
    let mut out = messages.to_vec();
    let mut refs_expanded = 0usize;
    let mut bytes_loaded = 0u64;

    let target_refs: Vec<String> = if options.refs.is_empty() {
        let mut found = Vec::new();
        for msg in messages {
            for cap in ref_re.find_iter(&extract_text(msg)) {
                found.push(cap.as_str().to_string());
            }
        }
        found.sort();
        found.dedup();
        found
    } else {
        options.refs.clone()
    };

    for uri in target_refs {
        let reference = StoreRef {
            uri: uri.clone(),
            byte_length: 0,
            content_hash: None,
        };
        let payload = match store.get(&reference).await {
            Ok(p) => p,
            Err(_) => continue,
        };
        let take = payload.len().min(options.max_bytes_per_ref);
        bytes_loaded += take as u64;
        let text = String::from_utf8_lossy(&payload[..take]);
        let appendix = format!("\n\n[rehydrated {uri}]\n{text}\n");

        let mut replaced = false;
        for msg in &mut out {
            let content = extract_text(msg);
            if content.contains(&uri) {
                if let Some(MessageContent::Text(t)) = &mut msg.content {
                    if !t.contains("[rehydrated") {
                        t.push_str(&appendix);
                        replaced = true;
                    }
                }
            }
        }
        if replaced {
            refs_expanded += 1;
        }
    }

    Ok(RehydrateResult {
        messages: out,
        refs_expanded,
        bytes_loaded,
    })
}

/// Parse `ref://session/key` from masked tool result text.
pub fn extract_refs_from_text(text: &str) -> Vec<String> {
    let ref_re = Regex::new(r"ref://[\w./-]+").expect("ref regex");
    ref_re
        .find_iter(text)
        .map(|m| m.as_str().to_string())
        .collect()
}

pub fn parse_store_ref(uri: &str) -> Result<StoreRef> {
    if !uri.starts_with("ref://") {
        return Err(CompilerError::Store(format!("invalid ref uri: {uri}")));
    }
    Ok(StoreRef {
        uri: uri.to_string(),
        byte_length: 0,
        content_hash: None,
    })
}
