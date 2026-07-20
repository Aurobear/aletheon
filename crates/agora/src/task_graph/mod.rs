//! Task graph — sub-task nodes, dependencies, and status (RFC-014).

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TaskStatus {
    Pending,
    Running,
    Done,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaskTransitionError {
    TaskNotFound {
        id: String,
    },
    DependenciesNotDone {
        id: String,
        dependencies: Vec<String>,
    },
    InvalidTransition {
        id: String,
        from: TaskStatus,
        to: TaskStatus,
    },
}

impl std::fmt::Display for TaskTransitionError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TaskNotFound { id } => write!(formatter, "task '{id}' was not found"),
            Self::DependenciesNotDone { id, dependencies } => write!(
                formatter,
                "task '{id}' cannot run until dependencies are done: {}",
                dependencies.join(", ")
            ),
            Self::InvalidTransition { id, from, to } => {
                write!(
                    formatter,
                    "task '{id}' cannot transition from {from:?} to {to:?}"
                )
            }
        }
    }
}

impl std::error::Error for TaskTransitionError {}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskNode {
    pub id: String,
    pub description: String,
    pub status: TaskStatus,
    /// IDs of tasks that must complete before this one.
    pub deps: Vec<String>,
}

/// A directed task graph keyed by task id.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TaskGraph {
    nodes: HashMap<String, TaskNode>,
}

impl TaskGraph {
    pub fn new() -> Self {
        Self {
            nodes: HashMap::new(),
        }
    }

    pub fn add(
        &mut self,
        id: impl Into<String>,
        description: impl Into<String>,
        deps: Vec<String>,
    ) {
        let id = id.into();
        self.nodes.insert(
            id.clone(),
            TaskNode {
                id,
                description: description.into(),
                status: TaskStatus::Pending,
                deps,
            },
        );
    }

    /// Validate without mutation. Repeating the current status is idempotently
    /// valid; terminal states cannot be reopened.
    pub fn validate_transition(
        &self,
        id: &str,
        status: &TaskStatus,
    ) -> Result<(), TaskTransitionError> {
        let current = self
            .nodes
            .get(id)
            .ok_or_else(|| TaskTransitionError::TaskNotFound { id: id.to_owned() })?
            .status
            .clone();
        if &current == status {
            return Ok(());
        }
        let legal = matches!(
            (&current, status),
            (TaskStatus::Pending, TaskStatus::Running)
                | (TaskStatus::Pending, TaskStatus::Done)
                | (TaskStatus::Pending, TaskStatus::Failed)
                | (TaskStatus::Running, TaskStatus::Done)
                | (TaskStatus::Running, TaskStatus::Failed)
        );
        if !legal {
            return Err(TaskTransitionError::InvalidTransition {
                id: id.to_owned(),
                from: current,
                to: status.clone(),
            });
        }
        if *status == TaskStatus::Running {
            let mut dependencies = self.nodes[id]
                .deps
                .iter()
                .filter(|dependency| {
                    self.nodes
                        .get(*dependency)
                        .is_none_or(|node| node.status != TaskStatus::Done)
                })
                .cloned()
                .collect::<Vec<_>>();
            dependencies.sort();
            dependencies.dedup();
            if !dependencies.is_empty() {
                return Err(TaskTransitionError::DependenciesNotDone {
                    id: id.to_owned(),
                    dependencies,
                });
            }
        }
        Ok(())
    }

    pub fn transition(&mut self, id: &str, status: TaskStatus) -> Result<(), TaskTransitionError> {
        self.validate_transition(id, &status)?;
        self.nodes
            .get_mut(id)
            .expect("task existence validated")
            .status = status;
        Ok(())
    }

    /// Compatibility adapter. Invalid and missing transitions both return
    /// `false`; new callers should use [`TaskGraph::transition`] for typed errors.
    pub fn set_status(&mut self, id: &str, status: TaskStatus) -> bool {
        self.transition(id, status).is_ok()
    }

    pub fn get(&self, id: &str) -> Option<&TaskNode> {
        self.nodes.get(id)
    }

    /// Tasks whose dependencies are all `Done` and are still `Pending`.
    pub fn ready(&self) -> Vec<&TaskNode> {
        self.nodes
            .values()
            .filter(|n| n.status == TaskStatus::Pending)
            .filter(|n| {
                n.deps.iter().all(|d| {
                    self.nodes
                        .get(d)
                        .map(|dn| dn.status == TaskStatus::Done)
                        .unwrap_or(false)
                })
            })
            .collect()
    }

    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ready_respects_deps() {
        let mut g = TaskGraph::new();
        g.add("a", "first", vec![]);
        g.add("b", "second", vec!["a".into()]);
        // Only `a` is ready initially.
        let ready: Vec<_> = g.ready().iter().map(|n| n.id.clone()).collect();
        assert_eq!(ready, vec!["a".to_string()]);
        // After a runs and completes, b becomes ready.
        assert!(g.set_status("a", TaskStatus::Running));
        assert!(g.set_status("a", TaskStatus::Done));
        let ready: Vec<_> = g.ready().iter().map(|n| n.id.clone()).collect();
        assert_eq!(ready, vec!["b".to_string()]);
    }

    #[test]
    fn legacy_set_status_preserves_forward_direct_completion() {
        let mut graph = TaskGraph::new();
        graph.add("a", "task", vec![]);
        assert!(graph.set_status("a", TaskStatus::Done));
        assert_eq!(graph.get("a").unwrap().status, TaskStatus::Done);
        assert!(!graph.set_status("a", TaskStatus::Pending));
    }

    #[test]
    fn running_requires_all_dependencies_done() {
        let mut graph = TaskGraph::new();
        graph.add("a", "dependency", vec![]);
        graph.add("b", "dependent", vec!["missing".into(), "a".into()]);
        assert_eq!(
            graph.transition("b", TaskStatus::Running),
            Err(TaskTransitionError::DependenciesNotDone {
                id: "b".into(),
                dependencies: vec!["a".into(), "missing".into()],
            })
        );
        assert_eq!(graph.get("b").unwrap().status, TaskStatus::Pending);
    }

    #[test]
    fn terminal_states_cannot_regress() {
        let mut graph = TaskGraph::new();
        graph.add("a", "task", vec![]);
        graph.transition("a", TaskStatus::Running).unwrap();
        graph.transition("a", TaskStatus::Done).unwrap();
        assert!(matches!(
            graph.transition("a", TaskStatus::Pending),
            Err(TaskTransitionError::InvalidTransition {
                from: TaskStatus::Done,
                to: TaskStatus::Pending,
                ..
            })
        ));
        assert_eq!(graph.get("a").unwrap().status, TaskStatus::Done);
    }

    #[test]
    fn same_status_is_idempotent_and_missing_task_is_typed() {
        let mut graph = TaskGraph::new();
        graph.add("a", "task", vec![]);
        assert_eq!(graph.transition("a", TaskStatus::Pending), Ok(()));
        assert_eq!(
            graph.transition("missing", TaskStatus::Running),
            Err(TaskTransitionError::TaskNotFound {
                id: "missing".into()
            })
        );
        assert!(!graph.set_status("missing", TaskStatus::Running));
    }
}
