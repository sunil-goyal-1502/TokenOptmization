/// Estimate token count for budgeting (conservative chars/4 heuristic).
/// Production deployments can wrap a tiktoken binding in orchestrator-specific adapters.
pub fn estimate_tokens(text: &str) -> u64 {
    if text.is_empty() {
        return 0;
    }
    let chars = text.chars().count() as u64;
    (chars / 4).max(1)
}

pub fn estimate_blocks_tokens(blocks: &[crate::ir::ContextBlock]) -> u64 {
    blocks
        .iter()
        .map(|b| estimate_tokens(&b.content))
        .sum()
}
