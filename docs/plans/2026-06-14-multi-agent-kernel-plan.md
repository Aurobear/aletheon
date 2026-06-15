# Multi-Agent Kernel Implementation Plan

> **For agentic workers:** Use `workflow-feature` or `writing-plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement an Agent Kernel (Layer 2) with Linux-inspired process/thread primitives, IPC mechanisms, and lifecycle supervision for multi-agent coordination.

**Architecture:** AgentKernel provides spawn/fork/wait/kill/send/recv primitives. AgentProcess is the isolated process unit, AgentFork is the lightweight context-sharing thread unit. Communication flows through MessageChannel (point-to-point), EventBus Topic (broadcast), and SharedScratchpad (shared memory). AgentSupervisor monitors health and restarts crashed agents with exponential backoff.

**Tech Stack:** Rust, tokio (async), aletheon-abi (types), aletheon-comm (EventBus), aletheon-runtime (processes), aletheon-brain (LLM)

---

## Phase 1: Kernel Core + IPC

### Task 1: ABI IPC Types

**Files:**
- Create: `crates/aletheon-abi/src/ipc.rs`
- Modify: `crates/aletheon-abi/src/lib.rs:24` (add `pub mod ipc;`)
- Modify: `crates/aletheon-abi/src/lib.rs:43-77` (add re-exports)

- [ ] **Step 1: Create ipc.rs with Message, MessageKind, Signal types**

```rust
// crates/aletheon-abi/src/ipc.rs
use serde::{Deserialize, Serialize};
use crate::agent::Pid;

/// Message kind for inter-agent communication
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum MessageKind {
    /// Task assignment from parent/orchestrator to worker
    Task,
    /// Task result from worker to parent/orchestrator
    Result,
    /// Query request (expects response)
    Query,
    /// Query response
    Response,
    /// Signal (see Signal enum)
    Signal(Signal),
}

/// Unix-like signals for agent lifecycle control
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum Signal {
    /// Request graceful termination (SIGTERM)
    Abort,
    /// Pause agent — stop receiving pulses (SIGSTOP)
    Pause,
    /// Resume agent (SIGCONT)
    Resume,
    /// Health check ping
    HealthCheck,
    /// Budget warning — agent is running low on tokens
    BudgetWarning,
}

/// A message sent between agent processes
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IpcMessage {
    pub from: Pid,
    pub to: Pid,
    pub kind: MessageKind,
    pub payload: String,
    pub timestamp_ms: u64,
}

impl IpcMessage {
    pub fn new(from: Pid, to: Pid, kind: MessageKind, payload: String) -> Self {
        Self {
            from,
            to,
            kind,
            payload,
            timestamp_ms: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64,
        }
    }

    pub fn task(from: Pid, to: Pid, task: String) -> Self {
        Self::new(from, to, MessageKind::Task, task)
    }

    pub fn result(from: Pid, to: Pid, result: String) -> Self {
        Self::new(from, to, MessageKind::Result, result)
    }

    pub fn signal(from: Pid, to: Pid, signal: Signal) -> Self {
        Self::new(from, to, MessageKind::Signal(signal), String::new())
    }
}

/// Fork directive — specifies how to create a lightweight fork
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ForkDirective {
    /// The specific task for this fork
    pub prompt: String,
    /// Whether to inherit parent's conversation history
    pub inherit_history: bool,
    /// Whether to inherit parent's tool set
    pub inherit_tools: bool,
    /// Fraction of parent's token budget to take (0.0 - 1.0)
    pub budget_ratio: f64,
}

impl Default for ForkDirective {
    fn default() -> Self {
        Self {
            prompt: String::new(),
            inherit_history: true,
            inherit_tools: true,
            budget_ratio: 0.3,
        }
    }
}

/// Result returned by a completed fork
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ForkResult {
    pub pid: Pid,
    pub parent_pid: Pid,
    pub output: String,
    pub tokens_consumed: u32,
    pub success: bool,
}

/// Agent group identifier for multicast
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct GroupId(pub u64);
```

- [ ] **Step 2: Add ipc module to lib.rs**

In `crates/aletheon-abi/src/lib.rs`, add after the existing module declarations (around line 24):

```rust
pub mod ipc;
```

And add re-exports (after line 77):

```rust
pub use ipc::{IpcMessage, MessageKind, Signal, ForkDirective, ForkResult, GroupId};
```

- [ ] **Step 3: Verify ABI crate compiles**

Run: `cargo check -p aletheon-abi`
Expected: Compiles successfully (warnings OK)

- [ ] **Step 4: Commit**

```bash
git add crates/aletheon-abi/src/ipc.rs crates/aletheon-abi/src/lib.rs
git commit -m "feat(abi): add IPC types — IpcMessage, Signal, ForkDirective, ForkResult"
```

---

### Task 2: AgentFork Type

**Files:**
- Create: `crates/aletheon-runtime/src/impl/agent/fork.rs`
- Modify: `crates/aletheon-runtime/src/impl/agent/mod.rs` (add `pub mod fork;`)

- [ ] **Step 1: Create fork.rs with AgentFork struct**

```rust
// crates/aletheon-runtime/src/impl/agent/fork.rs
use std::sync::Arc;
use aletheon_abi::{Pid, EventBus, EventType, Priority, ForkDirective, ForkResult, AgentForkCompletedPayload};
use aletheon_comm::ConcreteEvent;
use super::budget::TokenBudget;

/// State of a fork (lightweight thread-like agent)
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ForkState {
    /// Currently executing
    Running,
    /// Completed successfully
    Completed,
    /// Failed with error message
    Failed(String),
}

/// A lightweight, context-sharing sub-agent (analogous to a thread)
///
/// Forks inherit the parent's conversation history, system prompt, and tools.
/// They execute a single directive and return a result.
pub struct AgentFork {
    /// Unique process ID
    pub pid: Pid,
    /// Parent process ID
    pub parent_pid: Pid,
    /// The directive (task) this fork was given
    pub directive: String,
    /// Token budget allocated from parent
    pub budget: TokenBudget,
    /// Current state
    pub state: ForkState,
    /// Result (set when completed)
    pub result: Option<ForkResult>,
    /// Event bus for lifecycle events
    bus: Arc<dyn EventBus>,
}

