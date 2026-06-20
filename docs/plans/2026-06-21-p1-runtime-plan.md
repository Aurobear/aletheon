# P1: Runtime Layer Implementation Plan

> **For agentic workers:** Use `workflow-feature` or `writing-plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add session management, collaboration mode routing, interrupt mechanism, and sub-agent orchestration to `aletheon-runtime`.

**Architecture:** Extend existing `core/` modules with new files for session, mode routing, interrupt handling, and sub-agent spawning. Modify `orchestrator.rs` and `react_loop.rs` to integrate the new systems. Add new JSON-RPC methods to `handler.rs`.

**Tech Stack:** Rust, tokio, serde_json, aletheon-abi types from P0

**Depends on:** P0 (ABI types)

---

## File Map

```
aletheon-runtime/src/
├── core/
│   ├── mod.rs                    (MODIFY) — add module declarations
│   ├── session.rs                (NEW) — TuiSessionManager, Session, ContextState
│   ├── mode_router.rs            (NEW) — ModeRouter, CollaborationMode → Verdict mapping
│   ├── interrupt.rs              (NEW) — InterruptHandler, cancel flag
│   ├── sub_agent.rs              (NEW) — SubAgentSpawner, SubAgentHandle
│   ├── orchestrator.rs           (MODIFY) — integrate ModeRouter, InterruptHandler
│   └── react_loop.rs             (MODIFY) — add cancel check between iterations
├── impl/
│   └── daemon/
│       └── handler.rs            (MODIFY) — new JSON-RPC methods
```

---

### Task 1: Create `session.rs`

**Files:**
- Create: `crates/aletheon-runtime/src/core/session.rs`

- [ ] **Step 1: Create the file**

```rust
//! Session management for TUI ↔ Runtime communication.
//!
//! Each TUI connection creates a Session that tracks mode, context state,
//! and sub-agents. Sessions persist to JSONL for resume.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Instant;
use aletheon_abi::ui_event::CollaborationMode;
use aletheon_abi::permission::PermissionMode;

/// Context window usage tracking.
#[derive(Debug, Clone)]
pub struct ContextState {
    pub used_tokens: usize,
    pub max_tokens: usize,
    pub compaction_count: usize,
    pub last_compaction: Option<Instant>,
}

impl ContextState {
    pub fn new(max_tokens: usize) -> Self {
        Self {
            used_tokens: 0,
            max_tokens,
            compaction_count: 0,
            last_compaction: None,
        }
    }

    pub fn usage_percent(&self) -> f64 {
        if self.max_tokens == 0 {
            return 0.0;
        }
        (self.used_tokens as f64 / self.max_tokens as f64) * 100.0
    }

    pub fn is_near_limit(&self) -> bool {
        self.usage_percent() > 80.0
    }
}

/// A single session's state.
#[derive(Debug)]
pub struct Session {
    pub id: String,
    pub mode: CollaborationMode,
    pub context_state: ContextState,
    pub model_override: Option<String>,
    pub created_at: Instant,
    pub turn_count: usize,
}

impl Session {
    pub fn new(id: String, max_context_tokens: usize) -> Self {
        Self {
            id,
            mode: CollaborationMode::Default,
            context_state: ContextState::new(max_context_tokens),
            model_override: None,
            created_at: Instant::now(),
            turn_count: 0,
        }
    }

    /// Get the effective permission mode for the current collaboration mode.
    pub fn effective_permission_mode(&self) -> PermissionMode {
        match self.mode {
            CollaborationMode::Default => PermissionMode::Default,
            CollaborationMode::Plan => PermissionMode::Plan,
            CollaborationMode::Auto => PermissionMode::BypassAll,
            CollaborationMode::Sandbox => PermissionMode::Default,
        }
    }

    /// Check if a tool is allowed in the current mode.
    pub fn is_tool_allowed(&self, tool_name: &str) -> bool {
        match self.mode {
            CollaborationMode::Plan => {
                // Only allow read-only tools in plan mode
                matches!(tool_name, "glob" | "grep" | "read" | "web_fetch" | "web_search" | "status")
            }
            _ => true,
        }
    }
}

/// Manages multiple sessions.
#[derive(Debug)]
pub struct TuiSessionManager {
    sessions: HashMap<String, Session>,
    active_session: Option<String>,
    max_context_tokens: usize,
}

impl TuiSessionManager {
    pub fn new(max_context_tokens: usize) -> Self {
        Self {
            sessions: HashMap::new(),
            active_session: None,
            max_context_tokens,
        }
    }

