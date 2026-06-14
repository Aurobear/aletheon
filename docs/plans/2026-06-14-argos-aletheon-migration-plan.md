# Argos → Aletheon Migration Implementation Plan

> **For agentic workers:** Use `workflow-feature` or `writing-plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Migrate all unmigrated argos code into Aletheon's core/bridge/impl architecture, merge argos-types into aletheon-abi and argos-ipc into aletheon-comm, and write design docs for MetaRuntime/Coordinator.

**Architecture:** 9 aletheon crates following core/bridge/impl pattern. argos-types types merge into aletheon-abi. argos-ipc merges with aletheon-event-bus into aletheon-comm. argos-acix and argos-cli TUI components move into aletheon-body. argos-core/engine+config move into aletheon-runtime.

**Tech Stack:** Rust 2021, tokio, serde, async-trait, rusqlite, bincode, ratatui, crossterm, pulldown_cmark, syntect

---

## Phase 1: Foundation — aletheon-abi (merge argos-types)

### Task 1.1: Add argos-types modules to aletheon-abi

**Files:**
- Modify: `crates/aletheon-abi/Cargo.toml`
- Create: `crates/aletheon-abi/src/message.rs`
- Create: `crates/aletheon-abi/src/tool.rs`
- Create: `crates/aletheon-abi/src/sandbox.rs`
- Create: `crates/aletheon-abi/src/ipc_types.rs`
- Create: `crates/aletheon-abi/src/llm_types.rs`
- Modify: `crates/aletheon-abi/src/lib.rs`

- [ ] **Step 1: Update aletheon-abi Cargo.toml**

Add dependencies from argos-types/Cargo.toml:

```toml
[dependencies]
# ... existing deps ...
serde = { workspace = true }
serde_json = { workspace = true }
async-trait = { workspace = true }
anyhow = { workspace = true }
bincode = "1"
chrono = { workspace = true }
```

- [ ] **Step 2: Create aletheon-abi/src/message.rs**

Copy from `argos-types/src/message.rs`. Change module doc comment to reference aletheon-abi. Keep all types identical: `ContentBlock`, `ImageSource`, `Priority`, `Message`, `Role`. Keep all methods: `Message::user()`, `::assistant()`, `::system()`, `::tool_result()`, `::estimate_tokens()`, `ContentBlock::estimate_chars()`, `is_tool_message()`.

- [ ] **Step 3: Create aletheon-abi/src/tool.rs**

Copy from `argos-types/src/tool.rs`. Keep all types: `PermissionLevel`, `ToolContext`, `ToolResult`, `ToolResultMeta`, `ToolExposure`, `ConcurrencyClass`, `Tool` trait.

- [ ] **Step 4: Create aletheon-abi/src/sandbox.rs**

Copy from `argos-types/src/sandbox.rs`. Keep all types: `IsolationLevel`, `SandboxConfig`, `SandboxCapabilities`, `SandboxResult`, `SandboxBackend` trait.

- [ ] **Step 5: Create aletheon-abi/src/ipc_types.rs**

Copy from `argos-types/src/ipc.rs`. Keep all types: `AgentId`, `MessageType`, `IpcPriority`, `AgentMessage`, `IpcPreference`, `IpcProbeError`, `IpcBackend` trait.

- [ ] **Step 6: Create aletheon-abi/src/llm_types.rs**

Copy from `argos-types/src/llm.rs`. Keep: `ToolDefinition`.

- [ ] **Step 7: Update aletheon-abi/src/lib.rs**

Add module declarations and re-exports. Note: `PermissionLevel` already exists in `capability.rs` with different variants (ReadOnly/SandboxWrite/SystemChange/Destructive/SelfModify). The argos-types version (L0-L3) is renamed to `ToolPermissionLevel` to avoid conflict.

```rust
pub mod message;
pub mod tool;
pub mod sandbox;
pub mod ipc_types;
pub mod llm_types;

// Re-export key types from argos-types migration
pub use message::{Message, ContentBlock, Role, ImageSource, Priority as MessagePriority};
pub use tool::{Tool, ToolResult, ToolResultMeta, ToolContext, PermissionLevel as ToolPermissionLevel};
pub use llm_types::ToolDefinition;
pub use sandbox::{SandboxBackend, SandboxConfig, SandboxResult, SandboxCapabilities, IsolationLevel};
pub use ipc_types::{IpcBackend, IpcPreference, IpcProbeError, AgentMessage, AgentId, MessageType, IpcPriority};
```

- [ ] **Step 8: Verify compilation**

Run: `cargo build -p aletheon-abi`
Expected: PASS

- [ ] **Step 9: Commit**

```bash
git add crates/aletheon-abi/
git commit -m "feat(aletheon-abi): merge argos-types into aletheon-abi

- Add message.rs (Message, ContentBlock, Role, ImageSource, Priority)
- Add tool.rs (Tool trait, ToolResult, PermissionLevel, ToolExposure)
- Add sandbox.rs (SandboxBackend trait, SandboxConfig, SandboxResult)
- Add ipc_types.rs (IpcBackend trait, AgentMessage, AgentId)
- Add llm_types.rs (ToolDefinition)"
```

---

### Task 1.2: Add argos-core error types to aletheon-abi

**Files:**
- Create: `crates/aletheon-abi/src/error.rs`
- Modify: `crates/aletheon-abi/src/lib.rs`

- [ ] **Step 1: Create aletheon-abi/src/error.rs**

Read `argos-core/src/error.rs` and copy error types. If it's a simple `thiserror` or `anyhow` wrapper, keep it minimal.

- [ ] **Step 2: Update lib.rs**

Add `pub mod error;` and re-export.

- [ ] **Step 3: Verify**

Run: `cargo build -p aletheon-abi`

- [ ] **Step 4: Commit**

```bash
git add crates/aletheon-abi/src/error.rs crates/aletheon-abi/src/lib.rs
git commit -m "feat(aletheon-abi): add shared error types from argos-core"
```

---

## Phase 2: Foundation — aletheon-comm (merge EventBus + IPC)

### Task 2.1: Restructure aletheon-event-bus into aletheon-comm

**Files:**
- Rename: `crates/aletheon-event-bus/` → `crates/aletheon-comm/`
- Modify: `crates/aletheon-comm/Cargo.toml`
- Create: `crates/aletheon-comm/src/core/mod.rs`
- Create: `crates/aletheon-comm/src/core/event.rs`
- Create: `crates/aletheon-comm/src/core/bus.rs`
- Create: `crates/aletheon-comm/src/core/transport.rs`
- Create: `crates/aletheon-comm/src/bridge/mod.rs`
- Create: `crates/aletheon-comm/src/impl/mod.rs`
- Move: existing impl files into `crates/aletheon-comm/src/impl/`
- Modify: `Cargo.toml` (workspace members)

- [ ] **Step 1: Rename crate directory**

```bash
cd /home/aurobear/Bear-ws/work/argos
mv crates/aletheon-event-bus crates/aletheon-comm
```

- [ ] **Step 2: Update Cargo.toml**

In `crates/aletheon-comm/Cargo.toml`:
```toml
[package]
name = "aletheon-comm"
# ... rest stays the same

