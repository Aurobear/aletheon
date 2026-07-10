# Group A: Boundary Establishment — Implementation Plan

> **For agentic workers:** Use `workflow-feature` or `plans` to implement this plan task-by-task.
> Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Define 4 ops traits + Harness trait in `base`, create `CoreSystems` struct in `runtime`, remove 4 dead fields from `RequestHandler`, group remaining fields into `CoreSystems` achieving ≤10 fields on `RequestHandler`.

**Architecture:** Scaffold trait interfaces in `base/src/ops.rs` without breaking existing code. Create `CoreSystems` holding concrete subsystem types (trait wiring deferred to Group B). All `self.xxx` access in `chat.rs`/`init.rs` becomes `self.subsystems.xxx`. Dead fields removed entirely.

**Tech Stack:** Rust, async-trait (already in base/runtime deps), tokio

---

### Task 1: Define ops traits in base

**Files:**
- Create: `crates/base/src/ops.rs`
- Modify: `crates/base/src/lib.rs`

**Verify:** `cargo build -p base`

- [ ] **Step 1: Create `crates/base/src/ops.rs`**

`base` already depends on `async-trait`, `serde_json`, `anyhow`. Use `serde_json::Value` for untyped inter-subsystem data (typed signatures come in a follow-up).

```rust
//! Subsystem operation traits — the contract between Executive and subsystems.
//!
//! Each trait defines the interface that Executive uses to delegate work.
//! Implementations live in the respective subsystem crates and are wired
//! through `CoreSystems` in the runtime.

use async_trait::async_trait;
use anyhow::Result;

// ---------------------------------------------------------------------------
// Subsystem ops traits
// ---------------------------------------------------------------------------

/// Cognitive operations — reasoning, planning, reflection, learning.
#[async_trait]
pub trait CognitOps: Send + Sync {
    /// Build a working context for a session from raw messages and state.
    async fn build_context(
        &self,
        session_id: &str,
        messages: &[crate::Message],
    ) -> Result<serde_json::Value>;
    /// Generate a plan from context and goal description.
    async fn reason(&self, ctx: &serde_json::Value, goal: &str) -> Result<serde_json::Value>;
    /// Reflect on an execution outcome, returning structured reflection.
    async fn reflect(&self, outcome: &serde_json::Value) -> Result<serde_json::Value>;
}

/// Dasein (self-field) operations — identity, boundary, narrative, continuity.
#[async_trait]
pub trait DaseinOps: Send + Sync {
    /// Review an intent through the SelfField policy engine.
    async fn review(
        &self,
        intent: &crate::Intent,
        ctx: &crate::Context,
    ) -> Result<crate::Verdict>;
    /// Record a narrative event for continuity/future audit.
    async fn narrate(&self, event: &str, detail: &str);
    /// Snapshot current Dasein state (identity, cares, boundaries).
    async fn snapshot(&self) -> Result<serde_json::Value>;
}

/// Mnemosyne (memory) operations — recall, store, prompt composition.
#[async_trait]
pub trait MnemosyneOps: Send + Sync {
    /// Recall relevant facts/episodes for a query.
    async fn recall(&self, query: &str, limit: usize) -> Result<Vec<serde_json::Value>>;
    /// Store a memory block (fact, episode, or reflection).
    async fn store(&self, block: &serde_json::Value) -> Result<()>;
    /// Compose the memory portion of the system prompt for a session.
    async fn compose_prompt_block(&self, session_id: &str) -> Result<String>;
    /// Run background consolidation (decay, importance update, replay).
    async fn consolidate(&self) -> Result<()>;
}

/// Corpus (body) operations — tool execution, skill matching, hooks.
#[async_trait]
pub trait CorpusOps: Send + Sync {
    /// Execute a tool call with input and return the result.
    async fn execute_tool(
        &self,
        tool_name: &str,
        input: serde_json::Value,
        session_id: &str,
    ) -> Result<crate::ToolResult>;
    /// List all available tool definitions.
    async fn list_tools(&self) -> Result<Vec<crate::ToolDefinition>>;
    /// Run lifecycle hooks for an event; returns hook results.
    async fn run_hooks(&self, event: &crate::HookContext) -> Result<Vec<crate::HookResult>>;
}

// ---------------------------------------------------------------------------
// Harness traits
// ---------------------------------------------------------------------------

/// A tool executor — abstracts tool dispatch for harnesses.
#[async_trait]
pub trait ToolExecutor: Send + Sync {
    async fn execute(
        &self,
        name: &str,
        input: serde_json::Value,
    ) -> Result<crate::ToolResult>;
}

/// A cognitive harness orchestrates a reasoning pipeline.
///
/// Harnesses are pluggable:
/// - `LinearCognitiveHarness` (current ReAct equivalent)
/// - Future: `ResearchHarness`, `CodingHarness`, `RobotHarness`, `OSHarness`
#[async_trait]
pub trait CognitiveHarness: Send + Sync {
    /// Run the harness: input → (response text, metrics).
    async fn run(
        &self,
        input: &str,
        messages: &[crate::Message],
        tool_defs: &[crate::ToolDefinition],
        executor: &dyn ToolExecutor,
    ) -> Result<(String, serde_json::Value)>;
}
```

- [ ] **Step 2: Register module and re-exports in `crates/base/src/lib.rs`**

Add after `pub mod types;` (line 30):
```rust
pub mod ops;
```

Add before the closing line (after line 182, before the last blank line):
```rust
// Ops traits (from ops/)
pub use ops::{CognitOps, CognitiveHarness, CorpusOps, DaseinOps, MnemosyneOps, ToolExecutor};
```

- [ ] **Step 3: Verify**

Run: `cargo build -p base`
Expected: PASS

- [ ] **Step 4: Commit**

```bash
git add crates/base/src/ops.rs crates/base/src/lib.rs
git commit -m "feat(base): add subsystem ops traits and CognitiveHarness trait

- CognitOps/DaseinOps/MnemosyneOps/CorpusOps: the 4 subsystem contracts
- CognitiveHarness: pluggable reasoning pipeline interface
- ToolExecutor: tool dispatch abstraction for harnesses

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 2: Remove dead/parked fields from RequestHandler

**Files:**
- Modify: `crates/runtime/src/impl/daemon/handler/mod.rs` (struct + imports)
- Modify: `crates/runtime/src/impl/daemon/handler/init.rs` (constructor + imports)

**Verify:** `cargo build -p runtime`

The 4 dead fields with `#[allow(dead_code)]` or `parked` comments:
1. `agent_registry: Arc<AgentRegistry>` (mod.rs:100-101) — parked/multi-agent unwired
2. `checkpoint_store: Arc<Mutex<CheckpointStore>>` (mod.rs:146-147) — parked
3. `agent_loader: Arc<Mutex<AgentLoader>>` (mod.rs:152-153) — parked
4. `event_bus: Option<Arc<CommunicationBus>>` (mod.rs:173-174) — dead, replaced by `bus`

