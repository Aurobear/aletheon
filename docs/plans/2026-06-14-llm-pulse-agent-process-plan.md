# LLM Pulse + AgentProcess Implementation Plan

> **For agentic workers:** Use `workflow-feature` or `writing-plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Transform LLM from passive scheduler to active energy source, and give agents process-like identity with PID/state/lifecycle/sub-agent spawning.

**Architecture:** LlmPulse broadcasts CognitivePulseEvent periodically via EventBus. AgentProcess subscribes, consumes energy to run Engine's ReAct loop. AgentRegistry manages process table. Sub-agents spawn as child processes with halved energy budgets.

**Tech Stack:** Rust, tokio, aletheon-abi, aletheon-comm, aletheon-brain, aletheon-runtime

---

## File Map

| File | Action | Purpose |
|------|--------|---------|
| `crates/aletheon-abi/src/evolution.rs` | Modify | Add CognitivePulseEvent, ProviderHealth, AgentStarted/Stopped/Spawned payloads |
| `crates/aletheon-abi/src/event.rs` | Modify | Add EventType::CognitivePulse, AgentSpawned |
| `crates/aletheon-abi/src/agent.rs` | Create | Pid type |
| `crates/aletheon-abi/src/lib.rs` | Modify | Re-export agent module |
| `crates/aletheon-brain/src/impl/llm/pulse.rs` | Create | LlmPulse — periodic energy broadcaster |
| `crates/aletheon-brain/src/impl/llm/scheduler.rs` | Modify | Add health_check method |
| `crates/aletheon-brain/src/impl/llm/mod.rs` | Modify | Re-export pulse |
| `crates/aletheon-runtime/src/impl/agent/process.rs` | Create | AgentProcess — process-like entity |
| `crates/aletheon-runtime/src/impl/agent/budget.rs` | Create | TokenBudget — energy management |
| `crates/aletheon-runtime/src/impl/agent/mod.rs` | Modify | Re-export process, budget |
| `crates/aletheon-runtime/src/impl/orchestration/registry.rs` | Modify | Upgrade to process table |
| `crates/aletheon-runtime/src/impl/engine/cognitive_loop.rs` | Modify | Add run_turn_with_budget |
| `crates/binaries/aletheond/src/main.rs` | Modify | Start LlmPulse + AgentRegistry |

---

## Task 1: Add Pid and Pulse Event Types to ABI

**Files:**
- Create: `crates/aletheon-abi/src/agent.rs`
- Modify: `crates/aletheon-abi/src/evolution.rs`
- Modify: `crates/aletheon-abi/src/event.rs`
- Modify: `crates/aletheon-abi/src/lib.rs`

- [ ] **Step 1: Create Pid type**

Create `crates/aletheon-abi/src/agent.rs`:

```rust
//! Agent process identity types.

use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU64, Ordering};

/// Agent process identifier — unique per runtime session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Pid(u64);

impl Pid {
    /// Create a new unique PID.
    pub fn new() -> Self {
        static NEXT_PID: AtomicU64 = AtomicU64::new(1);
        Self(NEXT_PID.fetch_add(1, Ordering::Relaxed))
    }

    pub fn as_u64(&self) -> u64 {
        self.0
    }
}

impl Default for Pid {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for Pid {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "pid:{}", self.0)
    }
}
```

- [ ] **Step 2: Add pulse and agent lifecycle events to evolution.rs**

Add to `crates/aletheon-abi/src/evolution.rs`:

```rust
/// LLM energy pulse — broadcast periodically by LlmPulse.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CognitivePulseEvent {
    pub pulse_id: Uuid,
    pub timestamp: String,  // ISO 8601
    pub available_tokens: u32,
    pub provider_health: ProviderHealth,
}

/// Health status of an LLM provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderHealth {
    pub name: String,
    pub available: bool,
    pub latency_ms: u64,
    pub tokens_remaining: Option<u32>,
}