    /// Create a new session and set it as active.
    pub fn create_session(&mut self, id: String) -> String {
        let session = Session::new(id.clone(), self.max_context_tokens);
        self.sessions.insert(id.clone(), session);
        self.active_session = Some(id.clone());
        id
    }

    /// Get the active session.
    pub fn active(&self) -> Option<&Session> {
        self.active_session.as_ref().and_then(|id| self.sessions.get(id))
    }

    /// Get the active session (mutable).
    pub fn active_mut(&mut self) -> Option<&mut Session> {
        self.active_session.as_ref().and_then(|id| {
            // Workaround for borrow checker
            let id = id.clone();
            self.sessions.get_mut(&id)
        })
    }

    /// Switch the active session.
    pub fn switch_to(&mut self, id: &str) -> bool {
        if self.sessions.contains_key(id) {
            self.active_session = Some(id.to_string());
            true
        } else {
            false
        }
    }

    /// List all session IDs.
    pub fn list_sessions(&self) -> Vec<&str> {
        self.sessions.keys().map(|s| s.as_str()).collect()
    }

    /// Remove a session.
    pub fn remove(&mut self, id: &str) -> bool {
        let removed = self.sessions.remove(id).is_some();
        if self.active_session.as_deref() == Some(id) {
            self.active_session = self.sessions.keys().next().cloned();
        }
        removed
    }
}
```

- [ ] **Step 2: Register module in `core/mod.rs`**

Add to `crates/aletheon-runtime/src/core/mod.rs`:
```rust
pub mod session;
```

- [ ] **Step 3: Verify it compiles**

Run: `cargo check -p aletheon-runtime`
Expected: Compiles (may have unused warnings).

- [ ] **Step 4: Commit**

```bash
git add crates/aletheon-runtime/src/core/session.rs crates/aletheon-runtime/src/core/mod.rs
git commit -m "feat(runtime): add TuiSessionManager for TUI session tracking"
```

---

### Task 2: Create `mode_router.rs`

**Files:**
- Create: `crates/aletheon-runtime/src/core/mode_router.rs`

- [ ] **Step 1: Create the file**

```rust
//! Collaboration mode routing.
//!
//! Maps CollaborationMode to SelfField verdicts and tool filtering.

use std::collections::HashSet;
use aletheon_abi::ui_event::CollaborationMode;
use aletheon_abi::self_field::{Verdict, Intent};

/// Routes intents through different behavior paths based on collaboration mode.
#[derive(Debug)]
pub struct ModeRouter {
    current_mode: CollaborationMode,
    /// Tools that are read-only (allowed in Plan mode).
    read_only_tools: HashSet<String>,
}

impl ModeRouter {
    pub fn new() -> Self {
        let mut read_only_tools = HashSet::new();
        for name in &["glob", "grep", "read", "web_fetch", "web_search", "status", "file_read"] {
            read_only_tools.insert(name.to_string());
        }
        Self {
            current_mode: CollaborationMode::Default,
            read_only_tools,
        }
    }

    pub fn current_mode(&self) -> CollaborationMode {
        self.current_mode
    }

    pub fn set_mode(&mut self, mode: CollaborationMode) {
        self.current_mode = mode;
    }

    /// Check if a tool is allowed in the current mode.
    pub fn is_tool_allowed(&self, tool_name: &str) -> bool {
        match self.current_mode {
            CollaborationMode::Plan => self.read_only_tools.contains(tool_name),
            _ => true,
        }
    }

    /// Get the system prompt suffix for the current mode.
    pub fn system_prompt_suffix(&self) -> &'static str {
        self.current_mode.system_prompt_suffix()
    }

    /// Map the current mode to a SelfField verdict override for a given intent.
    /// Returns None if the mode doesn't override verdicts (use normal SelfField flow).
    pub fn verdict_override(&self, intent: &Intent) -> Option<Verdict> {
        match self.current_mode {
            CollaborationMode::Default => None, // Use normal SelfField flow
            CollaborationMode::Plan => {
                // Deny all mutations in plan mode
                if self.read_only_tools.contains(&intent.action) {
                    None // Allow normal flow for read-only tools
                } else {
                    Some(Verdict::Deny { reason: "Plan mode: mutations not allowed".to_string() })
                }
            }
            CollaborationMode::Auto => Some(Verdict::Allow), // Bypass all
            CollaborationMode::Sandbox => {
                // Force sandbox for side-effect tools
                if self.read_only_tools.contains(&intent.action) {
                    None
                } else {
                    Some(Verdict::SandboxFirst { reason: "Sandbox mode: side-effects sandboxed".to_string() })
                }
            }
        }
    }
}