- [ ] **Step 1: Remove fields from RequestHandler struct (mod.rs)**

Remove these 4 lines from the struct definition (~lines 100-101, 146-147, 152-153, 173-174):

```rust
// REMOVE (around line 100-101):
    #[allow(dead_code)]
    agent_registry: Arc<AgentRegistry>,

// REMOVE (around line 146-147):
    #[allow(dead_code)]
    checkpoint_store: Arc<Mutex<CheckpointStore>>,

// REMOVE (around line 152-153):
    #[allow(dead_code)]
    agent_loader: Arc<Mutex<AgentLoader>>,

// REMOVE (around line 173-174):
    #[allow(dead_code)]
    event_bus: Option<Arc<CommunicationBus>>,
```

- [ ] **Step 2: Remove field assignments from init.rs constructor**

In the `Self { ... }` block (init.rs ~line 581-628), remove:
```rust
// REMOVE:
            agent_registry,
// REMOVE:
            checkpoint_store,
// REMOVE:
            agent_loader,
// REMOVE:
            event_bus,
```

Also remove the code that creates these values earlier in `new()`:

**Remove agent_registry creation (~lines 254-284):** Delete from `// Agent registry` comment through the closing `}` of the `if agent_registry.count().await == 0` block.

**Remove checkpoint_store creation (~lines 494-497):**
```rust
// REMOVE:
        let session_dir = aletheon_dir.join("sessions").join(&session_id);
        std::fs::create_dir_all(&session_dir)?;
        let checkpoint_store = CheckpointStore::new(&session_dir);
        let checkpoint_store = Arc::new(Mutex::new(checkpoint_store));
```

**Remove agent_loader creation (~lines 509-515):**
```rust
// REMOVE:
        let mut agent_loader = AgentLoader::new();
        let agents_dir = aletheon_dir.join("agents");
        if agents_dir.exists() {
            let _ = agent_loader.load_from_dir(&agents_dir);
            info!("Loaded {} agent roles", agent_loader.list().len());
        }
        let agent_loader = Arc::new(Mutex::new(agent_loader));
```

- [ ] **Step 3: Clean up unused imports in init.rs**

Remove:
```rust
use crate::r#impl::orchestration::builtin::{CodeAgent, FsAgent, NetAgent};
use crate::r#impl::orchestration::registry::AgentRegistry;
use crate::core::checkpoint::CheckpointStore;
use crate::r#impl::agent_loader::AgentLoader;
```

Also remove `use base::Registry;` and `use base::Version;` if they were only used for agent_registry.

- [ ] **Step 4: Clean up unused imports in mod.rs**

Remove:
```rust
use crate::r#impl::orchestration::registry::AgentRegistry;
use crate::core::checkpoint::CheckpointStore;
use crate::r#impl::agent_loader::AgentLoader;
```

- [ ] **Step 5: Verify**

Run: `cargo build -p runtime 2>&1`
Expected: PASS (may need to add `#[allow(unused_imports)]` temporarily if imports were shared)

- [ ] **Step 6: Commit**

```bash
git add crates/runtime/src/impl/daemon/handler/mod.rs crates/runtime/src/impl/daemon/handler/init.rs
git commit -m "refactor(runtime): remove 4 dead fields from RequestHandler

Remove agent_registry (parked), checkpoint_store (parked),
agent_loader (parked), event_bus (dead/replaced by bus).

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 3: Create CoreSystems struct

**Files:**
- Create: `crates/runtime/src/core/core_systems.rs`
- Modify: `crates/runtime/src/core/mod.rs`

**Verify:** `cargo build -p runtime`

- [ ] **Step 1: Create `crates/runtime/src/core/core_systems.rs`**

```rust
//! CoreSystems — concrete subsystem type bundle.
//!
//! Holds all subsystem types that `RequestHandler` currently owns directly.
//! During Group B, each field transitions from its concrete type to
//! `Arc<dyn TraitOps>` from `base::ops` as each subsystem gets migrated.
//!
//! This is the intermediate step between the 36-field God Object and the
//! final trait-based architecture defined in RFC-010~013.

use std::sync::Arc;
use tokio::sync::Mutex;

use crate::core::config::HooksConfig;
use crate::core::orchestrator::AletheonRuntime;
use crate::core::storm_breaker::StormBreaker;
use crate::r#impl::goal::ObjectiveStore;
use crate::r#impl::hooks::registry::HookRegistry;
use crate::r#impl::memory::auto_memory::AutoMemory;
use crate::r#impl::memory::fact_store::FactStore;
use crate::r#impl::skill_router::SkillRouter;
use crate::r#impl::skills::loader::SkillLoader;
use crate::CoreMemory;
use crate::RecallMemory;
use cognit::core::reflector::Reflector;
use corpus::security::security::runner::ToolRunnerWithGuard;
use corpus::tools::tools::ToolRegistry;
use dasein::SelfField;
use memory::episodic::EpisodicMemory;
use metacog::{DefaultMetaRuntime, MorphogenesisPipeline};

/// Bundle of subsystem types held by RequestHandler.
///
/// In Group B, each field transitions to `Arc<dyn TraitOps>`.
pub struct CoreSystems {
    // --- Orchestrator ---
    pub runtime: AletheonRuntime,

    // --- Dasein (SelfField) ---
    pub self_field: Arc<Mutex<SelfField>>,

    // --- Mnemosyne (Memory) ---
    pub episodic_memory: Arc<Mutex<EpisodicMemory>>,
    pub recall_memory: Arc<Mutex<RecallMemory>>,
    pub core_memory: Arc<Mutex<CoreMemory>>,
    pub fact_store: Arc<Mutex<FactStore>>,
    pub auto_memory: Arc<Mutex<AutoMemory>>,
    pub objective_store: Arc<Mutex<ObjectiveStore>>,

    // --- Cognit ---
    pub reflector: Reflector,

    // --- Corpus ---
    pub tools: Arc<Mutex<ToolRegistry>>,
    pub tool_runner: Arc<Mutex<ToolRunnerWithGuard>>,
    pub skill_loader: Arc<Mutex<SkillLoader>>,
    pub skill_router: Arc<Mutex<SkillRouter>>,
    pub hook_registry: Arc<Mutex<HookRegistry>>,
    pub storm_breaker: Arc<Mutex<StormBreaker>>,
    pub hooks_config: HooksConfig,