[dependencies]
aletheon-abi = { path = "../aletheon-abi" }
# ... existing deps ...
```

- [ ] **Step 3: Create core/bridge/impl structure**

Create directories:
```bash
mkdir -p crates/aletheon-comm/src/core
mkdir -p crates/aletheon-comm/src/bridge
mkdir -p crates/aletheon-comm/src/impl/ipc
```

- [ ] **Step 4: Move existing event-bus files into impl/**

Move existing files:
- `src/kernel_event_bus.rs` → `src/impl/kernel_bus.rs`
- `src/event_log.rs` → `src/impl/event_log.rs`
- `src/routing_policy.rs` → `src/impl/routing_policy.rs`
- `src/subscription.rs` → `src/impl/subscription.rs`

- [ ] **Step 5: Create core/event.rs**

Extract the `Event` trait definition (currently in aletheon-abi) into a local re-export/extension module. This module re-exports `aletheon_abi::Event` and adds any comm-specific event utilities.

- [ ] **Step 6: Create core/bus.rs**

Define the `EventBus` trait extension for the comm crate. Re-exports `aletheon_abi::EventBus`.

- [ ] **Step 7: Create core/transport.rs**

New trait abstracting IPC transport:

```rust
use async_trait::async_trait;
use anyhow::Result;
use aletheon_abi::{AgentMessage, AgentId, IpcProbeError};

/// Transport layer abstraction for inter-process communication.
#[async_trait]
pub trait Transport: Send + Sync {
    async fn init(&mut self) -> Result<(), IpcProbeError>;
    async fn send(&self, message: &AgentMessage) -> Result<(), IpcProbeError>;
    async fn recv(&self) -> Result<AgentMessage, IpcProbeError>;
    async fn try_recv(&self) -> Option<AgentMessage>;
    fn is_available(&self) -> bool;
    fn name(&self) -> &str;
}
```

- [ ] **Step 8: Create bridge/mod.rs**

Bridge module connecting impl to core traits.

- [ ] **Step 9: Create impl/mod.rs**

Declare all impl submodules:

```rust
pub mod kernel_bus;
pub mod event_log;
pub mod routing_policy;
pub mod subscription;
pub mod ipc;
```

- [ ] **Step 10: Update lib.rs**

Restructure to core/bridge/impl pattern. Note: `impl` is a Rust keyword, so use `r#impl` for the module name.

```rust
pub mod core;
pub mod bridge;
#[path = "impl"]
pub mod r#impl;

// Re-exports
pub use r#impl::kernel_bus::KernelEventBus;
pub use r#impl::event_log::{EventLog, LogEntry};
pub use r#impl::routing_policy::{RoutingPolicy, RouteAction};
pub use r#impl::subscription::SubscriptionRegistry;
pub use r#impl::ipc::IpcManager;
```

**Convention for all aletheon crates:** Use `r#impl` as the module name. In code, reference as `crate::r#impl::module::*`. In file paths, the directory is still `src/impl/`.

- [ ] **Step 11: Verify compilation**

Run: `cargo build -p aletheon-comm`

- [ ] **Step 12: Commit**

```bash
git add crates/aletheon-comm/
git commit -m "refactor(aletheon-comm): restructure event-bus into core/bridge/impl pattern

- Rename aletheon-event-bus → aletheon-comm
- Move impl files into impl/ subdirectory
- Add core/transport.rs trait for IPC abstraction
- Create bridge/ and core/ modules"
```

---

### Task 2.2: Migrate argos-ipc into aletheon-comm/impl/ipc/

**Files:**
- Create: `crates/aletheon-comm/src/impl/ipc/mod.rs`
- Create: `crates/aletheon-comm/src/impl/ipc/unix_socket.rs`
- Create: `crates/aletheon-comm/src/impl/ipc/io_uring.rs`
- Create: `crates/aletheon-comm/src/impl/ipc/shared_mem.rs`
- Create: `crates/aletheon-comm/src/impl/ipc/json_rpc.rs`
- Create: `crates/aletheon-comm/src/impl/ipc/priority_queue.rs`
- Create: `crates/aletheon-comm/src/impl/ipc/manager.rs`
- Modify: `crates/aletheon-comm/Cargo.toml`

- [ ] **Step 1: Update Cargo.toml with IPC dependencies**

Add to `crates/aletheon-comm/Cargo.toml`:
```toml
nix = { workspace = true }
bincode = "1"
```

- [ ] **Step 2: Create impl/ipc/mod.rs**

```rust
pub mod unix_socket;
pub mod io_uring;
pub mod shared_mem;
pub mod json_rpc;
pub mod priority_queue;
pub mod manager;

pub use manager::{IpcManager, IpcBackendKind, Environment};
pub use priority_queue::PriorityQueue;
pub use json_rpc::JsonRpcAdapter;
```

- [ ] **Step 3: Create impl/ipc/unix_socket.rs**

Copy from `argos-ipc/src/unix_socket.rs` (316 lines). Change imports:
- `use crate::message::*` → `use aletheon_abi::{AgentMessage, AgentId, IpcProbeError};`
- `use crate::backend::*` → removed (types now in aletheon-abi)
- Keep `UnixSocketBackend` struct and all methods

- [ ] **Step 4: Create impl/ipc/io_uring.rs**

Copy from `argos-ipc/src/io_uring_backend.rs` (272 lines). Change imports to use aletheon-abi types.

- [ ] **Step 5: Create impl/ipc/shared_mem.rs**

Copy from `argos-ipc/src/shared_mem.rs` (243 lines). Change imports to use aletheon-abi types. Note: uses `AgentMessage::to_bytes()`/`from_bytes()` which are defined in argos-types/ipc.rs — these are now in aletheon-abi/src/ipc_types.rs.

- [ ] **Step 6: Create impl/ipc/json_rpc.rs**

Copy from `argos-ipc/src/json_rpc_adapter.rs` (92 lines). No type dependency changes needed (uses serde_json::Value, not argos types).

- [ ] **Step 7: Create impl/ipc/priority_queue.rs**

Copy from `argos-ipc/src/priority_queue.rs` (178 lines). Change imports to use `aletheon_abi::AgentMessage`.

- [ ] **Step 8: Create impl/ipc/manager.rs**

Copy from `argos-ipc/src/manager.rs` (315 lines). Change imports to use aletheon-abi types and local ipc module types.

- [ ] **Step 9: Verify compilation**

