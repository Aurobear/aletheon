//! Typed, host-authoritative context and rollout budget projections.
//!
//! Model context, profile input limits, history capacity, and Agent rollout
//! allowances have different lifetimes and meanings.  Separate token newtypes
//! prevent those values from being wired together accidentally across crate or
//! protocol boundaries.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

macro_rules! semantic_token_count {
    ($name:ident, $doc:literal) => {
        #[doc = $doc]
        #[derive(
            Debug,
            Clone,
            Copy,
            Default,
            PartialEq,
            Eq,
            PartialOrd,
            Ord,
            Serialize,
            Deserialize,
            JsonSchema,
        )]
        #[serde(transparent)]
        pub struct $name(pub u64);

        impl $name {
            pub const fn new(tokens: u64) -> Self {
                Self(tokens)
            }

            pub const fn get(self) -> u64 {
                self.0
            }
        }

        impl From<u64> for $name {
            fn from(tokens: u64) -> Self {
                Self(tokens)
            }
        }
    };
}
semantic_token_count!(
    ModelContextWindowTokens,
    "Maximum tokens accepted by one request to the effective runtime model."
);
semantic_token_count!(
    ProfileInputLimitTokens,
    "Input ceiling selected by the active Agent profile."
);
semantic_token_count!(
    ContextCostTokens,
    "Tokens consumed or reserved by one non-history context component."
);
semantic_token_count!(
    HistoryTokens,
    "Observed or projected conversation-history tokens for one request."
);
semantic_token_count!(
    HistoryBudgetTokens,
    "Capacity made available to conversation history for one request."
);
semantic_token_count!(
    RolloutBudgetTokens,
    "Cumulative tokens allowed or remaining across an Agent rollout."
);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ContextBudgetSourceKind {
    RuntimeModelCapability,
    ActiveAgentProfile,
    ContextBudgetPlanner,
    EffectiveAdmissionConfig,
    AgentRuntime,
}

impl ContextBudgetSourceKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::RuntimeModelCapability => "runtime model capability",
            Self::ActiveAgentProfile => "active Agent profile",
            Self::ContextBudgetPlanner => "context budget planner",
            Self::EffectiveAdmissionConfig => "effective admission config",
            Self::AgentRuntime => "Agent runtime",
        }
    }
}

/// Secret-safe description of the authority for a projected value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ContextBudgetSource {
    pub kind: ContextBudgetSourceKind,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub digest_sha256: Option<String>,
}

impl ContextBudgetSource {
    pub fn new(kind: ContextBudgetSourceKind, label: impl Into<String>) -> Self {
        Self {
            kind,
            label: label.into(),
            digest_sha256: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum BudgetMissingReason {
    NoActiveAgentRollout,
    RuntimeScopeNotReported,
    SourceUnavailable,
}

impl BudgetMissingReason {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NoActiveAgentRollout => "no active Agent rollout",
            Self::RuntimeScopeNotReported => "runtime scope not reported",
            Self::SourceUnavailable => "budget source unavailable",
        }
    }
}

/// A rollout value is either known with its source or explicitly unavailable
/// with a source-specific reason. There is no numeric fallback state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum RolloutBudgetValue {
    Known {
        value: RolloutBudgetTokens,
        source: ContextBudgetSource,
    },
    Unknown {
        source: ContextBudgetSource,
        reason: BudgetMissingReason,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RolloutBudgetProjection {
    pub root_remaining_tokens: RolloutBudgetValue,
    pub child_limit_tokens: RolloutBudgetValue,
    pub current_agent_remaining_tokens: RolloutBudgetValue,
}

/// One authoritative per-turn snapshot. Context values are request-scoped;
/// `rollout` values are cumulative and deliberately use a separate newtype.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ContextBudgetProjection {
    pub model_spec: String,
    pub model_context_tokens: ModelContextWindowTokens,
    pub profile_input_limit_tokens: ProfileInputLimitTokens,
    pub reserved_output_tokens: ContextCostTokens,
    pub system_and_skill_tokens: ContextCostTokens,
    pub tool_schema_tokens: ContextCostTokens,
    pub pending_input_tokens: HistoryTokens,
    pub safety_margin_tokens: ContextCostTokens,
    pub current_history_tokens: HistoryTokens,
    pub admissible_history_tokens: HistoryBudgetTokens,
    pub compaction_threshold_tokens: HistoryBudgetTokens,
    pub model_source: ContextBudgetSource,
    pub profile_source: ContextBudgetSource,
    pub history_source: ContextBudgetSource,
    pub rollout: RolloutBudgetProjection,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ContextCompactionMode {
    Preflight,
    Automatic,
}

/// Durable evidence for one applied context compaction.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ContextCompactionProjection {
    pub mode: ContextCompactionMode,
    pub trigger_threshold_tokens: HistoryBudgetTokens,
    pub tokens_before: HistoryTokens,
    pub tokens_after: HistoryTokens,
    pub budget_snapshot: ContextBudgetProjection,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_rollout_value_round_trips_with_a_reason_and_no_numeric_fallback() {
        let value = RolloutBudgetValue::Unknown {
            source: ContextBudgetSource::new(
                ContextBudgetSourceKind::AgentRuntime,
                "current Agent rollout",
            ),
            reason: BudgetMissingReason::NoActiveAgentRollout,
        };
        let json = serde_json::to_value(&value).unwrap();
        assert_eq!(json["status"], "unknown");
        assert_eq!(json["reason"], "no_active_agent_rollout");
        assert!(json.get("value").is_none());
        assert_eq!(
            serde_json::from_value::<RolloutBudgetValue>(json).unwrap(),
            value
        );
    }
}