/// Agent lifecycle events.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentStartedPayload {
    pub pid: u64,
    pub task: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentStoppedPayload {
    pub pid: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentSpawnedPayload {
    pub parent: u64,
    pub child: u64,
}
```

- [ ] **Step 3: Add new EventType variants**

In `crates/aletheon-abi/src/event.rs`, add to the enum:

```rust
    // Energy / agent lifecycle
    CognitivePulse,
    AgentSpawned,
```

- [ ] **Step 4: Re-export agent module**

In `crates/aletheon-abi/src/lib.rs`, add:

```rust
pub mod agent;
```

- [ ] **Step 5: Verify**

Run: `cargo check -p aletheon-abi`

- [ ] **Step 6: Commit**

```bash
git add crates/aletheon-abi/src/agent.rs crates/aletheon-abi/src/evolution.rs crates/aletheon-abi/src/event.rs crates/aletheon-abi/src/lib.rs
git commit -m "feat(abi): add Pid type and CognitivePulse event types"
```

---

## Task 2: Create LlmPulse

**Files:**
- Create: `crates/aletheon-brain/src/impl/llm/pulse.rs`
- Modify: `crates/aletheon-brain/src/impl/llm/scheduler.rs`
- Modify: `crates/aletheon-brain/src/impl/llm/mod.rs`

- [ ] **Step 1: Add health_check to LlmScheduler**

In `crates/aletheon-brain/src/impl/llm/scheduler.rs`, add:

```rust
use aletheon_abi::evolution::ProviderHealth;

impl LlmScheduler {
    /// Check health of all providers.
    pub async fn health_check(&self) -> ProviderHealth {
        // Phase 1: return basic status
        // Phase 2: actually ping providers
        ProviderHealth {
            name: self.default_provider.clone(),
            available: true,
            latency_ms: 0,
            tokens_remaining: None,
        }
    }
}
```

- [ ] **Step 2: Create LlmPulse**

Create `crates/aletheon-brain/src/impl/llm/pulse.rs`:

```rust
//! LlmPulse — the heart of the system.
//!
//! Periodically broadcasts cognitive energy to EventBus.
//! Agents consume this energy to think and act.

use std::sync::Arc;
use std::time::Duration;
use anyhow::Result;
use tokio::sync::watch;
use uuid::Uuid;
use aletheon_abi::{ConcreteEvent, EventBus, EventType, Priority};
use aletheon_abi::evolution::{CognitivePulseEvent, ProviderHealth};
use super::scheduler::LlmScheduler;

/// Configuration for LlmPulse.
#[derive(Debug, Clone)]
pub struct PulseConfig {
    /// Interval between pulses.
    pub interval: Duration,
    /// Token budget per pulse.
    pub token_budget_per_pulse: u32,
}

impl Default for PulseConfig {
    fn default() -> Self {
        Self {
            interval: Duration::from_secs(30),
            token_budget_per_pulse: 100_000,
        }
    }
}

/// The heart — periodically broadcasts cognitive energy to EventBus.
pub struct LlmPulse {
    scheduler: Arc<LlmScheduler>,
    bus: Arc<dyn EventBus>,
    config: PulseConfig,
}

impl LlmPulse {
    pub fn new(
        scheduler: Arc<LlmScheduler>,
        bus: Arc<dyn EventBus>,
        config: PulseConfig,
    ) -> Self {
        Self { scheduler, bus, config }
    }

    /// Start the pulse loop. Runs until shutdown signal.
    pub async fn run(&self, mut shutdown: watch::Receiver<bool>) {
        let mut interval = tokio::time::interval(self.config.interval);
        tracing::info!("LlmPulse started (interval: {:?})", self.config.interval);

        loop {
            tokio::select! {
                _ = interval.tick() => {
                    if let Err(e) = self.pulse().await {
                        tracing::error!("LlmPulse error: {}", e);
                    }
                }
                _ = shutdown.changed() => {
                    tracing::info!("LlmPulse shutting down");
                    break;
                }
            }
        }
    }

    /// Emit one cognitive pulse.
    async fn pulse(&self) -> Result<()> {
        let health = self.scheduler.health_check().await;

        let event = CognitivePulseEvent {
            pulse_id: Uuid::new_v4(),
            timestamp: chrono::Utc::now().to_rfc3339(),
            available_tokens: self.config.token_budget_per_pulse,
            provider_health: health,
        };

        let concrete = ConcreteEvent::new(
            EventType::CognitivePulse,
            Priority::High,
            "llm_pulse".to_string(),
            Box::new(event),
        );

        self.bus.publish(Box::new(concrete)).await
    }

    /// Emit a single pulse (for testing).
    pub async fn pulse_once(&self) -> Result<()> {
        self.pulse().await
    }
}
```

- [ ] **Step 3: Re-export**

In `crates/aletheon-brain/src/impl/llm/mod.rs`, add:

```rust
pub mod pulse;
pub use pulse::{LlmPulse, PulseConfig};
```

- [ ] **Step 4: Add chrono dependency**

In `crates/aletheon-brain/Cargo.toml`, add `chrono` if not already present.

- [ ] **Step 5: Verify**

Run: `cargo check -p aletheon-brain`

- [ ] **Step 6: Commit**

```bash
git add crates/aletheon-brain/src/impl/llm/pulse.rs crates/aletheon-brain/src/impl/llm/scheduler.rs crates/aletheon-brain/src/impl/llm/mod.rs crates/aletheon-brain/Cargo.toml
git commit -m "feat(brain): add LlmPulse — periodic cognitive energy broadcaster"
```

---

## Task 3: Create TokenBudget

**Files:**
- Create: `crates/aletheon-runtime/src/impl/agent/budget.rs`
- Modify: `crates/aletheon-runtime/src/impl/agent/mod.rs`

- [ ] **Step 1: Create TokenBudget**

Create `crates/aletheon-runtime/src/impl/agent/budget.rs`:

```rust
//! Token budget management for AgentProcess.
//!
//! Each agent has a per-pulse energy budget. When the pulse arrives,
//! the agent claims tokens from the pulse and consumes them during ReAct execution.

use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};