Run: `cargo build -p aletheon-comm`

- [ ] **Step 10: Commit**

```bash
git add crates/aletheon-comm/
git commit -m "feat(aletheon-comm): migrate argos-ipc into impl/ipc/

- Add unix_socket.rs, io_uring.rs, shared_mem.rs backends
- Add json_rpc.rs adapter
- Add priority_queue.rs data structure
- Add manager.rs with auto-detect and fallback"
```

---

## Phase 3: Core Modules — aletheon-brain-core, aletheon-body

### Task 3.1: Add grounding_provider to aletheon-brain-core

**Files:**
- Create: `crates/aletheon-brain-core/src/impl/grounding/mod.rs`
- Create: `crates/aletheon-brain-core/src/impl/grounding/vision.rs`
- Modify: `crates/aletheon-brain-core/src/impl/mod.rs`

- [ ] **Step 1: Create impl/grounding/mod.rs**

```rust
pub mod vision;

pub use vision::VisionGroundingProvider;
```

- [ ] **Step 2: Create impl/grounding/vision.rs**

Copy from `argos-core/src/grounding_provider.rs`. Change imports:
- `use argos_types::message::*` → `use aletheon_abi::{Message, ContentBlock, Role, ImageSource};`
- `use argos_driver::types::Image` → keep as-is (aletheon-body re-exports driver types)
- `use crate::llm::provider::LlmProvider` → `use crate::r#impl::llm::provider::LlmProvider;`

Keep `VisionGroundingProvider` struct and `GroundingProvider` trait implementation.

- [ ] **Step 3: Update impl/mod.rs**

Add `pub mod grounding;`

- [ ] **Step 4: Verify**

Run: `cargo build -p aletheon-brain-core`

- [ ] **Step 5: Commit**

```bash
git add crates/aletheon-brain-core/
git commit -m "feat(aletheon-brain-core): add vision grounding provider from argos-core"
```

---

### Task 3.2: Merge provider_registry into aletheon-brain-core

**Files:**
- Modify: `crates/aletheon-brain-core/src/impl/provider_registry.rs`

- [ ] **Step 1: Read both files and merge**

Read `argos-core/src/provider_registry.rs` and `aletheon-brain-core/src/impl/provider_registry.rs`. Merge any missing functionality from argos-core into aletheon-brain-core's version. The aletheon version likely already has the core; check for:
- Any missing provider registration methods
- Any missing provider lookup logic
- Config integration

- [ ] **Step 2: Verify**

Run: `cargo build -p aletheon-brain-core`

- [ ] **Step 3: Commit**

```bash
git add crates/aletheon-brain-core/
git commit -m "feat(aletheon-brain-core): merge provider_registry from argos-core"
```

---

### Task 3.3: Add acix to aletheon-body

**Files:**
- Create: `crates/aletheon-body/src/impl/acix/mod.rs`
- Create: `crates/aletheon-body/src/impl/acix/aci.rs`
- Create: `crates/aletheon-body/src/impl/acix/experience.rs`
- Create: `crates/aletheon-body/src/impl/acix/grounding.rs`
- Create: `crates/aletheon-body/src/impl/acix/task.rs`
- Modify: `crates/aletheon-body/src/impl/mod.rs`

- [ ] **Step 1: Create impl/acix/mod.rs**

```rust
pub mod aci;
pub mod experience;
pub mod grounding;
pub mod task;

pub use aci::Aci;
pub use experience::{ActionRecord, Embedder, Experience, ExperienceLevel, ExperienceMemory, MockEmbedder};
pub use grounding::{GroundingProvider, GroundingResult, MockGroundingProvider};
pub use task::{TaskAction, TaskDecomposer, TaskGraph, TaskManager, TaskNode, TaskStatus, TaskWorker};
```

- [ ] **Step 2: Create impl/acix/aci.rs**

Copy from `argos-acix/src/aci.rs` (307 lines). Change imports:
- `use argos_driver::*` → `use crate::r#impl::driver::*` (driver is already in aletheon-body)
- `use crate::grounding` → `use super::grounding`

- [ ] **Step 3: Create impl/acix/experience.rs**

Copy from `argos-acix/src/experience.rs` (340 lines). Change imports:
- `use argos_types::{ContentBlock, Message, Role}` → `use aletheon_abi::{ContentBlock, Message, Role}`

- [ ] **Step 4: Create impl/acix/grounding.rs**

Copy from `argos-acix/src/grounding.rs` (112 lines). Change imports:
- `use argos_driver::types::{Bounds, Image}` → `use crate::r#impl::driver::types::{Bounds, Image}`

- [ ] **Step 5: Create impl/acix/task.rs**

Copy from `argos-acix/src/task.rs` (745 lines). Change imports:
- `use argos_driver::types::{Key, ScrollDirection}` → `use crate::r#impl::driver::types::{Key, ScrollDirection}`
- `use crate::aci::Aci` → `use super::aci::Aci`

- [ ] **Step 6: Update impl/mod.rs**

Add `pub mod acix;`

- [ ] **Step 7: Verify**

Run: `cargo build -p aletheon-body`

- [ ] **Step 8: Commit**

```bash
git add crates/aletheon-body/
git commit -m "feat(aletheon-body): migrate argos-acix into impl/acix/

- Add aci.rs (Agent Computer Interface)
- Add experience.rs (Experience Replay + Embedder)
- Add grounding.rs (GroundingProvider trait)
- Add task.rs (TaskGraph, TaskManager, TaskWorker)"
```

---

### Task 3.4: Add platform adapters to aletheon-body

**Files:**
- Create: `crates/aletheon-body/src/impl/platform/mod.rs`
- Create: `crates/aletheon-body/src/impl/platform/adapter.rs`
- Create: `crates/aletheon-body/src/impl/platform/linux.rs`
- Create: `crates/aletheon-body/src/impl/platform/android.rs`
- Create: `crates/aletheon-body/src/impl/platform/boot.rs`
- Create: `crates/aletheon-body/src/impl/platform/awareness/mod.rs`
- Create: `crates/aletheon-body/src/impl/platform/awareness/communication.rs`
- Create: `crates/aletheon-body/src/impl/platform/awareness/conflict.rs`
- Create: `crates/aletheon-body/src/impl/platform/awareness/discovery.rs`
- Create: `crates/aletheon-body/src/impl/platform/awareness/lifecycle.rs`
- Modify: `crates/aletheon-body/src/impl/mod.rs`

- [ ] **Step 1: Create all platform files**

Copy from `argos-core/src/platform/` directory. Each file:
- `mod.rs` — module declarations
- `adapter.rs` — platform adapter trait
- `linux.rs` — Linux-specific (eBPF, systemd, /proc)
- `android.rs` — Android-specific (Binder, Accessibility)
- `boot.rs` — boot sequence
- `awareness/mod.rs` — awareness module
- `awareness/communication.rs` — inter-agent communication
- `awareness/conflict.rs` — conflict resolution
- `awareness/discovery.rs` — service discovery
- `awareness/lifecycle.rs` — lifecycle management

