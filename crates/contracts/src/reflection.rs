//! Dependency-neutral reflection and evolution records shared by cognition and memory.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ReflectionTrigger {
    TaskComplete,
    Impasse,
    Manual,
}

impl std::fmt::Display for ReflectionTrigger {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::TaskComplete => "task_complete",
            Self::Impasse => "impasse",
            Self::Manual => "manual",
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ReflectionOutcome {
    Success,
    Partial,
    Failure,
}

impl std::fmt::Display for ReflectionOutcome {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Success => "success",
            Self::Partial => "partial",
            Self::Failure => "failure",
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReflectionEntry {
    pub id: String,
    pub timestamp: DateTime<Utc>,
    pub trigger: ReflectionTrigger,
    pub task_summary: String,
    pub outcome: ReflectionOutcome,
    pub what_worked: Vec<String>,
    pub what_failed: Vec<String>,
    pub learned: Vec<String>,
    pub behavior_changes: Vec<String>,
    pub confidence: f64,
}

impl ReflectionEntry {
    pub fn to_json_bytes(&self) -> Vec<u8> {
        serde_json::to_vec(self).unwrap_or_default()
    }

    pub fn from_json_bytes(bytes: &[u8]) -> Option<Self> {
        serde_json::from_slice(bytes).ok()
    }

    pub fn summary(&self) -> String {
        let icon = match self.outcome {
            ReflectionOutcome::Success => "✅",
            ReflectionOutcome::Partial => "⚠️",
            ReflectionOutcome::Failure => "❌",
        };
        let trigger = match self.trigger {
            ReflectionTrigger::TaskComplete => "",
            ReflectionTrigger::Impasse => " [impasse]",
            ReflectionTrigger::Manual => " [manual]",
        };
        format!(
            "[{}] {}{} {}",
            self.timestamp.format("%Y-%m-%d %H:%M"),
            icon,
            trigger,
            self.task_summary
        )
    }

    pub fn detail(&self) -> String {
        let mut lines = vec![self.summary()];
        if !self.learned.is_empty() {
            lines.push("  学到:".to_string());
            lines.extend(self.learned.iter().map(|item| format!("    · {item}")));
        }
        if !self.behavior_changes.is_empty() {
            lines.push("  行为调整:".to_string());
            lines.extend(
                self.behavior_changes
                    .iter()
                    .map(|item| format!("    · {item}")),
            );
        }
        lines.join("\n")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BehaviorAdjustment {
    pub target: String,
    pub old_value: Option<f64>,
    pub new_value: Option<f64>,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvolutionLogEntry {
    pub id: String,
    pub timestamp: DateTime<Utc>,
    pub trigger: String,
    pub basis: Vec<String>,
    pub patterns_detected: Vec<String>,
    pub adjustments: Vec<BehaviorAdjustment>,
}

impl EvolutionLogEntry {
    pub fn to_json_bytes(&self) -> Vec<u8> {
        serde_json::to_vec(self).unwrap_or_default()
    }

    pub fn from_json_bytes(bytes: &[u8]) -> Option<Self> {
        serde_json::from_slice(bytes).ok()
    }
}
