# M-E — SubAgent Lifecycle — Implementation Plan

> **For agentic workers:** Use `workflow-feature` or `plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking. **Design-only handoff — do not execute product changes until the design-only gate is lifted.**

**Goal:** Give spawned sub-agents an explicit lifecycle. Today `SubAgentSpawner` is a bare tracking map with `spawn/update_status/remove` and no notion of *which* state changes are legal or of guaranteed teardown. This plan adds a `SubAgentState` enum with a legal-transition table and a `destroy()` that cancels in-flight work (via a `CancellationToken`) and frees the map slot.

**Architecture:** The lifecycle vocabulary (`SubAgentState`) lives in `base` (concept **ABI**), next to the existing UI-facing `SubAgentStatus`. The state machine and teardown live in `runtime` (concept **Runtime**) inside `SubAgentSpawner`. The spawner's map value changes from a bare `SubAgentHandle` to an internal `SubAgentEntry { handle, state, cancel }`, while `list()`/`get()` keep returning `&SubAgentHandle` so existing callers (`rpc.rs` `sub_agents`) are untouched. No new scheduling policy.

**Tech Stack:** Rust (Cargo workspace), `tokio` + `tokio-util` (`CancellationToken`), `serde`.

**Spec:** `docs/plans/2026-07-01-modules-roadmap-design.md` § "M-E. SubAgent lifecycle"

**Branch:** `auro/feat/20260701-aletheon-subagent-lifecycle` (own branch per repo policy).

---

## Ground truth (verified 2026-07-01)

| Claim | Evidence |
|---|---|
| `SubAgentSpawner` stores agents in a `HashMap<String, SubAgentHandle>` + `next_id` | `crates/runtime/src/core/sub_agent.rs:11-14` |
| Current API: `spawn(task, parent_turn_id) -> SubAgentHandle`, `update_status(id, status)`, `remove(id) -> bool`, `list() -> Vec<&SubAgentHandle>`, `get(id) -> Option<&SubAgentHandle>` | `sub_agent.rs:25-62` |
| **No state machine, no cancellation, no teardown guarantee today** — `remove` just drops the map slot | `sub_agent.rs:50-52` |
| No live task handle is stored per agent (map value is a plain `SubAgentHandle`) | `sub_agent.rs:12`, `sub_agent.rs:28-38` |
| UI status enum: `Planning`, `Executing{current_step}`, `WaitingApproval`, `Completed{summary}`, `Failed{error}` | `crates/base/src/events/ui_event.rs:114-121` |
| `SubAgentHandle` fields: `id, task, status, parent_turn_id, spawned_at_ms` | `ui_event.rs:123-131` |
| `base` re-exports `SubAgentHandle, SubAgentStatus` from `lib.rs` | `crates/base/src/lib.rs:127-129` |
| Spawner is owned by `Orchestrator` (field + `_mut` accessor) | `crates/runtime/src/core/orchestrator.rs:38`, `orchestrator.rs:390-396` |
| Only external reader of the map is the `sub_agents` RPC, via `.list()` → `a.id/a.task/a.status` | `crates/runtime/src/impl/daemon/handler/rpc.rs:729-744` |
| `runtime` module re-exports `SubAgentSpawner` | `crates/runtime/src/core/mod.rs:27` |
| Package names for `cargo` are `base` and `runtime` | `crates/base/Cargo.toml:2`, `crates/runtime/Cargo.toml:2` |
| `tokio-util` (for `CancellationToken`) is already a `runtime` dependency | `crates/runtime/Cargo.toml` `tokio-util = { workspace = true }` |
| `base` already depends on `serde` (derive) | `crates/base/Cargo.toml` `serde = { version = "1", features = ["derive"] }` |

---

## Design decisions (made for this plan)

1. **`SubAgentState` is a new, distinct enum — not a rename of `SubAgentStatus`.**
   `SubAgentStatus` (ABI, `ui_event.rs:114`) is the *UI display* projection and is
   serialized over RPC; the roadmap's lifecycle (`Created→Running→Waiting→
   Completed→Destroyed`) is a *control* concern. Keeping them separate avoids
   churning the wire shape of `SubAgentHandle`/`SubAgentStatusChanged`
   (`ui_event.rs:196`). `SubAgentState` lives in `base` alongside `SubAgentStatus`
   (spec: "status enum in `base`").
2. **Legal transitions are enforced by an explicit `can_transition_to` match**, not
   by scattered `if`s. `Destroyed` and `Failed`/`Completed` are terminal for the
   forward path; `Destroyed` is reachable from every non-terminal state so teardown
   can always run.
3. **The spawner map value becomes `SubAgentEntry { handle, state, cancel }`.**
   `cancel: CancellationToken` is created at `spawn` time and handed to callers via
   a new `cancel_token(id)` accessor so future task-spawning code can wire it into
   the child's async work. `destroy()` calls `cancel.cancel()` → any work awaiting
   `token.cancelled()` unblocks. This makes "cancel in-flight work" real *now* even
   though no task is wired yet.
4. **Public read API is preserved.** `list()`/`get()` still return `&SubAgentHandle`
   (mapped from `entry.handle`), so `rpc.rs:729-744` compiles unchanged. `remove`
   is kept as a thin alias that delegates to `destroy` (drops slot + cancels).

---

## File map

| File | Change |
|---|---|
| `crates/base/src/events/ui_event.rs` | add `SubAgentState` enum + `can_transition_to` + unit tests |
| `crates/base/src/lib.rs` | re-export `SubAgentState` alongside `SubAgentStatus` (`lib.rs:127-129`) |
| `crates/runtime/src/core/sub_agent.rs` | `SubAgentEntry` value type; `state`/`transition`/`cancel_token`/`destroy`; `remove` delegates to `destroy`; preserve `list`/`get`/`update_status` |

Default checks per phase: `cargo test -p base` (Phase 1) and
`cargo test -p runtime` (Phase 2); each phase ends with a commit.

---

## Phase 1 — `SubAgentState` + legal-transition table (ABI)

### Task 1: Add the lifecycle enum and transition rules to `base`

**Files:** Modify `crates/base/src/events/ui_event.rs` and `crates/base/src/lib.rs`.

- [ ] **Step 1: Write the failing test**

Add to `crates/base/src/events/ui_event.rs` (new tests module at end of file):

```rust
#[cfg(test)]
mod subagent_state_tests {
    use super::SubAgentState;