/// Manages token budget for an agent process.
pub struct TokenBudget {
    /// Maximum tokens per pulse.
    max_per_pulse: u32,
    /// Remaining tokens in current pulse.
    remaining: AtomicU32,
    /// Total tokens consumed across all pulses.
    total_consumed: AtomicU64,
}

impl TokenBudget {
    pub fn new(max_per_pulse: u32) -> Self {
        Self {
            max_per_pulse,
            remaining: AtomicU32::new(0),
            total_consumed: AtomicU64::new(0),
        }
    }

    /// Claim tokens from a pulse. Returns the amount claimed.
    pub fn claim(&self, pulse_available: u32) -> u32 {
        let claim = pulse_available.min(self.max_per_pulse);
        self.remaining.store(claim, Ordering::SeqCst);
        claim
    }

    /// Consume tokens. Returns remaining budget.
    pub fn consume(&self, tokens: u32) -> u32 {
        let prev = self.remaining.fetch_sub(tokens.min(self.remaining.load(Ordering::SeqCst)), Ordering::SeqCst);
        self.total_consumed.fetch_add(tokens as u64, Ordering::Relaxed);
        prev.saturating_sub(tokens)
    }

    /// Check if budget is exhausted.
    pub fn is_exhausted(&self) -> bool {
        self.remaining.load(Ordering::SeqCst) == 0
    }

    /// Get remaining tokens.
    pub fn remaining(&self) -> u32 {
        self.remaining.load(Ordering::SeqCst)
    }

    /// Get total consumed.
    pub fn total_consumed(&self) -> u64 {
        self.total_consumed.load(Ordering::Relaxed)
    }

    /// Reset for new pulse.
    pub fn reset(&self, pulse_available: u32) {
        let claim = pulse_available.min(self.max_per_pulse);
        self.remaining.store(claim, Ordering::SeqCst);
    }
}
```

- [ ] **Step 2: Register module**

In `crates/aletheon-runtime/src/impl/agent/mod.rs`, add:

```rust
pub mod budget;
pub use budget::TokenBudget;
```

- [ ] **Step 3: Verify**

Run: `cargo check -p aletheon-runtime`

- [ ] **Step 4: Commit**

```bash
git add crates/aletheon-runtime/src/impl/agent/budget.rs crates/aletheon-runtime/src/impl/agent/mod.rs
git commit -m "feat(runtime): add TokenBudget for agent energy management"
```

---

## Task 4: Create AgentProcess

**Files:**
- Create: `crates/aletheon-runtime/src/impl/agent/process.rs`
- Modify: `crates/aletheon-runtime/src/impl/agent/mod.rs`

- [ ] **Step 1: Create AgentProcess**

Create `crates/aletheon-runtime/src/impl/agent/process.rs`:

```rust
//! AgentProcess — a process-like agent entity.
//!
//! Has PID, state machine, energy budget, lifecycle management.
//! Can spawn child processes. Consumes LlmPulse energy to think and act.

