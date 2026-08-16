//! Persistent goal contracts for the M2 Goal Runtime.
//!
//! These types form the ABI between the GoalCoordinator, ObjectiveStore,
//! RPC handlers, and channel adapters. Goal state is independent of
//! kernel `ProcessState` — the coordinator bridges the two.

use serde::{Deserialize, Serialize};
use std::fmt;

use crate::types::admission::PrincipalId;
use crate::types::operation::ProcessId;

// ---------------------------------------------------------------------------
// GoalId
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct GoalId(pub i64);

impl fmt::Display for GoalId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

// ---------------------------------------------------------------------------
// GoalState
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GoalState {
    Draft,
    Ready,
    Running,
    Blocked,
    AwaitingHuman,
    Suspended,
    Completed,
    Failed,
    Cancelled,
}

impl GoalState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Draft => "draft",
            Self::Ready => "ready",
            Self::Running => "running",
            Self::Blocked => "blocked",
            Self::AwaitingHuman => "awaiting_human",
            Self::Suspended => "suspended",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }

    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "draft" => Some(Self::Draft),
            "ready" => Some(Self::Ready),
            "running" => Some(Self::Running),
            "blocked" => Some(Self::Blocked),
            "awaiting_human" => Some(Self::AwaitingHuman),
            "suspended" => Some(Self::Suspended),
            "completed" => Some(Self::Completed),
            "failed" => Some(Self::Failed),
            "cancelled" => Some(Self::Cancelled),
            _ => None,
        }
    }

    pub fn can_transition_to(self, next: Self) -> bool {
        use GoalState::*;
        matches!(
            (self, next),
            (Draft, Ready)
                | (Draft, Cancelled)
                | (Ready, Running)
                | (Ready, Cancelled)
                | (Running, Blocked)
                | (Running, AwaitingHuman)
                | (Running, Suspended)
                | (Running, Completed)
                | (Running, Failed)
                | (Running, Cancelled)
                | (Suspended, Ready)
                | (Suspended, Cancelled)
                | (Blocked, Ready)
                | (Blocked, Cancelled)
                | (AwaitingHuman, Ready)
                | (AwaitingHuman, Cancelled)
        )
    }

    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Cancelled)
    }
}

impl fmt::Display for GoalState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

// ---------------------------------------------------------------------------
// GoalWaitReason
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum GoalWaitReason {
    HumanInput { prompt: String },
    ExternalEvent { key: String },
    Backoff { until_ms: i64 },
}

// ---------------------------------------------------------------------------
// GoalBudget / GoalBudgetUsage
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GoalBudget {
    pub max_input_tokens: u64,
    pub max_output_tokens: u64,
    pub max_cost_usd: Option<f64>,
    pub max_attempts: u32,
    pub deadline_ms: Option<i64>,
}

