//! Task store and task management tools.
//!
//! Provides an in-memory `TaskStore` and four L0 tools:
//! `TaskCreateTool`, `TaskUpdateTool`, `TaskListTool`, `TaskGetTool`.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use fabric::wall_to_datetime;
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

    #[allow(clippy::should_implement_trait)]
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

    pub fn create(&mut self, subject: String, description: String, now: DateTime<Utc>) -> Task {
        let id = Uuid::new_v4().to_string();
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

    pub fn update_status(
        &mut self,
        id: &str,
        status: TaskStatus,
        now: DateTime<Utc>,
    ) -> Option<Task> {
        if let Some(task) = self.tasks.get_mut(id) {
            task.status = status;
            task.updated_at = now;
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
// Tools
// ---------------------------------------------------------------------------

pub struct TaskCreateTool {
    store: SharedTaskStore,
}

impl TaskCreateTool {
    pub fn new(store: SharedTaskStore) -> Self {
        Self { store }
    }
}

#[async_trait]
impl Tool for TaskCreateTool {
    fn name(&self) -> &str {
        "task_create"
    }

    fn description(&self) -> &str {
        "Create a new task with subject and description"
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "subject": {
                    "type": "string",
                    "description": "Short task subject"
                },
                "description": {
                    "type": "string",
                    "description": "Detailed task description"
                }
            },
            "required": ["subject", "description"]
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::L0
    }

    fn boxed_clone(&self) -> Box<dyn Tool> {
        Box::new(TaskCreateTool {
            store: Arc::clone(&self.store),
        })
    }

    fn concurrency_class(&self) -> ConcurrencyClass {
        ConcurrencyClass::SideEffect
    }

    async fn execute(&self, input: serde_json::Value, ctx: &ToolContext) -> ToolResult {
        let start = ctx.clock.mono_now();

        let subject = match input["subject"].as_str() {
            Some(s) if !s.is_empty() => s.to_string(),
            _ => {
                return ToolResult {
                    content: "Missing or empty 'subject'".to_string(),
                    is_error: true,
                    metadata: ToolResultMeta {
                        execution_time_ms: ctx.clock.mono_now().0.saturating_sub(start.0),
                        truncated: false,
                    },
                };
            }
        };

        let description = input["description"].as_str().unwrap_or("").to_string();

        let now = wall_to_datetime(ctx.clock.wall_now());
        let task = self.store.lock().create(subject, description, now);

        ToolResult {
            content: serde_json::to_string_pretty(&task).unwrap_or_default(),
            is_error: false,
            metadata: ToolResultMeta {
                execution_time_ms: ctx.clock.mono_now().0.saturating_sub(start.0),
                truncated: false,
            },
        }
    }
}

pub struct TaskUpdateTool {
    store: SharedTaskStore,
}

impl TaskUpdateTool {
    pub fn new(store: SharedTaskStore) -> Self {
        Self { store }
    }
}

#[async_trait]
impl Tool for TaskUpdateTool {
    fn name(&self) -> &str {
        "task_update"
    }

    fn description(&self) -> &str {
        "Update the status of an existing task"
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "id": {
                    "type": "string",
                    "description": "Task ID"
                },
                "status": {
                    "type": "string",
                    "enum": ["pending", "in_progress", "completed"],
                    "description": "New status value"
                }
            },
            "required": ["id", "status"]
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::L0
    }

    fn boxed_clone(&self) -> Box<dyn Tool> {
        Box::new(TaskUpdateTool {
            store: Arc::clone(&self.store),
        })
    }

    fn concurrency_class(&self) -> ConcurrencyClass {
        ConcurrencyClass::SideEffect
    }

    async fn execute(&self, input: serde_json::Value, ctx: &ToolContext) -> ToolResult {
        let start = ctx.clock.mono_now();

        let id = match input["id"].as_str() {
            Some(s) if !s.is_empty() => s,
            _ => {
                return ToolResult {
                    content: "Missing or empty 'id'".to_string(),
                    is_error: true,
                    metadata: ToolResultMeta {
                        execution_time_ms: ctx.clock.mono_now().0.saturating_sub(start.0),
                        truncated: false,
                    },
                };
            }
        };

        let status_str = match input["status"].as_str() {
            Some(s) => s,
            _ => {
                return ToolResult {
                    content: "Missing 'status'".to_string(),
                    is_error: true,
                    metadata: ToolResultMeta {
                        execution_time_ms: ctx.clock.mono_now().0.saturating_sub(start.0),
                        truncated: false,
                    },
                };
            }
        };

        let status = match TaskStatus::from_str(status_str) {
            Some(s) => s,
            None => {
                return ToolResult {
                    content: format!(
                        "Invalid status '{}', expected: pending, in_progress, completed",
                        status_str
                    ),
                    is_error: true,
                    metadata: ToolResultMeta {
                        execution_time_ms: ctx.clock.mono_now().0.saturating_sub(start.0),
                        truncated: false,
                    },
                };
            }
        };

        match self
            .store
            .lock()
            .update_status(id, status, wall_to_datetime(ctx.clock.wall_now()))
        {
            Some(task) => ToolResult {
                content: serde_json::to_string_pretty(&task).unwrap_or_default(),
                is_error: false,
                metadata: ToolResultMeta {
                    execution_time_ms: ctx.clock.mono_now().0.saturating_sub(start.0),
                    truncated: false,
                },
            },
            None => ToolResult {
                content: format!("Task not found: {}", id),
                is_error: true,
                metadata: ToolResultMeta {
                    execution_time_ms: ctx.clock.mono_now().0.saturating_sub(start.0),
                    truncated: false,
                },
            },
        }
    }
}

pub struct TaskListTool {
    store: SharedTaskStore,
}

impl TaskListTool {
    pub fn new(store: SharedTaskStore) -> Self {
        Self { store }
    }
}

#[async_trait]
impl Tool for TaskListTool {
    fn name(&self) -> &str {
        "task_list"
    }

    fn description(&self) -> &str {
        "List all tasks in the store"
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {}
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::L0
    }

    fn boxed_clone(&self) -> Box<dyn Tool> {
        Box::new(TaskListTool {
            store: Arc::clone(&self.store),
        })
    }

    fn concurrency_class(&self) -> ConcurrencyClass {
        ConcurrencyClass::ReadOnly
    }

    async fn execute(&self, _input: serde_json::Value, ctx: &ToolContext) -> ToolResult {
        let start = ctx.clock.mono_now();

        let tasks = self.store.lock().list();

        ToolResult {
            content: serde_json::to_string_pretty(&tasks).unwrap_or_default(),
            is_error: false,
            metadata: ToolResultMeta {
                execution_time_ms: ctx.clock.mono_now().0.saturating_sub(start.0),
                truncated: false,
            },
        }
    }
}

pub struct TaskGetTool {
    store: SharedTaskStore,
}

impl TaskGetTool {
    pub fn new(store: SharedTaskStore) -> Self {
        Self { store }
    }
}

#[async_trait]
impl Tool for TaskGetTool {
    fn name(&self) -> &str {
        "task_get"
    }

    fn description(&self) -> &str {
        "Get a single task by ID"
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "id": {
                    "type": "string",
                    "description": "Task ID"
                }
            },
            "required": ["id"]
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::L0
    }

    fn boxed_clone(&self) -> Box<dyn Tool> {
        Box::new(TaskGetTool {
            store: Arc::clone(&self.store),
        })
    }

    fn concurrency_class(&self) -> ConcurrencyClass {
        ConcurrencyClass::ReadOnly
    }

    async fn execute(&self, input: serde_json::Value, ctx: &ToolContext) -> ToolResult {
        let start = ctx.clock.mono_now();

        let id = match input["id"].as_str() {
            Some(s) if !s.is_empty() => s,
            _ => {
                return ToolResult {
                    content: "Missing or empty 'id'".to_string(),
                    is_error: true,
                    metadata: ToolResultMeta {
                        execution_time_ms: ctx.clock.mono_now().0.saturating_sub(start.0),
                        truncated: false,
                    },
                };
            }
        };

        match self.store.lock().get(id) {
            Some(task) => ToolResult {
                content: serde_json::to_string_pretty(&task).unwrap_or_default(),
                is_error: false,
                metadata: ToolResultMeta {
                    execution_time_ms: ctx.clock.mono_now().0.saturating_sub(start.0),
                    truncated: false,
                },
            },
            None => ToolResult {
                content: format!("Task not found: {}", id),
                is_error: true,
                metadata: ToolResultMeta {
                    execution_time_ms: ctx.clock.mono_now().0.saturating_sub(start.0),
                    truncated: false,
                },
            },
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn ctx() -> ToolContext {
        ToolContext {
            working_dir: PathBuf::from("/tmp"),
            session_id: "test".to_string(),
            clock: std::sync::Arc::new(aletheon_kernel::chronos::TestClock::default()),
        }
    }

    // --- TaskStore round-trip ---

    #[test]
    fn task_store_round_trip() {
        let mut store = TaskStore::new();

        // create
        let task = store.create(
            "Fix bug".to_string(),
            "Fix the null pointer".to_string(),
            Utc::now(),
        );
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
        let updated = store
            .update_status(&id, TaskStatus::InProgress, Utc::now())
            .unwrap();
        assert_eq!(updated.status, TaskStatus::InProgress);

        // confirm via get
        let got2 = store.get(&id).unwrap();
        assert_eq!(got2.status, TaskStatus::InProgress);
    }

    // --- Tool integration tests ---

    #[tokio::test]
    async fn task_create_tool_returns_id() {
        let store = new_shared_task_store();
        let tool = TaskCreateTool::new(Arc::clone(&store));

        let result = tool
            .execute(json!({"subject": "Test", "description": "desc"}), &ctx())
            .await;

        assert!(!result.is_error);
        let task: Task = serde_json::from_str(&result.content).unwrap();
        assert_eq!(task.subject, "Test");
        assert_eq!(task.status, TaskStatus::Pending);
    }

    #[tokio::test]
    async fn task_list_tool_shows_created() {
        let store = new_shared_task_store();
        store
            .lock()
            .create("A".to_string(), "".to_string(), Utc::now());

        let tool = TaskListTool::new(Arc::clone(&store));
        let result = tool.execute(json!({}), &ctx()).await;

        assert!(!result.is_error);
        let tasks: Vec<Task> = serde_json::from_str(&result.content).unwrap();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].subject, "A");
    }

    #[tokio::test]
    async fn task_update_tool_flips_status() {
        let store = new_shared_task_store();
        let task = store
            .lock()
            .create("B".to_string(), "".to_string(), Utc::now());
        let id = task.id.clone();

        let tool = TaskUpdateTool::new(Arc::clone(&store));
        let result = tool
            .execute(json!({"id": id, "status": "completed"}), &ctx())
            .await;

        assert!(!result.is_error);
        let updated: Task = serde_json::from_str(&result.content).unwrap();
        assert_eq!(updated.status, TaskStatus::Completed);
    }

    #[tokio::test]
    async fn task_get_tool_reflects_update() {
        let store = new_shared_task_store();
        let task = store
            .lock()
            .create("C".to_string(), "".to_string(), Utc::now());
        let id = task.id.clone();

        store
            .lock()
            .update_status(&id, TaskStatus::InProgress, Utc::now());

        let tool = TaskGetTool::new(Arc::clone(&store));
        let result = tool.execute(json!({"id": id}), &ctx()).await;

        assert!(!result.is_error);
        let got: Task = serde_json::from_str(&result.content).unwrap();
        assert_eq!(got.status, TaskStatus::InProgress);
    }

    #[tokio::test]
    async fn task_get_tool_not_found() {
        let store = new_shared_task_store();
        let tool = TaskGetTool::new(Arc::clone(&store));
        let result = tool.execute(json!({"id": "nonexistent"}), &ctx()).await;

        assert!(result.is_error);
        assert!(result.content.contains("not found"));
    }

    #[tokio::test]
    async fn task_update_tool_not_found() {
        let store = new_shared_task_store();
        let tool = TaskUpdateTool::new(Arc::clone(&store));
        let result = tool
            .execute(json!({"id": "nonexistent", "status": "completed"}), &ctx())
            .await;

        assert!(result.is_error);
        assert!(result.content.contains("not found"));
    }

    #[tokio::test]
    async fn task_create_tool_missing_subject() {
        let store = new_shared_task_store();
        let tool = TaskCreateTool::new(Arc::clone(&store));
        let result = tool.execute(json!({"description": "desc"}), &ctx()).await;

        assert!(result.is_error);
    }
}
