# Phase 2 Implementation Plan: TUI + Permission System + Daemon Approval

> **For agentic workers:** Use `workflow-feature` or `writing-plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax. Structured for **multiple developer agents in parallel** — see Dependency Graph and Parallel Batches.

**Goal:** Make the **interactive `aletheon` (CLI→daemon) path** usable *and* trustworthy: a real permission model (`PermissionMode`/`PermissionRule`), a cross-process `SocketApprovalGate` so the daemon can ask the user for L2+ approval (instead of `AutoDenyGate`), and a TUI approval dialog.

**Architecture:** Phase 1 made the **exec** path usable+trustworthy. Phase 2 brings the same to the **daemon** path. The daemon's guarded runner currently uses `AutoDenyGate` (so L2+ is silently denied in interactive mode); we replace it with a `SocketApprovalGate` that round-trips an approval prompt to the connected CLI/TUI over the existing unix socket as an out-of-band JSON-RPC notification. We add a `PermissionMode`/`PermissionRule` system in `aletheon-abi` + `body/security`, configured via `~/.aletheon/settings.toml`. TUI stays in `body/impl/ui/` (no new crate).

**Tech Stack:** Rust, async-trait, tokio, ratatui/crossterm, serde/toml, JSON-RPC over unix socket.

**Design spec:** [2026-06-19-cli-agent-design.md](./2026-06-19-cli-agent-design.md) §4–§5 (product blueprint); [2026-06-19-runtime-react-loop-wiring-design.md](./2026-06-19-runtime-react-loop-wiring-design.md) (Phase 1 foundation).

**Verified current state (read-only survey, 2026-06-19):**
- `ApprovalGate` trait exists (`crates/aletheon-body/src/impl/security/approval.rs:39`): `async fn request(&self, req: &ApprovalRequest) -> ApprovalDecision`. Impls: `AutoApproveGate`, `AutoDenyGate`, `TerminalApprovalGate`.
- Daemon guarded runner uses the default `AutoDenyGate` (`runner.rs:62`); the daemon never calls `with_approval_gate`.
- CLI↔daemon is line-delimited JSON-RPC over `/tmp/aletheon/aletheon.sock`; `chat` request → single `{result:{response,turn}}` response (`cli/mod.rs:239`, `handler.rs:729`). **No out-of-band notifications today.**
- TUI is ratatui three-panel, polling `try_read_response()` (`ui/mod.rs`). No approval dialog.
- **No `PermissionMode`/`PermissionRule`/`PermissionBehavior`** anywhere — only `PermissionLevel` L0–L3 (`abi/src/tool.rs:10`). `PolicyEngine` rules are hardcoded (`security/policy.rs:34` `with_defaults`).

---

## File Structure & Owner Boundaries

| Agent | Owns (writes) | Responsibility |
|-------|---------------|----------------|
| **A — Permission model** | `crates/aletheon-abi/src/permission.rs` (NEW), `crates/aletheon-abi/src/lib.rs`, `crates/aletheon-body/src/impl/security/permission_rules.rs` (NEW), `crates/aletheon-body/src/impl/security/mod.rs` | `PermissionMode`/`PermissionRule`/`PermissionBehavior`/`PermissionContext`; rule-matching engine; load `~/.aletheon/settings.toml [permissions]`. |
| **B — Socket approval protocol** | `crates/aletheon-body/src/impl/security/socket_approval.rs` (NEW), `crates/aletheon-body/src/impl/security/mod.rs` (re-export only — coordinate with A on this shared file via separate sections) | `SocketApprovalGate` implementing `ApprovalGate`, backed by an mpsc request/response channel pair. |
| **C — Daemon wiring** | `crates/aletheon-runtime/src/impl/daemon/handler.rs` | Out-of-band `approval_request` notification + `approval_response` method on the socket; install `SocketApprovalGate` on the guarded runner; bridge the gate's channel to the socket. |
| **D — TUI dialog** | `crates/aletheon-body/src/impl/ui/approval_dialog.rs` (NEW), `crates/aletheon-body/src/impl/ui/mod.rs`, `crates/aletheon-body/src/impl/cli/mod.rs` | Modal y/n/a/d approval widget; handle the `approval_request` notification in the TUI/CLI read loop and send back `approval_response`. |

**Shared-file coordination:** `security/mod.rs` is touched by A and B (both add `pub mod`/`pub use`). To avoid a write conflict, **A adds its lines first** (Batch 1), **B appends its re-export in Batch 2** after A's commit. They never edit the same lines.

**Read-only (no agent modifies):** `approval.rs` trait (consumed as-is), `runner.rs` (already consults the gate from Phase 1 — no change needed).

---

## Dependency Graph

```
Batch 1 (parallel):
  Agent A: A1 (permission types in abi) → A2 (rule engine + settings.toml loader)
  Agent D: D1 (approval_dialog widget)        [pure UI, no backend dep]

