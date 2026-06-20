# Phase 2: Merge comm into base Implementation Plan

> **For agentic workers:** Use `workflow-feature` or `writing-plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Merge the `comm` crate into `base` to form a complete foundation layer with interfaces + communication + common modules.

**Architecture:** Move comm's implementation code (impl/, bridge/) into base/src/comm/ submodule. Fold comm's core/ additions (ConcreteEvent, EventEnvelopeExt) into base's existing files. Update all dependencies and imports.

**Tech Stack:** Rust, Cargo workspace

---

## Merge Strategy

### File Mapping

| comm Source | base Destination | Action |
|---|---|---|
| `core/bus.rs` | (already in base) | Delete - re-exports only |
| `core/event.rs` | `base/src/event.rs` | Merge ConcreteEvent into existing file |
| `core/envelope.rs` | `base/src/envelope.rs` | Merge EventEnvelopeExt into existing file |
| `core/mod.rs` | - | Delete |
| `bridge/event_bridge.rs` | `base/src/comm/bridge/event_bridge.rs` | Move |
| `bridge/mod.rs` | `base/src/comm/bridge/mod.rs` | Move |
| `impl/communication_bus.rs` | `base/src/comm/impl/communication_bus.rs` | Move |
| `impl/debug_bus.rs` | `base/src/comm/impl/debug_bus.rs` | Move |
| `impl/event_log.rs` | `base/src/comm/impl/event_log.rs` | Move |
| `impl/in_process.rs` | `base/src/comm/impl/in_process.rs` | Move |
| `impl/kernel_bus.rs` | `base/src/comm/impl/kernel_bus.rs` | Move |
| `impl/pubsub.rs` | `base/src/comm/impl/pubsub.rs` | Move |
| `impl/request_response.rs` | `base/src/comm/impl/request_response.rs` | Move |
| `impl/routing_policy.rs` | `base/src/comm/impl/routing_policy.rs` | Move |
| `impl/subscription.rs` | `base/src/comm/impl/subscription.rs` | Move |
| `impl/unix_socket_transport.rs` | `base/src/comm/impl/unix_socket_transport.rs` | Move |
| `impl/ipc/*` | `base/src/comm/impl/ipc/*` | Move entire directory |
| `impl/mod.rs` | `base/src/comm/impl/mod.rs` | Move |
| `lib.rs` | `base/src/comm/mod.rs` | Merge re-exports |
| `tests/protocol_e2e.rs` | `base/tests/protocol_e2e.rs` | Move |

### Dependencies to Add to base

```toml
# Upgrade
tokio = { version = "1", features = ["full"] }  # was ["time", "rt"]

# New
tracing = "0.1"
parking_lot = "0.12"
nix = { workspace = true }
libc = { workspace = true }
dashmap = "6"
```

### Import Rewrites (24 occurrences)

| Old Import | New Import | Files |
|---|---|---|
| `use comm::CommunicationBus` | `use base::CommunicationBus` | 7 files |
| `use comm::envelope::Payload` | `use base::Payload` | 5 files |
| `use comm::KernelEventBus` | `use base::KernelEventBus` | 2 files |
| `use comm::ConcreteEvent` | `use base::ConcreteEvent` | 1 file |
| `use comm::core::event::ConcreteEvent` | `use base::ConcreteEvent` | 1 file |
| `use comm::r#impl::debug_bus::*` | `use base::debug_bus::*` | 2 files |

---

## Task 1: Create comm submodule structure in base

**Files:**
- Create: `crates/base/src/comm/mod.rs`
- Create: `crates/base/src/comm/bridge/mod.rs`
- Create: `crates/base/src/comm/impl/mod.rs`
- Create: `crates/base/src/comm/impl/ipc/mod.rs`

- [ ] **Step 1: Create comm submodule directories**

```bash
cd /home/aurobear/Bear-ws/work/aletheon
mkdir -p crates/base/src/comm/bridge
mkdir -p crates/base/src/comm/impl/ipc
```

- [ ] **Step 2: Create comm/mod.rs**

Create `crates/base/src/comm/mod.rs`:
```rust
//! Communication subsystem implementation.
//!
//! This module contains the concrete implementations of the communication
//! protocols and transports defined in the base crate.

pub mod bridge;
pub mod r#impl;

// Re-export main types at comm level for convenience
pub use bridge::event_bridge::EventBridge;
pub use r#impl::communication_bus::{BusConfig, CommunicationBus};
pub use r#impl::debug_bus::{DebugBusHook, EventFilter, EventRecorder, PerfCounter};
pub use r#impl::event_log::{EventLog, LogEntry};
pub use r#impl::in_process::InProcessTransport;
pub use r#impl::kernel_bus::KernelEventBus;
pub use r#impl::pubsub::PubSubProtocol;
pub use r#impl::request_response::RequestResponseProtocol;
pub use r#impl::routing_policy::{RouteAction, RoutingPolicy};
pub use r#impl::subscription::SubscriptionRegistry;
pub use r#impl::unix_socket_transport::UnixSocketTransport;
```

- [ ] **Step 3: Create comm/bridge/mod.rs**

Create `crates/base/src/comm/bridge/mod.rs`:
```rust
//! Bridge layer for communication subsystem.

pub mod event_bridge;
```

- [ ] **Step 4: Create comm/impl/mod.rs**

Create `crates/base/src/comm/impl/mod.rs`:
```rust
//! Implementation layer for communication subsystem.

pub mod communication_bus;
pub mod debug_bus;
pub mod event_log;
pub mod in_process;
pub mod ipc;
pub mod kernel_bus;
pub mod pubsub;
pub mod request_response;
pub mod routing_policy;
pub mod subscription;
pub mod unix_socket_transport;
```

- [ ] **Step 5: Create comm/impl/ipc/mod.rs**

Create `crates/base/src/comm/impl/ipc/mod.rs`:
```rust
//! IPC (Inter-Process Communication) implementations.

pub mod io_uring;
pub mod json_rpc;
pub mod manager;
pub mod priority_queue;
pub mod shared_mem;
pub mod unix_socket;
```

---

## Task 2: Move comm implementation files to base

**Files:**
- Move: All files from `crates/comm/src/impl/` to `crates/base/src/comm/impl/`
- Move: All files from `crates/comm/src/bridge/` to `crates/base/src/comm/bridge/`

- [ ] **Step 1: Move impl/ directory**

```bash
cd /home/aurobear/Bear-ws/work/aletheon
cp crates/comm/src/impl/communication_bus.rs crates/base/src/comm/impl/
cp crates/comm/src/impl/debug_bus.rs crates/base/src/comm/impl/
cp crates/comm/src/impl/event_log.rs crates/base/src/comm/impl/
cp crates/comm/src/impl/in_process.rs crates/base/src/comm/impl/
cp crates/comm/src/impl/kernel_bus.rs crates/base/src/comm/impl/
cp crates/comm/src/impl/pubsub.rs crates/base/src/comm/impl/
cp crates/comm/src/impl/request_response.rs crates/base/src/comm/impl/
cp crates/comm/src/impl/routing_policy.rs crates/base/src/comm/impl/
cp crates/comm/src/impl/subscription.rs crates/base/src/comm/impl/
cp crates/comm/src/impl/unix_socket_transport.rs crates/base/src/comm/impl/
cp crates/comm/src/impl/ipc/*.rs crates/base/src/comm/impl/ipc/
```

- [ ] **Step 2: Move bridge/ directory**

```bash
cp crates/comm/src/bridge/event_bridge.rs crates/base/src/comm/bridge/
```

- [ ] **Step 3: Move test file**

```bash
cp crates/comm/tests/protocol_e2e.rs crates/base/tests/
```

- [ ] **Step 4: Verify files copied**

```bash
ls -la crates/base/src/comm/impl/
ls -la crates/base/src/comm/bridge/
ls -la crates/base/tests/
```

Expected: All files present.

---

## Task 3: Merge comm/core/ additions into base

**Files:**
- Modify: `crates/base/src/event.rs` (add ConcreteEvent)
- Modify: `crates/base/src/envelope.rs` (add EventEnvelopeExt)

- [ ] **Step 1: Read comm/core/event.rs to understand ConcreteEvent**

```bash
cat crates/comm/src/core/event.rs
```

- [ ] **Step 2: Add ConcreteEvent to base/src/event.rs**

Read `crates/base/src/event.rs` and add the ConcreteEvent struct from comm's core/event.rs. Merge into the existing file.

- [ ] **Step 3: Read comm/core/envelope.rs to understand EventEnvelopeExt**

```bash
cat crates/comm/src/core/envelope.rs
```

- [ ] **Step 4: Add EventEnvelopeExt to base/src/envelope.rs**

Read `crates/base/src/envelope.rs` and add the EventEnvelopeExt trait from comm's core/envelope.rs. Merge into the existing file.

---

## Task 4: Update base/lib.rs to include comm module

**Files:**
- Modify: `crates/base/src/lib.rs`

- [ ] **Step 1: Read current base/lib.rs**

```bash
cat crates/base/src/lib.rs
```

- [ ] **Step 2: Add comm module declaration**

Add to `crates/base/src/lib.rs`:
```rust
pub mod comm;
```

- [ ] **Step 3: Add re-exports for comm types**

Add re-exports at the top level for commonly used comm types:
```rust
// Communication subsystem re-exports
pub use comm::{CommunicationBus, KernelEventBus, EventBridge};
pub use comm::r#impl::debug_bus::{DebugBusHook, EventFilter, PerfCounter};
pub use comm::r#impl::event_log::{EventLog, LogEntry};
```

---

## Task 5: Update base/Cargo.toml dependencies

**Files:**
- Modify: `crates/base/Cargo.toml`

- [ ] **Step 1: Read current base/Cargo.toml**

```bash
cat crates/base/Cargo.toml
```

- [ ] **Step 2: Update dependencies**

Update `crates/base/Cargo.toml`:

```toml
[dependencies]
# Existing
async-trait = "0.1"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
anyhow = "1"
uuid = { version = "1", features = ["v4", "serde"] }
chrono = { version = "0.4", features = ["serde"] }
bincode = "1"
toml = "0.8"

# Upgraded
tokio = { version = "1", features = ["full"] }  # was ["time", "rt"]

# New (from comm)
tracing = "0.1"
parking_lot = "0.12"
nix = { workspace = true }
libc = { workspace = true }
dashmap = "6"
```

- [ ] **Step 3: Add io_uring feature flag (optional)**

If io_uring support is needed, add:
```toml
[features]
default = []
io_uring = []
```

---

## Task 6: Fix internal imports in moved comm files

**Files:**
- Modify: All files in `crates/base/src/comm/` that reference `base::` or `crate::`

- [ ] **Step 1: Find all base:: references in moved files**

```bash
grep -rn "use base::" crates/base/src/comm/
grep -rn "base::" crates/base/src/comm/
```

- [ ] **Step 2: Replace base:: with crate::**

Since these files are now inside the base crate, `base::` should become `crate::`:

```bash
find crates/base/src/comm/ -name "*.rs" -exec sed -i 's/use base::/use crate::/g' {} +
find crates/base/src/comm/ -name "*.rs" -exec sed -i 's/base::/crate::/g' {} +
```

- [ ] **Step 3: Verify no remaining base:: references**

```bash
grep -rn "use base::" crates/base/src/comm/
```

Expected: No output.

---

## Task 7: Update external crates that depend on comm

**Files:**
- Modify: `crates/cognit/Cargo.toml`
- Modify: `crates/dasein/Cargo.toml`
- Modify: `crates/runtime/Cargo.toml`

- [ ] **Step 1: Remove comm dependency from cognit**

In `crates/cognit/Cargo.toml`, remove:
```toml
comm = { path = "../comm" }
```

- [ ] **Step 2: Remove comm dependency from dasein**

In `crates/dasein/Cargo.toml`, remove:
```toml
comm = { path = "../comm" }
```

- [ ] **Step 3: Remove comm dependency from runtime**

In `crates/runtime/Cargo.toml`, remove:
```toml
comm = { path = "../comm" }
```

- [ ] **Step 4: Verify no remaining comm dependencies**

```bash
grep -r "comm = " crates/*/Cargo.toml crates/binaries/*/Cargo.toml
```

Expected: No output.

---

## Task 8: Update use statements in external crates

**Files:**
- Modify: `crates/cognit/src/impl/llm/pulse.rs`
- Modify: 13 files in `crates/runtime/src/`

- [ ] **Step 1: Update cognit imports**

In `crates/cognit/src/impl/llm/pulse.rs`, change:
```rust
use comm::core::event::ConcreteEvent;
```
to:
```rust
use base::ConcreteEvent;
```

- [ ] **Step 2: Update runtime imports**

For each file in runtime that uses `comm::`, replace:
- `use comm::CommunicationBus` → `use base::CommunicationBus`
- `use comm::envelope::Payload` → `use base::Payload`
- `use comm::KernelEventBus` → `use base::KernelEventBus`
- `use comm::ConcreteEvent` → `use base::ConcreteEvent`
- `use comm::core::event::ConcreteEvent` → `use base::ConcreteEvent`
- `use comm::r#impl::debug_bus::*` → `use base::debug_bus::*`

- [ ] **Step 3: Update test file imports**

In `crates/base/tests/protocol_e2e.rs`, change:
```rust
use comm::CommunicationBus;
```
to:
```rust
use base::CommunicationBus;
```

- [ ] **Step 4: Verify no remaining comm:: references**

```bash
grep -rn "use comm::" crates/ examples/ --include="*.rs"
```

Expected: No output.

---

## Task 9: Remove comm crate

**Files:**
- Delete: `crates/comm/` directory
- Modify: `Cargo.toml` (workspace root)

- [ ] **Step 1: Remove comm from workspace members**

In root `Cargo.toml`, remove from workspace members:
```toml
"crates/comm",
```

- [ ] **Step 2: Delete comm directory**

```bash
rm -rf crates/comm
```

- [ ] **Step 3: Verify workspace resolves**

```bash
cargo metadata --format-version 1 | head -5
```

Expected: JSON output without comm in workspace members.

---

## Task 10: Verify compilation and tests

- [ ] **Step 1: Run cargo check**

```bash
cargo check --workspace
```

Expected: All crates compile without errors.

- [ ] **Step 2: Run cargo test**

```bash
cargo test --workspace
```

Expected: All tests pass.

- [ ] **Step 3: Verify no remaining comm references**

```bash
grep -rn "comm" crates/*/Cargo.toml crates/binaries/*/Cargo.toml --include="*.toml"
grep -rn "use comm::" crates/ examples/ --include="*.rs"
```

Expected: No output (except possibly in comments/docs).

---

## Task 11: Commit changes

- [ ] **Step 1: Stage all changes**

```bash
git add -A
```

- [ ] **Step 2: Commit**

```bash
git commit -m "refactor: merge comm into base — complete foundation layer

- Move comm/src/impl/ and comm/src/bridge/ into base/src/comm/
- Merge ConcreteEvent and EventEnvelopeExt into base's event.rs and envelope.rs
- Update base/Cargo.toml with comm's dependencies
- Rewrite all use comm:: to use base:: in cognit, runtime
- Remove comm crate from workspace

base now contains: interfaces + communication + common modules"
```

---

## Self-Review Checklist

1. **Spec coverage:** This plan covers Phase 2 of the architectural redesign spec (§8.2)
2. **Placeholder scan:** No TBD/TODO — all steps are concrete
3. **Type consistency:** Import rewrites are consistent throughout
4. **Verification:** Each task has explicit verification steps
5. **Risk mitigation:** Incremental verification at each step

---

## Execution Options

Plan complete and saved to `docs/plans/2026-06-21-phase2-merge-comm-into-base.md`.

Execution options:
1. **workflow-feature** — Multi-agent pipeline with approval gates
2. **Inline execution** — Execute tasks in this session with checkpoints

Which approach?
