// crates/runtime/src/core/react_loop/goal_tracker.rs
use std::collections::HashMap;
use fabric::Clock;
use std::sync::Arc;
use tracing::info;

/// A human-editable spec file that drives agent execution.
///
/// Spec describes the *target state* (not actions). The agent autonomously
/// works toward this state; test results compare actual vs desired and feed
/// deviations back into the spec for iteration.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SpecFile {
    /// The goal: what state the system should reach.
    pub goal: String,
    /// Sub-goals (ordered milestones toward the goal).
    #[serde(default)]
    pub sub_goals: Vec<String>,
    /// Success criteria: verifiable conditions that must hold.
    #[serde(default)]
    pub success_criteria: Vec<String>,
    /// Constraints: hard boundaries the agent must not cross.
    #[serde(default)]
    pub constraints: Vec<String>,
    /// Free-form metadata (author, version, etc.)
    #[serde(default)]
    pub metadata: HashMap<String, String>,
}

impl SpecFile {
    /// Load a spec from a YAML file path.
    pub fn load_from_file(path: &str) -> Result<Self, String> {
        let content = std::fs::read_to_string(path)
            .map_err(|e| format!("Failed to read spec file '{}': {}", path, e))?;
        serde_yaml::from_str(&content)
            .map_err(|e| format!("Failed to parse spec file '{}': {}", path, e))
    }

    /// Serialize the spec to YAML string.
    pub fn to_yaml(&self) -> Result<String, String> {
        serde_yaml::to_string(self).map_err(|e| format!("Failed to serialize spec: {}", e))
    }
}

/// Status of a goal.
#[derive(Debug, Clone, PartialEq)]
pub enum GoalStatus {
    InProgress,
    Completed,
    Failed,
    Adjusted,
}

/// A single goal with metadata.
#[derive(Debug, Clone)]
pub struct Goal {
    pub description: String,
    pub created_at: fabric::MonoTime,
    pub status: GoalStatus,
}

/// A sub-goal under the main goal.
#[derive(Debug, Clone)]
pub struct SubGoal {
    pub description: String,
    pub completed: bool,
}

/// Tracks the current goal and sub-goals for the agent.
pub struct GoalTracker {
    current_goal: Option<Goal>,
    sub_goals: Vec<SubGoal>,
    success_criteria: Vec<String>,
    constraints: Vec<String>,
    spec_source: Option<String>,
    clock: Arc<dyn Clock>,
}

impl std::fmt::Debug for GoalTracker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GoalTracker")
            .field("current_goal", &self.current_goal)
            .field("sub_goals", &self.sub_goals)
            .field("success_criteria", &self.success_criteria)
            .field("constraints", &self.constraints)
            .field("spec_source", &self.spec_source)
            .finish()
    }
}

impl GoalTracker {
    /// Create a new empty goal tracker.
    pub fn new(clock: Arc<dyn Clock>) -> Self {
        Self {
            current_goal: None,
            sub_goals: Vec::new(),
            success_criteria: Vec::new(),
            constraints: Vec::new(),
            spec_source: None,
            clock,
        }
    }
}

impl Default for GoalTracker {
    fn default() -> Self {
        Self::new(Arc::new(aletheon_kernel::chronos::SystemClock::new()))
    }
}

impl GoalTracker {
    /// Set the main goal for this turn.
    pub fn set_goal(&mut self, goal: String) {
        info!(goal = %goal, "Setting agent goal");
        self.current_goal = Some(Goal {
            description: goal,
            created_at: self.clock.mono_now(),
            status: GoalStatus::InProgress,
        });
    }

    /// Add a sub-goal.
    pub fn add_sub_goal(&mut self, sub_goal: String) {
        if self.sub_goals.len() < 3 {
            self.sub_goals.push(SubGoal {
                description: sub_goal,
                completed: false,
            });
        }
    }

    /// Add a success criterion.
    pub fn add_success_criterion(&mut self, criterion: String) {
        self.success_criteria.push(criterion);
    }

    /// Mark a sub-goal as completed.
    pub fn complete_sub_goal(&mut self, index: usize) {
        if let Some(sg) = self.sub_goals.get_mut(index) {
            sg.completed = true;
            info!(sub_goal = %sg.description, "Sub-goal completed");
        }
    }

    /// Mark the main goal as completed.
    pub fn complete_goal(&mut self) {
        if let Some(ref mut goal) = self.current_goal {
            goal.status = GoalStatus::Completed;
            info!(goal = %goal.description, "Goal completed");
        }
    }

    /// Mark the main goal as failed.
    pub fn fail_goal(&mut self, reason: &str) {
        if let Some(ref mut goal) = self.current_goal {
            goal.status = GoalStatus::Failed;
            info!(goal = %goal.description, reason = %reason, "Goal failed");
        }
    }

    /// Check if the goal is complete.
    pub fn is_complete(&self) -> bool {
        self.current_goal
            .as_ref()
            .map(|g| g.status == GoalStatus::Completed)
            .unwrap_or(false)
    }

    /// Check if all sub-goals are complete.
    pub fn all_sub_goals_complete(&self) -> bool {
        !self.sub_goals.is_empty() && self.sub_goals.iter().all(|sg| sg.completed)
    }