    // --- Metacog ---
    pub pipeline: Arc<MorphogenesisPipeline<DefaultMetaRuntime>>,
}
```

- [ ] **Step 2: Register in `crates/runtime/src/core/mod.rs`**

Add module declaration after `pub mod controller;`:
```rust
pub mod core_systems;
```

Add pub use after existing pub uses:
```rust
pub use core_systems::CoreSystems;
```

- [ ] **Step 3: Verify**

Run: `cargo build -p runtime`
Expected: PASS (unused warnings OK at this stage)

- [ ] **Step 4: Commit**

```bash
git add crates/runtime/src/core/core_systems.rs crates/runtime/src/core/mod.rs
git commit -m "feat(runtime): add CoreSystems struct to bundle subsystem types

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 4: Reduce RequestHandler to ≤10 fields via CoreSystems

**Files:**
- Modify: `crates/runtime/src/impl/daemon/handler/mod.rs` (struct + imports + method signatures)
- Modify: `crates/runtime/src/impl/daemon/handler/init.rs` (constructor)
- Modify: `crates/runtime/src/impl/daemon/handler/chat.rs` (all `self.xxx` → `self.subsystems.xxx`)
- Modify: `crates/runtime/src/impl/daemon/handler/session_routing.rs` (field accesses)
- Modify: `crates/runtime/src/impl/daemon/handler/connection.rs` (field accesses)
- Modify: `crates/runtime/src/impl/daemon/handler/turn_handler.rs` (field accesses)
- Modify: `crates/runtime/src/impl/daemon/handler/rpc.rs` (field accesses)

**Verify:** `cargo build -p runtime`

This is the main mechanical refactor. The strategy:
1. Define the new 10-field `RequestHandler`
2. Update all `self.xxx` accesses to `self.subsystems.xxx` for subsystem fields
3. Update `init.rs` to construct `CoreSystems` and the new slim `RequestHandler`

- [ ] **Step 1: Define new 10-field RequestHandler**

Replace the entire `RequestHandler` struct (mod.rs lines 80-179) with:

```rust
#[derive(Clone)]
pub struct RequestHandler {
    /// All subsystem types — will become `Arc<dyn TraitOps>` in Group B.
    pub(crate) subsystems: Arc<CoreSystems>,
    /// Multi-session registry: session_id → SessionManager.
    pub(crate) sessions: Arc<Mutex<HashMap<String, Arc<Mutex<SessionManager>>>>>,
    /// Session gateway for external agent debug access.
    pub(crate) session_gateway: Arc<SessionGateway>,
    /// Communication bus for inter-module messages and notifications.
    pub(crate) bus: Arc<CommunicationBus>,
    /// Default LLM provider.
    pub(crate) llm: Arc<dyn LlmProvider>,
    /// Model router for dynamic model selection per task type.
    pub(crate) model_router: Arc<ModelRouter>,
    /// Notification channel for out-of-band JSON-RPC notifications to the client.
    pub(crate) notify_tx: Option<mpsc::Sender<String>>,
    /// Active connection count.
    pub(crate) active_connections: Arc<AtomicUsize>,
    /// Daemon start time for uptime calculation.
    pub(crate) started_at: Instant,
    /// Daemon-level cancellation token for graceful shutdown.
    pub(crate) daemon_cancel_token: Option<CancellationToken>,
}
```

This is exactly 10 fields. Fields that went INTO CoreSystems: `state` (via `AletheonRuntime`), `self_field`, `recall_memory`, `episodic_memory`, `core_memory`, `fact_store`, `auto_memory`, `objective_store`, `reflector`, `tools`, `tool_runner`, `skill_loader`, `skill_router`, `hook_registry`, `storm_breaker`, `hooks_config`, `pipeline`, `cached_prefix`, `memory_queue`, `config_prompt`, `approval_rx`, `pending_approvals`, `session_approvals`, `debug_handler`, `debug_perf`, `cancel_token`, `data_dir`, `context_window`, `default_session_id`, `session_created_at`.

Wait — many of those remaining fields are still needed and can't just be deleted. Let me be more careful about what CoreSystems absorbs vs what stays in RequestHandler.

**Fields that go INTO CoreSystems (subsystem-owned):**
- `state: Arc<Mutex<SessionState>>` → `CoreSystems.runtime: AletheonRuntime` (already defined)
- `self_field` → already in CoreSystems
- `episodic_memory` → already in CoreSystems
- `recall_memory` → already in CoreSystems
- `core_memory` → already in CoreSystems
- `fact_store` → already in CoreSystems
- `auto_memory` → already in CoreSystems
- `objective_store` → already in CoreSystems
- `reflector` → already in CoreSystems
- `tools` → already in CoreSystems
- `tool_runner` → already in CoreSystems
- `skill_loader` → already in CoreSystems
- `skill_router` → already in CoreSystems
- `hook_registry` → already in CoreSystems
- `storm_breaker` → already in CoreSystems
- `hooks_config` → already in CoreSystems
- `pipeline` → already in CoreSystems

**Fields that go INTO CoreSystems (prefix/context building):**
- `cached_prefix: Arc<Mutex<String>>`
- `memory_queue: Arc<Mutex<Vec<String>>>`
- `config_prompt: String`

**Fields that go INTO CoreSystems (approval/security):**
- `approval_rx: Arc<Mutex<mpsc::Receiver<PendingApproval>>>`
- `pending_approvals: Arc<Mutex<HashMap<String, oneshot::Sender<ApprovalDecision>>>>`
- `session_approvals: Arc<Mutex<HashMap<String, bool>>>`

**Fields that go INTO CoreSystems (debug/observability):**
- `debug_handler: Arc<DebugHandler>`
- `debug_perf: Arc<PerfCounter>`
- `cancel_token: Arc<Mutex<Option<CancellationToken>>>`

**Fields that go INTO CoreSystems (session management):**
- `default_session_id: Arc<tokio::sync::Mutex<String>>`
- `session_created_at: Arc<Mutex<HashMap<String, Instant>>>`
- `data_dir: PathBuf`
- `context_window: usize`

**Fields STAYING in RequestHandler (host-level):**
- `sessions: Arc<Mutex<HashMap<String, Arc<Mutex<SessionManager>>>>>` — host-level
- `session_gateway: Arc<SessionGateway>` — host-level
- `bus: Arc<CommunicationBus>` — host-level (no Option, always Some after init)
- `llm: Arc<dyn LlmProvider>` — host-level (transient)
- `model_router: Arc<ModelRouter>` — host-level
- `notify_tx: Option<mpsc::Sender<String>>` — host-level (per-connection)
- `active_connections: Arc<AtomicUsize>` — host-level
- `started_at: Instant` — host-level
- `daemon_cancel_token: Option<CancellationToken>` — host-level

