//! Cascade routing hints: suggest fast vs full model tier after compile.

use serde::{Deserialize, Serialize};

use crate::oracle::SufficiencyResult;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelTier {
    Fast,
    Full,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoutingHint {
    pub tier: ModelTier,
    pub reason: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub estimated_complexity: Option<u8>,
}

pub fn compute_routing_hint(
    output_tokens: u64,
    token_budget: u64,
    sufficiency: &SufficiencyResult,
    turn_index: u32,
) -> RoutingHint {
    let over_half = output_tokens > token_budget / 2;
    let failed_slots = !sufficiency.missing_slots.is_empty();
    let deep_turn = turn_index > 15;

    if failed_slots || over_half || deep_turn {
        RoutingHint {
            tier: ModelTier::Full,
            reason: if failed_slots {
                "sufficiency slots missing — use full model".into()
            } else if over_half {
                "context over 50% of budget".into()
            } else {
                "deep multi-turn session".into()
            },
            estimated_complexity: Some(8),
        }
    } else {
        RoutingHint {
            tier: ModelTier::Fast,
            reason: "compressed context within budget".into(),
            estimated_complexity: Some(3),
        }
    }
}