use std::sync::Arc;
use std::time::Duration;
use anyhow::Result;
use tokio::sync::RwLock;
use aletheon_abi::{ConcreteEvent, EventBus, EventType, Priority, Pid};
use aletheon_abi::evolution::{
    AgentStartedPayload, AgentStoppedPayload, AgentSpawnedPayload,
    CognitivePulseEvent, ToolObservationPayload,
};
use super::budget::TokenBudget;
use crate::r#impl::engine::cognitive_loop::Engine;
use crate::r#impl::engine::config::EngineConfig;

/// Agent lifecycle state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentState {
    Idle,
    Thinking,
    Acting,
    Reflecting,
    Sleeping,
    Terminated,
}

/// Configuration for an AgentProcess.
#[derive(Debug, Clone)]
pub struct AgentProcessConfig {
    pub max_tokens_per_pulse: u32,
    pub max_children: usize,
    pub idle_timeout: Duration,
    pub can_spawn: bool,
}

impl Default for AgentProcessConfig {
    fn default() -> Self {
        Self {
            max_tokens_per_pulse: 50_000,
            max_children: 4,
            idle_timeout: Duration::from_secs(300),
            can_spawn: true,
        }
    }
}

/// Result of a single ReAct turn.
#[derive(Debug)]
pub enum TurnResult {
    Complete,
    NeedTool,
    NeedReflection,
    Error(String),
}

/// A process-like agent entity.
pub struct AgentProcess {
    pub pid: Pid,
    state: AgentState,
    parent: Option<Pid>,
    children: RwLock<Vec<Pid>>,
    energy: TokenBudget,
    engine: Option<Engine>,
    task: String,
    bus: Arc<dyn EventBus>,
    config: AgentProcessConfig,
}

impl AgentProcess {
    pub fn new(
        parent: Option<Pid>,
        task: String,
        bus: Arc<dyn EventBus>,
        config: AgentProcessConfig,
    ) -> Self {
        Self {
            pid: Pid::new(),
            state: AgentState::Idle,
            parent,
            children: RwLock::new(Vec::new()),
            energy: TokenBudget::new(config.max_tokens_per_pulse),
            engine: None,
            task,
            bus,
            config,
        }
    }

    /// Start the agent: publish AgentStarted event.
    pub async fn start(&mut self) -> Result<()> {
        self.state = AgentState::Idle;

        self.bus.publish(Box::new(ConcreteEvent::new(
            EventType::AgentStarted,
            Priority::Normal,
            format!("agent:{}", self.pid),
            Box::new(AgentStartedPayload {
                pid: self.pid.as_u64(),
                task: self.task.clone(),
            }),
        ))).await?;

        tracing::info!("Agent {} started: {}", self.pid, self.task);
        Ok(())
    }

    /// Handle a cognitive pulse — consume energy to think.
    pub async fn on_pulse(&mut self, pulse: &CognitivePulseEvent) -> Result<()> {
        if self.state == AgentState::Idle
            || self.state == AgentState::Sleeping
            || self.state == AgentState::Terminated
        {
            return Ok(());
        }

        let budget = self.energy.claim(pulse.available_tokens);
        if budget == 0 {
            return Ok(());
        }

        if let Some(engine) = &mut self.engine {
            self.state = AgentState::Thinking;

            // Run one ReAct iteration with budget
            match engine.run_turn_with_budget(budget).await {
                Ok(result) => {
                    self.state = match result {
                        TurnResult::NeedTool => AgentState::Acting,
                        TurnResult::Complete => AgentState::Idle,
                        TurnResult::NeedReflection => AgentState::Reflecting,
                        TurnResult::Error(_) => AgentState::Idle,
                    };
                }
                Err(e) => {
                    tracing::warn!("Agent {} turn error: {}", self.pid, e);
                    self.state = AgentState::Idle;
                }
            }
        }

        Ok(())
    }

