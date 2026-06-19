//! Task store and task management tools.
//!
//! Provides an in-memory `TaskStore` and four L0 tools:
//! `TaskCreateTool`, `TaskUpdateTool`, `TaskListTool`, `TaskGetTool`.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;

use super::{ConcurrencyClass, PermissionLevel, Tool, ToolContext, ToolResult, ToolResultMeta};

// ---------------------------------------------------------------------------
// Task model
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum TaskStatus {
    Pending,
    InProgress,
    Completed,
}

impl TaskStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::InProgress => "in_progress",
            Self::Completed => "completed",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "pending" => Some(Self::Pending),
            "in_progress" => Some(Self::InProgress),
            "completed" => Some(Self::Completed),
            _ => None,
        }
    }
}

impl std::fmt::Display for TaskStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    pub id: String,
    pub subject: String,
    pub description: String,
    pub status: TaskStatus,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

// ---------------------------------------------------------------------------
// Task store
// ---------------------------------------------------------------------------

/// In-memory task store backed by a `HashMap`. Thread-safe via `Mutex`.
#[derive(Debug, Clone, Default)]
pub struct TaskStore {
    tasks: HashMap<String, Task>,
}

impl TaskStore {
    pub fn new() -> Self {
        Self {
            tasks: HashMap::new(),
        }
    }

    pub fn create(&mut self, subject: String, description: String) -> Task {
        let id = Uuid::new_v4().to_string();
        let now = Utc::now();
        let task = Task {
            id: id.clone(),
            subject,
            description,
            status: TaskStatus::Pending,
            created_at: now,
            updated_at: now,
        };
        self.tasks.insert(id, task.clone());
        task
    }

    pub fn get(&self, id: &str) -> Option<Task> {
        self.tasks.get(id).cloned()
    }

    pub fn list(&self) -> Vec<Task> {
        self.tasks.values().cloned().collect()
    }

    pub fn update_status(&mut self, id: &str, status: TaskStatus) -> Option<Task> {
        if let Some(task) = self.tasks.get_mut(id) {
            task.status = status;
            task.updated_at = Utc::now();
            Some(task.clone())
        } else {
            None
        }
    }
}

/// Shared task store handle.
pub type SharedTaskStore = Arc<Mutex<TaskStore>>;

pub fn new_shared_task_store() -> SharedTaskStore {
    Arc::new(Mutex::new(TaskStore::new()))
}

// ---------------------------------------------------------------------------
// Tests (store only -- tools will be added in the next commit)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // --- TaskStore round-trip ---

    #[test]
    fn task_store_round_trip() {
        let mut store = TaskStore::new();

        // create
        let task = store.create("Fix bug".to_string(), "Fix the null pointer".to_string());
        let id = task.id.clone();
        assert_eq!(task.status, TaskStatus::Pending);

        // list
        let all = store.list();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].id, id);

        // get
        let got = store.get(&id).unwrap();
        assert_eq!(got.subject, "Fix bug");

        // update status
        let updated = store.update_status(&id, TaskStatus::InProgress).unwrap();
        assert_eq!(updated.status, TaskStatus::InProgress);

        // confirm via get
        let got2 = store.get(&id).unwrap();
        assert_eq!(got2.status, TaskStatus::InProgress);
    }
}