Batch 2 (after A2 + B available):
  Agent B: B1 (SocketApprovalGate + channel)  [needs ApprovalGate trait — already exists]
  Agent C: C1 (daemon out-of-band protocol + install gate)   [needs B1]
  Agent D: D2 (handle approval_request in CLI/TUI read loop)  [needs C1's wire format]

Batch 3:
  Agent A: A3 (wire PermissionContext into PolicyEngine/runner)  [needs A2 + C1]
  E1 (build + tests) → E2 (acceptance: daemon L2 prompts, n aborts)
```

`SocketApprovalGate` (B1) and the daemon protocol (C1) are the critical chain. The
permission model (A) and the TUI dialog widget (D1) are independent and start immediately.

---

## Parallel Batches (for the coordinator)

- **Batch 1:** Agent A (A1→A2) ‖ Agent D (D1). Barrier: A2 committed before B starts touching `security/mod.rs`.
- **Batch 2:** Agent B (B1) → then Agent C (C1) ‖ Agent D (D2). C1 defines the wire format; D2 consumes it — C must publish the exact JSON shape in its commit message / a shared constant before D2 finalizes.
- **Batch 3:** Agent A (A3) integration; then E1/E2.

Branch: `auro/feat/20260620-phase2-tui-permissions`. Commit per task.

---

## Agent A — Permission Model

### Task A1: Permission types in aletheon-abi

**Files:**
- Create: `crates/aletheon-abi/src/permission.rs`
- Modify: `crates/aletheon-abi/src/lib.rs`

- [ ] **Step 1: Write the failing test** (bottom of the new `permission.rs`):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mode_default_asks_for_dangerous() {
        assert_eq!(PermissionMode::default(), PermissionMode::Default);
    }
    #[test]
    fn rule_matches_glob_pattern() {
        let r = PermissionRule { tool: "bash_exec".into(), pattern: Some("git *".into()), behavior: PermissionBehavior::Allow };
        assert!(r.matches("bash_exec", "git status"));
        assert!(!r.matches("bash_exec", "rm -rf /"));
        assert!(!r.matches("file_write", "git status"));
    }
}
```

- [ ] **Step 2: Implement** `crates/aletheon-abi/src/permission.rs`:

```rust
//! Permission model: modes, rules, and a per-session context. Distinct from
//! `PermissionLevel` (L0–L3, the intrinsic risk of a tool) — this layer is the
//! *policy* over those tools (allow/deny/ask), sourced from config + session.

use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// How the agent treats actions that aren't explicitly allowed/denied by a rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PermissionMode {
    /// Dangerous (L2+) operations ask for approval. (default)
    #[default]
    Default,
    /// File edits auto-approve; other dangerous ops still ask.
    AcceptEdits,
    /// Read-only: any side-effecting tool is denied.
    Plan,
    /// All side-effecting ops auto-approve (restricted/sandboxed env only).
    BypassAll,
}

/// What to do when a rule matches.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionBehavior { Allow, Deny, Ask }

/// A single permission rule: tool (+ optional arg glob) → behavior.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PermissionRule {
    pub tool: String,
    /// Glob over the action summary (e.g. "git *", "rm -rf *"). None = match any args.
    pub pattern: Option<String>,
    pub behavior: PermissionBehavior,
}

impl PermissionRule {
    /// Does this rule apply to `(tool, action_summary)`? Supports a single trailing
    /// `*` wildcard in `pattern` (prefix match), matching the existing tool-pattern
    /// convention used by the learning RuleStore.
    pub fn matches(&self, tool: &str, action_summary: &str) -> bool {
        if self.tool != tool { return false; }
        match &self.pattern {
            None => true,
            Some(p) if p.ends_with('*') => action_summary.starts_with(&p[..p.len() - 1]),
            Some(p) => p == action_summary,
        }
    }
}

/// Per-session permission state: mode + ordered rules + session approvals.
#[derive(Debug, Clone, Default)]
pub struct PermissionContext {
    pub mode: PermissionMode,
    /// Rules in priority order (first match wins). Built from config sources.
    pub rules: Vec<PermissionRule>,
    /// Tools approved-for-session (from an `ApproveForSession` decision).
    pub session_approvals: HashSet<String>,
}

impl PermissionContext {
    /// Resolve a behavior for an action: first matching rule wins; otherwise fall
    /// back to the mode default for the given permission level (L0/L1 vs L2+).
    pub fn resolve(&self, tool: &str, action_summary: &str, is_dangerous: bool) -> PermissionBehavior {
        if self.session_approvals.contains(tool) {
            return PermissionBehavior::Allow;
        }
        for r in &self.rules {
            if r.matches(tool, action_summary) {
                return r.behavior;
            }
        }
        match self.mode {
            PermissionMode::BypassAll => PermissionBehavior::Allow,
            PermissionMode::Plan => if is_dangerous { PermissionBehavior::Deny } else { PermissionBehavior::Allow },
            PermissionMode::AcceptEdits => if is_dangerous { PermissionBehavior::Ask } else { PermissionBehavior::Allow },
            PermissionMode::Default => if is_dangerous { PermissionBehavior::Ask } else { PermissionBehavior::Allow },
        }
    }
}
```

- [ ] **Step 3: Export** — in `crates/aletheon-abi/src/lib.rs`, add next to the other `pub mod`/`pub use` lines:

```rust
pub mod permission;
pub use permission::{PermissionMode, PermissionBehavior, PermissionRule, PermissionContext};
```

- [ ] **Step 4: Test**

Run: `cargo test -p aletheon-abi permission -- --nocapture`
Expected: both tests PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/aletheon-abi/src/permission.rs crates/aletheon-abi/src/lib.rs
git commit -m "feat(abi): add PermissionMode/Rule/Behavior/Context"
```

---

### Task A2: Rule engine + settings.toml loader

**Files:**
- Create: `crates/aletheon-body/src/impl/security/permission_rules.rs`
- Modify: `crates/aletheon-body/src/impl/security/mod.rs`

- [ ] **Step 1: Write the failing test** (bottom of new file):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn loads_rules_from_toml_str() {
        let toml = r#"
[permissions]
mode = "default"
[[permissions.rules]]
tool = "bash_exec"
pattern = "git *"
behavior = "allow"
[[permissions.rules]]
tool = "bash_exec"
pattern = "rm -rf *"
behavior = "deny"
"#;
        let ctx = load_permission_context_from_str(toml).unwrap();
        assert_eq!(ctx.mode, aletheon_abi::PermissionMode::Default);
        assert_eq!(ctx.rules.len(), 2);
        assert_eq!(ctx.resolve("bash_exec", "git status", true), aletheon_abi::PermissionBehavior::Allow);
        assert_eq!(ctx.resolve("bash_exec", "rm -rf /tmp", true), aletheon_abi::PermissionBehavior::Deny);
    }
    #[test]
    fn missing_file_yields_default_context() {
        let ctx = load_permission_context(std::path::Path::new("/nonexistent/settings.toml"));
        assert_eq!(ctx.mode, aletheon_abi::PermissionMode::Default);
        assert!(ctx.rules.is_empty());
    }
}
```

- [ ] **Step 2: Implement** `permission_rules.rs`:

```rust
//! Loads a `PermissionContext` from `~/.aletheon/settings.toml [permissions]`.
//! Missing/invalid config falls back to a safe default (Default mode, no rules).

use std::path::Path;
use serde::Deserialize;
use aletheon_abi::{PermissionContext, PermissionMode, PermissionRule, PermissionBehavior};

#[derive(Deserialize)]
struct SettingsFile { permissions: Option<PermissionsSection> }

#[derive(Deserialize)]
struct PermissionsSection {
    #[serde(default)]
    mode: PermissionMode,
    #[serde(default)]
    rules: Vec<RuleToml>,
}

#[derive(Deserialize)]
struct RuleToml { tool: String, pattern: Option<String>, behavior: PermissionBehavior }

/// Parse a settings TOML string into a PermissionContext.
pub fn load_permission_context_from_str(s: &str) -> anyhow::Result<PermissionContext> {
    let parsed: SettingsFile = toml::from_str(s)?;
    let section = parsed.permissions.unwrap_or(PermissionsSection { mode: PermissionMode::default(), rules: vec![] });
    Ok(PermissionContext {
        mode: section.mode,
        rules: section.rules.into_iter().map(|r| PermissionRule {
            tool: r.tool, pattern: r.pattern, behavior: r.behavior,
        }).collect(),
        session_approvals: Default::default(),
    })
}

/// Load from a path; any error (missing file, parse failure) → safe default.
pub fn load_permission_context(path: &Path) -> PermissionContext {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| load_permission_context_from_str(&s).ok())
        .unwrap_or_default()
}
```

> `PermissionsSection` needs `mode` to default; `#[serde(default)]` uses
> `PermissionMode::default()` (derived `Default` from A1). `RuleToml.behavior` reuses the
> `#[serde(rename_all="snake_case")]` on `PermissionBehavior`.

- [ ] **Step 3: Register module** — append to `security/mod.rs` (A owns the first edit):

```rust
pub mod permission_rules;
pub use permission_rules::{load_permission_context, load_permission_context_from_str};
```

- [ ] **Step 4: Test**

Run: `cargo test -p aletheon-body permission_rules -- --nocapture`
Expected: both tests PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/aletheon-body/src/impl/security/permission_rules.rs crates/aletheon-body/src/impl/security/mod.rs
git commit -m "feat(security): load PermissionContext from settings.toml"
```

---

### Task A3: Wire PermissionContext into the runner (Batch 3, needs A2 + C1)

**Files:**
- Modify: `crates/aletheon-body/src/impl/security/runner.rs`

- [ ] **Step 1: Add a PermissionContext field + builder** to `ToolRunnerWithGuard`
  (mirroring the existing `with_approval_gate` pattern from Phase 1):

```rust
    permission_ctx: aletheon_abi::PermissionContext,
```
  init in `new()` with `aletheon_abi::PermissionContext::default()`, and add:
```rust
    pub fn with_permission_context(mut self, ctx: aletheon_abi::PermissionContext) -> Self {
        self.permission_ctx = ctx;
        self
    }
```

- [ ] **Step 2: Consult the context before the gate** — in `execute_tool`, in the
  `PolicyVerdict::RequireApproval` arm (the Phase 1 block), short-circuit using the mode:

```rust
            PolicyVerdict::RequireApproval { reason } => {
                if tool.permission_level() >= PermissionLevel::L2 {
                    let summary = input.get("command").and_then(|v| v.as_str())
                        .map(|c| format!("{}: {}", tool_name, c))
                        .unwrap_or_else(|| format!("{}: {}", tool_name, input));
                    use aletheon_abi::PermissionBehavior;
                    match self.permission_ctx.resolve(tool_name, &summary, true) {
                        PermissionBehavior::Allow => {} // rule/mode pre-approves
                        PermissionBehavior::Deny => {
                            self.log_audit(tool_name, &input, tool.permission_level(), turn_id, None, &start, "rule_denied").await;
                            return Err(ToolError::PolicyDenied { reason: format!("{}: denied by permission rule/mode", reason) });
                        }
                        PermissionBehavior::Ask => {
                            // existing Phase 1 approval-gate flow runs here
                            if self.session_approvals.contains(tool_name) {
                                // already approved this session
                            } else {
                                let req = ApprovalRequest {
                                    tool: tool_name.to_string(),
                                    action_summary: summary,
                                    risk_level: format!("{:?}", tool.permission_level()),
                                    detail: Some(input.to_string()),
                                };
                                match self.approval_gate.request(&req).await {
                                    ApprovalDecision::Approve => {}
                                    ApprovalDecision::ApproveForSession => { self.session_approvals.insert(tool_name.to_string()); }
                                    ApprovalDecision::Deny => {
                                        self.log_audit(tool_name, &input, tool.permission_level(), turn_id, None, &start, "approval_denied").await;
                                        return Err(ToolError::PolicyDenied { reason: format!("{}: denied by approval gate", reason) });
                                    }
                                }
                            }
                        }
                    }
                }
            }
```

- [ ] **Step 3: Test (no regression + mode behaviors)** — add tests for `BypassAll` (no
  prompt), `Plan` (deny dangerous), reusing the `DummyL2Tool` pattern from Phase 1's
  `approval_tests`.

Run: `cargo test -p aletheon-body runner -- --nocapture`
Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/aletheon-body/src/impl/security/runner.rs
git commit -m "feat(security): runner consults PermissionContext before approval gate"
```

---

## Agent B — Socket Approval Gate

### Task B1: SocketApprovalGate (Batch 2)

**Files:**
- Create: `crates/aletheon-body/src/impl/security/socket_approval.rs`
- Modify: `crates/aletheon-body/src/impl/security/mod.rs` (append re-export AFTER Agent A's lines)

- [ ] **Step 1: Write the failing test:**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::r#impl::security::approval::{ApprovalGate, ApprovalRequest, ApprovalDecision};

    #[tokio::test]
    async fn gate_forwards_request_and_returns_responder_decision() {
        let (gate, mut rx) = SocketApprovalGate::new();
        // Simulate the daemon side: read the forwarded request, answer Approve.
        let h = tokio::spawn(async move {
            let pending = rx.recv().await.unwrap();
            assert_eq!(pending.request.tool, "bash_exec");
            let _ = pending.respond.send(ApprovalDecision::Approve);
        });
        let req = ApprovalRequest { tool: "bash_exec".into(), action_summary: "rm x".into(), risk_level: "L2".into(), detail: None };
        let decision = gate.request(&req).await;
        assert_eq!(decision, ApprovalDecision::Approve);
        h.await.unwrap();
    }

    #[tokio::test]
    async fn gate_denies_if_channel_dropped() {
        let (gate, rx) = SocketApprovalGate::new();
        drop(rx); // no responder → fail-safe
        let req = ApprovalRequest { tool: "x".into(), action_summary: "y".into(), risk_level: "L2".into(), detail: None };
        assert_eq!(gate.request(&req).await, ApprovalDecision::Deny);
    }
}
```

- [ ] **Step 2: Implement** `socket_approval.rs`:

```rust
//! Cross-process approval gate. When the daemon's guarded runner needs approval,
//! this gate forwards the request over an mpsc channel to the daemon's request
//! handler, which relays it to the connected CLI/TUI and feeds the user's answer
//! back. Fail-safe: if the channel is gone or the responder is dropped → Deny.

use async_trait::async_trait;
use tokio::sync::{mpsc, oneshot};
use super::approval::{ApprovalGate, ApprovalRequest, ApprovalDecision};

/// A pending approval forwarded to the daemon side. The runner blocks on `respond`.
pub struct PendingApproval {
    pub request: ApprovalRequest,
    pub respond: oneshot::Sender<ApprovalDecision>,
}

/// Approval gate that forwards to a daemon-side receiver.
pub struct SocketApprovalGate {
    tx: mpsc::Sender<PendingApproval>,
}

impl SocketApprovalGate {
    /// Create the gate and the receiver the daemon handler will drain.
    pub fn new() -> (Self, mpsc::Receiver<PendingApproval>) {
        let (tx, rx) = mpsc::channel(8);
        (Self { tx }, rx)
    }
}

#[async_trait]
impl ApprovalGate for SocketApprovalGate {
    async fn request(&self, req: &ApprovalRequest) -> ApprovalDecision {
        let (respond, wait) = oneshot::channel();
        let pending = PendingApproval { request: req.clone(), respond };
        if self.tx.send(pending).await.is_err() {
            return ApprovalDecision::Deny; // daemon side gone → fail-safe
        }
        // Bound the wait so a disconnected client can't hang a turn forever.
        match tokio::time::timeout(std::time::Duration::from_secs(120), wait).await {
            Ok(Ok(decision)) => decision,
            _ => ApprovalDecision::Deny, // timeout or dropped responder → fail-safe
        }
    }
}
```

- [ ] **Step 3: Re-export** — append to `security/mod.rs` (after A's lines, no conflict):

```rust
pub mod socket_approval;
pub use socket_approval::{SocketApprovalGate, PendingApproval};
```

- [ ] **Step 4: Test**

Run: `cargo test -p aletheon-body socket_approval -- --nocapture`
Expected: both tests PASS.

- [ ] **Step 5: Commit** (commit message must document the channel API for Agent C/D):

```bash
git add crates/aletheon-body/src/impl/security/socket_approval.rs crates/aletheon-body/src/impl/security/mod.rs
git commit -m "feat(security): SocketApprovalGate (mpsc->oneshot, 120s fail-safe deny)

API for daemon: SocketApprovalGate::new() -> (gate, Receiver<PendingApproval>).
PendingApproval { request: ApprovalRequest, respond: oneshot::Sender<ApprovalDecision> }."
```

---

## Agent C — Daemon Approval Wiring (Batch 2, needs B1)

### Task C1: Out-of-band approval protocol + install gate

**Files:**
- Modify: `crates/aletheon-runtime/src/impl/daemon/handler.rs`

- [ ] **Step 1: Install the SocketApprovalGate on the guarded runner.** In
  `RequestHandler::new`, replace the `ToolRunnerWithGuard::new(sandbox, audit_logger)`
  construction so it uses a `SocketApprovalGate`, and store the `Receiver<PendingApproval>`
  on the handler:

```rust
        use aletheon_body::r#impl::security::socket_approval::{SocketApprovalGate, PendingApproval};
        let (approval_gate, approval_rx) = SocketApprovalGate::new();
        let tool_runner = Arc::new(Mutex::new(
            ToolRunnerWithGuard::new(sandbox, audit_logger)
                .with_approval_gate(Arc::new(approval_gate))
        ));
```
  Add fields to `RequestHandler`:
```rust
    tool_runner: Arc<Mutex<ToolRunnerWithGuard>>, // (already added in Phase 1)
    approval_rx: Arc<Mutex<tokio::sync::mpsc::Receiver<PendingApproval>>>,
```

- [ ] **Step 2: Define the socket wire format** (publish in the commit for Agent D):

  - Daemon → client out-of-band notification (no `id`, has `method`):
    ```json
    {"jsonrpc":"2.0","method":"approval_request","params":{"approval_id":"<uuid>","tool":"bash_exec","action_summary":"bash: rm -rf /tmp/x","risk_level":"L2","detail":"..."}}
    ```
  - Client → daemon response (a normal request):
    ```json
    {"jsonrpc":"2.0","id":N,"method":"approval_response","params":{"approval_id":"<uuid>","decision":"approve"}}
    ```
    `decision` ∈ `"approve" | "deny" | "approve_for_session"`.

- [ ] **Step 3: Pump pending approvals to the socket during a `chat` turn.** The `chat`
  handler currently `await`s `rt.process_react(...)` to completion. Wrap that await in a
  `tokio::select!` that also drains `approval_rx`: when a `PendingApproval` arrives, write
  an `approval_request` notification to the client socket, then await the matching
  `approval_response` (correlated by `approval_id`) and forward the decision via
  `pending.respond.send(...)`.

  Because the existing `handle()` returns a single `serde_json::Value`, this requires the
  `chat` arm to have access to the **socket write half**. Confirm how `server.rs` calls
  `handle()` and thread the writer in (the cleanest change: give `chat` a reference to an
  outbound notification sender the server owns). Implement the correlation map
  `HashMap<String, oneshot::Sender<ApprovalDecision>>` keyed by `approval_id`, and route
  incoming `approval_response` requests (Step 4) into it.

  > This is the most intricate task. Keep the change localized: a small
  > `ApprovalRelay { notify_tx, pending: Mutex<HashMap<...>> }` struct on the handler that
  > (a) the `chat` loop feeds from `approval_rx`, (b) the `approval_response` method
  > resolves. Do not restructure the JSON-RPC server.

- [ ] **Step 4: Add the `approval_response` method** to the `handle()` match:

```rust
            "approval_response" => {
                let aid = request["params"]["approval_id"].as_str().unwrap_or("").to_string();
                let decision = match request["params"]["decision"].as_str().unwrap_or("deny") {
                    "approve" => ApprovalDecision::Approve,
                    "approve_for_session" => ApprovalDecision::ApproveForSession,
                    _ => ApprovalDecision::Deny,
                };
                self.approval_relay.resolve(&aid, decision).await;
                json!({"jsonrpc":"2.0","id":id,"result":{"ok":true}})
            }
```

- [ ] **Step 5: Build + runtime tests**

Run: `cargo build -p aletheon-runtime && cargo test -p aletheon-runtime`
Expected: clean build, tests pass.

- [ ] **Step 6: Commit**

```bash
git add crates/aletheon-runtime/src/impl/daemon/handler.rs
git commit -m "feat(daemon): SocketApprovalGate + out-of-band approval_request/response"
```

---

## Agent D — TUI Approval Dialog

### Task D1: Approval dialog widget (Batch 1, no backend dep)

**Files:**
- Create: `crates/aletheon-body/src/impl/ui/approval_dialog.rs`
- Modify: `crates/aletheon-body/src/impl/ui/mod.rs`

- [ ] **Step 1: Write the failing test:**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn key_maps_to_decision() {
        assert_eq!(ApprovalDialog::key_to_decision('y'), Some(DialogDecision::Approve));
        assert_eq!(ApprovalDialog::key_to_decision('a'), Some(DialogDecision::ApproveForSession));
        assert_eq!(ApprovalDialog::key_to_decision('n'), Some(DialogDecision::Deny));
        assert_eq!(ApprovalDialog::key_to_decision('d'), Some(DialogDecision::Deny));
        assert_eq!(ApprovalDialog::key_to_decision('x'), None);
    }
}
```

- [ ] **Step 2: Implement** `approval_dialog.rs` — a modal ratatui widget holding the
  pending request text and mapping keys `y`/`a`/`n`/`d` to a decision:

```rust
//! Modal approval dialog shown when the daemon requests approval for an L2+ action.

use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};
use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Style, Color, Modifier};
use ratatui::Frame;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DialogDecision { Approve, ApproveForSession, Deny }

#[derive(Debug, Clone)]
pub struct ApprovalDialog {
    pub approval_id: String,
    pub tool: String,
    pub action_summary: String,
    pub risk_level: String,
}

impl ApprovalDialog {
    pub fn key_to_decision(c: char) -> Option<DialogDecision> {
        match c.to_ascii_lowercase() {
            'y' => Some(DialogDecision::Approve),
            'a' => Some(DialogDecision::ApproveForSession),
            'n' | 'd' => Some(DialogDecision::Deny),
            _ => None,
        }
    }

    /// Render centered over the given area.
    pub fn render(&self, f: &mut Frame, area: Rect) {
        let w = area.width.min(70).max(30);
        let h = 9u16.min(area.height);
        let x = area.x + (area.width.saturating_sub(w)) / 2;
        let y = area.y + (area.height.saturating_sub(h)) / 2;
        let rect = Rect { x, y, width: w, height: h };
        let body = format!(
            "Tool: {}\nRisk: {}\n\n{}\n\n[y]es  [a]lways  [N]o",
            self.tool, self.risk_level, self.action_summary,
        );
        f.render_widget(Clear, rect);
        let p = Paragraph::new(body)
            .block(Block::default().borders(Borders::ALL).title(" Approval required ")
                .border_style(Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)))
            .wrap(Wrap { trim: true })
            .alignment(Alignment::Left);
        f.render_widget(p, rect);
    }
}
```

> Verify the ratatui version's `Frame` signature in the existing `ui/mod.rs` render path
> (some versions use `Frame<'_>` without a backend generic). Match the existing widgets'
> render call style in `chat.rs`/`status.rs`.

- [ ] **Step 3: Register module + add to App state** — in `ui/mod.rs` add
  `mod approval_dialog;` and an `Option<ApprovalDialog>` field on `App` (e.g.
  `pending_approval: Option<ApprovalDialog>`); when `Some`, render it on top after the
  main panels.

- [ ] **Step 4: Test**

Run: `cargo test -p aletheon-body approval_dialog -- --nocapture`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/aletheon-body/src/impl/ui/approval_dialog.rs crates/aletheon-body/src/impl/ui/mod.rs
git commit -m "feat(ui): approval dialog widget (y/a/n/d)"
```

---

### Task D2: Handle approval_request in the CLI/TUI read loop (Batch 2, needs C1 wire format)

**Files:**
- Modify: `crates/aletheon-body/src/impl/ui/mod.rs`, `crates/aletheon-body/src/impl/cli/mod.rs`

- [ ] **Step 1:** In the TUI's socket read path (`try_read_response`), detect a frame whose
  `method == "approval_request"` (no `result`/`id`). On receipt, set
  `app.pending_approval = Some(ApprovalDialog { approval_id, tool, action_summary, risk_level })`
  from `params`.

- [ ] **Step 2:** In the key handler, when `pending_approval.is_some()`, route key presses
  to `ApprovalDialog::key_to_decision`; on a decision, write the `approval_response` request
  to the socket (with the stored `approval_id` and `decision` string) and clear
  `pending_approval`.

- [ ] **Step 3:** For the non-TTY/`single_message` CLI path (`cli/mod.rs`), handle an
  `approval_request` frame by prompting on stdin (reuse the y/a/N logic) and writing back
  `approval_response` — so `aletheon -m "..."` is also safe.

- [ ] **Step 4: Build**

Run: `cargo build -p aletheon-body`
Expected: clean.

- [ ] **Step 5: Commit**

```bash
git add crates/aletheon-body/src/impl/ui/mod.rs crates/aletheon-body/src/impl/cli/mod.rs
git commit -m "feat(ui): handle approval_request, send approval_response over socket"
```

---

## Batch 3 — Integration & Acceptance

### Task E1: Build + tests

- [ ] `cargo fmt --all && cargo build --workspace` → clean.
- [ ] `cargo test --workspace` → no failures; `>=` Phase-1 count + new tests.
- [ ] `cargo clippy --workspace -- -D warnings` → clean.
- [ ] Commit any fixups: `git commit -am "chore: fmt + clippy for phase 2"`.

### Task E2: Defining acceptance test (daemon path is now trustworthy)

- [ ] **Step 1: daemon L2 action prompts and aborts on No.** Start the daemon, connect the
  TUI (or `aletheon -m`), ask it to delete a file:

```bash
mkdir -p /tmp/aletheon && rm -f /tmp/aletheon/aletheon.sock
./target/debug/aletheond --socket /tmp/aletheon/aletheon.sock &
DPID=$!; sleep 2
cd /tmp && echo x > p2del.txt
printf 'n\n' | ./target/debug/aletheon --socket /tmp/aletheon/aletheon.sock \
  -m "Delete /tmp/p2del.txt using rm."
test -f /tmp/p2del.txt && echo "P2-SAFE-PASS (survived denial)" || echo "P2-SAFE-FAIL"
kill $DPID
```
Expected: an approval prompt appears via the socket; answering `n` leaves `p2del.txt`
intact → `P2-SAFE-PASS`. (Before Phase 2 the daemon would have silently denied via
`AutoDenyGate` — now it *asks*.)

- [ ] **Step 2: settings.toml rule pre-approves.** Add a `[[permissions.rules]]` allowing a
  specific safe command and confirm it runs without a prompt.

- [ ] **Step 3: Record outputs in the PR description.**

---

## Self-Review (spec coverage)

- PermissionMode/Rule/Behavior/Context → **A1**; settings.toml loader → **A2**; runner integration → **A3**.
- SocketApprovalGate (cross-process approval) → **B1**; daemon out-of-band protocol + install → **C1**.
- TUI approval dialog → **D1**; client handling of the protocol → **D2**.
- TUI stays in body (no new crate) → all UI tasks under `body/impl/ui/`. ✓
- Defining acceptance (daemon L2 asks; `n` aborts; rule pre-approves) → **E2**.
- Multi-agent: disjoint ownership (A=abi+permission, B=socket gate, C=daemon handler, D=ui),
  shared `security/mod.rs` ordered A-before-B. ✓

## Notes for implementing agents

- **Do not invent APIs.** Verify `ApprovalGate`/`ApprovalRequest`/`ApprovalDecision`,
  ratatui `Frame` signature, and the JSON-RPC server's `handle()` wiring against the real
  code before writing.
- **C1 is the hard task** (out-of-band socket flow). Keep it localized; do not restructure
  the JSON-RPC server. Publish the exact wire format in the commit so D2 matches it.
- Fail-safe everywhere: dropped channel / timeout / EOF → `Deny`. Never default to execute.
- Commit per task.
