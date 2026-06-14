//! Planner — generates Plans from intents and reasoning chains.
//!
//! The Planner takes an intent + reasoning chain and produces a Plan
//! containing PlanSteps with rollback actions and cost estimates.

use aletheon_abi::body::Action;
use aletheon_abi::brain::{CostEstimate, Plan, PlanStep};
use aletheon_abi::context::Context;
use aletheon_abi::self_field::{Intent, RiskLevel};
use uuid::Uuid;

/// The planner component.
///
/// Generates structured Plans from intents and reasoning chains.
/// Each PlanStep includes an optional rollback action for reversibility.
pub struct Planner;

impl Planner {
    pub fn new() -> Self {
        Self
    }

    /// Generate a plan from an intent and reasoning chain.
    pub fn generate_plan(
        &self,
        intent: &Intent,
        reasoning: &str,
        _ctx: &Context,
    ) -> Plan {
        let step_id = Uuid::new_v4();
        let action = self.intent_to_action(intent);
        let rollback = self.infer_rollback(intent);

        let step = PlanStep {
            id: step_id,
            action: action.clone(),
            depends_on: vec![],
            expected_outcome: format!("Execute '{}' successfully", intent.action),
            rollback_action: rollback,
        };

        let risk_level = self.estimate_risk(intent);
        let cost = self.estimate_cost(intent, reasoning);

        Plan {
            id: Uuid::new_v4(),
            steps: vec![step],
            estimated_cost: cost,
            risk_level,
            reasoning: reasoning.to_string(),
            alternatives: vec![],
        }
    }

    /// Generate a multi-step plan for complex intents.
    pub fn generate_multi_step_plan(
        &self,
        intent: &Intent,
        reasoning: &str,
        sub_actions: Vec<(String, serde_json::Value)>,
    ) -> Plan {
        let mut steps = Vec::new();
        let mut prev_id: Option<Uuid> = None;

        for (action_name, params) in &sub_actions {
            let step_id = Uuid::new_v4();
            let action = Action {
                name: action_name.clone(),
                parameters: params.clone(),
                requires_sandbox: false,
                timeout: None,
            };
            let rollback = self.infer_rollback_from_action(action_name);

            steps.push(PlanStep {
                id: step_id,
                action,
                depends_on: prev_id.into_iter().collect(),
                expected_outcome: format!("Complete '{}'", action_name),
                rollback_action: rollback,
            });

            prev_id = Some(step_id);
        }

        let risk_level = self.estimate_risk(intent);

        Plan {
            id: Uuid::new_v4(),
            steps,
            estimated_cost: CostEstimate {
                estimated_tokens: (sub_actions.len() as u32) * 500,
                estimated_time_ms: (sub_actions.len() as u64) * 1000,
                estimated_tool_calls: sub_actions.len(),
            },
            risk_level,
            reasoning: reasoning.to_string(),
            alternatives: vec![],
        }
    }

    /// Convert an intent into an Action.
    fn intent_to_action(&self, intent: &Intent) -> Action {
        Action {
            name: intent.action.clone(),
            parameters: intent.parameters.clone(),
            requires_sandbox: self.is_sandbox_worthy(intent),
            timeout: None,
        }
    }

    /// Infer a rollback action for an intent, if possible.
    fn infer_rollback(&self, intent: &Intent) -> Option<Action> {
        self.infer_rollback_from_action(&intent.action)
    }

    /// Infer a rollback action from an action name.
    fn infer_rollback_from_action(&self, action_name: &str) -> Option<Action> {
        match action_name {
            name if name.starts_with("file.create") => Some(Action {
                name: "file.delete".to_string(),
                parameters: serde_json::json!({}),
                requires_sandbox: false,
                timeout: None,
            }),
            name if name.starts_with("file.write") => Some(Action {
                name: "file.restore_backup".to_string(),
                parameters: serde_json::json!({}),
                requires_sandbox: false,
                timeout: None,
            }),
            name if name.starts_with("shell.execute") => None, // Shell commands are generally not reversible
            _ => None,
        }
    }

