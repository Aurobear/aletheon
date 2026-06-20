# P0: ABI Types Implementation Plan

> **For agentic workers:** Use `workflow-feature` or `writing-plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add new type definitions and traits to `aletheon-abi` for TUI/Runtime/Brain overhaul.

**Architecture:** Extend the existing flat module structure in `aletheon-abi/src/` with new files for UI events, commands, collaboration modes, awareness signals, sub-agent types, and hook extensions. Follow existing patterns: `pub mod` in `lib.rs`, root-level re-exports, `Debug + Clone + Serialize + Deserialize` derives.

**Tech Stack:** Rust, serde, serde_json, uuid, chrono

---

## File Map

```
aletheon-abi/src/
├── lib.rs                      (MODIFY) — add pub mod declarations + root re-exports
├── ui_event.rs                 (NEW) — UiEvent enum for TUI display
├── collaboration_mode.rs       (NEW) — CollaborationMode + ModeConfig
├── awareness_signal.rs         (NEW) — AwarenessSignal + AwarenessLevel (TUI-facing)
├── sub_agent.rs                (NEW) — SubAgentHandle, SubAgentStatus
└── hook_ext.rs                 (NEW) — HookConfig, HookResult extensions
```

---

### Task 1: Create `ui_event.rs`

**Files:**
- Create: `crates/aletheon-abi/src/ui_event.rs`

- [ ] **Step 1: Create the file with UiEvent enum**

```rust
//! UI event types for TUI display.
//!
//! These events flow from Runtime → TUI via the IPC channel.
//! They extend the existing `Event` enum in `event_sink.rs` with
//! TUI-specific display events.

use serde::{Deserialize, Serialize};
use crate::self_field::SelfState;
use crate::brain::{Plan, Critique, CriticismDimension};

/// Collaboration mode (user-facing).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CollaborationMode {
    /// Normal operation: SelfField reviews, approval for destructive tools.
    Default,
    /// Read-only explore + plan generation. User approves before execution.
    Plan,
    /// Full autonomy: no approval prompts.
    Auto,
    /// All side-effect tools run in sandbox first.
    Sandbox,
}

impl Default for CollaborationMode {
    fn default() -> Self {
        Self::Default
    }
}

impl CollaborationMode {
    /// Icon shown in TUI status bar.
    pub fn icon(&self) -> &'static str {
        match self {
            Self::Default => "💬",
            Self::Plan => "📋",
            Self::Auto => "⚡",
            Self::Sandbox => "🔒",
        }
    }

    /// Human-readable name.
    pub fn display_name(&self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::Plan => "plan",
            Self::Auto => "auto",
            Self::Sandbox => "sandbox",
        }
    }

    /// Mode-specific system prompt suffix injected into the LLM context.
    pub fn system_prompt_suffix(&self) -> &'static str {
        match self {
            Self::Default => "Operate normally. Ask for user approval before destructive operations.",
            Self::Plan => "You are in PLAN MODE. You may only use read-only tools (glob, grep, read, web_fetch). Generate a detailed plan. Do NOT execute any mutations. Wait for user approval before proceeding.",
            Self::Auto => "You are in AUTO MODE. Execute without asking for approval. Be thorough and autonomous. Persist until the task is fully handled.",
            Self::Sandbox => "You are in SANDBOX MODE. All side-effect operations run in a sandbox first. Review sandbox results before applying to the real environment.",
        }
    }
}

/// Awareness level for TUI status bar display.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AwarenessLevel {
    /// No critical issues detected.
    Confident,
    /// Hedging language or uncertainty detected.
    Hesitant,
    /// 3+ consecutive errors or impasse detected.
    Confused,
    /// Domain shift or new direction detected.
    Curious,
    /// BrainCore generating plan.
    Planning,
    /// Post-turn reflection running.
    Reflecting,
    /// Morphogenesis triggered.
    Evolving,
}

impl AwarenessLevel {
    /// Icon shown in TUI status bar.
    pub fn icon(&self) -> &'static str {
        match self {
            Self::Confident => "💚",
            Self::Hesitant => "🟡",
            Self::Confused => "🔴",
            Self::Curious => "🔵",
            Self::Planning => "📋",
            Self::Reflecting => "🔄",
            Self::Evolving => "⚡",
        }
    }

    /// Human-readable name.
    pub fn display_name(&self) -> &'static str {
        match self {
            Self::Confident => "confident",
            Self::Hesitant => "hesitant",
            Self::Confused => "confused",
            Self::Curious => "curious",
            Self::Planning => "planning",
            Self::Reflecting => "reflecting",
            Self::Evolving => "evolving",
        }
    }

    /// Whether this level warrants an inline message in the chat.
    pub fn is_notable(&self) -> bool {
        matches!(self, Self::Hesitant | Self::Confused | Self::Curious | Self::Evolving)
    }
}