impl AgentFork {
    /// Create a new fork from a parent process
    pub fn new(
        parent_pid: Pid,
        directive: ForkDirective,
        parent_budget: &TokenBudget,
        bus: Arc<dyn EventBus>,
    ) -> Self {
        let pid = Pid::new();
        let max_tokens = (parent_budget.remaining() as f64 * directive.budget_ratio) as u32;

        Self {
            pid,
            parent_pid,
            directive: directive.prompt,
            budget: TokenBudget::new(max_tokens),
            state: ForkState::Running,
            result: None,
            bus,
        }
    }

    /// Mark fork as completed with result
    pub fn complete(&mut self, output: String, tokens_consumed: u32) {
        self.state = ForkState::Completed;
        self.result = Some(ForkResult {
            pid: self.pid,
            parent_pid: self.parent_pid,
            output,
            tokens_consumed,
            success: true,
        });
        self.publish_completed();
    }

    /// Mark fork as failed
    pub fn fail(&mut self, error: String) {
        self.state = ForkState::Failed(error.clone());
        self.result = Some(ForkResult {
            pid: self.pid,
            parent_pid: self.parent_pid,
            output: error,
            tokens_consumed: self.budget.total_consumed() as u32,
            success: false,
        });
        self.publish_completed();
    }

    /// Whether this fork is still running
    pub fn is_running(&self) -> bool {
        self.state == ForkState::Running
    }

    fn publish_completed(&self) {
        let event = ConcreteEvent::new(
            EventType::AgentForkCompleted,
            Priority::Normal,
            "agent_fork",
            Box::new(AgentForkCompletedPayload {
                pid: self.pid.as_u64(),
                parent_pid: self.parent_pid.as_u64(),
                success: self.state == ForkState::Completed,
            }),
        );
        let _ = self.bus.publish(&event);
    }
}
```

- [ ] **Step 2: Add AgentForkCompleted to EventType and payload**

In `crates/aletheon-abi/src/event.rs`, add to the `EventType` enum (after `AgentSpawned`):

```rust
/// A fork (lightweight sub-agent) has completed
AgentForkCompleted,
```

In `crates/aletheon-abi/src/evolution.rs`, add after `AgentSpawnedPayload`:

```rust
/// Payload for fork completion events
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentForkCompletedPayload {
    pub pid: u64,
    pub parent_pid: u64,
    pub success: bool,
}
```

- [ ] **Step 3: Add fork module to agent mod.rs**

In `crates/aletheon-runtime/src/impl/agent/mod.rs`, add:

```rust
pub mod fork;
```

- [ ] **Step 4: Verify compilation**

Run: `cargo check -p aletheon-runtime`
Expected: Compiles successfully

- [ ] **Step 5: Commit**

```bash
git add crates/aletheon-abi/src/event.rs crates/aletheon-abi/src/evolution.rs \
        crates/aletheon-runtime/src/impl/agent/fork.rs crates/aletheon-runtime/src/impl/agent/mod.rs
git commit -m "feat(runtime): add AgentFork — lightweight context-sharing sub-agent"
```

---

### Task 3: MessageChannel IPC

**Files:**
- Create: `crates/aletheon-runtime/src/impl/kernel/ipc.rs`
- Create: `crates/aletheon-runtime/src/impl/kernel/mod.rs`

- [ ] **Step 1: Create kernel module directory and mod.rs**

```rust
// crates/aletheon-runtime/src/impl/kernel/mod.rs
pub mod ipc;
pub use ipc::{MessageChannel, SharedScratchpad};
```

- [ ] **Step 2: Create ipc.rs with MessageChannel and SharedScratchpad**

```rust
// crates/aletheon-runtime/src/impl/kernel/ipc.rs
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex, RwLock};
use aletheon_abi::{Pid, IpcMessage, MessageKind, Signal};

/// Point-to-point message channel between two agents (analogous to pipe)
///
/// Each agent has an inbox. send() delivers to the target's inbox.
/// recv() blocks until a message arrives in own inbox.
pub struct MessageChannel {
    /// Per-agent inbox senders (the receiver is stored in the agent's process)
    inboxes: RwLock<HashMap<Pid, mpsc::Sender<IpcMessage>>>,
    /// Buffer size per inbox
    buffer_size: usize,
}

impl MessageChannel {
    pub fn new(buffer_size: usize) -> Self {
        Self {
            inboxes: RwLock::new(HashMap::new()),
            buffer_size,
        }
    }

    /// Register a new agent, returning its inbox receiver
    pub async fn register(&self, pid: Pid) -> mpsc::Receiver<IpcMessage> {
        let (tx, rx) = mpsc::channel(self.buffer_size);
        self.inboxes.write().await.insert(pid, tx);
        rx
    }

    /// Unregister an agent (cleanup)
    pub async fn unregister(&self, pid: &Pid) {
        self.inboxes.write().await.remove(pid);
    }

    /// Send a message to a specific agent
    pub async fn send(&self, msg: IpcMessage) -> Result<(), IpcSendError> {
        let inboxes = self.inboxes.read().await;
        match inboxes.get(&msg.to) {
            Some(tx) => tx.send(msg).await.map_err(|_| IpcSendError::RecipientGone),
            None => Err(IpcSendError::RecipientNotFound(msg.to)),
        }
    }

    /// Number of registered agents
    pub async fn registered_count(&self) -> usize {
        self.inboxes.read().await.len()
    }
}

#[derive(Debug, Clone)]
pub enum IpcSendError {
    RecipientNotFound(Pid),
    RecipientGone,
}

impl std::fmt::Display for IpcSendError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::RecipientNotFound(pid) => write!(f, "recipient {} not found", pid),
            Self::RecipientGone => write!(f, "recipient inbox closed"),
        }
    }
}

impl std::error::Error for IpcSendError {}

/// Shared key-value scratchpad for agents working on the same task
/// (analogous to shared memory)
pub struct SharedScratchpad {
    task_id: String,
    entries: RwLock<HashMap<String, ScratchpadEntry>>,
}

#[derive(Debug, Clone)]
pub struct ScratchpadEntry {
    pub key: String,
    pub value: String,
    pub written_by: Pid,
    pub timestamp_ms: u64,
}

impl SharedScratchpad {
    pub fn new(task_id: String) -> Self {
        Self {
            task_id,
            entries: RwLock::new(HashMap::new()),
        }
    }

    pub fn task_id(&self) -> &str {
        &self.task_id
    }

    pub async fn read(&self, key: &str) -> Option<String> {
        self.entries.read().await.get(key).map(|e| e.value.clone())
    }

    pub async fn write(&self, key: &str, value: String, writer: Pid) {
        let entry = ScratchpadEntry {
            key: key.to_string(),
            value,
            written_by: writer,
            timestamp_ms: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64,
        };
        self.entries.write().await.insert(key.to_string(), entry);
    }