impl Default for ModeRouter {
    fn default() -> Self {
        Self::new()
    }
}
```

- [ ] **Step 2: Register module in `core/mod.rs`**

```rust
pub mod mode_router;
```

- [ ] **Step 3: Verify it compiles**

Run: `cargo check -p aletheon-runtime`

- [ ] **Step 4: Commit**

```bash
git add crates/aletheon-runtime/src/core/mode_router.rs crates/aletheon-runtime/src/core/mod.rs
git commit -m "feat(runtime): add ModeRouter for collaboration mode → verdict mapping"
```

---

### Task 3: Create `interrupt.rs`

**Files:**
- Create: `crates/aletheon-runtime/src/core/interrupt.rs`

- [ ] **Step 1: Create the file**

```rust
//! Interrupt handling for canceling streaming and in-flight operations.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use aletheon_abi::ui_event::InterruptReason;

/// Shared cancel flag for interrupting the ReAct loop.
#[derive(Debug, Clone)]
pub struct InterruptFlag {
    flag: Arc<AtomicBool>,
    reason: Arc<std::sync::Mutex<Option<InterruptReason>>>,
}

impl InterruptFlag {
    pub fn new() -> Self {
        Self {
            flag: Arc::new(AtomicBool::new(false)),
            reason: Arc::new(std::sync::Mutex::new(None)),
        }
    }

    /// Request an interrupt.
    pub fn request(&self, reason: InterruptReason) {
        *self.reason.lock().unwrap() = Some(reason);
        self.flag.store(true, Ordering::SeqCst);
    }

    /// Check if an interrupt has been requested.
    pub fn is_requested(&self) -> bool {
        self.flag.load(Ordering::SeqCst)
    }

    /// Take the interrupt reason (resets the flag).
    pub fn take_reason(&self) -> Option<InterruptReason> {
        if self.flag.swap(false, Ordering::SeqCst) {
            self.reason.lock().unwrap().take()
        } else {
            None
        }
    }

    /// Reset the flag (e.g., at the start of a new turn).
    pub fn reset(&self) {
        self.flag.store(false, Ordering::SeqCst);
        *self.reason.lock().unwrap() = None;
    }
}

impl Default for InterruptFlag {
    fn default() -> Self {
        Self::new()
    }
}
```

- [ ] **Step 2: Register module in `core/mod.rs`**

```rust
pub mod interrupt;
```

- [ ] **Step 3: Verify it compiles**

Run: `cargo check -p aletheon-runtime`

- [ ] **Step 4: Commit**

```bash
git add crates/aletheon-runtime/src/core/interrupt.rs crates/aletheon-runtime/src/core/mod.rs
git commit -m "feat(runtime): add InterruptFlag for canceling ReAct loop"
```

---

### Task 4: Create `sub_agent.rs`

**Files:**
- Create: `crates/aletheon-runtime/src/core/sub_agent.rs`

- [ ] **Step 1: Create the file**

```rust
//! Sub-agent spawning and tracking.
//!
//! Sub-agents are spawned by the LLM via the `agent` tool call.
//! Their status is tracked and emitted to the TUI via UiEvent.

use std::collections::HashMap;
use aletheon_abi::ui_event::{SubAgentHandle, SubAgentStatus};

/// Spawns and tracks sub-agents.
#[derive(Debug)]
pub struct SubAgentSpawner {
    agents: HashMap<String, SubAgentHandle>,
    next_id: usize,
}

impl SubAgentSpawner {
    pub fn new() -> Self {
        Self {
            agents: HashMap::new(),
            next_id: 0,
        }
    }

    /// Register a new sub-agent and return its handle.
    pub fn spawn(&mut self, task: String, parent_turn_id: String) -> SubAgentHandle {
        self.next_id += 1;
        let id = format!("agent-{}", self.next_id);
        let handle = SubAgentHandle {
            id: id.clone(),
            task,
            status: SubAgentStatus::Planning,
            parent_turn_id,
            spawned_at_ms: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64,
        };
        self.agents.insert(id, handle.clone());
        handle
    }

    /// Update an agent's status.
    pub fn update_status(&mut self, id: &str, status: SubAgentStatus) {
        if let Some(agent) = self.agents.get_mut(id) {
            agent.status = status;
        }
    }

