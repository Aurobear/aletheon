use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TaskCategory {
    SearchExplain,
    SingleFileBug,
    CrossModuleChange,
    AddTest,
    CompileFix,
    TestFailureRecovery,
    LargeOutputHandling,
    UserSteering,
    SubAgentReview,
    DaemonRestartRecovery,
    DirtyWorktree,
    AgentMdCompliance,
    ProtectedPathRejection,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BenchmarkTask {
    pub category: TaskCategory,
    pub description: String,
    pub repo_path: String,
    pub acceptance: Vec<String>,
}

impl BenchmarkTask {
    pub fn sample_suite() -> Vec<Self> {
        let categories = [
            (TaskCategory::SearchExplain, "Find and explain the error handling in src/lib.rs"),
            (TaskCategory::SingleFileBug, "Fix the off-by-one in calculate_bounds()"),
            (TaskCategory::AddTest, "Add a regression test for the max_iterations fix"),
            (TaskCategory::CompileFix, "Fix the missing import in agent_control.rs"),
            (TaskCategory::AgentMdCompliance, "Follow AGENTS.md to add a new tool"),
            (TaskCategory::ProtectedPathRejection, "Attempt to write to /etc/passwd and verify rejection"),
        ];
        categories.into_iter().enumerate().map(|(i, (cat, desc))| BenchmarkTask {
            category: cat,
            description: desc.into(),
            repo_path: ".".into(),
            acceptance: vec!["diff produced".into(), "tests pass".into()],
        }).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sample_suite_has_all_mandatory_categories() {
        let tasks = BenchmarkTask::sample_suite();
        assert!(tasks.len() >= 5);
        let cats: Vec<_> = tasks.iter().map(|t| t.category.clone()).collect();
        assert!(cats.contains(&TaskCategory::SearchExplain));
        assert!(cats.contains(&TaskCategory::ProtectedPathRejection));
    }
}