    pub async fn delete(&self, key: &str) -> bool {
        self.entries.write().await.remove(key).is_some()
    }

    pub async fn list_keys(&self) -> Vec<String> {
        self.entries.read().await.keys().cloned().collect()
    }

    pub async fn snapshot(&self) -> HashMap<String, String> {
        self.entries
            .read()
            .await
            .iter()
            .map(|(k, v)| (k.clone(), v.value.clone()))
            .collect()
    }

    pub async fn entry_count(&self) -> usize {
        self.entries.read().await.len()
    }
}
```

- [ ] **Step 3: Add kernel module to runtime impl/mod.rs**

In `crates/aletheon-runtime/src/impl/mod.rs`, add:

```rust
pub mod kernel;
```

- [ ] **Step 4: Verify compilation**

Run: `cargo check -p aletheon-runtime`
Expected: Compiles successfully

- [ ] **Step 5: Write tests for MessageChannel**

Add at the bottom of `crates/aletheon-runtime/src/impl/kernel/ipc.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_message_channel_send_recv() {
        let channel = MessageChannel::new(16);
        let pid_a = Pid::new();
        let pid_b = Pid::new();

        let mut rx_b = channel.register(pid_b).await;

        let msg = IpcMessage::task(pid_a, pid_b, "hello".to_string());
        channel.send(msg).await.unwrap();

        let received = rx_b.recv().await.unwrap();
        assert_eq!(received.from, pid_a);
        assert_eq!(received.to, pid_b);
        assert_eq!(received.payload, "hello");
        assert_eq!(received.kind, MessageKind::Task);
    }

    #[tokio::test]
    async fn test_message_channel_recipient_not_found() {
        let channel = MessageChannel::new(16);
        let pid_a = Pid::new();
        let pid_unknown = Pid::new();

        let msg = IpcMessage::task(pid_a, pid_unknown, "test".to_string());
        let result = channel.send(msg).await;
        assert!(matches!(result, Err(IpcSendError::RecipientNotFound(_))));
    }

    #[tokio::test]
    async fn test_shared_scratchpad_read_write() {
        let pad = SharedScratchpad::new("task-1".to_string());
        let pid = Pid::new();

        pad.write("key1", "value1".to_string(), pid).await;
        assert_eq!(pad.read("key1").await, Some("value1".to_string()));
        assert_eq!(pad.read("missing").await, None);
    }

    #[tokio::test]
    async fn test_shared_scratchpad_delete() {
        let pad = SharedScratchpad::new("task-1".to_string());
        let pid = Pid::new();

        pad.write("key1", "value1".to_string(), pid).await;
        assert!(pad.delete("key1").await);
        assert_eq!(pad.read("key1").await, None);
        assert!(!pad.delete("missing").await);
    }

    #[tokio::test]
    async fn test_shared_scratchpad_snapshot() {
        let pad = SharedScratchpad::new("task-1".to_string());
        let pid = Pid::new();

        pad.write("a", "1".to_string(), pid).await;
        pad.write("b", "2".to_string(), pid).await;

        let snap = pad.snapshot().await;
        assert_eq!(snap.len(), 2);
        assert_eq!(snap["a"], "1");
        assert_eq!(snap["b"], "2");
    }
}
```

- [ ] **Step 6: Run tests**

Run: `cargo test -p aletheon-runtime --lib kernel::ipc::tests`
Expected: 5 tests pass

- [ ] **Step 7: Commit**

```bash
git add crates/aletheon-runtime/src/impl/kernel/ crates/aletheon-runtime/src/impl/mod.rs
git commit -m "feat(runtime): add IPC — MessageChannel and SharedScratchpad"
```

---

### Task 4: AgentKernel Struct with spawn/fork/wait/kill

**Files:**
- Create: `crates/aletheon-runtime/src/impl/kernel/kernel.rs`
- Modify: `crates/aletheon-runtime/src/impl/kernel/mod.rs`

- [ ] **Step 1: Create kernel.rs with AgentKernel**

```rust
// crates/aletheon-runtime/src/impl/kernel/kernel.rs
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot, Mutex, RwLock};
use aletheon_abi::{Pid, EventBus, CognitivePulseEvent, ForkDirective, ForkResult, IpcMessage};
use crate::impl::agent::process::{AgentProcess, AgentProcessConfig};
use crate::impl::agent::fork::AgentFork;
use super::ipc::{MessageChannel, SharedScratchpad, IpcSendError};

/// Process state tracked by the kernel
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProcessEntry {
    /// Full process (spawned)
    Process(Pid),
    /// Lightweight fork
    Fork(Pid),
}

/// Pending wait requests
struct WaitHandle {
    pid: Pid,
    reply: oneshot::Sender<ForkResult>,
}

/// Agent Kernel — the core of the multi-agent system
///
/// Provides Linux-inspired primitives: spawn, fork, wait, kill, send, recv
pub struct AgentKernel {
    /// Running processes
    processes: RwLock<HashMap<Pid, Arc<Mutex<AgentProcess>>>>,
    /// Running forks
    forks: RwLock<HashMap<Pid, Arc<Mutex<AgentFork>>>>,
    /// Parent → children mapping
    children: RwLock<HashMap<Pid, Vec<Pid>>>,
    /// IPC message channel
    pub ipc: MessageChannel,
    /// Shared scratchpads by task_id
    scratchpads: RwLock<HashMap<String, Arc<SharedScratchpad>>>,
    /// Event bus
    bus: Arc<dyn EventBus>,
    /// Pending wait requests (parent waiting for child)
    wait_handles: Mutex<Vec<WaitHandle>>,
}

impl AgentKernel {
    pub fn new(bus: Arc<dyn EventBus>) -> Self {
        Self {
            processes: RwLock::new(HashMap::new()),
            forks: RwLock::new(HashMap::new()),
            children: RwLock::new(HashMap::new()),
            ipc: MessageChannel::new(64),
            scratchpads: RwLock::new(HashMap::new()),
            bus,
            wait_handles: Mutex::new(Vec::new()),
        }
    }

    // ── Process Management ──────────────────────────────────────────

    /// Spawn an independent agent process (analogous to fork+exec)
    pub async fn spawn(
        &self,
        task: String,
        config: AgentProcessConfig,
        parent: Option<Pid>,
    ) -> Pid {
        let process = AgentProcess::new(parent, task, self.bus.clone(), config);
        let pid = process.pid;

        // Register IPC inbox
        self.ipc.register(pid).await;

        // Store process
        self.processes.write().await.insert(pid, Arc::new(Mutex::new(process)));

        // Track parent-child relationship
        if let Some(parent_pid) = parent {
            self.children.write().await
                .entry(parent_pid)
                .or_default()
                .push(pid);
        }

        pid
    }