    /// Remove a completed/failed agent.
    pub fn remove(&mut self, id: &str) -> bool {
        self.agents.remove(id).is_some()
    }

    /// List all active agents.
    pub fn list(&self) -> Vec<&SubAgentHandle> {
        self.agents.values().collect()
    }

    /// Get a specific agent.
    pub fn get(&self, id: &str) -> Option<&SubAgentHandle> {
        self.agents.get(id)
    }
}

impl Default for SubAgentSpawner {
    fn default() -> Self {
        Self::new()
    }
}
```

- [ ] **Step 2: Register module in `core/mod.rs`**

```rust
pub mod sub_agent;
```

- [ ] **Step 3: Verify it compiles**

Run: `cargo check -p aletheon-runtime`

- [ ] **Step 4: Commit**

```bash
git add crates/aletheon-runtime/src/core/sub_agent.rs crates/aletheon-runtime/src/core/mod.rs
git commit -m "feat(runtime): add SubAgentSpawner for sub-agent orchestration"
```

---

### Task 5: Integrate ModeRouter and InterruptFlag into orchestrator

**Files:**
- Modify: `crates/aletheon-runtime/src/core/orchestrator.rs`

- [ ] **Step 1: Add fields to AletheonRuntime struct**

At line 26, add new fields to the struct:

```rust
pub struct AletheonRuntime {
    config: RuntimeConfig,
    react_loop: ReActLoop,
    evolution: Option<EvolutionCoordinator>,
    genome_config: GenomeConfig,
    memory: Option<Arc<MemoryRouter>>,
    verdict_handler: Arc<dyn VerdictHandler>,
    // NEW
    mode_router: ModeRouter,
    interrupt_flag: InterruptFlag,
    sub_agent_spawner: SubAgentSpawner,
}
```

- [ ] **Step 2: Initialize new fields in `new()`**

In the `new()` constructor (line 36), add:

```rust
mode_router: ModeRouter::new(),
interrupt_flag: InterruptFlag::new(),
sub_agent_spawner: SubAgentSpawner::new(),
```

- [ ] **Step 3: Add accessor methods**

```rust
impl AletheonRuntime {
    // ... existing methods ...

    pub fn mode_router(&self) -> &ModeRouter {
        &self.mode_router
    }

    pub fn mode_router_mut(&mut self) -> &mut ModeRouter {
        &mut self.mode_router
    }

    pub fn interrupt_flag(&self) -> &InterruptFlag {
        &self.interrupt_flag
    }

    pub fn sub_agent_spawner(&self) -> &SubAgentSpawner {
        &self.sub_agent_spawner
    }

    pub fn sub_agent_spawner_mut(&mut self) -> &mut SubAgentSpawner {
        &mut self.sub_agent_spawner
    }
}
```

- [ ] **Step 4: Integrate mode check in `process_react()`**

In `process_react()` (line 270), before running the ReAct loop, inject the mode's system prompt suffix:

```rust
// After building system prompt, before react_loop.run()
let mode_suffix = self.mode_router.system_prompt_suffix();
if !mode_suffix.is_empty() {
    // Append mode suffix to system prompt
    let current_prompt = react_loop.system_prompt();
    react_loop.set_system_prompt(&format!("{}\n\n{}", current_prompt, mode_suffix));
}
```

- [ ] **Step 5: Verify it compiles**

Run: `cargo check -p aletheon-runtime`
Expected: Compiles (may have unused import warnings).

- [ ] **Step 6: Commit**

```bash
git add crates/aletheon-runtime/src/core/orchestrator.rs
git commit -m "feat(runtime): integrate ModeRouter, InterruptFlag, SubAgentSpawner into orchestrator"
```

---

### Task 6: Add cancel check to ReAct loop

**Files:**
- Modify: `crates/aletheon-runtime/src/core/react_loop.rs`

- [ ] **Step 1: Add InterruptFlag field to ReActLoop**

Add to the `ReActLoop` struct (line 109):

```rust
pub struct ReActLoop {
    // ... existing fields ...
    interrupt_flag: Option<InterruptFlag>,
}
```

- [ ] **Step 2: Add setter method**

```rust
impl ReActLoop {
    // ... existing methods ...