Change imports from `crate::*` to appropriate aletheon paths.

- [ ] **Step 2: Update impl/mod.rs**

Add `pub mod platform;`

- [ ] **Step 3: Verify**

Run: `cargo build -p aletheon-body`

- [ ] **Step 4: Commit**

```bash
git add crates/aletheon-body/
git commit -m "feat(aletheon-body): add platform adapters from argos-core

- Linux adapter (eBPF, systemd, /proc)
- Android adapter (Binder, Accessibility)
- Boot sequence
- Awareness subsystem (discovery, communication, conflict, lifecycle)"
```

---

### Task 3.5: Add acix_tools to aletheon-body

**Files:**
- Modify: `crates/aletheon-body/src/impl/tools/mod.rs`
- Create: `crates/aletheon-body/src/impl/tools/acix_tools.rs`

- [ ] **Step 1: Create impl/tools/acix_tools.rs**

Copy from `argos-core/src/acix_tools.rs`. Change imports to use aletheon-body's acix and tool modules.

- [ ] **Step 2: Update tools/mod.rs**

Add `pub mod acix_tools;`

- [ ] **Step 3: Verify**

Run: `cargo build -p aletheon-body`

- [ ] **Step 4: Commit**

```bash
git add crates/aletheon-body/
git commit -m "feat(aletheon-body): add acix_tools from argos-core"
```

---

### Task 3.6: Add UI components to aletheon-body

**Files:**
- Create: `crates/aletheon-body/src/impl/ui/mod.rs`
- Create: `crates/aletheon-body/src/impl/ui/chat.rs`
- Create: `crates/aletheon-body/src/impl/ui/command.rs`
- Create: `crates/aletheon-body/src/impl/ui/computer.rs`
- Create: `crates/aletheon-body/src/impl/ui/event.rs`
- Create: `crates/aletheon-body/src/impl/ui/input.rs`
- Create: `crates/aletheon-body/src/impl/ui/markdown.rs`
- Create: `crates/aletheon-body/src/impl/ui/skill.rs`
- Create: `crates/aletheon-body/src/impl/ui/status.rs`
- Create: `crates/aletheon-body/src/impl/ui/term_compat.rs`
- Modify: `crates/aletheon-body/Cargo.toml`
- Modify: `crates/aletheon-body/src/impl/mod.rs`

- [ ] **Step 1: Update Cargo.toml**

Add UI dependencies:
```toml
ratatui = "0.28"
crossterm = "0.28"
pulldown-cmark = "0.10"
syntect = "5"
```

- [ ] **Step 2: Create all UI files**

Copy from `argos-cli/src/tui/` directory. Each file:
- `mod.rs` — 647 lines, `App` struct and event loop
- `chat.rs` — 655 lines, `ChatWidget`, word-wrapping, CJK support
- `command.rs` — 98 lines, command parsing
- `computer.rs` — 188 lines, computer commands (screenshot, click, type)
- `event.rs` — 58 lines, `TuiEvent`, `Action` enums
- `input.rs` — 379 lines, `InputArea`, cursor, history
- `markdown.rs` — 422 lines, markdown rendering with syntect
- `skill.rs` — 175 lines, `SkillLoader`
- `status.rs` — 119 lines, `StatusBar`
- `term_compat.rs` — 303 lines, `Theme`, `TermCaps`

Change imports:
- `use argos_acix::*` → `use crate::r#impl::acix::*`
- `use argos_driver::*` → `use crate::r#impl::driver::*`

- [ ] **Step 3: Update impl/mod.rs**

Add `pub mod ui;`

- [ ] **Step 4: Verify**

Run: `cargo build -p aletheon-body`

- [ ] **Step 5: Commit**

```bash
git add crates/aletheon-body/
git commit -m "feat(aletheon-body): add TUI components from argos-cli

- chat.rs (ChatWidget with CJK word-wrapping)
- command.rs (command parsing)
- computer.rs (ACIX computer commands)
- event.rs (TuiEvent/Action)
- input.rs (InputArea with history)
- markdown.rs (markdown renderer with syntect)
- skill.rs (SkillLoader)
- status.rs (StatusBar)
- term_compat.rs (Theme/TermCaps)"
```

---

## Phase 4: Runtime — aletheon-runtime

### Task 4.1: Add engine to aletheon-runtime

**Files:**
- Create: `crates/aletheon-runtime/src/impl/engine/mod.rs`
- Create: `crates/aletheon-runtime/src/impl/engine/config.rs`
- Create: `crates/aletheon-runtime/src/impl/engine/cognitive_loop.rs`
- Create: `crates/aletheon-runtime/src/impl/engine/tool_dispatch.rs`
- Create: `crates/aletheon-runtime/src/impl/engine/memory_integration.rs`
- Create: `crates/aletheon-runtime/src/impl/engine/streaming.rs`
- Modify: `crates/aletheon-runtime/src/impl/mod.rs`

- [ ] **Step 1: Split engine.rs into submodules**

The original `argos-core/src/engine.rs` is 1369 lines. Split into:
- `config.rs` — `EngineConfig` struct and `Default` impl
- `cognitive_loop.rs` — main `Engine` struct, `run()` method, ReAct loop
- `tool_dispatch.rs` — tool selection, execution, result handling
- `memory_integration.rs` — memory read/write, compaction, recall
- `streaming.rs` — LLM streaming, chunk handling

- [ ] **Step 2: Create each submodule**

Copy relevant sections from `argos-core/src/engine.rs`. Change imports:
- `use crate::learning::*` → `use aletheon_brain_core::impl::learning::*`
- `use crate::llm::*` → `use aletheon_brain_core::impl::llm::*`
- `use crate::memory::*` → `use aletheon_memory::*`
- `use crate::message::*` → `use aletheon_abi::{Message, ContentBlock, Role}`
- `use crate::orchestration::*` → `use crate::r#impl::orchestration::*`
- `use crate::perception::*` → `use aletheon_self_field::r#impl::perception::*`
- `use crate::hook::*` → `use aletheon_self_field::r#impl::hook::*`
- `use crate::security::*` → `use aletheon_self_field::r#impl::security::*`
- `use crate::session::*` → `use crate::r#impl::session::*`
- `use crate::tool::*` → `use aletheon_body::r#impl::tools::*`

- [ ] **Step 3: Create impl/engine/mod.rs**

```rust
pub mod config;
pub mod cognitive_loop;
pub mod tool_dispatch;
pub mod memory_integration;
pub mod streaming;

pub use config::EngineConfig;
pub use cognitive_loop::Engine;
```