And we still need `bus` to NOT be Optional anymore (it was always `Some` after init). And `cancel_token` (per-turn) moves into CoreSystems.

OK so the final list is:

```rust
pub struct RequestHandler {
    pub(crate) subsystems: Arc<CoreSystems>,                    // 1
    pub(crate) sessions: Arc<Mutex<HashMap<...>>>,              // 2
    pub(crate) session_gateway: Arc<SessionGateway>,            // 3
    pub(crate) bus: Arc<CommunicationBus>,                      // 4  (no Option)
    pub(crate) llm: Arc<dyn LlmProvider>,                       // 5
    pub(crate) model_router: Arc<ModelRouter>,                  // 6
    pub(crate) notify_tx: Option<mpsc::Sender<String>>,         // 7
    pub(crate) active_connections: Arc<AtomicUsize>,            // 8
    pub(crate) started_at: Instant,                             // 9
    pub(crate) daemon_cancel_token: Option<CancellationToken>,  // 10
}
```

That's 10 fields exactly. But I need to update `CoreSystems` to include all the fields being moved. Let me list the complete CoreSystems:

```rust
pub struct CoreSystems {
    // --- Orchestrator ---
    pub runtime: AletheonRuntime,

    // --- Dasein (SelfField) ---
    pub self_field: Arc<Mutex<SelfField>>,

    // --- Mnemosyne (Memory) ---
    pub episodic_memory: Arc<Mutex<EpisodicMemory>>,
    pub recall_memory: Arc<Mutex<RecallMemory>>,
    pub core_memory: Arc<Mutex<CoreMemory>>,
    pub fact_store: Arc<Mutex<FactStore>>,
    pub auto_memory: Arc<Mutex<AutoMemory>>,
    pub objective_store: Arc<Mutex<ObjectiveStore>>,

    // --- Cognit ---
    pub reflector: Reflector,

    // --- Corpus ---
    pub tools: Arc<Mutex<ToolRegistry>>,
    pub tool_runner: Arc<Mutex<ToolRunnerWithGuard>>,
    pub skill_loader: Arc<Mutex<SkillLoader>>,
    pub skill_router: Arc<Mutex<SkillRouter>>,
    pub hook_registry: Arc<Mutex<HookRegistry>>,
    pub storm_breaker: Arc<Mutex<StormBreaker>>,
    pub hooks_config: HooksConfig,

    // --- Metacog ---
    pub pipeline: Arc<MorphogenesisPipeline<DefaultMetaRuntime>>,

    // --- Approval / Security ---
    pub approval_rx: Arc<Mutex<mpsc::Receiver<PendingApproval>>>,
    pub pending_approvals: Arc<Mutex<HashMap<String, oneshot::Sender<ApprovalDecision>>>>,
    pub session_approvals: Arc<Mutex<HashMap<String, bool>>>,

    // --- Debug / Observability ---
    pub debug_handler: Arc<DebugHandler>,
    pub debug_perf: Arc<PerfCounter>,
    pub cancel_token: Arc<Mutex<Option<CancellationToken>>>,

    // --- Prefix / Context building ---
    pub cached_prefix: Arc<Mutex<String>>,
    pub memory_queue: Arc<Mutex<Vec<String>>>,
    pub config_prompt: String,

    // --- Session management ---
    pub default_session_id: Arc<tokio::sync::Mutex<String>>,
    pub session_created_at: Arc<Mutex<HashMap<String, Instant>>>,
    pub data_dir: PathBuf,
    pub context_window: usize,
}
```

That's ~30 fields in CoreSystems. The key mechanical change is: every `self.xxx` in chat.rs, init.rs, session_routing.rs, connection.rs, turn_handler.rs, and rpc.rs becomes `self.subsystems.xxx` for fields that moved to CoreSystems.

This is a LARGE mechanical refactor. Rather than listing every single change, I'll describe the systematic replacement pattern and cover the key ones. The actual execution should use a script or careful find-replace.

Actually, for the plan I should be explicit about which fields go where and what the mechanical changes are. Let me list the mapping:

**self.xxx → self.subsystems.xxx:**
- self.state → self.subsystems.runtime (different name!)
- self.self_field → self.subsystems.self_field
- self.recall_memory → self.subsystems.recall_memory
- self.episodic_memory → self.subsystems.episodic_memory
- self.core_memory → self.subsystems.core_memory
- self.fact_store → self.subsystems.fact_store
- self.auto_memory → self.subsystems.auto_memory
- self.objective_store → self.subsystems.objective_store
- self.reflector → self.subsystems.reflector
- self.tools → self.subsystems.tools
- self.tool_runner → self.subsystems.tool_runner
- self.skill_loader → self.subsystems.skill_loader
- self.skill_router → self.subsystems.skill_router
- self.hook_registry → self.subsystems.hook_registry
- self.storm_breaker → self.subsystems.storm_breaker
- self.hooks_config → self.subsystems.hooks_config
- self.pipeline → self.subsystems.pipeline
- self.approval_rx → self.subsystems.approval_rx
- self.pending_approvals → self.subsystems.pending_approvals
- self.session_approvals → self.subsystems.session_approvals
- self.debug_handler → self.subsystems.debug_handler
- self.debug_perf → self.subsystems.debug_perf
- self.cancel_token → self.subsystems.cancel_token
- self.cached_prefix → self.subsystems.cached_prefix
- self.memory_queue → self.subsystems.memory_queue
- self.config_prompt → self.subsystems.config_prompt
- self.default_session_id → self.subsystems.default_session_id
- self.session_created_at → self.subsystems.session_created_at
- self.data_dir → self.subsystems.data_dir
- self.context_window → self.subsystems.context_window

**self.xxx (unchanged — host-level):**
- self.sessions
- self.session_gateway
- self.bus (was Option, now Arc)
- self.llm
- self.model_router
- self.notify_tx
- self.active_connections
- self.started_at
- self.daemon_cancel_token

Given the size of this mechanical change (~100+ lines across 6+ files), I need to be realistic about the plan size. Let me structure this as one big task with clear instructions, and be clear about the systematic replacement.

Let me write the plan now.<｜end▁of▁thinking｜>This plan is large. Let me write it properly with all the detail needed for a mechanical refactor.

<｜｜DSML｜｜tool_calls>
<｜｜DSML｜｜invoke name="Write">
<｜｜DSML｜｜parameter name="content" string="true"># Group A: Boundary Establishment — Implementation Plan

> **For agentic workers:** Use `workflow-feature` or `plans` to implement this plan task-by-task.
> Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Define 4 ops traits + Harness trait in `base`, create `CoreSystems` struct, remove 4 dead fields, group remaining fields achieving ≤10 on `RequestHandler`.

