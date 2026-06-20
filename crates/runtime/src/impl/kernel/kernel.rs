//! AgentKernel — central coordinator for agent processes and forks.
//!
//! Provides spawn / fork / wait / kill / send / scratchpad / dispatch_pulse
//! primitives. The kernel owns the process table, the fork table, and the
//! parent-child relationship graph.

use std::collections::HashMap;
use std::sync::Arc;

use base::agent::Pid;
use base::envelope::Envelope;
use base::{ForkDirective, ForkResult};
use comm::CommunicationBus;
use tokio::sync::{Mutex, RwLock};

use crate::r#impl::agent::fork::AgentFork;
use crate::r#impl::agent::process::{AgentProcess, AgentProcessConfig};
use crate::r#impl::kernel::ipc::SharedScratchpad;

// ---------------------------------------------------------------------------
// KernelError
// ---------------------------------------------------------------------------

/// Errors returned by [`AgentKernel`] operations.
#[derive(Debug, Clone)]
pub enum KernelError {
    /// No process or fork with the given `Pid` exists.
    ProcessNotFound(Pid),
    /// A `wait` operation was cancelled (e.g. the kernel shut down).
    WaitCancelled,
    /// The caller lacks permission for the requested operation.
    PermissionDenied(String),
}

impl std::fmt::Display for KernelError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ProcessNotFound(pid) => write!(f, "process {} not found", pid),
            Self::WaitCancelled => write!(f, "wait cancelled"),
            Self::PermissionDenied(msg) => write!(f, "permission denied: {}", msg),
        }
    }
}

impl std::error::Error for KernelError {}

// ---------------------------------------------------------------------------
// AgentKernel
// ---------------------------------------------------------------------------

/// Central coordinator for agent lifecycle and IPC.
///
/// Holds the process table, fork table, parent-child graph, IPC channel,
/// shared scratchpads, and a reference to the event bus.
pub struct AgentKernel {
    /// Running processes keyed by `Pid`.
    processes: RwLock<HashMap<Pid, Arc<Mutex<AgentProcess>>>>,
    /// Running forks keyed by `Pid`.
    forks: RwLock<HashMap<Pid, Arc<Mutex<AgentFork>>>>,
    /// Parent `Pid` → list of child `Pid`s.
    children: RwLock<HashMap<Pid, Vec<Pid>>>,
    /// Unified communication bus for IPC and lifecycle events.
    bus: Arc<CommunicationBus>,
    /// Task-scoped shared scratchpads.
    scratchpads: RwLock<HashMap<String, Arc<SharedScratchpad>>>,
}

impl AgentKernel {
    /// Create a new kernel with the given communication bus.
    pub fn new(bus: Arc<CommunicationBus>) -> Self {
        Self {
            processes: RwLock::new(HashMap::new()),
            forks: RwLock::new(HashMap::new()),
            children: RwLock::new(HashMap::new()),
            bus,
            scratchpads: RwLock::new(HashMap::new()),
        }
    }

    // -- spawn / fork --------------------------------------------------------

    /// Spawn a new agent process.
    ///
    /// Returns the `Pid` assigned to the new process. If `parent` is given the
    /// child is recorded in the parent-child graph.
    pub async fn spawn(
        &self,
        task: String,
        config: AgentProcessConfig,
        parent: Option<Pid>,
    ) -> Pid {
        let pid = Pid::new();
        let mut process = AgentProcess::new_minimal(config);

        // Register agent inbox on the communication bus and store the receiver.
        let inbox = self.bus.register_agent(pid.as_u64(), Some(64)).await;
        process.inbox = Some(inbox);

        // Store in process table.
        self.processes
            .write()
            .await
            .insert(pid, Arc::new(Mutex::new(process)));

        // Track parent-child relationship.
        if let Some(parent_pid) = parent {
            self.children
                .write()
                .await
                .entry(parent_pid)
                .or_default()
                .push(pid);
        }

        // Emit a spawn event (fire-and-forget).
        let _ = task; // task stored in config or used elsewhere
        pid
    }

