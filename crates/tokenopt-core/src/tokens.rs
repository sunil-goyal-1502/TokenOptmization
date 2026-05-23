use serde::{Deserialize, Serialize};

/// How token counts were produced.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TokenCountMethod {
    Heuristic,
    Tiktoken,
}

pub fn token_count_method() -> TokenCountMethod {
    #[cfg(feature = "accurate-tokens")]
    {
        TokenCountMethod::Tiktoken
    }
    #[cfg(not(feature = "accurate-tokens"))]
    {
        TokenCountMethod::Heuristic
    }
}

/// Estimate token count for budgeting.
pub fn estimate_tokens(text: &str) -> u64 {
    estimate_tokens_for_model(text, "gpt-4o-mini")
}

/// Model-aware estimate when `accurate-tokens` feature is enabled.
pub fn estimate_tokens_for_model(text: &str, model: &str) -> u64 {
    if text.is_empty() {
        return 0;
    }
    #[cfg(feature = "accurate-tokens")]
    {
        if let Some(n) = tiktoken_count(text, model) {
            return n;
        }
    }
    #[cfg(not(feature = "accurate-tokens"))]
    let _ = model;
    let chars = text.chars().count() as u64;
    (chars / 4).max(1)
}

#[cfg(feature = "accurate-tokens")]
fn tiktoken_count(text: &str, model: &str) -> Option<u64> {
    use tiktoken_rs::CoreBPE;
    use tiktoken_rs::cl100k_base;
    use tiktoken_rs::o200k_base;

    let bpe: CoreBPE = if model.contains("gpt-4o") || model.contains("gpt-4") {
        o200k_base().ok()?
    } else {
        cl100k_base().ok()?
    };
    Some(bpe.encode_ordinary(text).len() as u64)
}

pub fn estimate_blocks_tokens(blocks: &[crate::ir::ContextBlock]) -> u64 {
    blocks
        .iter()
        .map(|b| estimate_tokens(&b.content))
        .sum()
}

pub fn estimate_messages_tokens(
    messages: &[crate::ir::TranscriptMessage],
    model: &str,
) -> u64 {
    let joined = messages
        .iter()
        .map(crate::ir::extract_text)
        .collect::<Vec<_>>()
        .join("\n");
    estimate_tokens_for_model(&joined, model)
}