/// Sub-agent status for inline TUI display.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SubAgentStatus {
    Planning,
    Executing { current_step: String },
    WaitingApproval,
    Completed { summary: String },
    Failed { error: String },
}

/// Sub-agent handle for tracking spawned agents.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubAgentHandle {
    pub id: String,
    pub task: String,
    pub status: SubAgentStatus,
    pub parent_turn_id: String,
    pub spawned_at_ms: u64,
}

/// Plan update for plan mode visualization.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanUpdate {
    /// Version number (1-indexed, increments on each critique-revise cycle).
    pub version: usize,
    /// The plan at this version.
    pub plan: Plan,
    /// Critique of the previous version (None for v1).
    pub critique: Option<Vec<Critique>>,
    /// Whether the plan is ready for user approval.
    pub ready_for_approval: bool,
}

/// Evolution progress for TUI display.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum EvolutionStage {
    Reflecting { reflections_accumulated: usize },
    PatternDetected { pattern: String },
    MorphogenesisTriggered { proposal: String },
    LineageRecorded { entries: usize },
}

/// Interrupt reason.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum InterruptReason {
    /// User pressed Ctrl+C during streaming.
    UserCancelled,
    /// Turn exceeded timeout.
    Timeout,
    /// Token/cost budget exceeded.
    BudgetExceeded,
}

/// Events emitted by the Runtime for TUI display.
///
/// These flow over the existing JSON-RPC notification channel
/// with `method: "event"` and a `type` field discriminator.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum UiEvent {
    // === Existing events (kept for compatibility) ===
    /// Streaming text delta.
    TextDelta { text: String },
    /// Thinking/reasoning text delta.
    ThinkingDelta { text: String },
    /// Tool call started.
    ToolCallStart { id: String, name: String, input: serde_json::Value },
    /// Tool call completed.
    ToolCallResult { id: String, output: String, success: bool },
    /// Token/cost usage update.
    Usage { tokens_in: u32, tokens_out: u32, cache_hit_tokens: u32, cache_miss_tokens: u32 },
    /// Turn completed.
    TurnDone { response: String, interrupted: bool },
    /// Error occurred.
    Error { message: String },
    /// Approval requested for a tool.
    ApprovalRequest { id: String, tool: String, input: serde_json::Value, risk: String },

    // === NEW events for the overhaul ===
    /// Brain awareness signal changed.
    AwarenessChanged { level: AwarenessLevel, context: String },
    /// Plan mode update (new version or critique).
    PlanUpdate(PlanUpdate),
    /// Sub-agent status changed.
    SubAgentStatusChanged { agent_id: String, status: SubAgentStatus },
    /// Collaboration mode changed.
    ModeChanged { old: CollaborationMode, new: CollaborationMode },
    /// Evolution progress update.
    EvolutionProgress { stage: EvolutionStage },
    /// Context usage update.
    ContextUpdate { used: usize, max: usize },
    /// Model switched.
    ModelSwitch { from: String, to: String },
    /// Interrupt acknowledged.
    Interrupted { reason: InterruptReason },
    /// Compaction started.
    CompactionStarted,
    /// Compaction completed.
    CompactionDone { summary_chars: usize },
}
```

- [ ] **Step 2: Verify it compiles**

Run: `cargo check -p aletheon-abi`
Expected: Compiles with no errors (may have unused warnings).

- [ ] **Step 3: Commit**

```bash
git add crates/aletheon-abi/src/ui_event.rs
git commit -m "feat(abi): add UiEvent, CollaborationMode, AwarenessLevel types"
```

---

### Task 2: Register `ui_event` module in `lib.rs`

**Files:**
- Modify: `crates/aletheon-abi/src/lib.rs` (lines 10-56 for module declarations, lines 58-119 for re-exports)

- [ ] **Step 1: Add module declaration**

Find the "Shared types" section (around line 27) and add:

```rust
pub mod ui_event;
```

- [ ] **Step 2: Add root-level re-exports**

Find the re-exports section (around line 58) and add:

```rust
pub use ui_event::{
    AwarenessLevel, CollaborationMode, EvolutionStage, InterruptReason,
    PlanUpdate, SubAgentHandle, SubAgentStatus, UiEvent,
};
```

- [ ] **Step 3: Verify it compiles**

Run: `cargo check -p aletheon-abi`
Expected: Compiles cleanly.

- [ ] **Step 4: Commit**

```bash
git add crates/aletheon-abi/src/lib.rs
git commit -m "feat(abi): register ui_event module and re-export types"
```

---

### Task 3: Create `hook_ext.rs` (Hook extensions)

**Files:**
- Create: `crates/aletheon-abi/src/hook_ext.rs`

- [ ] **Step 1: Create the file**

The existing `hook.rs` has `HookPoint`, `HookContext`, `HookResult`. We need to extend with configuration types for the new hooks system.

```rust
//! Extended hook types for the hooks system.
//!
//! Complements the existing `hook.rs` types with configuration
//! and event bus integration types.

