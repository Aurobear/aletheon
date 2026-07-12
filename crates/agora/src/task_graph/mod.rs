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

    pub fn set_status(&mut self, id: &str, status: TaskStatus) -> bool {
        if let Some(node) = self.nodes.get_mut(id) {
            node.status = status;
            true
        } else {
            false
        }
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
        // After a is done, b becomes ready.
        assert!(g.set_status("a", TaskStatus::Done));
        let ready: Vec<_> = g.ready().iter().map(|n| n.id.clone()).collect();
        assert_eq!(ready, vec!["b".to_string()]);
    }
}
