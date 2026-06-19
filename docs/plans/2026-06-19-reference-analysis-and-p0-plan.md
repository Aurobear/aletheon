# Reference Analysis & P0 Improvement Plan

**Date:** 2026-06-19
**Status:** Draft
**Scope:** Codex (Rust), OpenCode (TS), DeepSeek-Reasonix (Go) reference analysis; P0 improvements for aletheon

---

## 1. Executive Summary

Analysis of three CLI agent reference implementations reveals seven high-value patterns aletheon lacks. The three P0 items — cache-first prefix stability, transport-agnostic controller, and independent execpolicy — form the architectural foundation for production use. Total estimated effort: ~2000 lines of Rust across 3-5 new files and modifications to existing crates.

---

## 2. Reference Implementations Overview

### 2.1 Codex (Rust)

**Location:** `references/cli-agent/codex/codex-rs/`

OpenAI's CLI agent. 100+ Rust crates, production-grade sandbox, policy engine, and tool system.

**Unique contributions:**
- Two-stage sandbox (bubblewrap → seccomp) with `codex-linux-sandbox` separate binary
- `execpolicy` independent policy crate with `Decision { Allow | Prompt | Forbidden }` + overlay merge
- `ToolExecutor<Invocation>` trait with `ToolExposure { Direct | Deferred | Hidden }`
- `ExecExpiration` composable cancellation (timeout + token)
- `SandboxManager::transform()` pure function pipeline

### 2.2 OpenCode (TS/Bun)

**Location:** `references/cli-agent/opencode/`

Effect-TS based CLI agent with rich plugin ecosystem.

**Unique contributions:**
- Effect-TS service pattern with typed DI (`Layer`, `Scope`, `TaggedErrorClass`)
- Snapshot system using separate gitdir for tree objects
- Worktree isolation per session
- Wildcard-based permission ruleset with arity display
- Provider transform layer (~1200 lines of per-provider quirk normalization)
- SKILL.md convention with remote discovery

### 2.3 DeepSeek-Reasonix (Go)

**Location:** `references/DeepSeek-Reasonix/`

Go CLI/desktop agent optimized for DeepSeek prefix cache.

**Unique contributions:**
- Cache-first byte-stable prefix architecture
- Transport-agnostic `Controller` pattern
- Hierarchical memory (docs + auto-memory store with frontmatter)
- Token-budgeted compaction with storm breaker anti-loop
- Snapshot-based checkpoint/rewind with `Previewer` interface
- Tool parallelism partitioning (read-only batch / serial writer)

---

## 3. Gap Analysis

### 3.1 Architecture Gaps

| Component | Codex | OpenCode | Reasonix | aletheon |
|-----------|-------|----------|----------|----------|
| Transport abstraction | Agent loop (direct) | Event-emitter | Controller (agnostic) | DaemonHandler (coupled) |
| Sandbox execution | 2-stage bwrap+seccomp | None | None | SandboxProfile (empty) |
| Policy engine | execpolicy crate | Wildcard ruleset | deny>ask>allow | PolicyEngine (inline) |
| Cache optimization | Not explicit | Not explicit | Central principle | None |
| Checkpoint/rewind | Git-based | Git-based (separate gitdir) | Snapshot-based | None |
| Hook lifecycle | Pre/Post/Permission | Plugin triggers | None | Fire-and-forget |
| Event system | Channel-based | EventV2+GlobalBus | Typed Sink trait | String logging |

### 3.2 Tool System Gaps

| Feature | Codex | OpenCode | Reasonix | aletheon |
|---------|-------|---------|----------|----------|
| Tool trait | `ToolExecutor<Invocation>` | Schema-first `Def` | `Tool` interface | Basic `Tool` trait |
| Exposure levels | Direct/Deferred/Hidden | Model-aware selection | ReadOnly flag | None |
| Output type | `Box<dyn ToolOutput>` | `ExecuteResult` | `string` | `ToolResult` (concrete) |
| Parallelism | Per-tool | Sequential | Read-only batch | Sequential |
| Truncation | Per-tool | Wrapper-level | Per-tool | Per-tool |
| Previewer | N/A | N/A | `Previewer` interface | None |

