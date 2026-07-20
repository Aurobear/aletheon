//! Canonical Agent profile switch audit event.

use serde::{Deserialize, Serialize};

use super::agent_control::RiskTier;

pub const AGENT_PROFILE_SWITCH_EVENT_SCHEMA_V1: u16 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentProfileSwitchDecision {
    Accepted,
    Denied,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentProfileSwitchEventV1 {
    pub schema_version: u16,
    pub previous_profile: String,
    pub requested_profile: String,
    pub previous_risk_tier: RiskTier,
    pub requested_risk_tier: RiskTier,
    pub decision: AgentProfileSwitchDecision,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}