    /// Spawn a child agent.
    pub async fn spawn_child(&self, child_task: String) -> Result<Pid> {
        if !self.config.can_spawn {
            anyhow::bail!("Agent {} cannot spawn children", self.pid);
        }

        let children = self.children.read().await;
        if children.len() >= self.config.max_children {
            anyhow::bail!("Agent {} max children ({}) reached", self.pid, self.config.max_children);
        }
        drop(children);

        let child_config = AgentProcessConfig {
            max_tokens_per_pulse: self.config.max_tokens_per_pulse / 2,
            max_children: 0, // leaf agent
            can_spawn: false,
            ..self.config.clone()
        };

        let mut child = AgentProcess::new(
            Some(self.pid),
            child_task,
            self.bus.clone(),
            child_config,
        );
        child.start().await?;
        let child_pid = child.pid;

        self.children.write().await.push(child_pid);

        self.bus.publish(Box::new(ConcreteEvent::new(
            EventType::AgentSpawned,
            Priority::Normal,
            format!("agent:{}", self.pid),
            Box::new(AgentSpawnedPayload {
                parent: self.pid.as_u64(),
                child: child_pid.as_u64(),
            }),
        ))).await?;

        Ok(child_pid)
    }

    /// Terminate the agent.
    pub async fn terminate(&mut self) -> Result<()> {
        self.state = AgentState::Terminated;

        self.bus.publish(Box::new(ConcreteEvent::new(
            EventType::AgentStopped,
            Priority::Normal,
            format!("agent:{}", self.pid),
            Box::new(AgentStoppedPayload {
                pid: self.pid.as_u64(),
            }),
        ))).await?;

        tracing::info!("Agent {} terminated", self.pid);
        Ok(())
    }

    // Accessors
    pub fn pid(&self) -> Pid { self.pid }
    pub fn state(&self) -> AgentState { self.state }
    pub fn task(&self) -> &str { &self.task }
    pub fn parent(&self) -> Option<Pid> { self.parent }
    pub fn energy(&self) -> &TokenBudget { &self.energy }

    /// Attach an Engine to this agent.
    pub fn set_engine(&mut self, engine: Engine) {
        self.engine = Some(engine);
    }
}
```

- [ ] **Step 2: Register module and re-export**

In `crates/aletheon-runtime/src/impl/agent/mod.rs`, add:

```rust
pub mod process;
pub use process::{AgentProcess, AgentProcessConfig, AgentState, TurnResult};
```

- [ ] **Step 3: Verify**

Run: `cargo check -p aletheon-runtime`

- [ ] **Step 4: Commit**

```bash
git add crates/aletheon-runtime/src/impl/agent/process.rs crates/aletheon-runtime/src/impl/agent/mod.rs
git commit -m "feat(runtime): add AgentProcess — process-like agent entity"
```

---

## Task 5: Upgrade AgentRegistry to Process Table

**Files:**
- Modify: `crates/aletheon-runtime/src/impl/orchestration/registry.rs`

- [ ] **Step 1: Add process management to AgentRegistry**

Read the existing `AgentRegistry` first, then add:

```rust
use aletheon_abi::Pid;
use crate::r#impl::agent::{AgentProcess, AgentProcessConfig};
use aletheon_abi::evolution::CognitivePulseEvent;

impl AgentRegistry {
    /// Spawn a new agent process and register it.
    pub async fn spawn_process(
        &self,
        task: String,
        config: AgentProcessConfig,
        bus: Arc<dyn EventBus>,
    ) -> Result<Pid> {
        let mut process = AgentProcess::new(None, task, bus, config);
        process.start().await?;
        let pid = process.pid;
        // Store in a new field: processes: RwLock<HashMap<Pid, Arc<Mutex<AgentProcess>>>>
        Ok(pid)
    }

    /// Dispatch a cognitive pulse to all active agents.
    pub async fn dispatch_pulse(&self, pulse: &CognitivePulseEvent) {
        // Iterate all processes, call on_pulse on each
    }