    /// Get the current goal description, if set.
    pub fn current_goal_description(&self) -> Option<String> {
        self.current_goal.as_ref().map(|g| g.description.clone())
    }

    /// Get context string for LLM reasoning.
    pub fn get_context(&self) -> String {
        let mut parts = Vec::new();

        if let Some(ref goal) = self.current_goal {
            parts.push(format!("Current goal: {}", goal.description));
        }

        if !self.sub_goals.is_empty() {
            let sub_goal_strs: Vec<String> = self
                .sub_goals
                .iter()
                .enumerate()
                .map(|(i, sg)| {
                    let status = if sg.completed { "done" } else { "pending" };
                    format!("  [{}] {}. {}", status, i + 1, sg.description)
                })
                .collect();
            parts.push(format!("Sub-goals:\n{}", sub_goal_strs.join("\n")));
        }

        if !self.success_criteria.is_empty() {
            parts.push(format!(
                "Success criteria: {}",
                self.success_criteria.join(", ")
            ));
        }

        if !self.constraints.is_empty() {
            parts.push(format!(
                "Constraints (MUST NOT violate):\n{}",
                self.constraints
                    .iter()
                    .enumerate()
                    .map(|(i, c)| format!("  {}. {}", i + 1, c))
                    .collect::<Vec<_>>()
                    .join("\n")
            ));
        }

        parts.join("\n")
    }

    /// Get constraints (hard boundaries the agent must not cross).
    pub fn get_constraints(&self) -> &[String] {
        &self.constraints
    }

    /// Load a spec file into the goal tracker.
    pub fn load_spec(&mut self, spec: SpecFile, source_path: Option<String>) {
        info!(goal = %spec.goal, "Loading spec into goal tracker");
        self.set_goal(spec.goal);
        for sg in spec.sub_goals {
            self.add_sub_goal(sg);
        }
        for sc in spec.success_criteria {
            self.add_success_criterion(sc);
        }
        self.constraints = spec.constraints;
        self.spec_source = source_path;
    }

    /// Load a spec from a YAML file.
    pub fn load_spec_from_file(&mut self, path: &str) -> Result<(), String> {
        let spec = SpecFile::load_from_file(path)?;
        self.load_spec(spec, Some(path.to_string()));
        Ok(())
    }

    /// Export current tracker state as a SpecFile (for spec feedback/iteration).
    pub fn to_spec(&self) -> SpecFile {
        SpecFile {
            goal: self
                .current_goal
                .as_ref()
                .map(|g| g.description.clone())
                .unwrap_or_default(),
            sub_goals: self
                .sub_goals
                .iter()
                .map(|sg| sg.description.clone())
                .collect(),
            success_criteria: self.success_criteria.clone(),
            constraints: self.constraints.clone(),
            metadata: HashMap::new(),
        }
    }

    /// Write the current spec back to its source file (spec feedback loop).
    pub fn write_spec_back(&self) -> Result<(), String> {
        if let Some(ref path) = self.spec_source {
            let spec = self.to_spec();
            let yaml = spec.to_yaml()?;
            std::fs::write(path, yaml)
                .map_err(|e| format!("Failed to write spec back to '{}': {}", path, e))?;
            info!(path = path.as_str(), "Spec written back to file");
            Ok(())
        } else {
            Err("No spec source path set".to_string())
        }
    }

    /// Reset for a new turn. Preserves spec_source for cross-turn persistence.
    pub fn reset(&mut self) {
        self.current_goal = None;
        self.sub_goals.clear();
        self.success_criteria.clear();
        self.constraints.clear();
        // Note: spec_source is preserved across turns
    }

