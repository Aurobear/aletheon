//! Critic — multi-dimensional plan critique.
//!
//! Evaluates plans across multiple dimensions: correctness, completeness,
//! risk, efficiency, consistency, and reversibility. Produces Critique
//! items with severity levels and actionable suggestions.

use aletheon_abi::brain::{Critique, CriticismDimension, CriticismSeverity, Plan};

/// The critic component.
///
/// Evaluates plans before execution to catch issues early.
pub struct Critic;

impl Critic {
    pub fn new() -> Self {
        Self
    }

    /// Critique a plan across all dimensions.
    pub fn critique(&self, plan: &Plan) -> Vec<Critique> {
        let mut critiques = Vec::new();

        critiques.extend(self.check_completeness(plan));
        critiques.extend(self.check_risk(plan));
        critiques.extend(self.check_efficiency(plan));
        critiques.extend(self.check_reversibility(plan));
        critiques.extend(self.check_consistency(plan));

        critiques
    }

    /// Check if the plan has all necessary steps.
    fn check_completeness(&self, plan: &Plan) -> Vec<Critique> {
        let mut critiques = Vec::new();

        if plan.steps.is_empty() {
            critiques.push(Critique {
                dimension: CriticismDimension::Completeness,
                severity: CriticismSeverity::Fatal,
                description: "Plan has no steps — cannot execute.".to_string(),
                suggestion: Some("Add at least one PlanStep.".to_string()),
            });
        }

        // Check for broken dependency references
        let step_ids: std::collections::HashSet<uuid::Uuid> =
            plan.steps.iter().map(|s| s.id).collect();
        for step in &plan.steps {
            for dep in &step.depends_on {
                if !step_ids.contains(dep) {
                    critiques.push(Critique {
                        dimension: CriticismDimension::Completeness,
                        severity: CriticismSeverity::Error,
                        description: format!(
                            "Step '{}' depends on non-existent step '{}'.",
                            step.id, dep
                        ),
                        suggestion: Some("Fix dependency reference or add missing step.".to_string()),
                    });
                }
            }
        }

        critiques
    }

    /// Check for high-risk steps without rollback.
    fn check_risk(&self, plan: &Plan) -> Vec<Critique> {
        let mut critiques = Vec::new();

        for step in &plan.steps {
            let action_lower = step.action.name.to_lowercase();
            let is_destructive = action_lower.contains("delete")
                || action_lower.contains("rm")
                || action_lower.contains("destroy");

            if is_destructive && step.rollback_action.is_none() {
                critiques.push(Critique {
                    dimension: CriticismDimension::Risk,
                    severity: CriticismSeverity::Warning,
                    description: format!(
                        "Destructive action '{}' has no rollback action.",
                        step.action.name
                    ),
                    suggestion: Some(
                        "Add a rollback action or ensure backups exist.".to_string(),
                    ),
                });
            }
        }

        if plan.risk_level >= aletheon_abi::self_field::RiskLevel::High {
            critiques.push(Critique {
                dimension: CriticismDimension::Risk,
                severity: CriticismSeverity::Warning,
                description: format!("Plan risk level is {:?}.", plan.risk_level),
                suggestion: Some("Consider breaking into smaller, safer steps.".to_string()),
            });
        }

        critiques
    }

    /// Check for efficiency concerns.
    fn check_efficiency(&self, plan: &Plan) -> Vec<Critique> {
        let mut critiques = Vec::new();

        if plan.estimated_cost.estimated_tool_calls > 10 {
            critiques.push(Critique {
                dimension: CriticismDimension::Efficiency,
                severity: CriticismSeverity::Info,
                description: format!(
                    "Plan has {} tool calls — consider batching.",
                    plan.estimated_cost.estimated_tool_calls
                ),
                suggestion: Some("Group related operations to reduce overhead.".to_string()),
            });
        }

        if plan.estimated_cost.estimated_time_ms > 30_000 {
            critiques.push(Critique {
                dimension: CriticismDimension::Efficiency,
                severity: CriticismSeverity::Info,
                description: format!(
                    "Estimated execution time {}ms is high.",
                    plan.estimated_cost.estimated_time_ms
                ),
                suggestion: Some("Consider parallelizing independent steps.".to_string()),
            });
        }

        critiques
    }