    /// Create a lightweight fork from a parent process (analogous to fork/COW)
    pub async fn fork(
        &self,
        parent_pid: Pid,
        directive: ForkDirective,
    ) -> Result<Pid, KernelError> {
        // Get parent's budget
        let processes = self.processes.read().await;
        let parent = processes.get(&parent_pid)
            .ok_or(KernelError::ProcessNotFound(parent_pid))?;
        let parent_guard = parent.lock().await;

        let fork = AgentFork::new(parent_pid, directive, &parent_guard.energy, self.bus.clone());
        let fork_pid = fork.pid;
        drop(parent_guard);
        drop(processes);

        // Register IPC inbox
        self.ipc.register(fork_pid).await;

        // Store fork
        self.forks.write().await.insert(fork_pid, Arc::new(Mutex::new(fork)));

        // Track parent-child relationship
        self.children.write().await
            .entry(parent_pid)
            .or_default()
            .push(fork_pid);

        Ok(fork_pid)
    }

    /// Wait for a child process/fork to complete (analogous to waitpid)
    pub async fn wait(&self, child_pid: Pid) -> Result<ForkResult, KernelError> {
        // Check if it's a fork
        {
            let forks = self.forks.read().await;
            if let Some(fork) = forks.get(&child_pid) {
                let fork_guard = fork.lock().await;
                if let Some(result) = &fork_guard.result {
                    return Ok(result.clone());
                }
                // Fork still running — need to wait
                drop(fork_guard);
                drop(forks);

                let (tx, rx) = oneshot::channel();
                self.wait_handles.lock().await.push(WaitHandle {
                    pid: child_pid,
                    reply: tx,
                });

                return rx.await.map_err(|_| KernelError::WaitCancelled);
            }
        }

        // Check if it's a process
        {
            let processes = self.processes.read().await;
            if processes.contains_key(&child_pid) {
                drop(processes);
                // For processes, poll until terminated
                loop {
                    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                    let processes = self.processes.read().await;
                    if let Some(process) = processes.get(&child_pid) {
                        let p = process.lock().await;
                        if p.state == crate::impl::agent::process::AgentState::Terminated {
                            return Ok(ForkResult {
                                pid: child_pid,
                                parent_pid: Pid::new(), // TODO: track parent
                                output: String::new(),   // TODO: collect output
                                tokens_consumed: p.energy.total_consumed() as u32,
                                success: true,
                            });
                        }
                    } else {
                        return Err(KernelError::ProcessNotFound(child_pid));
                    }
                }
            }
        }

        Err(KernelError::ProcessNotFound(child_pid))
    }

    /// Kill a process or fork (analogous to kill(pid, SIGTERM))
    pub async fn kill(&self, pid: Pid) -> Result<(), KernelError> {
        // Try process first
        {
            let processes = self.processes.read().await;
            if let Some(process) = processes.get(&pid) {
                let mut p = process.lock().await;
                p.terminate();
                return Ok(());
            }
        }
        // Try fork
        {
            let forks = self.forks.read().await;
            if let Some(fork) = forks.get(&pid) {
                let mut f = fork.lock().await;
                f.fail("killed by kernel".to_string());
                return Ok(());
            }
        }
        Err(KernelError::ProcessNotFound(pid))
    }

    // ── IPC ─────────────────────────────────────────────────────────

    /// Send a message to a specific agent
    pub async fn send(&self, msg: IpcMessage) -> Result<(), IpcSendError> {
        self.ipc.send(msg).await
    }

    /// Create or get a shared scratchpad for a task
    pub async fn scratchpad(&self, task_id: &str) -> Arc<SharedScratchpad> {
        let mut pads = self.scratchpads.write().await;
        pads.entry(task_id.to_string())
            .or_insert_with(|| Arc::new(SharedScratchpad::new(task_id.to_string())))
            .clone()
    }

    // ── Pulse Dispatch ──────────────────────────────────────────────

    /// Dispatch a cognitive pulse to all running processes
    pub async fn dispatch_pulse(&self, pulse: &CognitivePulseEvent) {
        let processes = self.processes.read().await;
        for (_pid, process) in processes.iter() {
            let mut p = process.lock().await;
            p.on_pulse(pulse);
        }
    }

    // ── Queries ─────────────────────────────────────────────────────

    /// Get total process count (processes + forks)
    pub async fn total_count(&self) -> usize {
        self.processes.read().await.len() + self.forks.read().await.len()
    }

    /// Get process count
    pub async fn process_count(&self) -> usize {
        self.processes.read().await.len()
    }

    /// Get fork count
    pub async fn fork_count(&self) -> usize {
        self.forks.read().await.len()
    }

    /// Get children of a process
    pub async fn children_of(&self, pid: Pid) -> Vec<Pid> {
        self.children.read().await
            .get(&pid)
            .cloned()
            .unwrap_or_default()
    }

    /// Clean up terminated processes and completed forks
    pub async fn cleanup(&self) {
        // Remove completed forks
        let mut forks = self.forks.write().await;
        forks.retain(|_, fork| {
            let f = futures::executor::block_on(fork.lock());
            f.is_running()
        });

        // Remove terminated processes
        let mut processes = self.processes.write().await;
        processes.retain(|_, process| {
            let p = futures::executor::block_on(process.lock());
            p.state != crate::impl::agent::process::AgentState::Terminated
        });
    }
}

