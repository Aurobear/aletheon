# Aletheon P0-P3 Stabilization Design

**Date:** 2026-07-04  
**Baseline:** Current working tree, including the in-progress systemd/container/session gateway changes  
**Strategy:** Sequential stabilization; every phase must be independently testable and releasable

## 1. Objective

Move Aletheon from a rapidly evolving, dual-architecture codebase to a clean and documented runtime without mixing mechanical cleanup, behavior migration, structural decomposition, and documentation correction in one change set.

```text
P0 Engineering green line
  -> P1 Architecture convergence
  -> P2 Core module decomposition
  -> P3 Documentation and capability alignment
```

Every phase preserves the current uncommitted product work, uses tests before behavior-changing migrations, and ends with the complete workspace validation suite passing.

## 2. Cross-Phase Constraints

- Do not discard, overwrite, or silently revert existing working-tree changes.
- Keep P0 behavior-neutral; architecture behavior changes start in P1.
- Migrate callers before deleting compatibility implementations.
- Keep Event/IPC convergence separate from execution-loop convergence.
- Keep module moves separate from behavior changes.
- Treat claims without a code or test anchor as unverified.
- Each phase gets its own implementation plan and commit series.
- Each phase must pass:

```bash
cargo fmt --all -- --check
cargo check --workspace --all-targets
cargo test --workspace --all-targets
cargo clippy --workspace --all-targets -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps
```

## 3. P0: Engineering Green Line

### Goal

Restore a warning-free, reproducible development baseline without changing runtime behavior.

### Scope

1. Record the dirty-worktree baseline and current validation results.
2. Separate pre-existing warnings from warnings introduced by the in-progress host changes.
3. Fix unused imports, variables, visibility mismatches, invalid feature gates, dead code, and documentation warnings.
4. Align local validation commands with CI.
5. Add a guard that prevents new uses of APIs already marked deprecated.

### Exclusions

- No EventBus or Engine migration.
- No daemon, ReAct loop, or TUI decomposition.
- No new product capability.

### Flow

```text
Capture baseline
  -> mechanical warning fixes per crate
  -> per-crate validation
  -> full workspace validation
  -> freeze green baseline
```

### Acceptance Criteria

- All five cross-phase validation commands exit successfully.
- Clippy with `-D warnings` passes.
- Existing functional and integration tests remain unchanged unless a test itself contains a warning.
- Current systemd/container/session gateway work remains intact.
- The deprecated-API guard reports only the explicitly baselined legacy uses.

## 4. P1: Architecture Convergence

### Goal

Remove the two production dual tracks: legacy events/IPC and the legacy cognitive Engine.

### P1-A: Events and IPC

```text
Legacy: Event -> EventBus -> KernelEventBus
Target: Envelope -> CommunicationBus -> Transport
```

Migration order:

1. Inventory all production and test uses of `Event`, `EventBus`, and `IpcBackend`.
2. Add equivalence tests for routing, request/response, subscription, priority, serialization, and transport failure.
3. Migrate `base`, then `cognit`/`corpus`/`dasein`, then `runtime`, then examples and tests.
4. Switch the daemon production path to `CommunicationBus`.
5. Remove legacy bridges, deprecated traits, and compatibility re-exports.
6. Add a source guard preventing legacy symbols from returning.

### P1-B: Agent Execution Loop

```text
Legacy: Engine + cognitive_loop + streaming + tool_dispatch
Target: ReActLoop + EventSink + VerdictHandler
```

Migration order:

1. Lock down streaming, tool calls, approval, cancellation, compaction, audit, hooks, and memory behavior with characterization tests.
2. Move daemon chat handling to `ReActLoop`.
3. Move `aletheon-exec` and relevant tests to `ReActLoop`.
4. Integrate journal, memory, hooks, agent registry, and cancellation through explicit ReAct loop collaborators.
5. Delete the legacy Engine modules and deprecated fields.
6. Verify that the TUI event protocol remains wire-compatible.

### Risk Controls

- P1-A and P1-B are separate commit/MR sequences.
- Callers migrate before legacy definitions are removed.
- P2 file decomposition does not begin until P1 behavior tests pass.
- TUI scenario and daemon integration suites run after every production-path switch.

### Acceptance Criteria

- Production code contains no legacy `Event`, `EventBus`, or `IpcBackend` use.
- Daemon and non-interactive execution use only `ReActLoop`.
- Related deprecation annotations and compatibility re-exports are removed.
- IPC, streaming response, approval, cancellation, compaction, audit, and session recovery tests pass.
- The complete cross-phase validation suite passes.