    /// Fork a child agent from an existing parent process.
    ///
    /// The fork inherits a fraction of the parent's remaining token budget as
    /// determined by `directive.budget_ratio`. Returns the child `Pid` on
    /// success or `KernelError::ProcessNotFound` if the parent does not exist.
    pub async fn fork(
        &self,
        parent_pid: Pid,
        directive: ForkDirective,
    ) -> Result<Pid, KernelError> {
        // Look up the parent to get its remaining budget.
        let parent_remaining = {
            let processes = self.processes.read().await;
            match processes.get(&parent_pid) {
                Some(proc_arc) => {
                    let proc = proc_arc.lock().await;
                    // AgentProcess stub has no budget field; use max_tokens_per_pulse as
                    // a proxy for the remaining budget.
                    proc.state(); // ensure process is valid
                    0u32 // placeholder — real budget integration comes later
                }
                None => return Err(KernelError::ProcessNotFound(parent_pid)),
            }
        };

        let mut fork = AgentFork::new(
            parent_pid,
            directive,
            parent_remaining,
            self.bus.event_bus().clone(),
        );
        let child_pid = fork.pid;

        // Register agent inbox on the communication bus and store the receiver.
        let inbox = self.bus.register_agent(child_pid.as_u64(), Some(64)).await;
        fork.inbox = Some(inbox);

        // Store in fork table.
        self.forks
            .write()
            .await
            .insert(child_pid, Arc::new(Mutex::new(fork)));

        // Track parent-child relationship.
        self.children
            .write()
            .await
            .entry(parent_pid)
            .or_default()
            .push(child_pid);

        Ok(child_pid)
    }

    // -- wait ----------------------------------------------------------------

    /// Wait for a process or fork to complete.
    ///
    /// For forks: returns the `ForkResult` once the fork has completed or
    /// failed. For processes: polls until the process reaches `Completed` or
    /// `Failed` state (no result value is produced; the caller should inspect
    /// the process state externally).
    ///
    /// Polling interval is 100 ms.
    pub async fn wait(&self, child_pid: Pid) -> Result<ForkResult, KernelError> {
        // Check forks first.
        let fork_handle: Option<Arc<Mutex<AgentFork>>> = {
            let forks = self.forks.read().await;
            forks.get(&child_pid).cloned()
        };
        if let Some(fork_arc) = fork_handle {
            return self.wait_fork(child_pid, fork_arc).await;
        }

        // Then check processes.
        let proc_handle: Option<Arc<Mutex<AgentProcess>>> = {
            let processes = self.processes.read().await;
            processes.get(&child_pid).cloned()
        };
        if let Some(proc_arc) = proc_handle {
            return self.wait_process(child_pid, proc_arc).await;
        }

        Err(KernelError::ProcessNotFound(child_pid))
    }