    #[test]
    fn legal_forward_path_is_allowed() {
        use SubAgentState::*;
        assert!(Created.can_transition_to(&Running));
        assert!(Running.can_transition_to(&Waiting));
        assert!(Waiting.can_transition_to(&Running));
        assert!(Running.can_transition_to(&Completed));
        assert!(Completed.can_transition_to(&Destroyed));
    }

    #[test]
    fn destroy_is_reachable_from_every_non_terminal_state() {
        use SubAgentState::*;
        for s in [Created, Running, Waiting, Completed, Failed] {
            assert!(s.can_transition_to(&Destroyed), "{s:?} -> Destroyed must be legal");
        }
    }

    #[test]
    fn illegal_transitions_are_rejected() {
        use SubAgentState::*;
        assert!(!Created.can_transition_to(&Completed), "must run before completing");
        assert!(!Completed.can_transition_to(&Running), "terminal-forward: no resurrection");
        assert!(!Destroyed.can_transition_to(&Running), "Destroyed is terminal");
        assert!(!Destroyed.can_transition_to(&Destroyed), "no self-loop on Destroyed");
    }
}
```

- [ ] **Step 2: Run — expected FAIL** (`SubAgentState` does not exist yet).

Run: `cargo test -p base subagent_state_tests`
Expected: compile error `cannot find type SubAgentState`.

- [ ] **Step 3: Implement the enum + transition table**

Add near `SubAgentStatus` in `crates/base/src/events/ui_event.rs` (after `:121`):

```rust
/// Explicit sub-agent lifecycle state (control-plane; distinct from the
/// UI-facing `SubAgentStatus`). Roadmap M-E: Created → Running → Waiting →
/// Completed → Destroyed, with Failed as an alternate terminal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SubAgentState {
    Created,
    Running,
    Waiting,
    Completed,
    Failed,
    Destroyed,
}

