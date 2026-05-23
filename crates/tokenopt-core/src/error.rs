use thiserror::Error;

pub type Result<T> = std::result::Result<T, CompilerError>;

#[derive(Debug, Error)]
pub enum CompilerError {
    #[error("invalid trace: {0}")]
    InvalidTrace(String),

    #[error("store error: {0}")]
    Store(String),

    #[error("sufficiency check failed: {0}")]
    SufficiencyFailed(String),

    #[error("budget exceeded after transforms: need {needed} tokens, budget {budget}")]
    BudgetExceeded { needed: u64, budget: u64 },

    #[error("serialization error: {0}")]
    Serde(#[from] serde_json::Error),

    #[error("{0}")]
    Other(String),
}
