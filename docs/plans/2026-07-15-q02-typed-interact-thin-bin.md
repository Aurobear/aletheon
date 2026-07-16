# Q02 Typed Interact and Thin Bin Implementation Plan

**Goal:** Make Interact a typed reducer-driven protocol client and reduce Bin to host selection, argument parsing and startup.

**Architecture:** Fabric protocol schemas generate typed client messages; Interact reduces snapshots/events into UI state, and Bin delegates startup without constructing domains.

**Tech Stack:** Rust, Serde, Ratatui, typed Fabric protocol, snapshot testing

**Source requirements:** `docs/plans/2026-07-15-architecture-coupling-optimization-plan.md:1152-1160`.

**Prerequisites:** Q01 and R02.

## Current-code anchors

- Interact TUI entrypoints are `run_tui` and `run_with_config` at `crates/interact/src/tui/mod.rs:109-118`.
- TUI state and resume protocol are separate structures at `crates/interact/src/tui/state.rs:76` and `crates/interact/src/tui/session_protocol.rs:7-18`.
- Bin contains command/runtime behavior beyond startup in `crates/bin/src/main.rs`, including `run_exec` at `crates/bin/src/main.rs:282`.
- Interact currently depends directly on Kernel and Corpus at `crates/interact/Cargo.toml:9-12`; Q02 removes both edges.

## Invariants and non-goals

- Interact does not own domain policy or daemon state.
- Reconnect never reconstructs state from terminal pixels.
- Provider/model labels come from protocol state rather than constants.

## Key contracts

```rust
pub enum UiAction { Snapshot(UiSnapshot), Item(ItemEvent), Approval(ApprovalEvent), Agent(AgentEvent), Reconnected(EventCursor), Failed(UiError) }
pub fn reduce(state: &mut AppState, action: UiAction) -> Vec<UiEffect>;
```

## Task 1: Define generated typed protocol clients

**Create:** `crates/fabric/src/protocol/client.rs`
**Create:** `crates/fabric/tests/protocol_schema.rs`
**Modify:** `crates/fabric/src/lib.rs`

- [ ] Generate or derive typed request/response/event bindings from versioned protocol schemas.
- [ ] Reject unknown incompatible versions and retain forward-compatible optional fields explicitly.
- [ ] Cover reconnect cursor, snapshot request and incremental event subscription.

Run: `cargo test -p fabric --test protocol_schema`

## Task 2: Converge TUI state into a pure reducer

**Create:** `crates/interact/src/tui/reducer.rs`
**Modify:** `crates/interact/src/tui/state.rs`
**Modify:** `crates/interact/src/tui/session_protocol.rs`
**Create:** `crates/interact/tests/tui_reducer.rs`

- [ ] Define typed actions for snapshot, Item lifecycle, tool activity, approval, Agent status, reconnect and errors.
- [ ] Make reducer transitions pure and deterministic; keep terminal/render effects outside state mutation.
- [ ] Resume from R02 snapshot plus event cursor without duplicating completed Items.
- [ ] Remove parallel ad-hoc fields once reducer parity tests pass.

Run: `cargo test -p interact --test tui_reducer`

## Task 3: Add lifecycle snapshots

**Create:** `crates/interact/tests/snapshots/item_lifecycle.snap`
**Create:** `crates/interact/tests/snapshots/approval_lifecycle.snap`
**Create:** `crates/interact/tests/snapshots/reconnect_resume.snap`
**Create:** `crates/interact/tests/tui_snapshots.rs`

- [ ] Snapshot streaming-to-terminal Item behavior, collapsed tool output and approval transitions.
- [ ] Snapshot reconnect with no lost or duplicated content.
- [ ] Keep model/provider labels sourced from protocol state rather than hard-coded UI text.

Run: `cargo test -p interact --test tui_snapshots`

## Task 4: Remove domain/runtime knowledge from Interact

**Modify:** `crates/interact/Cargo.toml`
**Modify:** `crates/interact/src/tui/rpc_client.rs`
**Modify:** `scripts/architecture-check.sh`

- [ ] Depend on protocol contracts and transport client only.
- [ ] Reject direct Corpus, Kernel, domain store or daemon implementation imports.
- [ ] Keep filesystem/process operations behind explicit host requests.

Run: `bash scripts/architecture-check.sh && cargo tree -p interact --edges normal`

## Task 5: Reduce Bin to a host shell

**Modify:** `crates/bin/src/main.rs`
**Create:** `crates/bin/tests/host_routing.rs`

- [ ] Parse arguments, select daemon/exec/TUI host mode and delegate to Executive/Interact use cases.
- [ ] Remove direct domain construction, configuration merging and protocol state mutation.
- [ ] Test each CLI mode selects one host path and propagates exit status.

Run: `cargo test -p aletheon-bin --test host_routing && cargo test -p interact --all-targets`

## Final verification and commit

Run: `scripts/architecture-check.sh && cargo test --workspace --all-targets --no-fail-fast`

Inspect the staged diff, then commit with subject `refactor(interact): use typed reducer protocol` and a body that records the source requirement, authority/bypass problem, implemented boundaries, focused tests and deletion evidence.

## Completion evidence

- [ ] TUI reconnect/resume is deterministic from protocol state.
- [ ] Item and approval lifecycle snapshots cover normal and failure paths.
- [ ] Bin contains no domain/runtime construction and Interact has no Corpus/Kernel dependency.