**Architecture:** Scaffold traits in `base/src/ops.rs`. Create `CoreSystems` holding concrete subsystem types. All `self.xxx` subsystem accesses become `self.subsystems.xxx`. Dead fields removed entirely. Trait wiring deferred to Group B.

**Tech Stack:** Rust, async-trait (already in base/runtime deps), tokio

---

### Task 1: Define ops traits in `base`

**Files:**
- Create: `crates/base/src/ops.rs`
- Modify: `crates/base/src/lib.rs`

**Verify:** `cargo build -p base`

- [ ] **Step 1: Create `crates/base/src/ops.rs`**

`base` already has `async-trait = "0.1"`, `serde_json`, `anyhow`. Use `serde_json::Value` for inter-subsystem data (typed signatures deferred).

```rust
//! Subsystem operation traits — the contract between Executive and subsystems.
//!
//! Each trait defines the interface that Executive uses to delegate work.
//! Implementations live in the respective subsystem crates and are wired
//! through `CoreSystems` in the runtime (Group B).

use async_trait::async_trait;
use anyhow::Result;

// ---------------------------------------------------------------------------
// Subsystem ops traits
// ---------------------------------------------------------------------------

/// Cognitive operations — reasoning, planning, reflection, learning.
#[async_trait]
pub trait CognitOps: Send + Sync {
    async fn build_context(
        &self,
        session_id: &str,
        messages: &[crate::Message],
    ) -> Result<serde_json::Value>;
    async fn reason(&self, ctx: &serde_json::Value, goal: &str) -> Result<serde_json::Value>;
    async fn reflect(&self, outcome: &serde_json::Value) -> Result<serde_json::Value>;
}

/// Dasein (self-field) operations — identity, boundary, narrative.
#[async_trait]
pub trait DaseinOps: Send + Sync {
    async fn review(
        &self,
        intent: &crate::Intent,
        ctx: &crate::Context,
    ) -> Result<crate::Verdict>;
    async fn narrate(&self, event: &str, detail: &str);
    async fn snapshot(&self) -> Result<serde_json::Value>;
}

/// Mnemosyne (memory) operations — recall, store, prompt composition.
#[async_trait]
pub trait MnemosyneOps: Send + Sync {
    async fn recall(&self, query: &str, limit: usize) -> Result<Vec<serde_json::Value>>;
    async fn store(&self, block: &serde_json::Value) -> Result<()>;
    async fn compose_prompt_block(&self, session_id: &str) -> Result<String>;
    async fn consolidate(&self) -> Result<()>;
}

/// Corpus (body) operations — tool execution, skill matching, hooks.
#[async_trait]
pub trait CorpusOps: Send + Sync {
    async fn execute_tool(
        &self,
        tool_name: &str,
        input: serde_json::Value,
        session_id: &str,
    ) -> Result<crate::ToolResult>;
    async fn list_tools(&self) -> Result<Vec<crate::ToolDefinition>>;
    async fn run_hooks(&self, event: &crate::HookContext) -> Result<Vec<crate::HookResult>>;
}

// ---------------------------------------------------------------------------
// Harness traits
// ---------------------------------------------------------------------------

/// Tool executor — abstracts tool dispatch for harnesses.
#[async_trait]
pub trait ToolExecutor: Send + Sync {
    async fn execute(
        &self,
        name: &str,
        input: serde_json::Value,
    ) -> Result<crate::ToolResult>;
}

/// A cognitive harness orchestrates a reasoning pipeline.
///
/// Harnesses are pluggable:
/// - `LinearCognitiveHarness` (current ReAct equivalent)
/// - Future: `ResearchHarness`, `CodingHarness`, `RobotHarness`, `OSHarness`
#[async_trait]
pub trait CognitiveHarness: Send + Sync {
    async fn run(
        &self,
        input: &str,
        messages: &[crate::Message],
        tool_defs: &[crate::ToolDefinition],
        executor: &dyn ToolExecutor,
    ) -> Result<(String, serde_json::Value)>;
}
```

- [ ] **Step 2: Register in `crates/base/src/lib.rs`**

Add after `pub mod types;` (line 30):
```rust
pub mod ops;
```

Add re-exports before the closing line:
```rust
// Ops traits (from ops/)
pub use ops::{CognitOps, CognitiveHarness, CorpusOps, DaseinOps, MnemosyneOps, ToolExecutor};
```

- [ ] **Step 3: Verify**

Run: `cargo build -p base`
Expected: PASS

- [ ] **Step 4: Commit**

```bash
git add crates/base/src/ops.rs crates/base/src/lib.rs
git commit -m "feat(base): add subsystem ops traits and CognitiveHarness trait

- CognitOps/DaseinOps/MnemosyneOps/CorpusOps: 4 subsystem contracts
- CognitiveHarness: pluggable reasoning pipeline
- ToolExecutor: tool dispatch abstraction

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 2: Remove dead fields from RequestHandler

**Files:**
- Modify: `crates/runtime/src/impl/daemon/handler/mod.rs`
- Modify: `crates/runtime/src/impl/daemon/handler/init.rs`

**Verify:** `cargo build -p runtime`

4 dead fields to remove (all marked `#[allow(dead_code)]` or `parked`):
| # | Field | Reason |
|---|-------|--------|
| 1 | `agent_registry: Arc<AgentRegistry>` | parked — multi-agent unwired |
| 2 | `checkpoint_store: Arc<Mutex<CheckpointStore>>` | parked — future file-edit rewind |
| 3 | `agent_loader: Arc<Mutex<AgentLoader>>` | parked — multi-agent unwired |
| 4 | `event_bus: Option<Arc<CommunicationBus>>` | dead — replaced by `bus` field |

- [ ] **Step 1: Remove field declarations from struct in `mod.rs`**

Remove lines 100-102, 146-148, 152-154, 173-175:
```rust
// Lines ~100-102:
    #[allow(dead_code)]
    agent_registry: Arc<AgentRegistry>,

// Lines ~146-148:
    #[allow(dead_code)]
    checkpoint_store: Arc<Mutex<CheckpointStore>>,

// Lines ~152-154:
    #[allow(dead_code)]
    agent_loader: Arc<Mutex<AgentLoader>>,

// Lines ~173-175:
    #[allow(dead_code)]
    event_bus: Option<Arc<CommunicationBus>>,
```

- [ ] **Step 2: Remove field values from `Self { ... }` in `init.rs:new()`**

Remove these 4 lines from the struct literal (~lines 596, 615, 617, 625):
```rust
            agent_registry,
            checkpoint_store,
            agent_loader,
            event_bus,
```