- [ ] **Step 4: Update impl/mod.rs**

Add `pub mod engine;`

- [ ] **Step 5: Verify**

Run: `cargo build -p aletheon-runtime`

- [ ] **Step 6: Commit**

```bash
git add crates/aletheon-runtime/
git commit -m "feat(aletheon-runtime): add cognitive engine from argos-core

- config.rs (EngineConfig)
- cognitive_loop.rs (ReAct loop)
- tool_dispatch.rs (tool selection and execution)
- memory_integration.rs (memory read/write/compaction)
- streaming.rs (LLM streaming)"
```

---

### Task 4.2: Add config to aletheon-runtime

**Files:**
- Modify: `crates/aletheon-runtime/src/core/config.rs`

- [ ] **Step 1: Merge config files**

Read `argos-core/src/config.rs` and `aletheon-runtime/src/core/config.rs`. Merge `AppConfig`, `AgentConfig`, `ProviderConfig` from argos-core into aletheon-runtime's config module.

- [ ] **Step 2: Verify**

Run: `cargo build -p aletheon-runtime`

- [ ] **Step 3: Commit**

```bash
git add crates/aletheon-runtime/
git commit -m "feat(aletheon-runtime): merge AppConfig from argos-core"
```

---

## Phase 5: Verify and Add Testing Modules

### Task 5.1: Distribute testing mocks

**Files:**
- Create: `crates/aletheon-brain-core/src/testing/mock_llm.rs`
- Create: `crates/aletheon-memory/src/testing/mock_memory.rs`
- Create: `crates/aletheon-self-field/src/testing/mock_perception.rs`
- Create: `crates/aletheon-body/src/testing/mock_sandbox.rs`

- [ ] **Step 1: Copy mock files to respective crates**

From `argos-core/src/testing/`:
- `mock_llm.rs` → `aletheon-brain-core/src/testing/mock_llm.rs`
- `mock_memory.rs` → `aletheon-memory/src/testing/mock_memory.rs`
- `mock_perception.rs` → `aletheon-self-field/src/testing/mock_perception.rs`
- `mock_sandbox.rs` → `aletheon-body/src/testing/mock_sandbox.rs`

Change imports to use each crate's own types.

- [ ] **Step 2: Create testing/mod.rs in each crate**

Each crate's `testing/mod.rs` declares the mock modules and is gated behind `#[cfg(test)]`.

- [ ] **Step 3: Verify**

Run: `cargo test --workspace`

- [ ] **Step 4: Commit**

```bash
git add crates/aletheon-brain-core/src/testing/ crates/aletheon-memory/src/testing/ crates/aletheon-self-field/src/testing/ crates/aletheon-body/src/testing/
git commit -m "feat: distribute testing mocks to respective aletheon crates"
```

---

### Task 5.2: Verify already-migrated modules

**Files:** Read-only verification

