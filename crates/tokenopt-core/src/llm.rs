//! Optional HTTP integrations: LLM summarization and sufficiency oracle.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::error::Result;
#[cfg(feature = "llm-http")]
use crate::error::CompilerError;
use crate::ir::ContextBlock;
use crate::oracle::{Subgoal, SufficiencyOracle, SufficiencyResult};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LlmConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_openai_base")]
    pub api_base: String,
    #[serde(default)]
    pub api_key_env: String,
    #[serde(default = "default_model")]
    pub model: String,
}

fn default_openai_base() -> String {
    "https://api.openai.com/v1".into()
}

fn default_model() -> String {
    "gpt-4o-mini".into()
}

/// Summarize text via chat completion API (`llm-http` feature).
pub async fn llm_summarize_text(config: &LlmConfig, text: &str, max_out_chars: usize) -> Result<String> {
    if !config.enabled {
        return Ok(truncate_chars(text, max_out_chars));
    }
    #[cfg(feature = "llm-http")]
    {
        let summary = llm_chat(config, &format!(
            "Summarize the following agent transcript excerpt for future turns. \
             Preserve file paths, errors, and decisions. Max {} chars.\n\n{}",
            max_out_chars,
            &text[..text.len().min(24_000)]
        ))
        .await?;
        return Ok(truncate_chars(&summary, max_out_chars));
    }
    #[cfg(not(feature = "llm-http"))]
    {
        let _ = config;
        Ok(truncate_chars(text, max_out_chars))
    }
}

fn truncate_chars(s: &str, max: usize) -> String {
    s.chars().take(max).collect()
}

#[cfg(feature = "llm-http")]
pub async fn llm_chat(config: &LlmConfig, user_prompt: &str) -> Result<String> {
    let key_var = if config.api_key_env.is_empty() {
        "OPENAI_API_KEY"
    } else {
        &config.api_key_env
    };
    let api_key = std::env::var(key_var)
        .map_err(|_| CompilerError::Other(format!("missing env {key_var}")))?;
    let url = format!("{}/chat/completions", config.api_base.trim_end_matches('/'));
    let body = serde_json::json!({
        "model": config.model,
        "messages": [
            {"role": "system", "content": "You compress agent context faithfully."},
            {"role": "user", "content": user_prompt}
        ],
        "temperature": 0.1,
    });
    let client = reqwest::Client::new();
    let resp = client
        .post(&url)
        .bearer_auth(api_key)
        .json(&body)
        .timeout(std::time::Duration::from_secs(60))
        .send()
        .await
        .map_err(|e| CompilerError::Other(format!("llm chat: {e}")))?;
    let text = resp
        .text()
        .await
        .map_err(|e| CompilerError::Other(format!("llm chat body: {e}")))?;
    parse_chat_content(&text).ok_or_else(|| CompilerError::Other("llm chat: empty response".into()))
}

#[cfg(feature = "llm-http")]
fn parse_chat_content(response: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(response).ok()?;
    v.get("choices")?
        .as_array()?
        .first()?
        .get("message")?
        .get("content")?
        .as_str()
        .map(String::from)
}

/// LLM sufficiency check (optional; requires `llm-http` + API key).
pub struct LlmSufficiencyOracle {
    config: LlmConfig,
}

impl LlmSufficiencyOracle {
    pub fn new(config: LlmConfig) -> Self {
        Self { config }
    }
}

#[async_trait]
impl SufficiencyOracle for LlmSufficiencyOracle {
    async fn check(
        &self,
        compressed: &[ContextBlock],
        subgoals: &[Subgoal],
    ) -> Result<SufficiencyResult> {
        if !self.config.enabled {
            return Ok(SufficiencyResult {
                sufficient: true,
                missing_slots: vec![],
                message: None,
            });
        }
        #[cfg(feature = "llm-http")]
        {
            return llm_check_impl(&self.config, compressed, subgoals).await;
        }
        #[cfg(not(feature = "llm-http"))]
        {
            let _ = (compressed, subgoals);
            Ok(SufficiencyResult {
                sufficient: true,
                missing_slots: vec![],
                message: Some("llm-http feature not enabled".into()),
            })
        }
    }
}

#[cfg(feature = "llm-http")]
async fn llm_check_impl(
    config: &LlmConfig,
    compressed: &[ContextBlock],
    subgoals: &[Subgoal],
) -> Result<SufficiencyResult> {
    let key_var = if config.api_key_env.is_empty() {
        "OPENAI_API_KEY"
    } else {
        &config.api_key_env
    };
    let api_key = std::env::var(key_var)
        .map_err(|_| CompilerError::Other(format!("missing env {key_var} for LLM oracle")))?;

    let slots: Vec<String> = subgoals
        .iter()
        .flat_map(|s| s.required_slots.clone())
        .collect();
    if slots.is_empty() {
        return Ok(SufficiencyResult {
            sufficient: true,
            missing_slots: vec![],
            message: None,
        });
    }

    let corpus = compressed
        .iter()
        .map(|b| b.content.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    let prompt = format!(
        "Given compressed agent context, list which required slots are MISSING from this list: {:?}\n\
         Reply JSON only: {{\"missing\": [\"slot1\"]}}\n\nContext:\n{}",
        slots,
        &corpus[..corpus.len().min(12_000)]
    );

    let url = format!("{}/chat/completions", config.api_base.trim_end_matches('/'));
    let body = serde_json::json!({
        "model": config.model,
        "messages": [
            {"role": "system", "content": "You verify context sufficiency. JSON only."},
            {"role": "user", "content": prompt}
        ],
        "temperature": 0.0,
    });

    let client = reqwest::Client::new();
    let resp = client
        .post(&url)
        .bearer_auth(api_key)
        .json(&body)
        .timeout(std::time::Duration::from_secs(45))
        .send()
        .await
        .map_err(|e| CompilerError::Other(format!("llm oracle: {e}")))?;

    let text = resp
        .text()
        .await
        .map_err(|e| CompilerError::Other(format!("llm oracle body: {e}")))?;

    let missing = parse_missing_slots(&text, &slots);
    Ok(SufficiencyResult {
        sufficient: missing.is_empty(),
        missing_slots: missing.clone(),
        message: if missing.is_empty() {
            None
        } else {
            Some(format!("llm oracle missing: {}", missing.join(", ")))
        },
    })
}

#[cfg(feature = "llm-http")]
fn parse_missing_slots(response: &str, slots: &[String]) -> Vec<String> {
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(response) {
        if let Some(arr) = v.get("missing").and_then(|m| m.as_array()) {
            return arr
                .iter()
                .filter_map(|x| x.as_str().map(String::from))
                .collect();
        }
        if let Some(choices) = v.get("choices").and_then(|c| c.as_array()) {
            if let Some(content) = choices
                .first()
                .and_then(|c| c.get("message"))
                .and_then(|m| m.get("content"))
                .and_then(|c| c.as_str())
            {
                if let Ok(inner) = serde_json::from_str::<serde_json::Value>(content) {
                    if let Some(arr) = inner.get("missing").and_then(|m| m.as_array()) {
                        return arr
                            .iter()
                            .filter_map(|x| x.as_str().map(String::from))
                            .collect();
                    }
                }
            }
        }
    }
    let corpus = response.to_lowercase();
    slots
        .iter()
        .filter(|s| !corpus.contains(&s.to_lowercase()))
        .map(|s| s.clone())
        .collect()
}