#[derive(Debug, Clone)]
pub enum KernelError {
    ProcessNotFound(Pid),
    WaitCancelled,
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
```

- [ ] **Step 2: Update kernel/mod.rs**

```rust
// crates/aletheon-runtime/src/impl/kernel/mod.rs
pub mod ipc;
pub mod kernel;

pub use ipc::{MessageChannel, SharedScratchpad};
pub use kernel::{AgentKernel, KernelError};
```

- [ ] **Step 3: Verify compilation**

Run: `cargo check -p aletheon-runtime`
Expected: Compiles (may have unused warnings — OK)

- [ ] **Step 4: Commit**

```bash
git add crates/aletheon-runtime/src/impl/kernel/
git commit -m "feat(runtime): add AgentKernel — spawn/fork/wait/kill/send primitives"
```

---

### Task 5: Integration Tests for Kernel Primitives

**Files:**
- Create: `crates/aletheon-runtime/tests/kernel_test.rs`

- [ ] **Step 1: Write integration tests**

```rust
// crates/aletheon-runtime/tests/kernel_test.rs
use std::sync::Arc;
use aletheon_abi::{Pid, EventBus, ForkDirective, IpcMessage, MessageKind};
use aletheon_comm::KernelEventBus;
use aletheon_runtime::impl::kernel::{AgentKernel};
use aletheon_runtime::impl::agent::process::AgentProcessConfig;

fn make_kernel() -> AgentKernel {
    let bus: Arc<dyn EventBus> = Arc::new(KernelEventBus::new(256));
    AgentKernel::new(bus)
}

#[tokio::test]
async fn test_spawn_creates_process() {
    let kernel = make_kernel();
    let pid = kernel.spawn("test task".into(), AgentProcessConfig::default(), None).await;
    assert!(pid.as_u64() > 0);
    assert_eq!(kernel.process_count().await, 1);
}

#[tokio::test]
async fn test_spawn_with_parent() {
    let kernel = make_kernel();
    let parent = kernel.spawn("parent".into(), AgentProcessConfig::default(), None).await;
    let child = kernel.spawn("child".into(), AgentProcessConfig::default(), Some(parent)).await;
    assert_eq!(kernel.process_count().await, 2);
    assert_eq!(kernel.children_of(parent).await, vec![child]);
}

#[tokio::test]
async fn test_kill_process() {
    let kernel = make_kernel();
    let pid = kernel.spawn("doomed".into(), AgentProcessConfig::default(), None).await;
    kernel.kill(pid).await.unwrap();
    // Process should still be in table (cleanup is separate)
    assert_eq!(kernel.process_count().await, 1);
}

#[tokio::test]
async fn test_kill_not_found() {
    let kernel = make_kernel();
    let fake_pid = Pid::new();
    assert!(kernel.kill(fake_pid).await.is_err());
}

#[tokio::test]
async fn test_ipc_send_recv() {
    let kernel = make_kernel();
    let pid_a = kernel.spawn("sender".into(), AgentProcessConfig::default(), None).await;
    let pid_b = kernel.spawn("receiver".into(), AgentProcessConfig::default(), None).await;

    let msg = IpcMessage::task(pid_a, pid_b, "hello".into());
    kernel.send(msg).await.unwrap();

    // Message should be in pid_b's inbox (we can't easily recv here without
    // exposing the receiver, but send succeeded = no error)
}

#[tokio::test]
async fn test_scratchpad_shared() {
    let kernel = make_kernel();
    let pad = kernel.scratchpad("task-1").await;
    let pid = Pid::new();

    pad.write("key", "value".into(), pid).await;
    assert_eq!(pad.read("key").await, Some("value".into()));

    // Same task_id returns same scratchpad
    let pad2 = kernel.scratchpad("task-1").await;
    assert_eq!(pad2.read("key").await, Some("value".into()));
}

#[tokio::test]
async fn test_total_count() {
    let kernel = make_kernel();
    assert_eq!(kernel.total_count().await, 0);

    kernel.spawn("a".into(), AgentProcessConfig::default(), None).await;
    assert_eq!(kernel.total_count().await, 1);
}
```

- [ ] **Step 2: Run integration tests**

Run: `cargo test -p aletheon-runtime --test kernel_test`
Expected: 7 tests pass

- [ ] **Step 3: Commit**

```bash
git add crates/aletheon-runtime/tests/kernel_test.rs
git commit -m "test(runtime): integration tests for AgentKernel primitives"
```

---

## Phase 2: Harness + Resource Management

### Task 6: AgentHarness Trait + ReActHarness

**Files:**
- Create: `crates/aletheon-runtime/src/impl/agent/harness.rs`
- Modify: `crates/aletheon-runtime/src/impl/agent/mod.rs`

- [ ] **Step 1: Create harness.rs with trait and ReActHarness**

```rust
// crates/aletheon-runtime/src/impl/agent/harness.rs
use async_trait::async_trait;
use aletheon_abi::{Message, Tool};
use super::budget::TokenBudget;

/// Harness context — what the core layer tells the harness about the run
#[derive(Debug, Clone)]
pub struct HarnessContext {
    pub provider: String,
    pub model: String,
}

/// Bid from a harness indicating it can handle this context
#[derive(Debug, Clone)]
pub struct HarnessBid {
    pub supported: bool,
    pub priority: u8,
}

/// Prepared attempt parameters — everything the harness needs to execute one turn
pub struct AttemptParams {
    pub prompt: String,
    pub tools: Vec<Tool>,
    pub system_prompt: String,
    pub messages: Vec<Message>,
    pub budget: TokenBudget,
    pub runtime_plan: RuntimePlan,
}

/// Runtime plan — policy bundle for the attempt
#[derive(Debug, Clone)]
pub struct RuntimePlan {
    pub max_turns: u32,
    pub timeout_ms: u64,
    pub compaction_threshold: usize,
}

impl Default for RuntimePlan {
    fn default() -> Self {
        Self {
            max_turns: 10,
            timeout_ms: 30_000,
            compaction_threshold: 4096,
        }
    }
}

/// Result of a harness attempt
#[derive(Debug, Clone)]
pub struct AttemptResult {
    pub response: String,
    pub tokens_used: u32,
    pub turn_count: u32,
    pub status: AttemptStatus,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AttemptStatus {
    /// Completed successfully
    Complete,
    /// Needs more turns to finish
    NeedMoreTurns,
    /// Token budget exhausted
    BudgetExhausted,
    /// Error occurred
    Error(String),
}

/// AgentHarness — turn-level executor (OpenClaw pattern)
///
/// The core layer prepares everything (prompt, tools, context).
/// The harness only executes the turn.
#[async_trait]
pub trait AgentHarness: Send + Sync {
    /// Whether this harness can handle the given context
    fn supports(&self, ctx: &HarnessContext) -> HarnessBid;