### 3.3 Memory & Context Gaps

| Feature | Codex | OpenCode | Reasonix | aletheon |
|---------|-------|---------|----------|----------|
| Memory docs | CLAUDE.md | Similar | Hierarchical REASONIX.md | None |
| Auto-memory | None | Similar | Frontmatter store + index | None |
| Context epochs | None | Baseline/snapshot model | None | None |
| Compaction | Basic | Basic | Token-budgeted + storm breaker | AdvancedCompressor (basic) |
| Cache diagnostics | None | None | PrefixShape hashes | None |

---

## 4. P0 Improvements

### 4.1 Cache-First Prefix Stability

**Source:** Reasonix
**Impact:** 💰💰💰 Direct API cost reduction via provider prefix cache hits
**Effort:** ~150 lines

#### 4.1.1 Problem

aletheon's `ReActLoop` reconstructs the full message array every turn. If the system prompt changes (e.g., plan mode toggle, memory update), the provider's prefix cache is invalidated. DeepSeek and Anthropic both offer automatic prefix caching — a warm cache can save 50-90% on input tokens.

#### 4.1.2 Design

**Principle:** System prompt (base + tools + memory) is assembled once at boot. Mid-session changes ride the user message tail.

```
Boot assembly (immutable):
  base_prompt + tool_schemas + memory_index
  → SHA-256 hash → PrefixShape { system_hash, tools_hash, prefix_hash }

Per-turn composition (mutable):
  user_message + plan_marker + memory_updates + bg_job_completions
  → injected into user message, NOT system prompt
```

**New types:**

```rust
// crates/aletheon-runtime/src/core/cache_shape.rs

pub struct PrefixShape {
    pub system_hash: String,   // SHA-256 of base prompt
    pub tools_hash: String,    // SHA-256 of tool schemas (sorted by name)
    pub prefix_hash: String,   // combined hash
}

impl PrefixShape {
    pub fn new(system: &str, tools: &[ToolSpec]) -> Self { ... }
    pub fn compare(&self, other: &Self) -> Option<CacheMissReason> { ... }
}

pub enum CacheMissReason {
    SystemChanged,
    ToolsChanged,
    BothChanged,
}
```

**Modify `ReActLoop`:**

```rust
// crates/aletheon-runtime/src/core/react_loop.rs

impl ReActLoop {
    /// Assemble user message with mid-session injections.
    /// Changes go here, NOT into system prompt.
    fn compose_user_message(&self, input: &str) -> String {
        let mut parts = Vec::new();

        // Plan mode marker
        if self.plan_mode {
            parts.push("[PLAN MODE ACTIVE: Think step-by-step before acting.]".to_string());
        }

        // Memory updates (from SelfField or quick-add)
        if !self.pending_memory.is_empty() {
            let updates = self.pending_memory.iter()
                .map(|m| format!("- {}", m))
                .collect::<Vec<_>>()
                .join("\n");
            parts.push(format!("<memory-update>\n{}\n</memory-update>", updates));
        }

        // Original input
        parts.push(input.to_string());

        parts.join("\n\n")
    }

    /// The system prompt is set once and never mutated.
    pub fn system_prompt(&self) -> &str {
        &self.system_prompt  // immutable after construction
    }
}
```

**Cache diagnostics:**

```rust
// Track cache hits/misses per session
pub struct CacheDiagnostics {
    pub hits: usize,
    pub misses: usize,
    pub last_miss_reason: Option<CacheMissReason>,
}
```

#### 4.1.3 Files to modify

| File | Change |
|------|--------|
| `crates/aletheon-runtime/src/core/cache_shape.rs` | **New.** PrefixShape, CacheMissReason |
| `crates/aletheon-runtime/src/core/react_loop.rs` | Add compose_user_message(), make system_prompt immutable |
| `crates/aletheon-runtime/src/core/mod.rs` | Export cache_shape module |
| `crates/aletheon-runtime/src/impl/daemon/handler.rs` | Use compose_user_message() for user input |

