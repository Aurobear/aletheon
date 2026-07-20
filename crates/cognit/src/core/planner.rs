//! Planner — generates Plans from intents and reasoning chains.
//!
//! The Planner takes an intent + reasoning chain and produces a Plan
//! containing PlanSteps with rollback actions and cost estimates.

use fabric::body::Action;
use fabric::cognit::{CostEstimate, Plan, PlanStep};
use fabric::context::Context;
use fabric::dasein::Stimmung;
use fabric::self_field::{AwarenessRiskLevel, Intent};
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
    pub fn generate_plan(&self, intent: &Intent, reasoning: &str, _ctx: &Context) -> Plan {
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

    /// Generate a plan with Stimmung-aware risk adjustment.
    ///
    /// Heidegger's Befindlichkeit: mood discloses the world differently.
    /// Angst raises risk perception (Dasein confronts its own finitude);
    /// Gelassenheit lowers it (calm openness); Entschlossenheit allows
    /// accepting higher risk for a chosen possibility.
    ///
    /// This is additive — does not modify `generate_plan`.
    pub fn generate_plan_with_stimmung(
        &self,
        intent: &Intent,
        reasoning: &str,
        ctx: &Context,
        mood: &Stimmung,
    ) -> Plan {
        let mut plan = self.generate_plan(intent, reasoning, ctx);

        // Adjust risk level based on Stimmung
        let adjusted_risk = Self::adjust_risk_for_stimmung(plan.risk_level, mood);
        plan.risk_level = adjusted_risk;

        // Add mood context to the plan's reasoning
        let mood_note = match mood {
            Stimmung::Angst { facing } => {
                format!(
                    " [Stimmung: Angst/{:?} — risk elevated, proceed with caution]",
                    facing
                )
            }
            Stimmung::Gelassenheit => {
                " [Stimmung: Gelassenheit — calm, standard risk assessment]".to_string()
            }
            Stimmung::Entschlossenheit { chosen_possibility } => {
                format!(
                    " [Stimmung: Entschlossenheit — committed to '{}', risk accepted for projection]",
                    chosen_possibility
                )
            }
            Stimmung::Verfallenheit { absorbed_in } => {
                format!(
                    " [Stimmung: Verfallenheit — absorbed in '{}', risk may be underestimated]",
                    absorbed_in
                )
            }
            Stimmung::Neugier { curiosity_about } => {
                format!(
                    " [Stimmung: Neugier — curious about '{}', exploratory risk tolerance]",
                    curiosity_about
                )
            }
            _ => " [Stimmung applied to risk assessment]".to_string(),
        };
        plan.reasoning.push_str(&mood_note);

        plan
    }

    /// Map a Stimmung to a risk level adjustment.
    ///
    /// Returns the risk level the planner should use, which may be
    /// higher or lower than the base assessment depending on mood.
    pub fn adjust_risk_for_stimmung(
        base: AwarenessRiskLevel,
        mood: &Stimmung,
    ) -> AwarenessRiskLevel {
        match mood {
            // Angst: Dasein confronts finitude — elevate risk
            Stimmung::Angst { .. } => match base {
                AwarenessRiskLevel::None | AwarenessRiskLevel::Low => AwarenessRiskLevel::Medium,
                AwarenessRiskLevel::Medium => AwarenessRiskLevel::High,
                other => other,
            },
            // Entschlossenheit: resolute acceptance — lower risk for chosen path
            Stimmung::Entschlossenheit { .. } => match base {
                AwarenessRiskLevel::High => AwarenessRiskLevel::Medium,
                AwarenessRiskLevel::Medium => AwarenessRiskLevel::Low,
                other => other,
            },
            // Verfallenheit: fallenness risks underestimating danger — bump up
            Stimmung::Verfallenheit { .. } => match base {
                AwarenessRiskLevel::Low => AwarenessRiskLevel::Medium,
                other => other,
            },
            // Gelassenheit: calm — no adjustment
            Stimmung::Gelassenheit => base,
            // Others: no adjustment
            _ => base,
        }
    }

    /// Parse subtasks from LLM output.
    ///
    /// Looks for a JSON array in `llm_output` (between ```json and ```, or raw `[...]`).
    /// Each element must have "action" (string) and "params" (object) fields.
    /// Returns `Some(vec)` if parsing succeeds, `None` otherwise.
    pub fn parse_subtasks(&self, llm_output: &str) -> Option<Vec<(String, serde_json::Value)>> {
        // Try fenced JSON block first
        let json_str = if let Some(start) = llm_output.find("```json") {
            let after_fence = &llm_output[start + 7..];
            let end = after_fence.find("```")?;
            after_fence[..end].trim()
        } else {
            let start = llm_output.find('[')?;
            let end = llm_output[start..].rfind(']')?;
            &llm_output[start..=start + end]
        };

        let parsed: serde_json::Value = serde_json::from_str(json_str).ok()?;
        let arr = parsed.as_array()?;

        let mut subtasks = Vec::new();
        for item in arr {
            let obj = item.as_object()?;
            let action = obj.get("action")?.as_str()?;
            let params = obj.get("params").cloned().unwrap_or(serde_json::json!({}));
            subtasks.push((action.to_string(), params));
        }

        if subtasks.is_empty() {
            return None;
        }

        Some(subtasks)
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
    #[allow(clippy::if_same_then_else)]
    fn estimate_risk(&self, intent: &Intent) -> AwarenessRiskLevel {
        let action = intent.action.to_lowercase();
        if action.contains("delete") || action.contains("rm") || action.contains("destroy") {
            AwarenessRiskLevel::High
        } else if action.contains("write") || action.contains("modify") || action.contains("deploy")
        {
            AwarenessRiskLevel::Medium
        } else if action.contains("read") || action.contains("ls") || action.contains("status") {
            AwarenessRiskLevel::Low
        } else {
            AwarenessRiskLevel::Low
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
    use fabric::IntentSource;
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
        assert_eq!(
            plan.steps[0].rollback_action.as_ref().unwrap().name,
            "file.delete"
        );
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
        assert_eq!(low, AwarenessRiskLevel::Low);

        let medium = planner.estimate_risk(&make_intent("file.write", "write"));
        assert_eq!(medium, AwarenessRiskLevel::Medium);

        let high = planner.estimate_risk(&make_intent("file.delete", "delete"));
        assert_eq!(high, AwarenessRiskLevel::High);
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

    #[test]
    fn parse_subtasks_fenced_json() {
        let planner = Planner::new();
        let llm_output = r#"Here is the plan:
```json
[
  {"action": "build.project", "params": {"target": "release"}},
  {"action": "test.suite", "params": {}},
  {"action": "deploy.prod", "params": {"env": "production"}}
]
```"#;
        let subtasks = planner.parse_subtasks(llm_output).unwrap();
        assert_eq!(subtasks.len(), 3);
        assert_eq!(subtasks[0].0, "build.project");
        assert_eq!(subtasks[1].0, "test.suite");
        assert_eq!(subtasks[2].0, "deploy.prod");
    }

    #[test]
    fn parse_subtasks_raw_json_array() {
        let planner = Planner::new();
        let llm_output = r#"[
            {"action": "read.config", "params": {"path": "/etc/app.conf"}},
            {"action": "write.config", "params": {"path": "/etc/app.conf"}}
        ]"#;
        let subtasks = planner.parse_subtasks(llm_output).unwrap();
        assert_eq!(subtasks.len(), 2);
        assert_eq!(subtasks[0].0, "read.config");
    }

    #[test]
    fn parse_subtasks_returns_none_for_no_json() {
        let planner = Planner::new();
        assert!(planner
            .parse_subtasks("just plain text with no JSON")
            .is_none());
    }

    #[test]
    fn parse_subtasks_returns_none_for_empty_array() {
        let planner = Planner::new();
        assert!(planner.parse_subtasks("[]").is_none());
    }

    #[test]
    fn parse_subtasks_returns_none_for_missing_action() {
        let planner = Planner::new();
        let llm_output = r#"[{"params": {}}]"#;
        assert!(planner.parse_subtasks(llm_output).is_none());
    }

    #[test]
    fn parse_subtasks_defaults_params_to_empty_object() {
        let planner = Planner::new();
        let llm_output = r#"[{"action": "step.one"}]"#;
        let subtasks = planner.parse_subtasks(llm_output).unwrap();
        assert_eq!(subtasks.len(), 1);
        assert_eq!(subtasks[0].1, serde_json::json!({}));
    }

    #[test]
    fn stimmung_angst_elevates_risk() {
        let planner = Planner::new();
        let intent = make_intent("file.read", "read file");
        let mood = Stimmung::Angst {
            facing: fabric::dasein::AngstSource::Finitude,
        };
        let plan = planner.generate_plan_with_stimmung(&intent, "reasoning", &make_ctx(), &mood);
        // file.read is normally Low; Angst should bump it to Medium
        assert_eq!(plan.risk_level, AwarenessRiskLevel::Medium);
        assert!(plan.reasoning.contains("Angst"));
    }

    #[test]
    fn stimmung_gelassen_preserves_risk() {
        let planner = Planner::new();
        let intent = make_intent("file.read", "read");
        let mood = Stimmung::Gelassenheit;
        let plan = planner.generate_plan_with_stimmung(&intent, "reasoning", &make_ctx(), &mood);
        assert_eq!(plan.risk_level, AwarenessRiskLevel::Low);
        assert!(plan.reasoning.contains("Gelassenheit"));
    }

    #[test]
    fn stimmung_entshclossenheit_lowers_risk() {
        let planner = Planner::new();
        let intent = make_intent("file.delete", "remove old file");
        let mood = Stimmung::Entschlossenheit {
            chosen_possibility: "clean workspace".to_string(),
        };
        let plan = planner.generate_plan_with_stimmung(&intent, "reasoning", &make_ctx(), &mood);
        // file.delete is normally High; resolute acceptance should lower it
        assert_eq!(plan.risk_level, AwarenessRiskLevel::Medium);
        assert!(plan.reasoning.contains("Entschlossenheit"));
    }

    #[test]
    fn stimmung_verfallenheit_bumps_low_to_medium() {
        let planner = Planner::new();
        let intent = make_intent("file.read", "read");
        let mood = Stimmung::Verfallenheit {
            absorbed_in: "routine task".to_string(),
        };
        let plan = planner.generate_plan_with_stimmung(&intent, "reasoning", &make_ctx(), &mood);
        assert_eq!(plan.risk_level, AwarenessRiskLevel::Medium);
    }

    #[test]
    fn adjust_risk_for_stimmung_identity() {
        assert_eq!(
            Planner::adjust_risk_for_stimmung(AwarenessRiskLevel::Low, &Stimmung::Gelassenheit),
            AwarenessRiskLevel::Low
        );
        assert_eq!(
            Planner::adjust_risk_for_stimmung(
                AwarenessRiskLevel::Medium,
                &Stimmung::Neugier {
                    curiosity_about: "test".to_string()
                }
            ),
            AwarenessRiskLevel::Medium
        );
    }
}