    /// Execute one prepared attempt
    async fn run_attempt(&self, params: AttemptParams) -> AttemptResult;
}
```

- [ ] **Step 2: Add harness module to agent/mod.rs**

In `crates/aletheon-runtime/src/impl/agent/mod.rs`, add:

```rust
pub mod harness;
```

- [ ] **Step 3: Verify compilation**

Run: `cargo check -p aletheon-runtime`
Expected: Compiles successfully

- [ ] **Step 4: Commit**

```bash
git add crates/aletheon-runtime/src/impl/agent/harness.rs crates/aletheon-runtime/src/impl/agent/mod.rs
git commit -m "feat(runtime): add AgentHarness trait — turn-level executor abstraction"
```

---

### Task 7: GlobalTokenPool

**Files:**
- Create: `crates/aletheon-runtime/src/impl/kernel/global_pool.rs`
- Modify: `crates/aletheon-runtime/src/impl/kernel/mod.rs`

- [ ] **Step 1: Create global_pool.rs**

```rust
// crates/aletheon-runtime/src/impl/kernel/global_pool.rs
use std::collections::BinaryHeap;
use std::cmp::Reverse;
use std::sync::atomic::{AtomicU32, AtomicBool, Ordering};
use std::sync::Mutex;
use aletheon_abi::Pid;

/// Priority claim in the global pool
#[derive(Debug, Clone, Eq, PartialEq)]
struct PriorityClaim {
    pid: Pid,
    requested: u32,
    priority: u8,
}

impl Ord for PriorityClaim {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        // Lower priority number = higher priority
        self.priority.cmp(&other.priority)
            .then_with(|| self.requested.cmp(&other.requested))
    }
}

impl PartialOrd for PriorityClaim {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

/// Global token pool — system-wide resource management
///
/// All agents share this pool. Higher-priority agents get tokens first.
pub struct GlobalTokenPool {
    /// Total budget per pulse cycle
    total_budget: AtomicU32,
    /// Currently allocated tokens
    allocated: AtomicU32,
    /// Whether a pulse cycle is active
    active: AtomicBool,
    /// Pending claims (priority queue)
    claims: Mutex<BinaryHeap<Reverse<PriorityClaim>>>,
}

impl GlobalTokenPool {
    pub fn new(total_budget: u32) -> Self {
        Self {
            total_budget: AtomicU32::new(total_budget),
            allocated: AtomicU32::new(0),
            active: AtomicBool::new(false),
            claims: Mutex::new(BinaryHeap::new()),
        }
    }

    /// Start a new pulse cycle — resets allocated to 0
    pub fn begin_pulse(&self, total: u32) {
        self.total_budget.store(total, Ordering::SeqCst);
        self.allocated.store(0, Ordering::SeqCst);
        self.active.store(true, Ordering::SeqCst);
    }

    /// End the current pulse cycle
    pub fn end_pulse(&self) {
        self.active.store(false, Ordering::SeqCst);
    }

    /// Claim tokens from the pool. Returns actual amount allocated (may be less than requested).
    pub fn claim(&self, pid: Pid, requested: u32, priority: u8) -> u32 {
        if !self.active.load(Ordering::SeqCst) {
            return 0;
        }

        let total = self.total_budget.load(Ordering::SeqCst);
        let current = self.allocated.load(Ordering::SeqCst);
        let available = total.saturating_sub(current);

        if available == 0 {
            return 0;
        }

        let granted = requested.min(available);

        // Try to atomically allocate
        loop {
            let current = self.allocated.load(Ordering::SeqCst);
            let new_total = current + granted;
            if new_total > total {
                // Not enough — reduce grant
                let actual = total.saturating_sub(current);
                if actual == 0 {
                    return 0;
                }
                if self.allocated.compare_exchange(current, current + actual, Ordering::SeqCst, Ordering::SeqCst).is_ok() {
                    return actual;
                }
            } else {
                if self.allocated.compare_exchange(current, new_total, Ordering::SeqCst, Ordering::SeqCst).is_ok() {
                    return granted;
                }
            }
        }
    }

    /// Release unused tokens back to the pool
    pub fn release(&self, unused: u32) {
        self.allocated.fetch_sub(unused, Ordering::SeqCst);
    }

    /// Get total budget
    pub fn total(&self) -> u32 {
        self.total_budget.load(Ordering::SeqCst)
    }

    /// Get currently allocated amount
    pub fn allocated(&self) -> u32 {
        self.allocated.load(Ordering::SeqCst)
    }

    /// Get remaining available tokens
    pub fn available(&self) -> u32 {
        let total = self.total_budget.load(Ordering::SeqCst);
        let alloc = self.allocated.load(Ordering::SeqCst);
        total.saturating_sub(alloc)
    }
}
```

- [ ] **Step 2: Update kernel/mod.rs**

```rust
// crates/aletheon-runtime/src/impl/kernel/mod.rs
pub mod ipc;
pub mod kernel;
pub mod global_pool;

pub use ipc::{MessageChannel, SharedScratchpad};
pub use kernel::{AgentKernel, KernelError};
pub use global_pool::GlobalTokenPool;
```

- [ ] **Step 3: Write tests**

Add at the bottom of `crates/aletheon-runtime/src/impl/kernel/global_pool.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_claim_basic() {
        let pool = GlobalTokenPool::new(1000);
        pool.begin_pulse(1000);

        let pid = Pid::new();
        let granted = pool.claim(pid, 500, 2);
        assert_eq!(granted, 500);
        assert_eq!(pool.allocated(), 500);
        assert_eq!(pool.available(), 500);
    }

    #[test]
    fn test_claim_exceeds_available() {
        let pool = GlobalTokenPool::new(1000);
        pool.begin_pulse(1000);

        let pid_a = Pid::new();
        let pid_b = Pid::new();

        pool.claim(pid_a, 800, 2);
        let granted = pool.claim(pid_b, 500, 2);
        assert_eq!(granted, 200); // only 200 remaining
    }

    #[test]
    fn test_claim_when_inactive() {
        let pool = GlobalTokenPool::new(1000);
        // Don't begin_pulse
        let pid = Pid::new();
        assert_eq!(pool.claim(pid, 500, 2), 0);
    }

    #[test]
    fn test_release() {
        let pool = GlobalTokenPool::new(1000);
        pool.begin_pulse(1000);

        let pid = Pid::new();
        pool.claim(pid, 800, 2);
        pool.release(300);
        assert_eq!(pool.available(), 500);
    }