---

### 4.2 Transport-Agnostic Controller

**Source:** Reasonix
**Impact:** 🏗️ Architectural foundation for multi-frontend support
**Effort:** ~500 lines

#### 4.2.1 Problem

`DaemonHandler` couples JSON-RPC transport with agent logic. Adding a TUI or HTTP frontend means duplicating agent orchestration code. The current architecture:

```
DaemonHandler (JSON-RPC)
  ├── handle_chat() → directly calls process_react()
  ├── handle_approve() → directly manages approval state
  └── handle_new_session() → directly manages session
```

#### 4.2.2 Design

Extract a `Controller` that all frontends drive identically:

```
Frontend (TUI / Daemon / HTTP / Desktop)
         ↓ Send() / Approve() / Cancel()
    Controller (transport-agnostic)
         ↓
    AgentRunner + ToolRegistry + EventSink + ApprovalGate + MemorySet
```

**New types:**

```rust
// crates/aletheon-runtime/src/core/controller.rs

use tokio::sync::{mpsc, Mutex};
use std::sync::Arc;

pub struct Controller {
    runner: Arc<Mutex<ReActLoop>>,
    tools: Arc<ToolRegistry>,
    event_sink: mpsc::Sender<Event>,
    approval_gate: Arc<dyn ApprovalGate>,
    memory: Arc<MemorySet>,
    session_store: Arc<SessionStore>,
    // State
    running: Mutex<bool>,
    cancel_token: CancellationToken,
}

impl Controller {
    pub async fn send(&self, input: String) -> Result<()> {
        let mut running = self.running.lock().await;
        if *running {
            return Err(anyhow!("Turn already in progress"));
        }
        *running = true;
        drop(running);

        let cancel = self.cancel_token.clone();
        let runner = self.runner.clone();
        let sink = self.event_sink.clone();
        let composed = self.compose_user_message(&input);

        tokio::spawn(async move {
            let result = tokio::select! {
                result = runner.lock().await.run(&composed) => result,
                _ = cancel.cancelled() => Err(anyhow!("Cancelled")),
            };

            sink.send(Event::TurnDone { result }).await.ok();
            // Reset running state
        });

        Ok(())
    }

    pub async fn approve(&self, id: &str, allow: bool) -> Result<()> {
        self.approval_gate.respond(id, allow).await
    }

    pub async fn cancel(&self) {
        self.cancel_token.cancel();
    }

    pub fn subscribe_events(&self) -> mpsc::Receiver<Event> {
        // Each frontend gets its own receiver via broadcast channel
        todo!()
    }
}
```

**Typed event stream:**

```rust
// crates/aletheon-runtime/src/core/event.rs

#[derive(Debug, Clone)]
pub enum Event {
    TurnStarted,
    Text { text: String },
    Reasoning { text: String },
    ToolDispatch { name: String, args: serde_json::Value },
    ToolResult { name: String, result: ToolResult },
    Usage { tokens_in: u32, tokens_out: u32 },
    ApprovalRequest { id: String, request: ApprovalRequest },
    AskRequest { id: String, question: String },
    CompactionStarted,
    CompactionDone { summary_chars: usize },
    TurnDone { result: Result<String> },
    Error { message: String },
}

pub trait EventSink: Send + Sync {
    fn emit(&self, event: Event);
}

/// mpsc-based sink for async frontends
pub struct ChannelEventSink {
    tx: mpsc::Sender<Event>,
}

/// Broadcast sink for multiple subscribers
pub struct BroadcastEventSink {
    tx: broadcast::Sender<Event>,
}
```

#### 4.2.3 Files to modify

| File | Change |
|------|--------|
| `crates/aletheon-runtime/src/core/controller.rs` | **New.** Controller struct + methods |
| `crates/aletheon-runtime/src/core/event.rs` | **New.** Event enum, EventSink trait |
| `crates/aletheon-runtime/src/core/mod.rs` | Export controller, event modules |
| `crates/aletheon-runtime/src/impl/daemon/handler.rs` | Refactor to use Controller |
| `crates/binaries/aletheon-exec/src/main.rs` | Refactor to use Controller |