- [ ] **Step 3: Remove construction code from `init.rs:new()`**

Delete these blocks:

**agent_registry** (~lines 254-284): Entire `// Agent registry` section through end of built-in agent registration block.

**checkpoint_store** (~lines 494-497):
```rust
let checkpoint_store = CheckpointStore::new(&session_dir);
let checkpoint_store = Arc::new(Mutex::new(checkpoint_store));
```
Note: also remove `let session_dir = aletheon_dir.join("sessions").join(&session_id);` and `std::fs::create_dir_all(&session_dir)?;` if only used by checkpoint_store.

**agent_loader** (~lines 509-515):
```rust
let mut agent_loader = AgentLoader::new();
let agents_dir = aletheon_dir.join("agents");
if agents_dir.exists() {
    let _ = agent_loader.load_from_dir(&agents_dir);
    info!("Loaded {} agent roles", agent_loader.list().len());
}
let agent_loader = Arc::new(Mutex::new(agent_loader));
```

- [ ] **Step 4: Clean unused imports in `init.rs`**

Remove:
```rust
use crate::r#impl::orchestration::builtin::{CodeAgent, FsAgent, NetAgent};
use crate::r#impl::orchestration::registry::AgentRegistry;
use crate::core::checkpoint::CheckpointStore;
use crate::r#impl::agent_loader::AgentLoader;
```

- [ ] **Step 5: Clean unused imports in `mod.rs`**

Remove:
```rust
use crate::r#impl::orchestration::registry::AgentRegistry;
use crate::core::checkpoint::CheckpointStore;
use crate::r#impl::agent_loader::AgentLoader;
```

- [ ] **Step 6: Verify**

Run: `cargo build -p runtime 2>&1`
Expected: PASS

- [ ] **Step 7: Commit**

```bash
git add crates/runtime/src/impl/daemon/handler/mod.rs crates/runtime/src/impl/daemon/handler/init.rs
git commit -m "refactor(runtime): remove 4 dead fields from RequestHandler

agent_registry, checkpoint_store, agent_loader (all parked),
event_bus (dead, replaced by bus field).

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 3: Create CoreSystems struct

**Files:**
- Create: `crates/runtime/src/core/core_systems.rs`
- Modify: `crates/runtime/src/core/mod.rs`

**Verify:** `cargo build -p runtime`

- [ ] **Step 1: Create `crates/runtime/src/core/core_systems.rs`**

```rust
//! CoreSystems — concrete subsystem type bundle.
//!
//! Holds all subsystem types that RequestHandler currently owns directly.
//! During Group B, each field transitions to `Arc<dyn TraitOps>` from
//! `base::ops` as each subsystem gets its trait implementation migrated.
//!
//! This is the intermediate step between the God Object and the final
//! trait-based architecture defined in RFC-010~013.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use tokio::sync::{mpsc, oneshot, Mutex};
use tokio_util::sync::CancellationToken;

use base::kernel::debug_bus::PerfCounter;
use base::CommunicationBus;
use corpus::security::security::approval::ApprovalDecision;
use corpus::security::security::runner::ToolRunnerWithGuard;
use corpus::security::security::socket_approval::PendingApproval;
use corpus::tools::tools::ToolRegistry;
use dasein::SelfField;
use memory::episodic::EpisodicMemory;
use metacog::{DefaultMetaRuntime, MorphogenesisPipeline};

use crate::core::config::HooksConfig;
use crate::core::orchestrator::AletheonRuntime;
use crate::core::storm_breaker::StormBreaker;
use crate::r#impl::goal::ObjectiveStore;
use crate::r#impl::hooks::registry::HookRegistry;
use crate::r#impl::memory::auto_memory::AutoMemory;
use crate::r#impl::memory::fact_store::FactStore;
use crate::r#impl::skill_router::SkillRouter;
use crate::r#impl::skills::loader::SkillLoader;
use crate::CoreMemory;
use crate::RecallMemory;

use super::super::debug_handler::DebugHandler;
use cognit::core::reflector::Reflector;

/// Bundle of subsystem types.
///
/// In Group B, each field transitions to `Arc<dyn TraitOps>`.
pub struct CoreSystems {
    // --- Orchestrator ---
    pub runtime: AletheonRuntime,

    // --- Dasein (SelfField) ---
    pub self_field: Arc<Mutex<SelfField>>,

    // --- Mnemosyne (Memory) ---
    pub episodic_memory: Arc<Mutex<EpisodicMemory>>,
    pub recall_memory: Arc<Mutex<RecallMemory>>,
    pub core_memory: Arc<Mutex<CoreMemory>>,
    pub fact_store: Arc<Mutex<FactStore>>,
    pub auto_memory: Arc<Mutex<AutoMemory>>,
    pub objective_store: Arc<Mutex<ObjectiveStore>>,

    // --- Cognit ---
    pub reflector: Reflector,

    // --- Corpus ---
    pub tools: Arc<Mutex<ToolRegistry>>,
    pub tool_runner: Arc<Mutex<ToolRunnerWithGuard>>,
    pub skill_loader: Arc<Mutex<SkillLoader>>,
    pub skill_router: Arc<Mutex<SkillRouter>>,
    pub hook_registry: Arc<Mutex<HookRegistry>>,
    pub storm_breaker: Arc<Mutex<StormBreaker>>,
    pub hooks_config: HooksConfig,

    // --- Metacog ---
    pub pipeline: Arc<MorphogenesisPipeline<DefaultMetaRuntime>>,

    // --- Approval / Security ---
    pub approval_rx: Arc<Mutex<mpsc::Receiver<PendingApproval>>>,
    pub pending_approvals:
        Arc<Mutex<HashMap<String, oneshot::Sender<ApprovalDecision>>>>,
    pub session_approvals: Arc<Mutex<HashMap<String, bool>>>,

    // --- Debug / Observability ---
    pub debug_handler: Arc<DebugHandler>,
    pub debug_perf: Arc<PerfCounter>,
    pub cancel_token: Arc<Mutex<Option<CancellationToken>>>,

    // --- Prefix / Context ---
    pub cached_prefix: Arc<Mutex<String>>,
    pub memory_queue: Arc<Mutex<Vec<String>>>,
    pub config_prompt: String,

    // --- Session management ---
    pub default_session_id: Arc<tokio::sync::Mutex<String>>,
    pub session_created_at: Arc<Mutex<HashMap<String, Instant>>>,
    pub data_dir: PathBuf,
    pub context_window: usize,
}
```

- [ ] **Step 2: Register in `crates/runtime/src/core/mod.rs`**

After `pub mod controller;` add:
```rust
pub mod core_systems;
```

After existing pub uses add:
```rust
pub use core_systems::CoreSystems;
```

- [ ] **Step 3: Verify**

Run: `cargo build -p runtime`
Expected: PASS (unused warnings OK)

- [ ] **Step 4: Commit**

```bash
git add crates/runtime/src/core/core_systems.rs crates/runtime/src/core/mod.rs
git commit -m "feat(runtime): add CoreSystems struct to bundle subsystem types

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 4: Shrink RequestHandler to 10 fields via CoreSystems