    #[test]
    fn test_pulse_cycle() {
        let pool = GlobalTokenPool::new(1000);
        pool.begin_pulse(1000);

        let pid = Pid::new();
        pool.claim(pid, 600, 2);
        assert_eq!(pool.allocated(), 600);

        pool.end_pulse();
        pool.begin_pulse(2000);
        assert_eq!(pool.allocated(), 0);
        assert_eq!(pool.total(), 2000);
    }
}
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p aletheon-runtime --lib kernel::global_pool::tests`
Expected: 5 tests pass

- [ ] **Step 5: Commit**

```bash
git add crates/aletheon-runtime/src/impl/kernel/global_pool.rs crates/aletheon-runtime/src/impl/kernel/mod.rs
git commit -m "feat(runtime): add GlobalTokenPool — priority-based system-wide token management"
```

---

## Phase 3: Supervisor + Integration

### Task 8: AgentSupervisor

**Files:**
- Create: `crates/aletheon-runtime/src/impl/kernel/supervisor.rs`
- Modify: `crates/aletheon-runtime/src/impl/kernel/mod.rs`

- [ ] **Step 1: Create supervisor.rs**

```rust
// crates/aletheon-runtime/src/impl/kernel/supervisor.rs
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use aletheon_abi::{Pid, EventBus, EventType, Priority};
use aletheon_comm::ConcreteEvent;
use super::kernel::AgentKernel;
use crate::impl::agent::process::AgentProcessConfig;

/// Restart policy configuration
#[derive(Debug, Clone)]
pub struct RestartPolicy {
    /// Initial delay before first restart
    pub initial_delay: Duration,
    /// Maximum delay (exponential backoff cap)
    pub max_delay: Duration,
    /// Backoff multiplier
    pub backoff_multiplier: f64,
    /// Window for fast-fail detection
    pub fast_fail_window: Duration,
    /// Number of crashes in window to trigger parking
    pub fast_fail_threshold: u32,
    /// Exit codes that mean "never restart"
    pub permanent_exit_codes: Vec<i32>,
}

impl Default for RestartPolicy {
    fn default() -> Self {
        Self {
            initial_delay: Duration::from_secs(2),
            max_delay: Duration::from_secs(120),
            backoff_multiplier: 2.0,
            fast_fail_window: Duration::from_secs(10),
            fast_fail_threshold: 5,
            permanent_exit_codes: vec![78],
        }
    }
}

/// Supervised process state
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SupervisedState {
    Running,
    Suspended,
    Restarting,
    Parked,
    Terminated,
}

struct SupervisedProcess {
    config: AgentProcessConfig,
    task: String,
    restart_count: u32,
    last_heartbeat: Instant,
    crash_times: Vec<Instant>,
    state: SupervisedState,
    parent: Option<Pid>,
}

/// AgentSupervisor — lifecycle management for agent processes
///
/// Monitors health, restarts crashed agents with exponential backoff,
/// detects fast-fail patterns, and manages graceful shutdown.
pub struct AgentSupervisor {
    supervised: RwLock<HashMap<Pid, SupervisedProcess>>,
    policy: RestartPolicy,
    health_check_interval: Duration,
}

impl AgentSupervisor {
    pub fn new(policy: RestartPolicy) -> Self {
        Self {
            supervised: RwLock::new(HashMap::new()),
            health_check_interval: Duration::from_secs(30),
            policy,
        }
    }

    /// Register a process for supervision
    pub async fn supervise(
        &self,
        pid: Pid,
        task: String,
        config: AgentProcessConfig,
        parent: Option<Pid>,
    ) {
        self.supervised.write().await.insert(pid, SupervisedProcess {
            config,
            task,
            restart_count: 0,
            last_heartbeat: Instant::now(),
            crash_times: Vec::new(),
            state: SupervisedState::Running,
            parent,
        });
    }

    /// Update heartbeat for a process
    pub async fn heartbeat(&self, pid: Pid) {
        if let Some(proc) = self.supervised.write().await.get_mut(&pid) {
            proc.last_heartbeat = Instant::now();
        }
    }

    /// Check if a process should be restarted after crash
    pub async fn on_crash(&self, pid: Pid, exit_code: Option<i32>) -> RestartDecision {
        let mut supervised = self.supervised.write().await;
        let proc = match supervised.get_mut(&pid) {
            Some(p) => p,
            None => return RestartDecision::Ignore,
        };

        // Check permanent exit code
        if let Some(code) = exit_code {
            if self.policy.permanent_exit_codes.contains(&code) {
                proc.state = SupervisedState::Parked;
                return RestartDecision::Park;
            }
        }

        // Record crash time
        let now = Instant::now();
        proc.crash_times.push(now);

        // Fast-fail detection: count crashes within window
        let window_start = now - self.policy.fast_fail_window;
        proc.crash_times.retain(|t| *t >= window_start);
        if proc.crash_times.len() >= self.policy.fast_fail_threshold as usize {
            proc.state = SupervisedState::Parked;
            return RestartDecision::Park;
        }

        // Calculate restart delay with exponential backoff
        proc.restart_count += 1;
        proc.state = SupervisedState::Restarting;
        let delay = self.calculate_delay(proc.restart_count);

        RestartDecision::Restart {
            delay,
            task: proc.task.clone(),
            config: proc.config.clone(),
            parent: proc.parent,
        }
    }

    /// Mark a process as successfully running (resets restart count after stability)
    pub async fn mark_stable(&self, pid: Pid) {
        if let Some(proc) = self.supervised.write().await.get_mut(&pid) {
            proc.restart_count = 0;
            proc.crash_times.clear();
            proc.state = SupervisedState::Running;
        }
    }

    /// Get count of supervised processes
    pub async fn supervised_count(&self) -> usize {
        self.supervised.read().await.len()
    }

    /// Get state of a supervised process
    pub async fn state_of(&self, pid: Pid) -> Option<SupervisedState> {
        self.supervised.read().await.get(&pid).map(|p| p.state.clone())
    }

    fn calculate_delay(&self, restart_count: u32) -> Duration {
        let base = self.policy.initial_delay.as_secs_f64();
        let delay = base * self.policy.backoff_multiplier.powi(restart_count as i32 - 1);
        Duration::from_secs_f64(delay.min(self.policy.max_delay.as_secs_f64()))
    }
}

/// Decision from the supervisor after a crash
#[derive(Debug, Clone)]
pub enum RestartDecision {
    /// Restart after delay
    Restart {
        delay: Duration,
        task: String,
        config: AgentProcessConfig,
        parent: Option<Pid>,
    },
    /// Park — too many crashes, don't restart
    Park,
    /// Ignore — unknown process
    Ignore,
}
```

- [ ] **Step 2: Update kernel/mod.rs**

```rust
// crates/aletheon-runtime/src/impl/kernel/mod.rs
pub mod ipc;
pub mod kernel;
pub mod global_pool;
pub mod supervisor;

pub use ipc::{MessageChannel, SharedScratchpad};
pub use kernel::{AgentKernel, KernelError};
pub use global_pool::GlobalTokenPool;
pub use supervisor::{AgentSupervisor, RestartPolicy, RestartDecision, SupervisedState};
```

- [ ] **Step 3: Write tests**

Add at the bottom of `crates/aletheon-runtime/src/impl/kernel/supervisor.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn default_config() -> AgentProcessConfig {
        AgentProcessConfig::default()
    }