    pub fn set_interrupt_flag(&mut self, flag: InterruptFlag) {
        self.interrupt_flag = Some(flag);
    }
}
```

- [ ] **Step 3: Add cancel check in `run_streaming()` loop**

In `run_streaming()` (line 456), at the top of each iteration:

```rust
// Check for interrupt
if let Some(ref flag) = self.interrupt_flag {
    if let Some(reason) = flag.take_reason() {
        // Emit partial response and return
        event_sink.emit(Event::TurnDone {
            result: Ok(format!("[Interrupted: {:?}]", reason))
        });
        return Ok((format!("[Interrupted: {:?}]", reason), TurnMetrics {
            tool_calls_made,
            tool_errors,
            elapsed_ms: start.elapsed().as_millis() as u64,
            iterations: self.iteration,
            completed_normally: false,
        }));
    }
}
```

- [ ] **Step 4: Verify it compiles**

Run: `cargo check -p aletheon-runtime`

- [ ] **Step 5: Commit**

```bash
git add crates/aletheon-runtime/src/core/react_loop.rs
git commit -m "feat(runtime): add interrupt check to ReAct loop"
```

---

### Task 7: Add new JSON-RPC methods to handler

**Files:**
- Modify: `crates/aletheon-runtime/src/impl/daemon/handler.rs`

- [ ] **Step 1: Add `interrupt` method**

In the `handle()` method (line 737), add a new match arm:

```rust
"interrupt" => {
    let reason = match params.and_then(|p| p.get("reason").and_then(|r| r.as_str())).unwrap_or("user_cancelled") {
        "user_cancelled" => InterruptReason::UserCancelled,
        "timeout" => InterruptReason::Timeout,
        "budget_exceeded" => InterruptReason::BudgetExceeded,
        _ => InterruptReason::UserCancelled,
    };
    self.runtime.interrupt_flag().request(reason);
    serde_json::json!({"status": "interrupt_requested", "reason": format!("{:?}", reason)})
}
```

- [ ] **Step 2: Add `mode_switch` method**

```rust
"mode_switch" => {
    let mode_str = params
        .and_then(|p| p.get("mode").and_then(|m| m.as_str()))
        .unwrap_or("default");
    let mode = match mode_str {
        "plan" => CollaborationMode::Plan,
        "auto" => CollaborationMode::Auto,
        "sandbox" => CollaborationMode::Sandbox,
        _ => CollaborationMode::Default,
    };
    let old_mode = self.runtime.mode_router().current_mode();
    self.runtime.mode_router_mut().set_mode(mode);
    serde_json::json!({
        "status": "mode_switched",
        "old": old_mode.display_name(),
        "new": mode.display_name()
    })
}
```

- [ ] **Step 3: Add `sub_agents` method**

```rust
"sub_agents" => {
    let agents: Vec<_> = self.runtime.sub_agent_spawner().list().iter().map(|a| {
        serde_json::json!({
            "id": a.id,
            "task": a.task,
            "status": format!("{:?}", a.status),
        })
    }).collect();
    serde_json::json!({"agents": agents})
}
```

- [ ] **Step 4: Verify it compiles**

Run: `cargo check -p aletheon-runtime`

- [ ] **Step 5: Commit**

```bash
git add crates/aletheon-runtime/src/impl/daemon/handler.rs
git commit -m "feat(runtime): add interrupt, mode_switch, sub_agents JSON-RPC methods"
```

---

### Task 8: Run full Runtime test suite

- [ ] **Step 1: Run tests**

Run: `cargo test -p aletheon-runtime`
Expected: All tests pass.

- [ ] **Step 2: Run clippy**

Run: `cargo clippy -p aletheon-runtime -- -D warnings`
Expected: No warnings.

- [ ] **Step 3: Final commit**

```bash
git add -A
git commit -m "chore(runtime): P1 Runtime complete — session, mode router, interrupt, sub-agents"
```

---

## Summary

P1 adds 4 new files and modifies 3 existing files in `aletheon-runtime`:

| File | Action | What Added |
|------|--------|------------|
| `session.rs` | NEW | `TuiSessionManager`, `Session`, `ContextState` |
| `mode_router.rs` | NEW | `ModeRouter` — mode → verdict mapping |
| `interrupt.rs` | NEW | `InterruptFlag` — shared cancel flag |
| `sub_agent.rs` | NEW | `SubAgentSpawner` — sub-agent tracking |
| `orchestrator.rs` | MODIFY | Integrate new fields + accessors |
| `react_loop.rs` | MODIFY | Add interrupt check between iterations |
| `handler.rs` | MODIFY | New JSON-RPC methods: interrupt, mode_switch, sub_agents |