impl SubAgentState {
    /// Whether a transition from `self` to `next` is legal.
    ///
    /// `Destroyed` is reachable from any non-terminal state (teardown may run at
    /// any time) but is itself terminal. `Completed`/`Failed` only advance to
    /// `Destroyed`.
    pub fn can_transition_to(&self, next: &SubAgentState) -> bool {
        use SubAgentState::*;
        matches!(
            (self, next),
            (Created, Running)
                | (Created, Failed)
                | (Created, Destroyed)
                | (Running, Waiting)
                | (Running, Completed)
                | (Running, Failed)
                | (Running, Destroyed)
                | (Waiting, Running)
                | (Waiting, Completed)
                | (Waiting, Failed)
                | (Waiting, Destroyed)
                | (Completed, Destroyed)
                | (Failed, Destroyed)
        )
    }
}
```

Re-export it in `crates/base/src/lib.rs` (extend the block at `:127-129`):

```rust
pub use events::ui_event::{
    // ...existing names...
    SubAgentHandle, SubAgentState, SubAgentStatus, UiEvent,
};
```

> `Serialize`/`Deserialize` are derived to match the other `ui_event` types; the
> enum is not yet placed on the wire, but keeping it serializable lets a later
> RPC surface it without another ABI change.

- [ ] **Step 4: Run — expected PASS**

Run: `cargo test -p base subagent_state_tests`
Then the crate: `cargo test -p base`. Expected: all pass.

- [ ] **Step 5: Commit**

```bash
git add crates/base/src/events/ui_event.rs crates/base/src/lib.rs
git commit -m "feat(base): add SubAgentState lifecycle enum + legal-transition table"
```

---

## Phase 2 — State machine + `destroy()` teardown in the spawner (Runtime)

### Task 2: Track state per agent, enforce legal-only transitions, cancel + free on destroy

**Files:** Modify `crates/runtime/src/core/sub_agent.rs`.

- [ ] **Step 1: Write the failing test**

Add to `crates/runtime/src/core/sub_agent.rs` (new tests module at end of file):

```rust
#[cfg(test)]
mod lifecycle_tests {
    use super::*;
    use base::SubAgentState;

    #[test]
    fn spawn_starts_in_created_and_legal_transitions_advance() {
        let mut s = SubAgentSpawner::new();
        let h = s.spawn("task".into(), "turn-1".into());
        assert_eq!(s.state(&h.id), Some(SubAgentState::Created));
        assert!(s.transition(&h.id, SubAgentState::Running).is_ok());
        assert!(s.transition(&h.id, SubAgentState::Waiting).is_ok());
        assert_eq!(s.state(&h.id), Some(SubAgentState::Waiting));
    }

    #[test]
    fn illegal_transition_is_rejected_and_state_unchanged() {
        let mut s = SubAgentSpawner::new();
        let h = s.spawn("task".into(), "turn-1".into());
        // Created -> Completed is illegal (must Run first).
        assert!(s.transition(&h.id, SubAgentState::Completed).is_err());
        assert_eq!(s.state(&h.id), Some(SubAgentState::Created));
    }