## 5. P2: Core Module Decomposition

### Goal

Reduce runtime, daemon, and TUI coupling after behavior has converged, while preserving public and wire protocols.

### Target Boundaries

```text
daemon/
|-- protocol/       JSON-RPC parsing, responses, errors
|-- connection/     Unix socket and client lifecycle
|-- session/        session creation, resume, snapshots
|-- turn/           single-turn coordination
|-- approval/       tool approval and callbacks
`-- handlers/       thin chat, goal, memory, debug adapters

react_loop/
|-- state.rs
|-- inference.rs
|-- tool_execution.rs
|-- cancellation.rs
|-- compaction.rs
`-- metrics.rs

tui/
|-- controller/     input and state transitions
|-- transport/      daemon communication
|-- model/          application state
`-- view/           pure rendering
```

### Decomposition Rules

- Write characterization tests before moving responsibilities.
- Move one responsibility at a time without changing behavior.
- Handlers adapt protocols; they do not own business workflows.
- Session, turn, and approval boundaries use explicit input/output types.
- Align `App` and handler visibility instead of exposing functions whose argument types are private.
- Prefer files below 500 lines when that follows natural responsibilities; do not create artificial abstractions solely to meet a line target.
- Preserve JSON-RPC fields and TUI event formats.

### Initial Targets

- `crates/runtime/src/impl/daemon/handler/rpc.rs`
- `crates/runtime/src/impl/daemon/handler/mod.rs`
- `crates/runtime/src/core/react_loop/mod.rs`
- `crates/runtime/src/core/session_gateway/gateway.rs`
- `crates/interact/src/tui/mod.rs`
- `crates/interact/src/tui/response.rs`
- `crates/interact/src/tui/cli.rs`

### Acceptance Criteria

- Existing JSON-RPC and TUI event formats remain compatible.
- Extracted units have focused unit tests and documented responsibilities.
- No new crate dependency cycle or cross-layer reverse dependency is introduced.
- Core files have one primary responsibility and narrow public surfaces.
- The complete cross-phase validation suite and TUI scenario suite pass.

## 6. P3: Documentation and Capability Alignment

### Goal

Make public and developer documentation describe the post-P1/P2 implementation rather than mixing present capabilities with roadmap goals.

### Capability Status Model

| Status | Meaning |
|---|---|
| Stable | Default build, production path, and automated tests exist |
| Experimental | Runnable behind a feature, environment dependency, or incomplete test coverage |
| Planned | Interface, placeholder, or design exists without a production implementation |
| Deprecated | Kept temporarily for migration and not recommended for new use |

### Documentation Work

1. Rewrite the README capability section to separate vision, current state, and roadmap.
2. Remove or qualify unverified performance claims such as `<1ms`.
3. Correct crate, binary, example, and documentation links.
4. Add a capability matrix for daemon, CLI/TUI, providers, memory, sandbox, IPC, eBPF, FUSE, io_uring, Android, and embedded targets.
5. Anchor every Stable capability to an implementation entry point and automated test.
6. Update architecture documents to describe only the converged production path.
7. Document request, tool approval, memory, and session lifecycles with compact ASCII diagrams.
8. Document validation commands, feature flags, system dependencies, platform limits, and module ownership.
9. Add checks for README-local links, documented commands, and workspace crate/binary inventory drift.

### Acceptance Criteria

- Every Stable claim has a code and test anchor.
- All local links resolve and all documented smoke commands run successfully.
- Roadmap capabilities are not presented as currently available.
- A new developer can build, start the daemon, connect CLI/TUI, and run tests from the documentation.
- The complete cross-phase validation suite and documentation drift checks pass.

## 7. Delivery Structure

The design is implemented through four plans:

1. `P0 Engineering Green Line` — behavior-neutral cleanup and CI parity.
2. `P1 Architecture Convergence` — separate P1-A and P1-B migration sequences.
3. `P2 Core Module Decomposition` — responsibility-preserving extraction.
4. `P3 Documentation Alignment` — evidence-backed public and developer documentation.

Each plan must identify exact files and symbols from the then-current working tree, use failing-first tests for behavior contracts, provide deterministic verification commands, and define small commits that do not include unrelated existing changes.

## 8. Explicit Non-Goals

- Implementing new eBPF, FUSE, Android, embedded, vector database, or io_uring capabilities.
- Changing the JSON-RPC or TUI wire protocol for aesthetic consistency.
- Replacing SQLite, Tokio, Ratatui, or the current provider abstraction.
- Reorganizing crates beyond what is required to remove verified dependency inversions.
- Rewriting working subsystems merely to reduce line count.