impl Default for GoalBudget {
    fn default() -> Self {
        Self {
            max_input_tokens: 1_000_000,
            max_output_tokens: 500_000,
            max_cost_usd: None,
            max_attempts: 10,
            deadline_ms: None,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct GoalBudgetUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cost_usd: f64,
    pub attempts: u32,
}

// ---------------------------------------------------------------------------
// GoalSpec
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GoalSpec {
    pub original_intent: String,
    pub desired_state: Vec<String>,
    pub constraints: Vec<String>,
    pub acceptance_criteria: Vec<String>,
    pub budget: GoalBudget,
}

// ---------------------------------------------------------------------------
// GoalSnapshot
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GoalSnapshot {
    pub id: GoalId,
    pub owner: PrincipalId,
    pub state: GoalState,
    pub spec: GoalSpec,
    pub usage: GoalBudgetUsage,
    pub wait_reason: Option<GoalWaitReason>,
    pub process_id: Option<ProcessId>,
    pub version: u64,
    pub created_at: String,
    pub updated_at: String,
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn goal_state_transitions_are_explicit() {
        use GoalState::*;
        assert!(Draft.can_transition_to(Ready));
        assert!(Ready.can_transition_to(Running));
        assert!(Running.can_transition_to(Blocked));
        assert!(Running.can_transition_to(AwaitingHuman));
        assert!(Running.can_transition_to(Suspended));
        assert!(Suspended.can_transition_to(Ready));
        assert!(Running.can_transition_to(Completed));
        assert!(Running.can_transition_to(Failed));
        assert!(Draft.can_transition_to(Cancelled));
        assert!(!Completed.can_transition_to(Running));
        assert!(!Cancelled.can_transition_to(Ready));
    }

    #[test]
    fn goal_state_terminal() {
        use GoalState::*;
        assert!(Completed.is_terminal());
        assert!(Failed.is_terminal());
        assert!(Cancelled.is_terminal());
        assert!(!Draft.is_terminal());
        assert!(!Ready.is_terminal());
        assert!(!Running.is_terminal());
        assert!(!Blocked.is_terminal());
        assert!(!AwaitingHuman.is_terminal());
        assert!(!Suspended.is_terminal());
    }

    #[test]
    fn goal_state_serde_roundtrip() {
        use GoalState::*;
        for state in [
            Draft,
            Ready,
            Running,
            Blocked,
            AwaitingHuman,
            Suspended,
            Completed,
            Failed,
            Cancelled,
        ] {
            let json = serde_json::to_string(&state).unwrap();
            let back: GoalState = serde_json::from_str(&json).unwrap();
            assert_eq!(state, back);
        }
    }

    #[test]
    fn goal_state_str_roundtrip() {
        for state in [
            GoalState::Draft,
            GoalState::Ready,
            GoalState::Running,
            GoalState::Blocked,
            GoalState::AwaitingHuman,
            GoalState::Suspended,
            GoalState::Completed,
            GoalState::Failed,
            GoalState::Cancelled,
        ] {
            let s = state.as_str();
            let back = GoalState::from_str(s);
            assert_eq!(back, Some(state), "roundtrip failed for {s}");
        }
        assert_eq!(GoalState::from_str("bogus"), None);
    }

    #[test]
    fn goal_id_display() {
        let id = GoalId(42);
        assert_eq!(id.to_string(), "42");
    }

    #[test]
    fn goal_budget_default() {
        let b = GoalBudget::default();
        assert_eq!(b.max_input_tokens, 1_000_000);
        assert_eq!(b.max_output_tokens, 500_000);
        assert_eq!(b.max_cost_usd, None);
        assert_eq!(b.max_attempts, 10);
        assert_eq!(b.deadline_ms, None);
    }

    #[test]
    fn goal_spec_roundtrip() {
        let spec = GoalSpec {
            original_intent: "ship feature X".into(),
            desired_state: vec!["feature X deployed".into()],
            constraints: vec!["no breaking changes".into()],
            acceptance_criteria: vec!["tests pass".into()],
            budget: GoalBudget::default(),
        };
        let json = serde_json::to_string(&spec).unwrap();
        let back: GoalSpec = serde_json::from_str(&json).unwrap();
        assert_eq!(spec, back);
        assert_eq!(back.original_intent, "ship feature X");
    }

    #[test]
    fn goal_snapshot_roundtrip() {
        let snapshot = GoalSnapshot {
            id: GoalId(1),
            owner: PrincipalId("test-owner".into()),
            state: GoalState::Draft,
            spec: GoalSpec {
                original_intent: "test goal".into(),
                desired_state: vec![],
                constraints: vec![],
                acceptance_criteria: vec![],
                budget: GoalBudget::default(),
            },
            usage: GoalBudgetUsage::default(),
            wait_reason: None,
            process_id: None,
            version: 0,
            created_at: "2026-07-14T00:00:00Z".into(),
            updated_at: "2026-07-14T00:00:00Z".into(),
        };
        let json = serde_json::to_string(&snapshot).unwrap();
        let back: GoalSnapshot = serde_json::from_str(&json).unwrap();
        assert_eq!(snapshot, back);
    }

    #[test]
    fn goal_wait_reason_serde() {
        let reason = GoalWaitReason::HumanInput {
            prompt: "approve deployment?".into(),
        };
        let json = serde_json::to_string(&reason).unwrap();
        let back: GoalWaitReason = serde_json::from_str(&json).unwrap();
        assert_eq!(reason, back);

        let reason2 = GoalWaitReason::Backoff { until_ms: 1000 };
        let json2 = serde_json::to_string(&reason2).unwrap();
        let back2: GoalWaitReason = serde_json::from_str(&json2).unwrap();
        assert_eq!(reason2, back2);
    }
}