use serde::{Deserialize, Serialize};
use crate::hook::HookPoint;

/// Hook execution type.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HookType {
    /// Spawn a child process. Environment variables injected.
    Command,
    /// Inject as system message into the conversation.
    Prompt,
    /// Emit to the event bus.
    Event,
}

/// Hook configuration (from config.toml).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookConfig {
    /// Which lifecycle point this hook fires at.
    pub point: HookPoint,
    /// How to execute the hook.
    pub hook_type: HookType,
    /// For Command hooks: the shell command to run.
    pub command: Option<String>,
    /// For Prompt hooks: the text to inject.
    pub prompt: Option<String>,
    /// For Event hooks: the event type to emit.
    pub event_type: Option<String>,
    /// Timeout in milliseconds (default: 5000).
    pub timeout_ms: Option<u64>,
}

/// Result returned by a command hook process.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandHookResult {
    /// Whether the hook wants to modify the original action.
    pub modify: bool,
    /// Modification data (only meaningful when modify=true).
    pub data: Option<serde_json::Value>,
    /// Optional message to inject into conversation.
    pub inject_message: Option<String>,
    /// Whether to block the original action.
    pub block: bool,
    /// Reason for blocking (only meaningful when block=true).
    pub block_reason: Option<String>,
}
```

- [ ] **Step 2: Register in `lib.rs`**

Add module declaration and re-exports to `lib.rs`:

```rust
pub mod hook_ext;
```

Re-exports:
```rust
pub use hook_ext::{CommandHookResult, HookConfig, HookType};
```

- [ ] **Step 3: Verify it compiles**

Run: `cargo check -p aletheon-abi`
Expected: Compiles cleanly.

- [ ] **Step 4: Commit**

```bash
git add crates/aletheon-abi/src/hook_ext.rs crates/aletheon-abi/src/lib.rs
git commit -m "feat(abi): add hook extension types (HookConfig, HookType, CommandHookResult)"
```

---

### Task 4: Extend existing `permission.rs` with mode-aware types

**Files:**
- Modify: `crates/aletheon-abi/src/permission.rs`

- [ ] **Step 1: Add ModeConfig struct**

After the existing `PermissionContext` struct (line 74), add:

```rust
/// Mode-specific configuration that overrides permission defaults.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModeConfig {
    /// System prompt suffix for this mode.
    pub system_prompt_suffix: String,
    /// Default approval policy for this mode.
    pub approval_policy: PermissionMode,
    /// Allowed tools (None = all tools allowed).
    pub allowed_tools: Option<Vec<String>>,
    /// Maximum iterations before forcing stop (None = use global default).
    pub max_iterations: Option<usize>,
}

impl Default for ModeConfig {
    fn default() -> Self {
        Self {
            system_prompt_suffix: String::new(),
            approval_policy: PermissionMode::Default,
            allowed_tools: None,
            max_iterations: None,
        }
    }
}
```

- [ ] **Step 2: Add re-export in `lib.rs`**

```rust
pub use permission::{ModeConfig, PermissionBehavior, PermissionContext, PermissionMode, PermissionRule};
```

- [ ] **Step 3: Verify it compiles**

Run: `cargo check -p aletheon-abi`
Expected: Compiles cleanly.

- [ ] **Step 4: Commit**

```bash
git add crates/aletheon-abi/src/permission.rs crates/aletheon-abi/src/lib.rs
git commit -m "feat(abi): add ModeConfig to permission module"
```

---

### Task 5: Run full ABI test suite

- [ ] **Step 1: Run tests**

Run: `cargo test -p aletheon-abi`
Expected: All tests pass.

- [ ] **Step 2: Run clippy**

Run: `cargo clippy -p aletheon-abi -- -D warnings`
Expected: No warnings.

- [ ] **Step 3: Final commit**

```bash
git add -A
git commit -m "chore(abi): P0 ABI types complete — UiEvent, CollaborationMode, AwarenessLevel, HookConfig"
```

---

## Summary

P0 adds 4 new files and modifies 2 existing files in `aletheon-abi`:

| File | Action | Types Added |
|------|--------|-------------|
| `ui_event.rs` | NEW | `CollaborationMode`, `AwarenessLevel`, `SubAgentStatus`, `SubAgentHandle`, `PlanUpdate`, `EvolutionStage`, `InterruptReason`, `UiEvent` |
| `hook_ext.rs` | NEW | `HookType`, `HookConfig`, `CommandHookResult` |
| `permission.rs` | MODIFY | `ModeConfig` |
| `lib.rs` | MODIFY | Module declarations + re-exports |

All types follow existing ABI patterns: flat module structure, `Debug + Clone + Serialize + Deserialize` derives, root-level re-exports for key types.
