//! Persistent objective types shared across the workspace.
//!
//! These types form the ABI for the goal layer. `ObjectiveStatus` mirrors
//! `runtime::core::react_loop::goal_tracker::GoalStatus` and the SQLite CHECK
//! constraint on `objectives.status`.

use serde::{Deserialize, Serialize};

/// The status of an objective, matching GoalStatus + DB constraint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObjectiveStatus {
    InProgress,
    Completed,
    Failed,
    Adjusted,
}

impl ObjectiveStatus {
    /// String representation for JSON-RPC responses and DB storage.
    pub fn as_str(&self) -> &'static str {
        match self {
            ObjectiveStatus::InProgress => "in_progress",
            ObjectiveStatus::Completed => "completed",
            ObjectiveStatus::Failed => "failed",
            ObjectiveStatus::Adjusted => "adjusted",
        }
    }

    /// Parse from a DB status string.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "in_progress" => Some(ObjectiveStatus::InProgress),
            "completed" => Some(ObjectiveStatus::Completed),
            "failed" => Some(ObjectiveStatus::Failed),
            "adjusted" => Some(ObjectiveStatus::Adjusted),
            _ => None,
        }
    }
}

impl std::fmt::Display for ObjectiveStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// A persisted objective (top-level goal or sub-goal via parent_id).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Objective {
    pub objective_id: i64,
    pub description: String,
    pub status: ObjectiveStatus,
    pub parent_id: Option<i64>,
    pub session_id: String,
    pub scope: String,
    pub created_at: String,
    pub updated_at: String,
}

/// Lightweight summary for list views (no parent/session/scope noise).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObjectiveSummary {
    pub objective_id: i64,
    pub description: String,
    pub status: ObjectiveStatus,
}

impl Objective {
    /// Convert to a summary suitable for list display.
    pub fn to_summary(&self) -> ObjectiveSummary {
        ObjectiveSummary {
            objective_id: self.objective_id,
            description: self.description.clone(),
            status: self.status,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_serde_roundtrip() {
        let statuses = vec![
            ObjectiveStatus::InProgress,
            ObjectiveStatus::Completed,
            ObjectiveStatus::Failed,
            ObjectiveStatus::Adjusted,
        ];
        for s in statuses {
            let json = serde_json::to_string(&s).unwrap();
            let back: ObjectiveStatus = serde_json::from_str(&json).unwrap();
            assert_eq!(s, back);
        }
    }

    #[test]
    fn status_str_roundtrip() {
        assert_eq!(
            ObjectiveStatus::from_str("in_progress"),
            Some(ObjectiveStatus::InProgress)
        );
        assert_eq!(
            ObjectiveStatus::from_str("completed"),
            Some(ObjectiveStatus::Completed)
        );
        assert_eq!(ObjectiveStatus::from_str("bogus"), None);
    }

    #[test]
    fn objective_json_roundtrip() {
        let obj = Objective {
            objective_id: 1,
            description: "ship goal layer".into(),
            status: ObjectiveStatus::InProgress,
            parent_id: None,
            session_id: "sess-1".into(),
            scope: "project".into(),
            created_at: "2026-07-02T00:00:00".into(),
            updated_at: "2026-07-02T00:00:00".into(),
        };
        let json = serde_json::to_string_pretty(&obj).unwrap();
        let back: Objective = serde_json::from_str(&json).unwrap();
        assert_eq!(back.objective_id, 1);
        assert_eq!(back.status, ObjectiveStatus::InProgress);
    }

    #[test]
    fn summary_conversion() {
        let obj = Objective {
            objective_id: 42,
            description: "test".into(),
            status: ObjectiveStatus::Completed,
            parent_id: Some(1),
            session_id: "s".into(),
            scope: "session".into(),
            created_at: String::new(),
            updated_at: String::new(),
        };
        let summary = obj.to_summary();
        assert_eq!(summary.objective_id, 42);
        assert_eq!(summary.description, "test");
        assert_eq!(summary.status, ObjectiveStatus::Completed);
    }
}