    #[tokio::test]
    async fn test_supervise_and_heartbeat() {
        let supervisor = AgentSupervisor::new(RestartPolicy::default());
        let pid = Pid::new();
        supervisor.supervise(pid, "test".into(), default_config(), None).await;
        assert_eq!(supervisor.supervised_count().await, 1);
        assert_eq!(supervisor.state_of(pid).await, Some(SupervisedState::Running));

        supervisor.heartbeat(pid).await;
        assert_eq!(supervisor.state_of(pid).await, Some(SupervisedState::Running));
    }

    #[tokio::test]
    async fn test_crash_restart_decision() {
        let supervisor = AgentSupervisor::new(RestartPolicy::default());
        let pid = Pid::new();
        supervisor.supervise(pid, "test".into(), default_config(), None).await;

        let decision = supervisor.on_crash(pid, Some(1)).await;
        match decision {
            RestartDecision::Restart { delay, .. } => {
                assert_eq!(delay, Duration::from_secs(2)); // initial_delay
            }
            _ => panic!("expected Restart"),
        }
    }

    #[tokio::test]
    async fn test_permanent_exit_code_parks() {
        let supervisor = AgentSupervisor::new(RestartPolicy::default());
        let pid = Pid::new();
        supervisor.supervise(pid, "test".into(), default_config(), None).await;

        let decision = supervisor.on_crash(pid, Some(78)).await;
        assert!(matches!(decision, RestartDecision::Park));
        assert_eq!(supervisor.state_of(pid).await, Some(SupervisedState::Parked));
    }

    #[tokio::test]
    async fn test_fast_fail_parks() {
        let policy = RestartPolicy {
            fast_fail_window: Duration::from_secs(60),
            fast_fail_threshold: 3,
            ..Default::default()
        };
        let supervisor = AgentSupervisor::new(policy);
        let pid = Pid::new();
        supervisor.supervise(pid, "test".into(), default_config(), None).await;

        // Crash 3 times rapidly
        supervisor.on_crash(pid, Some(1)).await;
        supervisor.on_crash(pid, Some(1)).await;
        let decision = supervisor.on_crash(pid, Some(1)).await;
        assert!(matches!(decision, RestartDecision::Park));
    }

    #[tokio::test]
    async fn test_exponential_backoff() {
        let supervisor = AgentSupervisor::new(RestartPolicy::default());
        let pid = Pid::new();
        supervisor.supervise(pid, "test".into(), default_config(), None).await;

        // First crash: 2s
        let d1 = supervisor.on_crash(pid, Some(1)).await;
        // Second crash: 4s
        let d2 = supervisor.on_crash(pid, Some(1)).await;
        // Third crash: 8s
        let d3 = supervisor.on_crash(pid, Some(1)).await;

        if let RestartDecision::Restart { delay, .. } = d1 {
            assert_eq!(delay, Duration::from_secs(2));
        }
        if let RestartDecision::Restart { delay, .. } = d2 {
            assert_eq!(delay, Duration::from_secs(4));
        }
        if let RestartDecision::Restart { delay, .. } = d3 {
            assert_eq!(delay, Duration::from_secs(8));
        }
    }

    #[tokio::test]
    async fn test_mark_stable_resets_count() {
        let supervisor = AgentSupervisor::new(RestartPolicy::default());
        let pid = Pid::new();
        supervisor.supervise(pid, "test".into(), default_config(), None).await;

        supervisor.on_crash(pid, Some(1)).await;
        supervisor.on_crash(pid, Some(1)).await;
        supervisor.mark_stable(pid).await;

        // Next crash should have initial delay again
        let decision = supervisor.on_crash(pid, Some(1)).await;
        if let RestartDecision::Restart { delay, .. } = decision {
            assert_eq!(delay, Duration::from_secs(2));
        }
    }
}
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p aletheon-runtime --lib kernel::supervisor::tests`
Expected: 6 tests pass

- [ ] **Step 5: Commit**

```bash
git add crates/aletheon-runtime/src/impl/kernel/supervisor.rs crates/aletheon-runtime/src/impl/kernel/mod.rs
git commit -m "feat(runtime): add AgentSupervisor — lifecycle management with exponential backoff"
```

---

### Task 9: Upgrade AgentProcess with IPC and Heartbeat

**Files:**
- Modify: `crates/aletheon-runtime/src/impl/agent/process.rs`

- [ ] **Step 1: Add message inbox and heartbeat to AgentProcess**

In `crates/aletheon-runtime/src/impl/agent/process.rs`, modify the `AgentProcess` struct to add:

```rust
/// Message inbox receiver (set after kernel registration)
pub inbox: Option<tokio::sync::mpsc::Receiver<aletheon_abi::IpcMessage>>,
/// Last heartbeat timestamp (epoch millis)
pub last_heartbeat_ms: AtomicU64,
```

Add a method to update heartbeat:

```rust
/// Update heartbeat timestamp
pub fn touch_heartbeat(&self) {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    self.last_heartbeat_ms.store(now, Ordering::Relaxed);
}
```

Update `on_pulse` to call `touch_heartbeat()` at the start.

- [ ] **Step 2: Verify compilation**

Run: `cargo check -p aletheon-runtime`
Expected: Compiles successfully

- [ ] **Step 3: Run all existing tests**

Run: `cargo test -p aletheon-runtime`
Expected: All existing tests still pass

- [ ] **Step 4: Commit**

```bash
git add crates/aletheon-runtime/src/impl/agent/process.rs
git commit -m "feat(runtime): add inbox and heartbeat to AgentProcess"
```

---

### Task 10: Full Workspace Verification

**Files:** None (verification only)

- [ ] **Step 1: Run full workspace check**

Run: `cargo check --workspace`
Expected: Compiles successfully

- [ ] **Step 2: Run full workspace tests**

Run: `cargo test --workspace`
Expected: All tests pass (including new kernel, IPC, global pool, supervisor tests)

- [ ] **Step 3: Run clippy**

Run: `cargo clippy --workspace -- -D warnings`
Expected: No warnings

- [ ] **Step 4: Final commit if needed**

```bash
git add -A
git commit -m "chore: final cleanup for multi-agent kernel"
```