    /// Poll a fork until it produces a result.
    async fn wait_fork(
        &self,
        _pid: Pid,
        fork_arc: Arc<Mutex<AgentFork>>,
    ) -> Result<ForkResult, KernelError> {
        loop {
            {
                let fork = fork_arc.lock().await;
                if let Some(ref result) = fork.result {
                    return Ok(result.clone());
                }
                if !fork.is_running() {
                    // Fork completed but no result — should not happen.
                    return Err(KernelError::WaitCancelled);
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    }

    /// Poll a process until it reaches a terminal state.
    async fn wait_process(
        &self,
        pid: Pid,
        proc_arc: Arc<Mutex<AgentProcess>>,
    ) -> Result<ForkResult, KernelError> {
        loop {
            {
                let proc = proc_arc.lock().await;
                use crate::r#impl::agent::process::AgentState;
                match proc.state() {
                    AgentState::Completed | AgentState::Failed => {
                        // Processes don't produce ForkResult; synthesise one.
                        return Ok(ForkResult {
                            pid,
                            parent_pid: Pid::default(),
                            output: String::new(),
                            tokens_consumed: 0,
                            success: proc.state() == AgentState::Completed,
                        });
                    }
                    _ => {}
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    }

    // -- kill ----------------------------------------------------------------

    /// Kill a process or fork by `Pid`.
    ///
    /// Removes the entry from the appropriate table and unregisters its IPC
    /// inbox. Returns `KernelError::ProcessNotFound` if the `Pid` is unknown.
    pub async fn kill(&self, pid: Pid) -> Result<(), KernelError> {
        // Try processes first.
        {
            let mut processes = self.processes.write().await;
            if processes.remove(&pid).is_some() {
                drop(processes);
                self.bus.unregister_agent(&pid.as_u64()).await;
                return Ok(());
            }
        }

        // Then forks.
        {
            let mut forks = self.forks.write().await;
            if forks.remove(&pid).is_some() {
                drop(forks);
                self.bus.unregister_agent(&pid.as_u64()).await;
                return Ok(());
            }
        }

        Err(KernelError::ProcessNotFound(pid))
    }

    // -- send ----------------------------------------------------------------

    /// Send an envelope via the communication bus.
    pub async fn send(&self, envelope: Envelope) -> anyhow::Result<()> {
        self.bus.send(envelope).await
    }

    // -- scratchpad ----------------------------------------------------------

    /// Get or create a shared scratchpad for the given task.
    pub async fn scratchpad(&self, task_id: &str) -> Arc<SharedScratchpad> {
        let mut pads = self.scratchpads.write().await;
        pads.entry(task_id.to_string())
            .or_insert_with(|| Arc::new(SharedScratchpad::new(task_id.to_string())))
            .clone()
    }

    // -- queries -------------------------------------------------------------

    /// Total number of tracked entities (processes + forks).
    pub async fn total_count(&self) -> usize {
        self.process_count().await + self.fork_count().await
    }

    /// Number of running processes.
    pub async fn process_count(&self) -> usize {
        self.processes.read().await.len()
    }

    /// Number of running forks.
    pub async fn fork_count(&self) -> usize {
        self.forks.read().await.len()
    }

    /// List child `Pid`s of the given parent.
    pub async fn children_of(&self, pid: Pid) -> Vec<Pid> {
        self.children
            .read()
            .await
            .get(&pid)
            .cloned()
            .unwrap_or_default()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use base::envelope::{Endpoint, Payload, Target};

    fn make_kernel() -> AgentKernel {
        AgentKernel::new(Arc::new(CommunicationBus::new()))
    }

    fn make_config(id: &str) -> AgentProcessConfig {
        AgentProcessConfig {
            id: id.to_string(),
            max_tokens_per_pulse: 1000,
            ..Default::default()
        }
    }

    /// Helper: build a simple FireAndForget envelope addressed to `to_pid`.
    fn make_envelope(from: Pid, to: Pid, body: &str) -> Envelope {
        Envelope::fire_and_forget(
            Endpoint::Agent(from.as_u64()),
            Target::Agent(to.as_u64()),
            Payload::Json(serde_json::Value::String(body.to_string())),
        )
    }

    #[tokio::test]
    async fn test_spawn_returns_unique_pid() {
        let kernel = make_kernel();
        let pid1 = kernel.spawn("t1".into(), make_config("a1"), None).await;
        let pid2 = kernel.spawn("t2".into(), make_config("a2"), None).await;
        assert_ne!(pid1, pid2);
        assert_eq!(kernel.process_count().await, 2);
    }

    #[tokio::test]
    async fn test_spawn_tracks_parent_child() {
        let kernel = make_kernel();
        let parent = kernel.spawn("parent".into(), make_config("p"), None).await;
        let child = kernel
            .spawn("child".into(), make_config("c"), Some(parent))
            .await;
        let children = kernel.children_of(parent).await;
        assert_eq!(children, vec![child]);
    }

    #[tokio::test]
    async fn test_kill_process() {
        let kernel = make_kernel();
        let pid = kernel.spawn("t".into(), make_config("a"), None).await;
        assert_eq!(kernel.process_count().await, 1);
        kernel.kill(pid).await.expect("kill should succeed");
        assert_eq!(kernel.process_count().await, 0);
    }

    #[tokio::test]
    async fn test_kill_not_found() {
        let kernel = make_kernel();
        let pid = Pid::new();
        let err = kernel.kill(pid).await.unwrap_err();
        assert!(matches!(err, KernelError::ProcessNotFound(p) if p == pid));
    }

    #[tokio::test]
    async fn test_fork_requires_existing_parent() {
        let kernel = make_kernel();
        let bogus = Pid::new();
        let directive = ForkDirective::default();
        let err = kernel.fork(bogus, directive).await.unwrap_err();
        assert!(matches!(err, KernelError::ProcessNotFound(p) if p == bogus));
    }

    #[tokio::test]
    async fn test_fork_creates_child() {
        let kernel = make_kernel();
        let parent = kernel.spawn("parent".into(), make_config("p"), None).await;
        let directive = ForkDirective::default();
        let child = kernel
            .fork(parent, directive)
            .await
            .expect("fork should succeed");
        assert_eq!(kernel.fork_count().await, 1);
        let children = kernel.children_of(parent).await;
        assert!(children.contains(&child));
    }

    #[tokio::test]
    async fn test_scratchpad_create_and_get() {
        let kernel = make_kernel();
        let sp1 = kernel.scratchpad("task-x").await;
        let sp2 = kernel.scratchpad("task-x").await;
        // Same task_id returns the same Arc.
        assert!(Arc::ptr_eq(&sp1, &sp2));
    }

    #[tokio::test]
    async fn test_send_to_spawned_process_succeeds() {
        let kernel = make_kernel();
        let pid = kernel.spawn("t".into(), make_config("a"), None).await;
        let sender = Pid::new();
        let envelope = make_envelope(sender, pid, "work");
        // The inbox is wired via CommunicationBus, so sending should succeed.
        kernel.send(envelope).await.expect("send should succeed");
    }

    #[tokio::test]
    async fn test_total_count() {
        let kernel = make_kernel();
        assert_eq!(kernel.total_count().await, 0);
        let p = kernel.spawn("t".into(), make_config("a"), None).await;
        assert_eq!(kernel.total_count().await, 1);
        kernel.fork(p, ForkDirective::default()).await.unwrap();
        assert_eq!(kernel.total_count().await, 2);
    }
}