---

### 4.3 Independent Execpolicy Engine

**Source:** Codex
**Impact:** 🔐 Security foundation, testable policy logic
**Effort:** ~600 lines

#### 4.3.1 Problem

`PolicyEngine` is embedded in `ToolRunnerWithGuard`. Policy logic can't be tested independently, can't be used in other contexts (pre-commit hooks, CI), and lacks overlay merge for project/user/system rule layers.

#### 4.3.2 Design

Extract policy into `crates/aletheon-abi/src/execpolicy.rs`:

```rust
// crates/aletheon-abi/src/execpolicy.rs

use std::collections::HashMap;

/// Policy decision, ordered by severity (Ord: Allow < Prompt < Forbidden)
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Decision {
    Allow,
    Prompt,
    Forbidden,
}

/// Result of checking a command against policy
#[derive(Debug)]
pub struct Evaluation {
    pub decision: Decision,
    pub matched_rules: Vec<RuleMatch>,
}

/// A single policy rule
pub trait Rule: Send + Sync {
    fn matches(&self, cmd: &[String]) -> Option<RuleMatch>;
    fn program(&self) -> &str;
    fn as_any(&self) -> &dyn std::any::Any;
}

/// Prefix-based rule (primary rule type)
pub struct PrefixRule {
    pub program: String,
    pub pattern: Vec<PatternToken>,
    pub decision: Decision,
}

pub enum PatternToken {
    Exact(String),
    Alternatives(Vec<String>),
}

/// The policy engine
pub struct Policy {
    rules: HashMap<String, Vec<Arc<dyn Rule>>>,
    network_rules: Vec<NetworkRule>,
}

impl Policy {
    /// Check a command against the policy
    pub fn check(&self, cmd: &[String], heuristics: impl Fn(&[String]) -> Decision) -> Evaluation {
        let program = &cmd[0];
        let matched: Vec<RuleMatch> = self.rules
            .get(program)
            .map(|rules| rules.iter().filter_map(|r| r.matches(cmd)).collect())
            .unwrap_or_default();

        if matched.is_empty() {
            // No explicit rule — use heuristics fallback
            let decision = heuristics(cmd);
            return Evaluation { decision, matched_rules: vec![] };
        }

        // Take the most severe decision
        let decision = matched.iter()
            .map(|m| m.decision())
            .max()
            .unwrap_or(Decision::Prompt);

        Evaluation { decision, matched_rules: matched }
    }

    /// Merge a higher-precedence overlay
    pub fn merge_overlay(&mut self, overlay: Policy) {
        for (program, rules) in overlay.rules {
            self.rules.entry(program).or_default().extend(rules);
        }
        self.network_rules.extend(overlay.network_rules);
    }
}

/// Load policy from TOML config
pub fn load_policy_from_str(toml: &str) -> Result<Policy> { ... }

/// Load policy from layered config files
pub fn load_policy(
    system: Option<&str>,   // /etc/aletheon/policy.toml
    user: Option<&str>,     // ~/.aletheon/policy.toml
    project: Option<&str>,  // .aletheon/policy.toml
) -> Result<Policy> {
    let mut policy = Policy::default();
    if let Some(toml) = system {
        policy.merge_overlay(load_policy_from_str(toml)?);
    }
    if let Some(toml) = user {
        policy.merge_overlay(load_policy_from_str(toml)?);
    }
    if let Some(toml) = project {
        policy.merge_overlay(load_policy_from_str(toml)?);
    }
    Ok(policy)
}
```

**Heuristics fallback:**

```rust
/// Default heuristics for unmatched commands
pub fn default_heuristics(cmd: &[String]) -> Decision {
    let program = &cmd[0];
    match program.as_str() {
        "cat" | "ls" | "pwd" | "echo" | "which" | "whoami" => Decision::Allow,
        "rm" | "rmdir" | "mkfs" | "dd" | "format" => Decision::Forbidden,
        _ => Decision::Prompt,
    }
}
```