    /// Check reversibility of the plan.
    fn check_reversibility(&self, plan: &Plan) -> Vec<Critique> {
        let mut critiques = Vec::new();
        let total = plan.steps.len();
        let with_rollback = plan.steps.iter().filter(|s| s.rollback_action.is_some()).count();

        if total > 0 {
            let ratio = with_rollback as f64 / total as f64;
            if ratio < 0.5 && total > 1 {
                critiques.push(Critique {
                    dimension: CriticismDimension::Reversibility,
                    severity: CriticismSeverity::Info,
                    description: format!(
                        "Only {}/{} steps have rollback actions ({:.0}%).",
                        with_rollback, total, ratio * 100.0
                    ),
                    suggestion: Some(
                        "Consider adding rollback actions for critical steps.".to_string(),
                    ),
                });
            }
        }

        critiques
    }

    /// Check internal consistency.
    fn check_consistency(&self, plan: &Plan) -> Vec<Critique> {
        let mut critiques = Vec::new();

        // Check for circular dependencies
        if self.has_cycle(plan) {
            critiques.push(Critique {
                dimension: CriticismDimension::Consistency,
                severity: CriticismSeverity::Fatal,
                description: "Plan has circular dependencies.".to_string(),
                suggestion: Some("Remove circular step dependencies.".to_string()),
            });
        }

        critiques
    }

    /// Detect cycles in step dependencies using DFS.
    fn has_cycle(&self, plan: &Plan) -> bool {
        use std::collections::{HashMap, HashSet};

        let mut graph: HashMap<uuid::Uuid, Vec<uuid::Uuid>> = HashMap::new();
        for step in &plan.steps {
            graph.insert(step.id, step.depends_on.clone());
        }

        let mut visited = HashSet::new();
        let mut in_stack = HashSet::new();

        for &node in graph.keys() {
            if Self::dfs_cycle(&graph, node, &mut visited, &mut in_stack) {
                return true;
            }
        }
        false
    }

    fn dfs_cycle(
        graph: &std::collections::HashMap<uuid::Uuid, Vec<uuid::Uuid>>,
        node: uuid::Uuid,
        visited: &mut std::collections::HashSet<uuid::Uuid>,
        in_stack: &mut std::collections::HashSet<uuid::Uuid>,
    ) -> bool {
        if in_stack.contains(&node) {
            return true;
        }
        if visited.contains(&node) {
            return false;
        }
        visited.insert(node);
        in_stack.insert(node);

        if let Some(deps) = graph.get(&node) {
            for &dep in deps {
                if Self::dfs_cycle(graph, dep, visited, in_stack) {
                    return true;
                }
            }
        }

        in_stack.remove(&node);
        false
    }
}

impl Default for Critic {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aletheon_abi::body::Action;
    use aletheon_abi::brain::{CostEstimate, PlanStep};
    use aletheon_abi::self_field::RiskLevel;
    use uuid::Uuid;

    fn make_action(name: &str) -> Action {
        Action {
            name: name.to_string(),
            parameters: serde_json::json!({}),
            requires_sandbox: false,
            timeout: None,
        }
    }

    fn make_plan(steps: Vec<PlanStep>) -> Plan {
        Plan {
            id: Uuid::new_v4(),
            steps,
            estimated_cost: CostEstimate::default(),
            risk_level: RiskLevel::Low,
            reasoning: "test".to_string(),
            alternatives: vec![],
        }
    }

    #[test]
    fn empty_plan_is_fatal() {
        let critic = Critic::new();
        let plan = make_plan(vec![]);
        let critiques = critic.critique(&plan);
        assert!(critiques.iter().any(|c| c.severity == CriticismSeverity::Fatal));
        assert!(critiques.iter().any(|c| matches!(c.dimension, CriticismDimension::Completeness)));
    }