- [ ] **Step 1: Verify argos-driver → aletheon-body/driver/**

Check these files exist in aletheon-body:
- `src/impl/driver/proc/mod.rs` — process management
- `src/impl/driver/io/mod.rs` — I/O operations

If missing, copy from `argos-driver/src/proc/` and `argos-driver/src/io/`.

- [ ] **Step 2: Verify argos-perception → aletheon-self-field/perception/**

Check `bridge.rs` exists. If missing, copy from `argos-perception/src/bridge.rs`.

- [ ] **Step 3: Verify argos-sandbox → aletheon-body/sandbox/**

Check `backend.rs` exists. If missing, copy from `argos-sandbox/src/backend.rs`.

- [ ] **Step 4: Verify argos-tools → aletheon-body/tools/**

Check `exposure.rs` exists. If missing, copy from `argos-tools/src/exposure.rs`.

- [ ] **Step 5: Verify argos-security → aletheon-self-field/security/**

Check these files exist in aletheon-self-field:
- `src/impl/security/policy.rs`
- `src/impl/security/circuit_breaker.rs`
- `src/impl/security/loop_detector.rs`
- `src/impl/security/risk_classifier.rs`
- `src/impl/security/audit.rs`
- `src/impl/security/runner.rs`
- `src/impl/security/output_guardrail.rs`
- `src/impl/security/rollback/mod.rs`
- `src/impl/security/rollback/types.rs`
- `src/impl/security/sandbox/mod.rs`
- `src/impl/security/sandbox/writable_root.rs`
- `src/impl/security/self_protection/emergency_killswitch.rs`
- `src/impl/security/self_protection/input_sanitizer.rs`
- `src/impl/security/self_protection/integrity_monitor.rs`
- `src/impl/security/self_protection/mod.rs`
- `src/impl/security/self_protection/resource_governor.rs`
- `src/impl/security/rate_limiting/backpressure.rs`
- `src/impl/security/rate_limiting/flood_protector.rs`
- `src/impl/security/rate_limiting/mod.rs`
- `src/impl/security/rate_limiting/token_limiter.rs`
- `src/impl/security/rate_limiting/tool_limiter.rs`

For any missing file, copy from `argos-security/src/` and adapt imports.

- [ ] **Step 6: Fix any missing files**

For each missing file found in steps 1-5, copy and adapt the import paths.

- [ ] **Step 6: Commit if changes made**

```bash
git add crates/aletheon-body/ crates/aletheon-self-field/
git commit -m "fix: complete previously migrated modules (driver/proc, driver/io, perception/bridge, sandbox/backend, tools/exposure)"
```

---

## Phase 6: Design Documents

### Task 6.1: Write aletheon-meta design document

**Files:**
- Create: `crates/aletheon-meta/README.md`
- Create: `crates/aletheon-meta/src/lib.rs` (stub)
- Create: `crates/aletheon-meta/src/core/traits.rs` (trait definitions)
- Create: `crates/aletheon-meta/src/core/types.rs` (type definitions)

- [ ] **Step 1: Create crate skeleton**

```bash
mkdir -p crates/aletheon-meta/src/core
mkdir -p crates/aletheon-meta/src/bridge
mkdir -p crates/aletheon-meta/src/impl/genome
mkdir -p crates/aletheon-meta/src/impl/meta_runtime
mkdir -p crates/aletheon-meta/src/impl/morphogenesis
```

- [ ] **Step 2: Create Cargo.toml**

```toml
[package]
name = "aletheon-meta"
version.workspace = true
edition.workspace = true

[dependencies]
aletheon-abi = { path = "../aletheon-abi" }
serde = { workspace = true }
serde_yaml = "0.9"
anyhow = { workspace = true }
async-trait = { workspace = true }
tracing = { workspace = true }
```

- [ ] **Step 3: Create core/traits.rs**

Define trait stubs implementing `aletheon_abi::MetaRuntimeOps`. Method names/signatures MUST match the existing ABI trait in `aletheon-abi/src/meta.rs`:

- `read_genome()` → `Result<Genome>`
- `generate_candidate(&MutationIntent)` → `Result<RuntimeCandidate>`
- `sandbox_test(&RuntimeCandidate)` → `Result<TestResult>`
- `evaluate(&RuntimeCandidate, &TestResult)` → `Result<Evaluation>`
- `migrate(&RuntimeCandidate)` → `Result<MigrationResult>`
- `rollback()` → `Result<()>`
- `current_version()` → `Version`

Note: All methods are `todo!()` stubs — this is intentional for the design skeleton. Implementation comes in a future round.

```rust
use async_trait::async_trait;
use anyhow::Result;
use aletheon_abi::{
    MetaRuntimeOps, RuntimeCandidate, TestResult, Evaluation, MigrationResult,
    Genome, MutationIntent, Subsystem, SubsystemHealth, SubsystemContext, Version,
};

/// Concrete MetaRuntime implementation (design skeleton).
pub struct DefaultMetaRuntime {
    version: Version,
}

impl DefaultMetaRuntime {
    pub fn new(version: Version) -> Self {
        Self { version }
    }
}

// Subsystem trait required by MetaRuntimeOps
#[async_trait]
impl Subsystem for DefaultMetaRuntime {
    fn name(&self) -> &str { "meta-runtime" }
    fn version(&self) -> &Version { &self.version }
    async fn init(&mut self, _ctx: &SubsystemContext) -> Result<()> { Ok(()) }
    async fn shutdown(&mut self) -> Result<()> { Ok(()) }
    async fn health(&self) -> Result<SubsystemHealth> {
        Ok(SubsystemHealth {
            healthy: true,
            message: "design skeleton".to_string(),
            checks: vec![],
        })
    }
}

#[async_trait]
impl MetaRuntimeOps for DefaultMetaRuntime {
    /// Read the current genome.
    async fn read_genome(&self) -> Result<Genome> { todo!("MetaRuntime: read_genome not yet implemented") }

    /// Generate a candidate runtime from a mutation intent.
    async fn generate_candidate(&self, _intent: &MutationIntent) -> Result<RuntimeCandidate> { todo!("MetaRuntime: generate_candidate not yet implemented") }

    /// Test a candidate in sandbox.
    async fn sandbox_test(&self, _candidate: &RuntimeCandidate) -> Result<TestResult> { todo!("MetaRuntime: sandbox_test not yet implemented") }

    /// Evaluate a candidate after testing.
    async fn evaluate(&self, _candidate: &RuntimeCandidate, _test: &TestResult) -> Result<Evaluation> { todo!("MetaRuntime: evaluate not yet implemented") }

    /// Migrate to a new runtime.
    async fn migrate(&self, _candidate: &RuntimeCandidate) -> Result<MigrationResult> { todo!("MetaRuntime: migrate not yet implemented") }

    /// Rollback to the previous runtime version.
    async fn rollback(&self) -> Result<()> { todo!("MetaRuntime: rollback not yet implemented") }

    /// Get the current runtime version.
    fn current_version(&self) -> Version { self.version.clone() }
}
```

- [ ] **Step 4: Create core/types.rs**

Define extended Genome types. The ABI's `aletheon-abi/src/genome.rs` already defines the base `Genome` struct with `Topology`, `IdentitySpec`, `BoundarySpec`, `CareSpec`, `MemorySpec`, `MutationSpec`, `LifecycleSpec`. This file adds the **evaluator** and **morphogenesis-specific** types that the ABI doesn't have yet.

Note: The design spec's simpler YAML format (flat `refuse: Vec<String>`) maps to the ABI's more structured `BoundarySpec { rules: Vec<BoundaryRuleSpec> }`. The YAML loader in `impl/genome/` will handle this conversion.

```rust
use serde::{Deserialize, Serialize};

// Re-export ABI Genome types for convenience
pub use aletheon_abi::genome::{
    Genome, Topology, SubsystemSpec, SubsystemType,
    IdentitySpec, BoundarySpec, BoundaryRuleSpec,
    CareSpec, CarePriority, MemorySpec, MutationSpec, LifecycleSpec,
};

/// Evaluator specification — not in ABI yet, defined here.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvaluatorSpec {
    pub metrics: Vec<EvaluatorMetric>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvaluatorMetric {
    pub name: String,
    pub weight: f64,
}

/// Morphogenesis candidate — a proposed change to the genome.
#[derive(Debug, Clone)]
pub struct MorphogenesisCandidate {
    pub id: String,
    pub description: String,
    pub genome_patch: GenomePatch,
    pub reason: String,
}

/// A patch to apply to the genome.
#[derive(Debug, Clone)]
pub struct GenomePatch {
    pub target: String,       // e.g., "boundary.rules", "care.priorities"
    pub operation: PatchOperation,
    pub value: serde_json::Value,
}

#[derive(Debug, Clone)]
pub enum PatchOperation {
    Add,
    Remove,
    Replace,
    Modify,
}
```

- [ ] **Step 5: Create impl/morphogenesis/ pipeline stubs**

Create stub files for the Morphogenesis pipeline (spec Section 4.2):

`src/impl/morphogenesis/mod.rs`:
```rust
pub mod pipeline;
pub mod mutation_intent;
pub mod candidate;
```

`src/impl/morphogenesis/pipeline.rs`:
```rust
//! Morphogenesis Pipeline — the self-evolution flow.
//!
//! Pipeline: run → reflect → mutate spec → generate candidate → evaluate → migrate → become
//!
//! This is a design skeleton. Implementation comes in a future round.

use anyhow::Result;
use aletheon_abi::{MetaRuntimeOps, RuntimeCandidate, TestResult, Evaluation, MigrationResult};

/// Orchestrates the full morphogenesis pipeline.
pub struct MorphogenesisPipeline<M: MetaRuntimeOps> {
    meta_runtime: M,
}

impl<M: MetaRuntimeOps> MorphogenesisPipeline<M> {
    pub fn new(meta_runtime: M) -> Self {
        Self { meta_runtime }
    }

    /// Run the full pipeline: reflect → mutate → generate → test → evaluate → migrate.
    pub async fn run(&self) -> Result<PipelineResult> {
        todo!("Morphogenesis pipeline not yet implemented")
    }
}

#[derive(Debug)]
pub struct PipelineResult {
    pub success: bool,
    pub candidate: Option<RuntimeCandidate>,
    pub evaluation: Option<Evaluation>,
    pub migration: Option<MigrationResult>,
    pub message: String,
}
```

`src/impl/morphogenesis/mutation_intent.rs`:
```rust
//! Mutation intent generation — how the agent decides what to change.
//!
//! Design skeleton. Implementation comes in a future round.

use aletheon_abi::MutationIntent;

/// Generate mutation intents from reflection and experience.
pub struct MutationIntentGenerator;

impl MutationIntentGenerator {
    pub fn new() -> Self { Self }

    /// Generate mutation intents based on recent experience and reflection.
    pub async fn generate(&self, _context: &str) -> Vec<MutationIntent> {
        todo!("Mutation intent generation not yet implemented")
    }
}
```

`src/impl/morphogenesis/candidate.rs`:
```rust
//! Candidate runtime generation — building the next version of the agent.
//!
//! Design skeleton. Implementation comes in a future round.

use anyhow::Result;
use aletheon_abi::{Genome, RuntimeCandidate, MutationIntent};

/// Generates candidate runtimes from genome mutations.
pub struct CandidateGenerator;

impl CandidateGenerator {
    pub fn new() -> Self { Self }

    /// Generate a candidate runtime from a genome and mutation intent.
    pub async fn generate(&self, _genome: &Genome, _intent: &MutationIntent) -> Result<RuntimeCandidate> {
        todo!("Candidate generation not yet implemented")
    }
}
```

- [ ] **Step 6: Create impl/genome/ loader stubs**

Create stub files for Genome YAML loading:

`src/impl/genome/mod.rs`:
```rust
pub mod loader;

pub use loader::GenomeLoader;
```

`src/impl/genome/loader.rs`:
```rust
//! Genome YAML loader — reads genome files and produces a Genome struct.
//!
//! Design skeleton. Implementation comes in a future round.

use anyhow::Result;
use std::path::Path;
use crate::core::types::Genome;

/// Loads a genome from a directory of YAML files.
pub struct GenomeLoader;

impl GenomeLoader {
    pub fn new() -> Self { Self }

    /// Load genome from a directory containing topology.yaml, identity.yaml, etc.
    pub async fn load(&self, _dir: &Path) -> Result<Genome> {
        todo!("Genome YAML loading not yet implemented")
    }

    /// Save genome to a directory of YAML files.
    pub async fn save(&self, _genome: &Genome, _dir: &Path) -> Result<()> {
        todo!("Genome YAML saving not yet implemented")
    }
}
```

- [ ] **Step 7: Create impl/meta_runtime/ stubs**

Create stub files for the MetaRuntime components:

`src/impl/meta_runtime/mod.rs`:
```rust
pub mod self_reader;
pub mod spec_editor;
pub mod runtime_builder;
pub mod sandbox_runner;
pub mod evaluator;
pub mod rollback;
pub mod migration;
pub mod lineage;
```

Each file (self_reader.rs, spec_editor.rs, etc.) should contain a stub struct with `todo!()` methods. These are design placeholders for the future implementation round.

- [ ] **Step 8: Create lib.rs**

```rust
pub mod core;
pub mod bridge;
pub mod r#impl;

pub use core::traits::DefaultMetaRuntime;
pub use core::types::*;
pub use r#impl::morphogenesis::pipeline::MorphogenesisPipeline;
pub use r#impl::genome::loader::GenomeLoader;
```

- [ ] **Step 6: Write README.md**

Document the MetaRuntime/Morphogenesis/Genome design from the design spec. Include:
- Architecture overview
- Genome YAML format specification
- Morphogenesis pipeline description
- Key constraints and invariants

- [ ] **Step 7: Add to workspace**

In root `Cargo.toml`, add `"crates/aletheon-meta"` to workspace members.

- [ ] **Step 8: Verify**

Run: `cargo build -p aletheon-meta`

- [ ] **Step 9: Commit**

```bash
git add crates/aletheon-meta/
git commit -m "feat(aletheon-meta): create MetaRuntime/Morphogenesis/Genome design skeleton

- Core trait stubs implementing MetaRuntimeOps
- Genome YAML type definitions (topology, identity, boundary, care, memory, mutation, evaluator)
- README with full design documentation"
```

---

### Task 6.2: Add Coordinator design to aletheon-runtime

**Files:**
- Create: `crates/aletheon-runtime/src/impl/coordinator.rs`
- Modify: `crates/aletheon-runtime/src/impl/mod.rs`

- [ ] **Step 1: Create impl/coordinator.rs**

Uses types from `aletheon_abi::self_field` (Verdict, RiskLevel) and `aletheon_abi::brain` (Plan). Does NOT define local duplicates.

```rust
//! Coordinator — temporary arbitrator for high-risk decisions.
//!
//! The Coordinator is NOT the supreme authority. It only integrates
//! results from SelfField, BrainCore, BodyRuntime, and Memory
//! to produce a final verdict for a specific event.
//!
//! Design note: arbitrate() is async to match the spec's contract,
//! even though the current implementation has no async operations.

use std::time::Duration;
use aletheon_abi::self_field::{Verdict, RiskLevel};
use aletheon_abi::brain::Plan;

/// Result of coordination arbitration.
#[derive(Debug, Clone)]
pub enum ArbitrationResult {
    /// Execute the given plan.
    Execute(Plan),
    /// Deny with reason (matches Verdict::Deny semantics).
    Deny(String),
    /// Delay execution by the given duration.
    Delay(Duration),
    /// Test in sandbox first, then execute if passed.
    SandboxFirst(Plan),
    /// Ask user for confirmation.
    AskConfirmation(String),
    /// Trigger reflection cycle.
    Reflect,
    /// Trigger mutation cycle.
    Mutate,
}

/// Memory context for decision making.
#[derive(Debug, Clone)]
pub struct MemoryContext {
    pub relevant_experiences: Vec<String>,
    pub past_failures: Vec<String>,
    pub learned_rules: Vec<String>,
}

/// The Coordinator arbitrates between subsystems.
pub struct Coordinator;

impl Coordinator {
    pub fn new() -> Self {
        Self
    }

    /// Integrate results from all subsystems and produce a verdict.
    ///
    /// Decision logic:
    /// 1. If SelfField denies → Deny
    /// 2. If risk is Critical → SandboxFirst or Deny
    /// 3. If risk is High and memory shows past failures → AskConfirmation
    /// 4. If risk is High → SandboxFirst
    /// 5. If BrainCore has a plan → Execute
    /// 6. Otherwise → Reflect
    pub async fn arbitrate(
        &self,
        self_field_verdict: &Verdict,
        brain_plan: Option<&Plan>,
        risk_level: RiskLevel,
        memory_context: &MemoryContext,
    ) -> ArbitrationResult {
        // 1. SelfField denial takes priority
        if matches!(self_field_verdict, Verdict::Deny { .. }) {
            return ArbitrationResult::Deny(
                "SelfField denied the action".to_string()
            );
        }

        // 2. Critical risk → sandbox or deny
        if risk_level == RiskLevel::Critical {
            if let Some(plan) = brain_plan {
                return ArbitrationResult::SandboxFirst(plan.clone());
            }
            return ArbitrationResult::Deny(
                "Critical risk with no plan".to_string()
            );
        }

        // 3. High risk with past failures → ask confirmation
        if risk_level == RiskLevel::High
            && !memory_context.past_failures.is_empty()
        {
            return ArbitrationResult::AskConfirmation(
                format!(
                    "High risk action with {} past failures. Proceed?",
                    memory_context.past_failures.len()
                )
            );
        }

        // 4. High risk → sandbox first
        if risk_level == RiskLevel::High {
            if let Some(plan) = brain_plan {
                return ArbitrationResult::SandboxFirst(plan.clone());
            }
        }

        // 5. Normal execution
        if let Some(plan) = brain_plan {
            return ArbitrationResult::Execute(plan.clone());
        }

        // 6. No plan → reflect
        ArbitrationResult::Reflect
    }
}
```

- [ ] **Step 2: Update impl/mod.rs**

Add `pub mod coordinator;`

- [ ] **Step 3: Verify**

Run: `cargo build -p aletheon-runtime`

- [ ] **Step 4: Create README.md for Coordinator design**

Create `crates/aletheon-runtime/docs/coordinator-design.md` with:
- Definition: Coordinator is a temporary arbitrator, not supreme authority
- Input: SelfField verdict, BrainCore plan, risk level, memory context
- Output: ArbitrationResult (Execute/Deny/Delay/SandboxFirst/AskConfirmation/Reflect/Mutate)
- Decision tree (same as arbitrate() logic)
- Relationship to Engine: Engine handles Cognitive Path, Coordinator handles Volitional Path

- [ ] **Step 5: Commit**

```bash
git add crates/aletheon-runtime/
git commit -m "feat(aletheon-runtime): add Coordinator arbitrator

- ArbitrationResult enum (Execute/Deny/Delay/SandboxFirst/AskConfirmation/Reflect/Mutate)
- Uses ABI types (Verdict, RiskLevel, Plan) — no local duplicates
- async arbitrate() method
- Decision logic: SelfField deny > Critical risk > High risk + past failures > High risk > Normal > Reflect
- Design README"
```

---

## Phase 7: Cleanup

### Task 7.1: Update argos-cli to use aletheon-body

**Files:**
- Modify: `crates/argos-cli/Cargo.toml`
- Modify: `crates/argos-cli/src/main.rs`
- Modify: `crates/argos-cli/src/tui/computer.rs`

- [ ] **Step 1: Update Cargo.toml**

Replace:
```toml
argos-acix = { path = "../argos-acix" }
argos-driver = { path = "../argos-driver" }
```
With:
```toml
aletheon-body = { path = "../aletheon-body" }
```

- [ ] **Step 2: Update imports in main.rs**

Change any `argos_*` imports to `aletheon_*`.

- [ ] **Step 3: Update imports in tui/computer.rs**

Change:
```rust
use argos_acix::{Aci, GroundingProvider, MockGroundingProvider};
use argos_driver::{a11y, display, input, ocr, factory};
```
To:
```rust
use aletheon_body::r#impl::acix::{Aci, GroundingProvider, MockGroundingProvider};
use aletheon_body::r#impl::driver::{a11y, display, input, ocr, factory};
```

- [ ] **Step 4: Verify**

Run: `cargo build -p argos-cli`

- [ ] **Step 5: Commit**

```bash
git add crates/argos-cli/
git commit -m "refactor(argos-cli): use aletheon-body instead of argos-acix/argos-driver"
```

---

### Task 7.2: Update argosd to use aletheon-runtime

**Files:**
- Modify: `crates/argosd/Cargo.toml`
- Modify: `crates/argosd/src/handler.rs`
- Modify: `crates/argosd/src/server.rs`

- [ ] **Step 1: Update Cargo.toml**

Replace `argos-core` dependency with `aletheon-runtime`.

- [ ] **Step 2: Update imports**

Change `use argos_core::*` to `use aletheon_runtime::*`.

- [ ] **Step 3: Verify**

Run: `cargo build -p argosd`

- [ ] **Step 4: Commit**

```bash
git add crates/argosd/
git commit -m "refactor(argosd): use aletheon-runtime instead of argos-core"
```

---

### Task 7.3: Remove old crates and update workspace

**Files:**
- Modify: `Cargo.toml` (workspace root)
- Delete: `crates/argos-types/`
- Delete: `crates/argos-ipc/`
- Delete: `crates/argos-acix/`
- Delete: `crates/argos-core/`

- [ ] **Step 1: Update workspace Cargo.toml**

Remove from `members`:
```
"crates/argos-types",
"crates/argos-ipc",
"crates/argos-acix",
"crates/argos-core",
```

Add:
```
"crates/aletheon-comm",
"crates/aletheon-meta",
```

- [ ] **Step 2: Remove old crate directories**

```bash
rm -rf crates/argos-types crates/argos-ipc crates/argos-acix crates/argos-core
```

- [ ] **Step 3: Verify full workspace**

Run: `cargo build --workspace`
Run: `cargo test --workspace`

- [ ] **Step 4: Commit**

```bash
git add -A
git commit -m "chore: remove old argos-* crates, finalize migration

- Remove argos-types (merged into aletheon-abi)
- Remove argos-ipc (merged into aletheon-comm)
- Remove argos-acix (merged into aletheon-body)
- Remove argos-core (distributed to aletheon-runtime, aletheon-brain-core, aletheon-body)
- Update workspace members"
```

---

### Task 7.4: Final verification

- [ ] **Step 1: Full build**

Run: `cargo build --workspace`
Expected: PASS, no errors

- [ ] **Step 2: Full test**

Run: `cargo test --workspace`
Expected: All tests pass

- [ ] **Step 3: Verify crate structure**

Run: `find crates/aletheon-*/src -type d | sort`
Expected: Each crate has core/, bridge/, impl/ directories

- [ ] **Step 4: Verify no stale references**

Run: `grep -r "argos_types\|argos_ipc\|argos_acix\|argos_core" crates/`
Expected: No matches (all old references removed)

- [ ] **Step 5: Commit if any fixes needed**

```bash
git add -A
git commit -m "fix: final cleanup and verification for argos→aletheon migration"
```

---

## Appendix: File Count Summary

| Phase | New Files | Modified Files | Deleted Files |
|---|---|---|---|
| Phase 1 (aletheon-abi) | 6 | 2 | 0 |
| Phase 2 (aletheon-comm) | 12 | 3 | 0 |
| Phase 3 (brain-core + body) | 22 | 6 | 0 |
| Phase 4 (runtime) | 7 | 3 | 0 |
| Phase 5 (testing + verify) | 4 | 0-4 | 0 |
| Phase 6 (design docs) | 5 | 2 | 0 |
| Phase 7 (cleanup) | 0 | 4 | 4 |
| **Total** | **~56** | **~20-24** | **4** |