**Integration with ToolRunnerWithGuard:**

```rust
// crates/aletheon-body/src/impl/security/runner.rs

impl ToolRunnerWithGuard {
    async fn execute_tool(&self, name: &str, args: serde_json::Value, ctx: &ToolContext) -> ToolResult {
        let tool = self.tools.get(name).unwrap();

        // 1. Policy check (independent)
        let eval = self.policy.check(&[name.to_string()], default_heuristics);

        match eval.decision {
            Decision::Allow => {
                tool.execute(args, ctx).await
            }
            Decision::Forbidden => {
                ToolResult::error(format!("Policy forbids tool: {}", name))
            }
            Decision::Prompt => {
                // 2. Session approvals
                if self.session_approvals.contains(name) {
                    return tool.execute(args, ctx).await;
                }
                // 3. Approval gate
                match self.approval_gate.request(ApprovalRequest { ... }).await {
                    ApprovalDecision::Allow { persist } => {
                        if persist { self.session_approvals.insert(name.to_string()); }
                        tool.execute(args, ctx).await
                    }
                    ApprovalDecision::Deny => ToolResult::error("Denied by user"),
                }
            }
        }
    }
}
```

#### 4.3.3 Files to modify

| File | Change |
|------|--------|
| `crates/aletheon-abi/src/execpolicy.rs` | **New.** Policy, Rule, Decision, Evaluation, PrefixRule |
| `crates/aletheon-abi/src/lib.rs` | Export execpolicy module |
| `crates/aletheon-body/src/impl/security/runner.rs` | Use Policy instead of inline PolicyEngine |
| `crates/aletheon-body/src/impl/security/permission_rules.rs` | Migrate to execpolicy loader |

---

## 5. Implementation Order

```
Phase 1: Cache-First Prefix Stability (~150 lines)
  ├── cache_shape.rs (PrefixShape, CacheMissReason)
  ├── react_loop.rs (compose_user_message, immutable system prompt)
  └── handler.rs (use compose_user_message)

Phase 2: Independent Execpolicy (~600 lines)
  ├── execpolicy.rs (Policy, Rule, Decision, Evaluation)
  ├── runner.rs (integrate Policy)
  └── permission_rules.rs (migrate to execpolicy loader)

Phase 3: Controller + Event System (~500 lines)
  ├── controller.rs (Controller struct)
  ├── event.rs (Event enum, EventSink trait)
  ├── handler.rs (refactor to Controller)
  └── main.rs (refactor to Controller)
```

Total: ~1250 lines of new/modified Rust code.

---

## 6. Validation Criteria

### 6.1 Cache-First
- [ ] System prompt hash stable across turns (no mid-session mutation)
- [ ] Memory updates appear in user message, not system prompt
- [ ] PrefixShape diagnostics log cache miss reasons
- [ ] Existing tests pass

### 6.2 Execpolicy
- [ ] Policy::check returns correct Decision for known commands
- [ ] Overlay merge applies precedence (project > user > system)
- [ ] Heuristics fallback classifies safe/dangerous commands
- [ ] ToolRunnerWithGuard uses Policy instead of inline checks
- [ ] Unit tests: 10+ policy scenarios

### 6.3 Controller
- [ ] Controller::send drives ReActLoop without transport coupling
- [ ] Event stream delivers TurnStarted/Text/ToolResult/TurnDone
- [ ] DaemonHandler refactored to use Controller
- [ ] aletheon-exec refactored to use Controller
- [ ] Existing integration tests pass

---

## 7. Future Work (P1/P2)

| Item | Source | Priority |
|------|--------|----------|
| Checkpoint/rewind snapshot | Reasonix | P1 |
| Tool parallelism partitioning | Reasonix | P1 |
| Storm breaker anti-loop | Reasonix | P1 |
| Snapshot independent gitdir | OpenCode | P2 |
| Provider transform normalization | OpenCode | P2 |
| Wildcard permission ruleset | OpenCode | P2 |
| ToolExposure Deferred/Hidden | Codex | P2 |
| Sandbox landlock integration | Codex | P2 |
