// crates/runtime/src/core/react_loop/goal_tracker.rs
use std::time::Instant;
use tracing::info;

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
    pub created_at: Instant,
    pub status: GoalStatus,
}

/// A sub-goal under the main goal.
#[derive(Debug, Clone)]
pub struct SubGoal {
    pub description: String,
    pub completed: bool,
}

/// Tracks the current goal and sub-goals for the agent.
#[derive(Debug)]
pub struct GoalTracker {
    current_goal: Option<Goal>,
    sub_goals: Vec<SubGoal>,
    success_criteria: Vec<String>,
}

impl GoalTracker {
    /// Create a new empty goal tracker.
    pub fn new() -> Self {
        Self {
            current_goal: None,
            sub_goals: Vec::new(),
            success_criteria: Vec::new(),
        }
    }

    /// Set the main goal for this turn.
    pub fn set_goal(&mut self, goal: String) {
        info!(goal = %goal, "Setting agent goal");
        self.current_goal = Some(Goal {
            description: goal,
            created_at: Instant::now(),
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

        parts.join("\n")
    }

    /// Reset for a new turn.
    pub fn reset(&mut self) {
        self.current_goal = None;
        self.sub_goals.clear();
        self.success_criteria.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_goal_setting() {
        let mut tracker = GoalTracker::new();
        assert!(!tracker.is_complete());

        tracker.set_goal("Create a hello world program".into());
        assert!(!tracker.is_complete());

        tracker.complete_goal();
        assert!(tracker.is_complete());
    }

    #[test]
    fn test_sub_goals() {
        let mut tracker = GoalTracker::new();
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
        let mut tracker = GoalTracker::new();
        tracker.add_sub_goal("1".into());
        tracker.add_sub_goal("2".into());
        tracker.add_sub_goal("3".into());
        tracker.add_sub_goal("4".into()); // Should be ignored

        assert_eq!(tracker.sub_goals.len(), 3);
    }

    #[test]
    fn test_context_generation() {
        let mut tracker = GoalTracker::new();
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
        let mut tracker = GoalTracker::new();
        tracker.set_goal("test".into());
        tracker.add_sub_goal("sub".into());

        tracker.reset();
        assert!(!tracker.is_complete());
        assert!(tracker.sub_goals.is_empty());
    }
}