**Files:**
- Modify: `crates/runtime/src/impl/daemon/handler/mod.rs`
- Modify: `crates/runtime/src/impl/daemon/handler/init.rs`
- Modify: `crates/runtime/src/impl/daemon/handler/chat.rs`

**Verify:** `cargo build -p runtime`

This is a mechanical refactor. Every `self.<field>` that moved to `CoreSystems` becomes `self.subsystems.<field>`. `self.bus` changes from `Option<Arc<CommunicationBus>>` to `Arc<CommunicationBus>` (it's always Some after init).

**Field migration map:**

| Old `self.xxx` | New `self.subsystems.xxx` |
|---|---|
| `self.state` | `self.subsystems.runtime` |
| `self.self_field` | `self.subsystems.self_field` |
| `self.episodic_memory` | `self.subsystems.episodic_memory` |
| `self.recall_memory` | `self.subsystems.recall_memory` |
| `self.core_memory` | `self.subsystems.core_memory` |
| `self.fact_store` | `self.subsystems.fact_store` |
| `self.auto_memory` | `self.subsystems.auto_memory` |
| `self.objective_store` | `self.subsystems.objective_store` |
| `self.reflector` | `self.subsystems.reflector` |
| `self.tools` | `self.subsystems.tools` |
| `self.tool_runner` | `self.subsystems.tool_runner` |
| `self.skill_loader` | `self.subsystems.skill_loader` |
| `self.skill_router` | `self.subsystems.skill_router` |
| `self.hook_registry` | `self.subsystems.hook_registry` |
| `self.storm_breaker` | `self.subsystems.storm_breaker` |
| `self.hooks_config` | `self.subsystems.hooks_config` |
| `self.pipeline` | `self.subsystems.pipeline` |
| `self.approval_rx` | `self.subsystems.approval_rx` |
| `self.pending_approvals` | `self.subsystems.pending_approvals` |
| `self.session_approvals` | `self.subsystems.session_approvals` |
| `self.debug_handler` | `self.subsystems.debug_handler` |
| `self.debug_perf` | `self.subsystems.debug_perf` |
| `self.cancel_token` | `self.subsystems.cancel_token` |
| `self.cached_prefix` | `self.subsystems.cached_prefix` |
| `self.memory_queue` | `self.subsystems.memory_queue` |
| `self.config_prompt` | `self.subsystems.config_prompt` |
| `self.default_session_id` | `self.subsystems.default_session_id` |
| `self.session_created_at` | `self.subsystems.session_created_at` |
| `self.data_dir` | `self.subsystems.data_dir` |
| `self.context_window` | `self.subsystems.context_window` |

**Fields staying at `self.` level:**
`sessions`, `session_gateway`, `bus` (de-Optioned), `llm`, `model_router`, `notify_tx`, `active_connections`, `started_at`, `daemon_cancel_token`

- [ ] **Step 1: Replace RequestHandler struct in `mod.rs`**

Replace the entire struct definition (~lines 80-179) with:

```rust
#[derive(Clone)]
pub struct RequestHandler {
    /// All subsystem types — becomes `Arc<dyn TraitOps>` in Group B.
    pub(crate) subsystems: Arc<CoreSystems>,
    /// Multi-session registry.
    pub(crate) sessions: Arc<Mutex<HashMap<String, Arc<Mutex<SessionManager>>>>>,
    /// Session gateway for external agent debug access.
    pub(crate) session_gateway: Arc<SessionGateway>,
    /// Communication bus (always available after init).
    pub(crate) bus: Arc<CommunicationBus>,
    /// Default LLM provider.
    pub(crate) llm: Arc<dyn LlmProvider>,
    /// Model router for per-task-type model selection.
    pub(crate) model_router: Arc<ModelRouter>,
    /// Per-connection notification channel for JSON-RPC push.
    pub(crate) notify_tx: Option<mpsc::Sender<String>>,
    /// Active connection count.
    pub(crate) active_connections: Arc<AtomicUsize>,
    /// Daemon start time.
    pub(crate) started_at: Instant,
    /// Daemon-level cancellation token for graceful shutdown.
    pub(crate) daemon_cancel_token: Option<CancellationToken>,
}
```

Also remove `struct SessionState` and its `pending_input` wrapper — `AletheonRuntime` moves directly into `CoreSystems.runtime`.

- [ ] **Step 2: Update imports in `mod.rs`**

Remove imports that are no longer needed at handler level (they move to core_systems.rs):
```rust
// REMOVE:
use crate::core::checkpoint::CheckpointStore;
use crate::core::config::HooksConfig;
use crate::core::orchestrator::AletheonRuntime;
use crate::core::storm_breaker::StormBreaker;
use crate::r#impl::agent_loader::AgentLoader;
use crate::r#impl::engine::modules::{SelfFieldRequest, SelfFieldResponse};
use crate::r#impl::goal::ObjectiveStore;
use crate::r#impl::hooks::registry::HookRegistry;
use crate::r#impl::memory::auto_memory::AutoMemory;
use crate::r#impl::memory::fact_store::FactStore;
use crate::r#impl::skill_router::SkillRouter;
use crate::r#impl::skills::loader::SkillLoader;
use crate::CoreMemory;
use crate::RecallMemory;
use cognit::core::reflector::Reflector;
use corpus::security::security::runner::ToolRunnerWithGuard;
use corpus::security::security::socket_approval::PendingApproval;
use corpus::security::security::approval::ApprovalDecision;
use corpus::tools::tools::ToolRegistry;
use dasein::SelfField;
use memory::episodic::EpisodicMemory;
use metacog::{DefaultMetaRuntime, MorphogenesisPipeline};
use base::kernel::debug_bus::PerfCounter;
use std::collections::HashMap;
```

Add import:
```rust
use crate::core::core_systems::CoreSystems;
```

Also keep needed imports:
```rust
use super::super::model_router::ModelRouter;
use super::super::session_manager::SessionManager;
use super::super::debug_handler::DebugHandler;
use crate::core::session_gateway::SessionGateway;
use base::CommunicationBus;
use cognit::r#impl::llm::LlmProvider;
use tokio::sync::{mpsc, Mutex};
use tokio_util::sync::CancellationToken;
```

Check: `SelfFieldRequest`, `SelfFieldResponse`, `PendingApproval`, `ApprovalDecision` are used in `sf_review`/`sf_narrate` methods — keep them.

- [ ] **Step 3: Update `mod.rs` method bodies**

Perform these mechanical replacements across ALL methods in mod.rs (sf_review, sf_narrate, coordinate, compose_memory_block, run_hook_scripts, handle, etc.):

```
self.state → self.subsystems.runtime
self.self_field → self.subsystems.self_field
self.episodic_memory → self.subsystems.episodic_memory
self.recall_memory → self.subsystems.recall_memory
self.core_memory → self.subsystems.core_memory
self.fact_store → self.subsystems.fact_store
self.auto_memory → self.subsystems.auto_memory
self.objective_store → self.subsystems.objective_store
self.reflector → self.subsystems.reflector
self.tools → self.subsystems.tools
self.tool_runner → self.subsystems.tool_runner
self.skill_loader → self.subsystems.skill_loader
self.skill_router → self.subsystems.skill_router
self.hook_registry → self.subsystems.hook_registry
self.storm_breaker → self.subsystems.storm_breaker
self.hooks_config → self.subsystems.hooks_config
self.pipeline → self.subsystems.pipeline
self.approval_rx → self.subsystems.approval_rx
self.pending_approvals → self.subsystems.pending_approvals
self.session_approvals → self.subsystems.session_approvals
self.debug_handler → self.subsystems.debug_handler
self.debug_perf → self.subsystems.debug_perf
self.cancel_token → self.subsystems.cancel_token
self.cached_prefix → self.subsystems.cached_prefix
self.memory_queue → self.subsystems.memory_queue
self.config_prompt → self.subsystems.config_prompt
self.default_session_id → self.subsystems.default_session_id
self.session_created_at → self.subsystems.session_created_at
self.data_dir → self.subsystems.data_dir
self.context_window → self.subsystems.context_window
self.bus (as Option) → self.bus (remove .as_ref(), .unwrap(), etc.)
```

- [ ] **Step 4: Apply same replacements in `chat.rs`**

Replace all 30 field accesses in chat.rs (~1100 lines). Use sed or manual find-replace.

- [ ] **Step 5: Rewrite `init.rs:new()` to construct CoreSystems and slim RequestHandler**

The new constructor builds `CoreSystems` first, then the 10-field `RequestHandler`. Key structural change:

```rust
pub async fn new(...) -> anyhow::Result<Self> {
    // ... (LLM, session store, SelfField, etc. — same setup code) ...

    // Build CoreSystems
    let subsystems = Arc::new(CoreSystems {
        runtime,
        self_field,
        episodic_memory,
        recall_memory,
        core_memory,
        fact_store,
        auto_memory,
        objective_store,
        reflector,
        tools,
        tool_runner,
        skill_loader: Arc::new(Mutex::new(skill_loader)),
        skill_router,
        hook_registry,
        storm_breaker,
        hooks_config,
        pipeline,
        approval_rx: Arc::new(Mutex::new(approval_rx)),
        pending_approvals: Arc::new(Mutex::new(HashMap::new())),
        session_approvals: Arc::new(Mutex::new(HashMap::new())),
        debug_handler,
        debug_perf,
        cancel_token: Arc::new(Mutex::new(None)),
        cached_prefix: Arc::new(Mutex::new(cached_prefix)),
        memory_queue: Arc::new(Mutex::new(Vec::new())),
        config_prompt: config.system_prompt.clone(),
        default_session_id,
        session_created_at,
        data_dir,
        context_window,
    });

    let handler = Self {
        subsystems,
        sessions,
        session_gateway,
        bus: Arc::new(CommunicationBus::new()), // always created in init
        llm,
        model_router,
        notify_tx: None,
        active_connections,
        started_at: Instant::now(),
        daemon_cancel_token: Some(cancel_token),
    };

    // ... (param registry, OnSessionStart hook — update self.xxx to self.subsystems.xxx) ...

    Ok(handler)
}
```

- [ ] **Step 6: Fix `connect()` and `disconnect()` methods**

In `connection.rs`, update field accesses for `self.active_connections` (unchanged) and remove any dead references.

- [ ] **Step 7: Verify**

Run: `cargo build -p runtime 2>&1`
Expected: PASS (fix any remaining `self.xxx` that should be `self.subsystems.xxx`)

- [ ] **Step 8: Commit**

```bash
git add crates/runtime/src/impl/daemon/handler/
git commit -m "refactor(runtime): shrink RequestHandler to 10 fields via CoreSystems

Group 30 subsystem fields into CoreSystems struct. RequestHandler now
holds only host-level concerns: sessions, bus, llm, model_router,
notify_tx, active_connections, started_at, daemon_cancel_token.

All subsystem access goes through self.subsystems.xxx.

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 5: Fix CI — pre-existing `chat.rs:947` lifetime bug

**Files:**
- Modify: `crates/runtime/src/impl/daemon/handler/chat.rs`

**Verify:** `cargo build -p runtime && cargo clippy -p runtime`

- [ ] **Step 1: Fix the borrow-checker error at line ~947**

The bug: `sm_arc` is dropped while its `MutexGuard` still borrows it.

Current (buggy):
```rust
let (_sid, sm_arc) = self.get_or_create_session(None).await;
sm_arc.lock().await.turn_count()
```

Fix:
```rust
let (_sid, sm_arc) = self.get_or_create_session(None).await;
let turn = sm_arc.lock().await.turn_count();
turn
```

- [ ] **Step 2: Verify full workspace**

Run: `cargo build --workspace && cargo clippy --workspace -- -D warnings`
Expected: PASS

- [ ] **Step 3: Commit**

```bash
git add crates/runtime/src/impl/daemon/handler/chat.rs
git commit -m "fix(runtime): resolve borrow-checker lifetime error in chat.rs:947

sm_arc dropped while MutexGuard still borrows it. Extract turn_count
into a local before the guard is dropped.

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

## Group A Verification Checklist

- [ ] `cargo build --workspace` — PASS
- [ ] `cargo test --workspace` — PASS
- [ ] `cargo clippy --workspace -- -D warnings` — PASS (no new warnings)
- [ ] `grep -c "pub.*fn" crates/base/src/ops.rs` — 4 traits with 3+2+4+3 = 12 methods total
- [ ] `grep "pub.*subsystems" crates/runtime/src/impl/daemon/handler/mod.rs` — `subsystems: Arc<CoreSystems>` field exists
- [ ] Count fields in RequestHandler — ≤10
- [ ] `grep "agent_registry\|checkpoint_store\|agent_loader\|event_bus" crates/runtime/src/impl/daemon/handler/mod.rs` — no matches (dead fields removed)
