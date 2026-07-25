//! MCP-specific background task supervision.

use std::collections::BTreeMap;
use std::future::Future;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use futures::FutureExt;
use serde::Serialize;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use super::lifecycle::{reduce_mcp_connection, McpConnectionEvent};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum McpServerHealthState {
    Connecting,
    Connected,
    Reconnecting,
    Degraded,
    Stopped,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct McpServerHealth {
    pub server: String,
    pub state: McpServerHealthState,
    pub reason: Option<String>,
    pub reconnect_count: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum McpTaskState {
    Running,
    Stopped,
    Failed,
    Aborted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum McpTaskExitPolicy {
    /// A normal return without shutdown means the long-lived task failed.
    DegradeOnExit,
    /// A normal return is the expected terminal decision for one-shot work.
    Complete,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct McpTaskHealth {
    pub name: String,
    pub server: String,
    pub state: McpTaskState,
    pub termination_reason: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct McpHealthSnapshot {
    pub accepting_tasks: bool,
    pub servers: Vec<McpServerHealth>,
    pub tasks: Vec<McpTaskHealth>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct McpShutdownReport {
    pub completed_tasks: usize,
    pub aborted_tasks: Vec<String>,
}

struct TaskRecord {
    id: u64,
    name: String,
    handle: JoinHandle<()>,
}

struct TaskRegistry {
    accepting: bool,
    tasks: Vec<TaskRecord>,
}

pub struct McpTaskSupervisor {
    cancel: CancellationToken,
    next_id: AtomicU64,
    registry: Mutex<TaskRegistry>,
    servers: Arc<RwLock<BTreeMap<String, McpServerHealth>>>,
    tasks: Arc<RwLock<BTreeMap<u64, McpTaskHealth>>>,
}

impl McpTaskSupervisor {
    const MAX_TASK_HEALTH_ENTRIES: usize = 256;

    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            cancel: CancellationToken::new(),
            next_id: AtomicU64::new(1),
            registry: Mutex::new(TaskRegistry {
                accepting: true,
                tasks: Vec::new(),
            }),
            servers: Arc::new(RwLock::new(BTreeMap::new())),
            tasks: Arc::new(RwLock::new(BTreeMap::new())),
        })
    }

    pub fn cancellation_token(&self) -> CancellationToken {
        self.cancel.child_token()
    }

    pub fn register_server(&self, server: &str) {
        self.apply_server_event(server, McpConnectionEvent::Register, None);
    }

    pub fn mark_connected(&self, server: &str) {
        self.apply_server_event(server, McpConnectionEvent::ConnectionEstablished, None);
    }

    /// A successful ping clears connection/reconnect degradation, but must not
    /// hide an independently failed notification task.
    pub fn mark_ping_healthy(&self, server: &str) {
        let task_failure = self
            .servers
            .read()
            .expect("MCP health lock poisoned")
            .get(server)
            .and_then(|health| health.reason.as_deref())
            .is_some_and(|reason| reason.starts_with("task:"));
        if !task_failure {
            self.apply_server_event(server, McpConnectionEvent::PingHealthy, None);
        }
    }

    pub fn mark_reconnecting(&self, server: &str, reason: impl Into<String>) {
        self.apply_server_event(
            server,
            McpConnectionEvent::Reconnect,
            Some(bounded_reason(reason.into())),
        );
    }

    pub fn mark_degraded(&self, server: &str, reason: impl Into<String>) {
        self.apply_server_event(
            server,
            McpConnectionEvent::Degrade,
            Some(bounded_reason(reason.into())),
        );
    }

    pub fn spawn<F>(self: &Arc<Self>, name: impl Into<String>, server: impl Into<String>, task: F)
    where
        F: Future<Output = ()> + Send + 'static,
    {
        self.spawn_with_policy(name, server, McpTaskExitPolicy::DegradeOnExit, task);
    }

    pub fn spawn_with_policy<F>(
        self: &Arc<Self>,
        name: impl Into<String>,
        server: impl Into<String>,
        exit_policy: McpTaskExitPolicy,
        task: F,
    ) where
        F: Future<Output = ()> + Send + 'static,
    {
        let name = name.into();
        let server = server.into();
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let tasks = self.tasks.clone();
        let servers = self.servers.clone();
        let cancel = self.cancel.clone();

        let mut registry = self.registry.lock().expect("MCP task registry poisoned");
        if !registry.accepting {
            tracing::warn!(task = %name, server = %server, "MCP supervisor rejected task after shutdown began");
            return;
        }
        // Completed JoinHandles need no shutdown wait. Keep health history
        // separately, with a hard bound for long-running reconnect cycles.
        registry.tasks.retain(|record| !record.handle.is_finished());
        let mut task_health = self.tasks.write().expect("MCP task health lock poisoned");
        while task_health.len() >= Self::MAX_TASK_HEALTH_ENTRIES {
            if let Some(oldest) = task_health.keys().next().copied() {
                task_health.remove(&oldest);
            }
        }
        task_health.insert(
            id,
            McpTaskHealth {
                name: name.clone(),
                server: server.clone(),
                state: McpTaskState::Running,
                termination_reason: None,
            },
        );
        drop(task_health);
        let task_name = name.clone();
        let handle = tokio::spawn(async move {
            let outcome = std::panic::AssertUnwindSafe(task).catch_unwind().await;
            let (state, reason) = if cancel.is_cancelled() {
                (McpTaskState::Stopped, Some("cancelled".to_owned()))
            } else if outcome.is_err() {
                (McpTaskState::Failed, Some("panic".to_owned()))
            } else if exit_policy == McpTaskExitPolicy::Complete {
                (McpTaskState::Stopped, Some("completed".to_owned()))
            } else {
                (McpTaskState::Failed, Some("unexpected_exit".to_owned()))
            };
            if state == McpTaskState::Failed {
                apply_server_transition(
                    &servers,
                    &server,
                    McpConnectionEvent::Degrade,
                    Some(format!("task:{task_name}:{}", reason.as_deref().unwrap())),
                );
                tracing::error!(task = %task_name, server = %server, reason = ?reason, "supervised MCP task terminated");
            }
            if let Some(health) = tasks
                .write()
                .expect("MCP task health lock poisoned")
                .get_mut(&id)
            {
                health.state = state;
                health.termination_reason = reason;
            }
        });
        registry.tasks.push(TaskRecord { id, name, handle });
    }

    pub fn snapshot(&self) -> McpHealthSnapshot {
        let accepting_tasks = self
            .registry
            .lock()
            .expect("MCP task registry poisoned")
            .accepting;
        McpHealthSnapshot {
            accepting_tasks,
            servers: self
                .servers
                .read()
                .expect("MCP health lock poisoned")
                .values()
                .cloned()
                .collect(),
            tasks: self
                .tasks
                .read()
                .expect("MCP task health lock poisoned")
                .values()
                .cloned()
                .collect(),
        }
    }

    pub async fn shutdown(&self, timeout: Duration) -> McpShutdownReport {
        let records = {
            let mut registry = self.registry.lock().expect("MCP task registry poisoned");
            registry.accepting = false;
            self.cancel.cancel();
            std::mem::take(&mut registry.tasks)
        };
        let deadline = tokio::time::Instant::now() + timeout;
        let mut report = McpShutdownReport::default();
        for mut record in records {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero()
                || tokio::time::timeout(remaining, &mut record.handle)
                    .await
                    .is_err()
            {
                record.handle.abort();
                let _ = tokio::time::timeout(Duration::from_millis(100), record.handle).await;
                report.aborted_tasks.push(record.name.clone());
                if let Some(task) = self
                    .tasks
                    .write()
                    .expect("MCP task health lock poisoned")
                    .get_mut(&record.id)
                {
                    task.state = McpTaskState::Aborted;
                    task.termination_reason = Some("shutdown_timeout".into());
                }
            } else {
                report.completed_tasks += 1;
            }
        }
        let server_names: Vec<String> = self
            .servers
            .read()
            .expect("MCP health lock poisoned")
            .keys()
            .cloned()
            .collect();
        for server in server_names {
            self.apply_server_event(
                &server,
                McpConnectionEvent::Shutdown,
                Some("shutdown".into()),
            );
        }
        report
    }

    fn apply_server_event(&self, server: &str, event: McpConnectionEvent, reason: Option<String>) {
        apply_server_transition(&self.servers, server, event, reason);
    }
}

fn apply_server_transition(
    servers: &Arc<RwLock<BTreeMap<String, McpServerHealth>>>,
    server: &str,
    event: McpConnectionEvent,
    reason: Option<String>,
) {
    let mut servers = servers.write().expect("MCP health lock poisoned");
    let previous = servers.get(server).map(|health| health.state);
    let Ok(transition) = reduce_mcp_connection(previous, event) else {
        tracing::warn!(
            server,
            ?event,
            ?previous,
            "rejected invalid MCP connection transition"
        );
        return;
    };
    let reconnect_count = servers
        .get(server)
        .map_or(0, |health| health.reconnect_count)
        .saturating_add(u64::from(event == McpConnectionEvent::Reconnect));
    servers.insert(
        server.to_owned(),
        McpServerHealth {
            server: server.to_owned(),
            state: transition.next_state,
            reason,
            reconnect_count,
        },
    );
}

impl Drop for McpTaskSupervisor {
    fn drop(&mut self) {
        self.cancel.cancel();
        if let Ok(mut registry) = self.registry.lock() {
            registry.accepting = false;
            for task in registry.tasks.drain(..) {
                task.handle.abort();
            }
        }
    }
}

fn bounded_reason(mut reason: String) -> String {
    const MAX_REASON_BYTES: usize = 256;
    if reason.len() <= MAX_REASON_BYTES {
        return reason;
    }
    let mut boundary = MAX_REASON_BYTES;
    while !reason.is_char_boundary(boundary) {
        boundary -= 1;
    }
    reason.truncate(boundary);
    reason
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn panic_is_observable_as_degraded_and_stopped_by_policy() {
        let supervisor = McpTaskSupervisor::new();
        supervisor.register_server("test");
        supervisor.spawn("mcp:test:health", "test", async { panic!("injected") });
        tokio::task::yield_now().await;
        tokio::time::sleep(Duration::from_millis(10)).await;
        let snapshot = supervisor.snapshot();
        assert_eq!(snapshot.servers[0].state, McpServerHealthState::Degraded);
        assert_eq!(snapshot.tasks[0].state, McpTaskState::Failed);
        assert_eq!(
            snapshot.tasks[0].termination_reason.as_deref(),
            Some("panic")
        );
    }

    #[tokio::test]
    async fn shutdown_cancels_tasks_and_rejects_new_tasks() {
        let supervisor = McpTaskSupervisor::new();
        let cancel = supervisor.cancellation_token();
        supervisor.spawn("mcp:test:reconnect", "test", async move {
            cancel.cancelled().await;
        });
        let report = supervisor.shutdown(Duration::from_secs(1)).await;
        assert_eq!(report.completed_tasks, 1);
        assert!(report.aborted_tasks.is_empty());
        supervisor.spawn("mcp:test:late", "test", async {});
        assert!(!supervisor.snapshot().accepting_tasks);
        assert_eq!(supervisor.snapshot().tasks.len(), 1);
    }

    #[tokio::test]
    async fn shutdown_timeout_aborts_non_cooperative_task() {
        let supervisor = McpTaskSupervisor::new();
        supervisor.spawn("mcp:test:stuck", "test", async {
            std::future::pending::<()>().await;
        });
        let report = supervisor.shutdown(Duration::from_millis(1)).await;
        assert_eq!(report.aborted_tasks, vec!["mcp:test:stuck"]);
        assert_eq!(supervisor.snapshot().tasks[0].state, McpTaskState::Aborted);
    }

    #[tokio::test]
    async fn one_shot_completion_does_not_degrade_server() {
        let supervisor = McpTaskSupervisor::new();
        supervisor.register_server("test");
        supervisor.mark_connected("test");
        supervisor.spawn_with_policy(
            "mcp:test:initial_reconnect",
            "test",
            McpTaskExitPolicy::Complete,
            async {},
        );
        tokio::task::yield_now().await;
        assert_eq!(
            supervisor.snapshot().servers[0].state,
            McpServerHealthState::Connected
        );
        assert_eq!(supervisor.snapshot().tasks[0].state, McpTaskState::Stopped);
    }

    #[test]
    fn healthy_ping_does_not_mask_an_independent_task_failure() {
        let supervisor = McpTaskSupervisor::new();
        supervisor.mark_degraded("test", "task:mcp:test:notifications:unexpected_exit");
        supervisor.mark_ping_healthy("test");
        assert_eq!(
            supervisor.snapshot().servers[0].state,
            McpServerHealthState::Degraded
        );
    }
}