    #[test]
    fn destructive_without_rollback_warns() {
        let critic = Critic::new();
        let plan = make_plan(vec![PlanStep {
            id: Uuid::new_v4(),
            action: make_action("file.delete"),
            depends_on: vec![],
            expected_outcome: "deleted".to_string(),
            rollback_action: None,
        }]);
        let critiques = critic.critique(&plan);
        assert!(critiques.iter().any(|c| {
            matches!(c.dimension, CriticismDimension::Risk)
                && c.severity == CriticismSeverity::Warning
        }));
    }

    #[test]
    fn destructive_with_rollback_no_risk_warning() {
        let critic = Critic::new();
        let plan = make_plan(vec![PlanStep {
            id: Uuid::new_v4(),
            action: make_action("file.delete"),
            depends_on: vec![],
            expected_outcome: "deleted".to_string(),
            rollback_action: Some(make_action("file.create")),
        }]);
        let critiques = critic.critique(&plan);
        // Should not have the "no rollback" warning
        assert!(!critiques.iter().any(|c| c.description.contains("no rollback")));
    }

    #[test]
    fn broken_dependency_detected() {
        let critic = Critic::new();
        let plan = make_plan(vec![PlanStep {
            id: Uuid::new_v4(),
            action: make_action("test"),
            depends_on: vec![Uuid::new_v4()], // non-existent dependency
            expected_outcome: "test".to_string(),
            rollback_action: None,
        }]);
        let critiques = critic.critique(&plan);
        assert!(critiques.iter().any(|c| c.description.contains("non-existent")));
    }

    #[test]
    fn cycle_detection() {
        let critic = Critic::new();
        let id_a = Uuid::new_v4();
        let id_b = Uuid::new_v4();
        let plan = make_plan(vec![
            PlanStep {
                id: id_a,
                action: make_action("a"),
                depends_on: vec![id_b],
                expected_outcome: "a".to_string(),
                rollback_action: None,
            },
            PlanStep {
                id: id_b,
                action: make_action("b"),
                depends_on: vec![id_a],
                expected_outcome: "b".to_string(),
                rollback_action: None,
            },
        ]);
        let critiques = critic.critique(&plan);
        assert!(critiques.iter().any(|c| c.description.contains("circular")));
    }

    #[test]
    fn no_cycle_in_linear_plan() {
        let critic = Critic::new();
        let id_a = Uuid::new_v4();
        let id_b = Uuid::new_v4();
        let plan = make_plan(vec![
            PlanStep {
                id: id_a,
                action: make_action("a"),
                depends_on: vec![],
                expected_outcome: "a".to_string(),
                rollback_action: None,
            },
            PlanStep {
                id: id_b,
                action: make_action("b"),
                depends_on: vec![id_a],
                expected_outcome: "b".to_string(),
                rollback_action: None,
            },
        ]);
        let critiques = critic.critique(&plan);
        assert!(!critiques.iter().any(|c| c.description.contains("circular")));
    }

    #[test]
    fn high_tool_calls_suggests_batching() {
        let critic = Critic::new();
        let steps: Vec<PlanStep> = (0..15)
            .map(|i| PlanStep {
                id: Uuid::new_v4(),
                action: make_action(&format!("action_{}", i)),
                depends_on: vec![],
                expected_outcome: "done".to_string(),
                rollback_action: None,
            })
            .collect();
        let mut plan = make_plan(steps);
        plan.estimated_cost.estimated_tool_calls = 15;
        let critiques = critic.critique(&plan);
        assert!(critiques.iter().any(|c| c.description.contains("tool calls")));
    }

    #[test]
    fn good_plan_minimal_critiques() {
        let critic = Critic::new();
        let plan = make_plan(vec![PlanStep {
            id: Uuid::new_v4(),
            action: make_action("file.read"),
            depends_on: vec![],
            expected_outcome: "read file".to_string(),
            rollback_action: None,
        }]);
        let critiques = critic.critique(&plan);
        // Read-only actions should have minimal critiques
        assert!(critiques.iter().all(|c| c.severity <= CriticismSeverity::Info));
    }
}