    /// Seed the tracker from a persisted objective.
    ///
    /// Used to resume a cross-session objective on daemon start.
    /// Call exactly once, before the first turn. `reset()` semantics
    /// (clearing goal/sub-goals/criteria/constraints, preserving spec_source)
    /// are unchanged for subsequent turns.
    pub fn hydrate_from(&mut self, description: &str, sub_goals: &[String]) {
        self.set_goal(description.to_string());
        for sg in sub_goals {
            self.add_sub_goal(sg.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_tracker() -> GoalTracker {
        GoalTracker::new(Arc::new(aletheon_kernel::chronos::TestClock::default()))
    }

    #[test]
    fn test_goal_setting() {
        let mut tracker = make_tracker();
        assert!(!tracker.is_complete());

        tracker.set_goal("Create a hello world program".into());
        assert!(!tracker.is_complete());

        tracker.complete_goal();
        assert!(tracker.is_complete());
    }

    #[test]
    fn test_sub_goals() {
        let mut tracker = make_tracker();
        tracker.set_goal("Build a website".into());
        tracker.add_sub_goal("Create HTML file".into());
        tracker.add_sub_goal("Add CSS styling".into());
        tracker.add_sub_goal("Add JavaScript".into());

        assert!(!tracker.all_sub_goals_complete());

        tracker.complete_sub_goal(0);
        tracker.complete_sub_goal(1);
        tracker.complete_sub_goal(2);

        assert!(tracker.all_sub_goals_complete());
    }

    #[test]
    fn test_max_sub_goals() {
        let mut tracker = make_tracker();
        tracker.add_sub_goal("1".into());
        tracker.add_sub_goal("2".into());
        tracker.add_sub_goal("3".into());
        tracker.add_sub_goal("4".into()); // Should be ignored

        assert_eq!(tracker.sub_goals.len(), 3);
    }

    #[test]
    fn test_context_generation() {
        let mut tracker = make_tracker();
        tracker.set_goal("Write tests".into());
        tracker.add_sub_goal("Unit tests".into());
        tracker.add_success_criterion("All tests pass".into());

        let ctx = tracker.get_context();
        assert!(ctx.contains("Write tests"));
        assert!(ctx.contains("Unit tests"));
        assert!(ctx.contains("All tests pass"));
    }

    #[test]
    fn test_reset() {
        let mut tracker = make_tracker();
        tracker.set_goal("test".into());
        tracker.add_sub_goal("sub".into());

        tracker.reset();
        assert!(!tracker.is_complete());
        assert!(tracker.sub_goals.is_empty());
    }

    #[test]
    fn test_spec_file_roundtrip() {
        let yaml = r#"
goal: "Implement auth"
sub_goals:
  - "Create middleware"
  - "Add login endpoint"
success_criteria:
  - "All tests pass"
constraints:
  - "Do not modify user schema"
metadata:
  author: "test"
"#;
        let spec: SpecFile = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(spec.goal, "Implement auth");
        assert_eq!(spec.sub_goals.len(), 2);
        assert_eq!(spec.success_criteria.len(), 1);
        assert_eq!(spec.constraints.len(), 1);
        assert_eq!(spec.metadata["author"], "test");

        // Roundtrip
        let serialized = spec.to_yaml().unwrap();
        let spec2: SpecFile = serde_yaml::from_str(&serialized).unwrap();
        assert_eq!(spec2.goal, spec.goal);
    }

    #[test]
    fn test_load_spec() {
        let mut tracker = make_tracker();
        let spec = SpecFile {
            goal: "Build API".into(),
            sub_goals: vec!["Design schema".into(), "Implement endpoints".into()],
            success_criteria: vec!["All endpoints return 200".into()],
            constraints: vec!["No breaking changes".into()],
            metadata: HashMap::new(),
        };

        tracker.load_spec(spec, None);
        assert_eq!(tracker.current_goal_description(), Some("Build API".into()));
        assert_eq!(tracker.sub_goals.len(), 2);
        assert_eq!(tracker.success_criteria.len(), 1);
        assert_eq!(tracker.get_constraints().len(), 1);
        assert_eq!(tracker.get_constraints()[0], "No breaking changes");
    }

    #[test]
    fn test_to_spec() {
        let mut tracker = make_tracker();
        tracker.set_goal("Deploy service".into());
        tracker.add_sub_goal("Build image".into());
        tracker.add_success_criterion("Health check passes".into());

        let spec = tracker.to_spec();
        assert_eq!(spec.goal, "Deploy service");
        assert_eq!(spec.sub_goals, vec!["Build image"]);
        assert_eq!(spec.success_criteria, vec!["Health check passes"]);
    }

    #[test]
    fn test_constraints_in_context() {
        let mut tracker = make_tracker();
        tracker.set_goal("Refactor module".into());
        tracker.add_success_criterion("Tests pass".into());

        // No constraints — should not appear
        let ctx = tracker.get_context();
        assert!(!ctx.contains("Constraints"));

        // With constraints
        let spec = SpecFile {
            goal: "Refactor module".into(),
            sub_goals: vec![],
            success_criteria: vec!["Tests pass".into()],
            constraints: vec!["Keep API stable".into(), "No new deps".into()],
            metadata: HashMap::new(),
        };
        tracker.load_spec(spec, None);

        let ctx = tracker.get_context();
        assert!(ctx.contains("Constraints"));
        assert!(ctx.contains("Keep API stable"));
        assert!(ctx.contains("No new deps"));
    }

    #[test]
    fn test_reset_preserves_spec_source() {
        let mut tracker = make_tracker();
        let spec = SpecFile {
            goal: "Test".into(),
            sub_goals: vec![],
            success_criteria: vec![],
            constraints: vec!["C1".into()],
            metadata: HashMap::new(),
        };
        tracker.load_spec(spec, Some("/tmp/test.spec.yaml".into()));

        tracker.reset();
        assert!(tracker.spec_source.is_some());
        assert!(tracker.constraints.is_empty()); // cleared on reset
    }

    #[test]
    fn hydrate_from_persisted_objective() {
        let mut tracker = make_tracker();
        tracker.hydrate_from(
            "ship goal layer",
            &["persist store".to_string(), "wire rpc".to_string()],
        );
        assert_eq!(
            tracker.current_goal_description(),
            Some("ship goal layer".into())
        );
        let ctx = tracker.get_context();
        assert!(ctx.contains("persist store"));
        assert!(ctx.contains("wire rpc"));

        // reset clears the hydrated goal
        tracker.reset();
        assert!(tracker.current_goal_description().is_none());
    }
}