    /// Estimate risk level of an intent.
    fn estimate_risk(&self, intent: &Intent) -> RiskLevel {
        let action = intent.action.to_lowercase();
        if action.contains("delete") || action.contains("rm") || action.contains("destroy") {
            RiskLevel::High
        } else if action.contains("write") || action.contains("modify") || action.contains("deploy") {
            RiskLevel::Medium
        } else if action.contains("read") || action.contains("ls") || action.contains("status") {
            RiskLevel::Low
        } else {
            RiskLevel::Low
        }
    }

    /// Whether the action should run in a sandbox.
    fn is_sandbox_worthy(&self, intent: &Intent) -> bool {
        let action = intent.action.to_lowercase();
        action.contains("deploy") || action.contains("delete") || action.contains("rm")
    }

    /// Estimate cost of executing a plan.
    fn estimate_cost(&self, _intent: &Intent, reasoning: &str) -> CostEstimate {
        let reasoning_tokens = (reasoning.len() / 4) as u32; // rough token estimate
        CostEstimate {
            estimated_tokens: reasoning_tokens + 1000,
            estimated_time_ms: 500,
            estimated_tool_calls: 1,
        }
    }
}

impl Default for Planner {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aletheon_abi::IntentSource;
    use serde_json::json;
    use std::path::PathBuf;

    fn make_intent(action: &str, desc: &str) -> Intent {
        Intent {
            action: action.to_string(),
            parameters: json!({"command": "ls"}),
            source: IntentSource::User,
            description: desc.to_string(),
        }
    }

    fn make_ctx() -> Context {
        Context::new("test", PathBuf::from("/tmp"))
    }

    #[test]
    fn basic_plan_generation() {
        let planner = Planner::new();
        let intent = make_intent("shell.execute", "list files");
        let plan = planner.generate_plan(&intent, "reasoning chain", &make_ctx());

        assert_eq!(plan.steps.len(), 1);
        assert_eq!(plan.steps[0].action.name, "shell.execute");
        assert!(plan.steps[0].rollback_action.is_none()); // shell has no rollback
        assert!(plan.reasoning.contains("reasoning chain"));
    }

    #[test]
    fn file_create_gets_rollback() {
        let planner = Planner::new();
        let intent = make_intent("file.create", "create config");
        let plan = planner.generate_plan(&intent, "reasoning", &make_ctx());

        assert!(plan.steps[0].rollback_action.is_some());
        assert_eq!(plan.steps[0].rollback_action.as_ref().unwrap().name, "file.delete");
    }

    #[test]
    fn multi_step_plan() {
        let planner = Planner::new();
        let intent = make_intent("deploy.app", "deploy application");
        let sub_actions = vec![
            ("build.project".to_string(), json!({})),
            ("test.suite".to_string(), json!({})),
            ("deploy.prod".to_string(), json!({})),
        ];
        let plan = planner.generate_multi_step_plan(&intent, "multi-step deploy", sub_actions);

        assert_eq!(plan.steps.len(), 3);
        // Second step depends on first
        assert!(!plan.steps[1].depends_on.is_empty());
        // Third step depends on second
        assert!(!plan.steps[2].depends_on.is_empty());
    }

    #[test]
    fn risk_estimation() {
        let planner = Planner::new();

        let low = planner.estimate_risk(&make_intent("file.read", "read"));
        assert_eq!(low, RiskLevel::Low);

        let medium = planner.estimate_risk(&make_intent("file.write", "write"));
        assert_eq!(medium, RiskLevel::Medium);

        let high = planner.estimate_risk(&make_intent("file.delete", "delete"));
        assert_eq!(high, RiskLevel::High);
    }

    #[test]
    fn sandbox_worthy() {
        let planner = Planner::new();
        assert!(planner.is_sandbox_worthy(&make_intent("deploy.prod", "deploy")));
        assert!(planner.is_sandbox_worthy(&make_intent("file.delete", "rm")));
        assert!(!planner.is_sandbox_worthy(&make_intent("file.read", "read")));
    }

    #[test]
    fn cost_estimate() {
        let planner = Planner::new();
        let intent = make_intent("shell.execute", "test");
        let cost = planner.estimate_cost(&intent, "a reasoning chain of some length");
        assert!(cost.estimated_tokens > 0);
        assert_eq!(cost.estimated_tool_calls, 1);
    }
}