    #[tokio::test]
    async fn destroy_cancels_in_flight_work_and_frees_the_slot() {
        let mut s = SubAgentSpawner::new();
        let h = s.spawn("task".into(), "turn-1".into());
        let token = s.cancel_token(&h.id).expect("token exists while agent is live");

        // Simulate in-flight work awaiting cancellation.
        let worker = tokio::spawn(async move {
            token.cancelled().await;
            "cancelled"
        });

        assert!(s.destroy(&h.id), "destroy returns true for a live agent");
        assert_eq!(worker.await.unwrap(), "cancelled", "destroy must cancel the token");
        assert!(s.get(&h.id).is_none(), "map slot is freed after destroy");
        assert_eq!(s.state(&h.id), None);
        assert!(!s.destroy(&h.id), "second destroy is a no-op");
    }
}
```

- [ ] **Step 2: Run — expected FAIL** (`state`/`transition`/`cancel_token`/`destroy` are undefined; map value is still `SubAgentHandle`).

Run: `cargo test -p runtime lifecycle_tests`

- [ ] **Step 3: Implement the entry type + methods**

Rewrite `crates/runtime/src/core/sub_agent.rs`. Change the imports, the map value
type, and add the lifecycle methods. Keep `list`/`get`/`update_status` behavior
intact (they project `&entry.handle`):

```rust
//! Sub-agent spawning and tracking.
//!
//! Sub-agents are spawned by the LLM via the `agent` tool call.
//! Their status is tracked and emitted to the TUI via UiEvent, and their
//! control-plane lifecycle is enforced via `SubAgentState`.

use std::collections::HashMap;
use base::ui_event::{SubAgentHandle, SubAgentStatus};
use base::SubAgentState;
use tokio_util::sync::CancellationToken;

/// Error returned when an illegal lifecycle transition is requested.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransitionError {
    /// No agent with the given id is tracked.
    Unknown(String),
    /// The transition `from -> to` is not legal.
    Illegal { from: SubAgentState, to: SubAgentState },
}

impl std::fmt::Display for TransitionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TransitionError::Unknown(id) => write!(f, "unknown sub-agent: {id}"),
            TransitionError::Illegal { from, to } => {
                write!(f, "illegal transition {from:?} -> {to:?}")
            }
        }
    }
}
impl std::error::Error for TransitionError {}

/// Internal per-agent record: the UI handle, the control-plane state, and a
/// cancellation token for in-flight work.
#[derive(Debug)]
struct SubAgentEntry {
    handle: SubAgentHandle,
    state: SubAgentState,
    cancel: CancellationToken,
}

/// Spawns and tracks sub-agents.
#[derive(Debug, Default)]
pub struct SubAgentSpawner {
    agents: HashMap<String, SubAgentEntry>,
    next_id: usize,
}

impl SubAgentSpawner {
    pub fn new() -> Self {
        Self { agents: HashMap::new(), next_id: 0 }
    }

    /// Register a new sub-agent and return its handle. Starts in `Created`.
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
        self.agents.insert(
            id,
            SubAgentEntry {
                handle: handle.clone(),
                state: SubAgentState::Created,
                cancel: CancellationToken::new(),
            },
        );
        handle
    }

    /// Update an agent's UI status (unchanged UI-display behavior).
    pub fn update_status(&mut self, id: &str, status: SubAgentStatus) {
        if let Some(entry) = self.agents.get_mut(id) {
            entry.handle.status = status;
        }
    }

    /// Current control-plane state of an agent, if tracked.
    pub fn state(&self, id: &str) -> Option<SubAgentState> {
        self.agents.get(id).map(|e| e.state)
    }

    /// A clone of the agent's cancellation token (for wiring into spawned work).
    pub fn cancel_token(&self, id: &str) -> Option<CancellationToken> {
        self.agents.get(id).map(|e| e.cancel.clone())
    }

    /// Attempt a legal-only lifecycle transition.
    pub fn transition(
        &mut self,
        id: &str,
        next: SubAgentState,
    ) -> Result<(), TransitionError> {
        let entry = self
            .agents
            .get_mut(id)
            .ok_or_else(|| TransitionError::Unknown(id.to_string()))?;
        if entry.state.can_transition_to(&next) {
            entry.state = next;
            Ok(())
        } else {
            Err(TransitionError::Illegal { from: entry.state, to: next })
        }
    }

    /// Tear an agent down: cancel its in-flight work, drop its handle, free the
    /// map slot. Returns `false` if no such agent was tracked (idempotent).
    pub fn destroy(&mut self, id: &str) -> bool {
        match self.agents.remove(id) {
            Some(entry) => {
                entry.cancel.cancel();
                true
            }
            None => false,
        }
    }

    /// Remove a completed/failed agent (delegates to `destroy` for teardown).
    pub fn remove(&mut self, id: &str) -> bool {
        self.destroy(id)
    }

    /// List all active agents.
    pub fn list(&self) -> Vec<&SubAgentHandle> {
        self.agents.values().map(|e| &e.handle).collect()
    }

    /// Get a specific agent's handle.
    pub fn get(&self, id: &str) -> Option<&SubAgentHandle> {
        self.agents.get(id).map(|e| &e.handle)
    }
}
```

> The hand-written `Default for SubAgentSpawner` at `sub_agent.rs:65-69` is
> replaced by `#[derive(Default)]` on the struct (both fields are `Default`).
> Delete the old `impl Default` block.