    /// Get an agent process by PID.
    pub async fn get_process(&self, pid: &Pid) -> Option<Arc<Mutex<AgentProcess>>> {
        // Look up in processes map
    }
}
```

- [ ] **Step 2: Verify**

Run: `cargo check -p aletheon-runtime`

- [ ] **Step 3: Commit**

```bash
git add crates/aletheon-runtime/src/impl/orchestration/registry.rs
git commit -m "feat(runtime): upgrade AgentRegistry to process table"
```

---

## Task 6: Add run_turn_with_budget to Engine

**Files:**
- Modify: `crates/aletheon-runtime/src/impl/engine/cognitive_loop.rs`

- [ ] **Step 1: Add budget-aware turn execution**

Add to Engine:

```rust
use crate::r#impl::agent::TurnResult;

impl Engine {
    /// Execute a ReAct turn with token budget.
    pub async fn run_turn_with_budget(&mut self, budget: u32) -> Result<TurnResult> {
        // Similar to run_turn but:
        // 1. Track token usage per LLM call
        // 2. Stop if budget exhausted
        // 3. Return TurnResult instead of full response
        
        // For now, delegate to existing run_turn and map result
        // Phase 2: add actual budget tracking
        Ok(TurnResult::Complete)
    }
}
```

- [ ] **Step 2: Verify**

Run: `cargo check -p aletheon-runtime`

- [ ] **Step 3: Commit**

```bash
git add crates/aletheon-runtime/src/impl/engine/cognitive_loop.rs
git commit -m "feat(runtime): add run_turn_with_budget to Engine"
```

---

## Task 7: Wire LlmPulse into Daemon

**Files:**
- Modify: `crates/binaries/aletheond/src/main.rs`

- [ ] **Step 1: Start LlmPulse in daemon**

Read the existing `main.rs` first, then add LlmPulse startup:

```rust
use aletheon_brain::r#impl::llm::pulse::{LlmPulse, PulseConfig};
use aletheon_brain::r#impl::llm::scheduler::LlmScheduler;

// In the daemon startup:
let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);

let pulse = LlmPulse::new(scheduler.clone(), bus.clone(), PulseConfig::default());
let pulse_handle = tokio::spawn(async move {
    pulse.run(shutdown_rx).await;
});

// On shutdown:
shutdown_tx.send(true).ok();
pulse_handle.await.ok();
```

- [ ] **Step 2: Verify**

Run: `cargo check -p aletheond`

- [ ] **Step 3: Commit**

```bash
git add crates/binaries/aletheond/src/main.rs
git commit -m "feat(daemon): start LlmPulse in aletheond"
```

---

## Task 8: Integration Test

**Files:**
- Create: `crates/aletheon-runtime/tests/agent_process_test.rs`

- [ ] **Step 1: Create tests**

```rust
#[tokio::test]
async fn test_agent_process_lifecycle() {
    // Create AgentProcess, verify states
}

#[tokio::test]
async fn test_pulse_drives_agent() {
    // Create LlmPulse + AgentProcess
    // Emit pulse, verify agent consumes energy
}

#[tokio::test]
async fn test_spawn_child_agent() {
    // Parent spawns child, verify parent/child relationship
}
```

- [ ] **Step 2: Run tests**

Run: `cargo test -p aletheon-runtime agent_process`

- [ ] **Step 3: Commit**

```bash
git add crates/aletheon-runtime/tests/agent_process_test.rs
git commit -m "test(runtime): add AgentProcess integration tests"
```

---

## Task 9: Final Verification

- [ ] **Step 1: Full workspace test**

Run: `cargo test --workspace`

- [ ] **Step 2: Clippy**

Run: `cargo clippy --workspace -- -D warnings`

- [ ] **Step 3: Final commit**

```bash
git add -A
git commit -m "feat: complete LLM Pulse + AgentProcess

- LlmPulse: periodic cognitive energy broadcaster
- AgentProcess: PID/state/lifecycle/sub-agent spawning
- TokenBudget: energy management per pulse
- AgentRegistry: process table upgrade
- Engine: run_turn_with_budget
- Daemon: LlmPulse integration"
```
