//! Dasein-owned inputs for governed self-evolution.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LearnedRuleSnapshot {
    pub id: Uuid,
    pub condition: String,
    pub action: String,
    pub confidence: f64,
    pub source_reflections: Vec<Uuid>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvolutionTrigger {
    pub trigger_reason: String,
    pub recent_reflections: Vec<Uuid>,
    pub current_rules_snapshot: Vec<LearnedRuleSnapshot>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BehaviorAdjustment {
    pub target: String,
    pub old_value: Option<f64>,
    pub new_value: Option<f64>,
    pub reason: String,
}