- [ ] **Step 4: Run — expected PASS**

Run: `cargo test -p runtime lifecycle_tests`
Then the crate: `cargo test -p runtime`.
Sanity: `cargo build -p runtime` — the `sub_agents` RPC (`rpc.rs:729-744`) still
compiles because `list()` still yields `&SubAgentHandle` with `.id/.task/.status`.

- [ ] **Step 5: Commit**

```bash
git add crates/runtime/src/core/sub_agent.rs
git commit -m "feat(runtime): SubAgent state machine + destroy() teardown (cancel + free slot)"
```

---

## Self-review checklist (done at plan-write time)

- **Spec coverage:** `SubAgentState` enum + legal transitions (Task 1) ↔ M-E
  "explicit `SubAgentState` enum + transitions"; `destroy()` cancels in-flight work
  and frees the slot (Task 2) ↔ M-E "`destroy()` that guarantees resource cleanup
  (cancel tasks, drop handles)"; legal-only enforcement test + destroy test ↔ M-E
  "Test: lifecycle transitions are legal-only; destroy cancels in-flight work and
  frees the map slot".
- **Affected files match spec:** `runtime/src/core/sub_agent.rs` + status enum in
  `base` — exactly the two the roadmap names (`modules-roadmap-design.md:417`).
- **Placeholder scan:** none — exact enum, exact match table, exact methods, exact
  cargo commands.
- **Type consistency:** map value change `SubAgentHandle → SubAgentEntry` is
  contained; `list`/`get` re-project `&SubAgentHandle` so the only external reader
  (`rpc.rs:729-744`) is unchanged; `spawn`/`update_status`/`remove` keep their
  public signatures; `CancellationToken` comes from `tokio-util`, already a
  `runtime` dep.
- **Non-goals honored:** no scheduling policy, no changes to `SubAgentStatus` wire
  shape, no new RPC surface.

## Risks / notes for the implementer

- **Low blast radius.** The map value type change is internal; run
  `cargo build -p runtime` after Task 2 to confirm no other reader of
  `SubAgentSpawner.agents` exists (grep found only `list()` via `rpc.rs:729-744`).
- **No live task is wired to the token yet.** `cancel_token(id)` exists so the
  future `agent`-tool task-spawn path can pass the child token into its async work;
  until then, `destroy()`'s cancellation is a correct no-op for agents that never
  registered work. Do **not** add task-spawning here — that is out of M-E scope.
- **`remove` semantics changed subtly:** it now also cancels the token before
  dropping the slot. This is strictly stronger than today's plain
  `HashMap::remove` (`sub_agent.rs:50-52`) and safe for existing callers.
- **`Destroyed` state is never stored in the map** — `destroy()` removes the entry,
  so `state(id)` returns `None` afterward (consistent with "frees the map slot").
  If a later requirement needs a lingering `Destroyed` record for audit, that is a
  follow-up (would keep the entry and flip `state`, changing `list()` semantics).
- **Keep `SubAgentState` and `SubAgentStatus` distinct.** Do not collapse them; the
  UI enum carries payloads (`Executing{current_step}`, `Completed{summary}`) that
  the control-plane lifecycle deliberately omits.
