# Aletheon Cleanup Implementation Plan

> **For agentic workers:** Use `workflow-feature` or `writing-plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix workspace build failure, rename all ~310 argos references to aletheon, establish CI, and update documentation.

**Architecture:** Five sequential phases — workspace fix first (unblocks compilation), then path constants, then code identifiers, then docs, then CI. Each phase ends with `cargo check` validation.

**Tech Stack:** Rust, Cargo, GitHub Actions

---

## Phase 1: Fix Workspace Mismatch (Crate Rename)

### Task 1.1: Rename crate `aletheon-brain-core` → `aletheon-brain`

**Files:**
- Modify: `crates/aletheon-brain/Cargo.toml:2`

- [ ] **Step 1: Change crate name**

```toml
# crates/aletheon-brain/Cargo.toml line 2
# OLD:
name = "aletheon-brain-core"
# NEW:
name = "aletheon-brain"
```

- [ ] **Step 2: Commit**

```bash
git add crates/aletheon-brain/Cargo.toml
git commit -m "refactor: rename crate aletheon-brain-core → aletheon-brain"
```

---

### Task 1.2: Rename crate `aletheon-self-field` → `aletheon-self`

**Files:**
- Modify: `crates/aletheon-self/Cargo.toml:2`

- [ ] **Step 1: Change crate name**

```toml
# crates/aletheon-self/Cargo.toml line 2
# OLD:
name = "aletheon-self-field"
# NEW:
name = "aletheon-self"
```

- [ ] **Step 2: Commit**

```bash
git add crates/aletheon-self/Cargo.toml
git commit -m "refactor: rename crate aletheon-self-field → aletheon-self"
```

---

### Task 1.3: Update workspace Cargo.toml members

**Files:**
- Modify: `Cargo.toml` (workspace root)

- [ ] **Step 1: Update members list**

```toml
# Cargo.toml workspace members
# OLD:
"crates/aletheon-self-field",
"crates/aletheon-brain-core",
# NEW:
"crates/aletheon-self",
"crates/aletheon-brain",
```

- [ ] **Step 2: Commit**

```bash
git add Cargo.toml
git commit -m "refactor: update workspace members to match new crate names"
```

---

### Task 1.4: Update aletheon-runtime dependencies

**Files:**
- Modify: `crates/aletheon-runtime/Cargo.toml`

- [ ] **Step 1: Update dependency names and paths**

```toml
# crates/aletheon-runtime/Cargo.toml
# OLD:
aletheon-brain-core = { path = "../aletheon-brain-core" }
...
aletheon-self-field = { path = "../aletheon-self-field" }
# NEW:
aletheon-brain = { path = "../aletheon-brain" }
...
aletheon-self = { path = "../aletheon-self" }
```

- [ ] **Step 2: Commit**

```bash
git add crates/aletheon-runtime/Cargo.toml
git commit -m "refactor: update aletheon-runtime deps to new crate names"
```

---

### Task 1.5: Update all `use aletheon_brain_core::*` imports

**Files:**
- Modify: `crates/aletheon-runtime/src/lib.rs:19`
- Modify: `crates/aletheon-runtime/src/impl/daemon/mod.rs:80`
- Modify: `crates/aletheon-runtime/src/impl/daemon/handler.rs` (none found, skip)
- Modify: `crates/aletheon-runtime/src/impl/engine/cognitive_loop.rs:9,10`
- Modify: `crates/aletheon-runtime/src/impl/engine/memory_integration.rs:3`
- Modify: `crates/aletheon-runtime/src/impl/engine/streaming.rs:6`
- Modify: `crates/aletheon-runtime/src/impl/memory/compaction.rs:1`
- Modify: `crates/aletheon-runtime/src/impl/memory/compressor/mod.rs:7`
- Modify: `crates/aletheon-runtime/src/impl/orchestration/builtin/code_agent.rs:4`
- Modify: `crates/aletheon-runtime/src/impl/orchestration/builtin/fs_agent.rs:4`
- Modify: `crates/aletheon-runtime/src/impl/orchestration/builtin/net_agent.rs:4`
- Modify: `crates/aletheon-runtime/src/impl/orchestration/config_agent.rs:7`
- Modify: `crates/aletheon-runtime/src/impl/orchestration/registry.rs:7`
- Modify: `crates/aletheon-runtime/src/impl/orchestration/selector.rs:4`

- [ ] **Step 1: Global find-and-replace**

In all files under `crates/aletheon-runtime/src/`, replace:
```
aletheon_brain_core
```
with:
```
aletheon_brain
```

- [ ] **Step 2: Commit**

```bash
git add crates/aletheon-runtime/src/
git commit -m "refactor: update aletheon_brain_core imports → aletheon_brain"
```

---

### Task 1.6: Update all `use aletheon_self_field::*` imports

**Files:**
- Modify: `crates/aletheon-runtime/src/impl/daemon/mod.rs:121,122,129,140`
- Modify: `crates/aletheon-runtime/src/impl/daemon/handler.rs:14`
- Modify: `crates/aletheon-runtime/src/impl/engine/cognitive_loop.rs:18,19,20,21`
- Modify: `crates/aletheon-runtime/src/impl/engine/streaming.rs:10,11`

- [ ] **Step 1: Global find-and-replace**

In all files under `crates/aletheon-runtime/src/`, replace:
```
aletheon_self_field
```
with:
```
aletheon_self
```

- [ ] **Step 2: Commit**

```bash
git add crates/aletheon-runtime/src/
git commit -m "refactor: update aletheon_self_field imports → aletheon_self"
```

---

### Task 1.7: Validate Phase 1

- [ ] **Step 1: Check compilation**

```bash
cargo check --workspace 2>&1
```

Expected: Compiles successfully (or only pre-existing errors unrelated to rename)

- [ ] **Step 2: Run tests**

```bash
cargo test --workspace 2>&1
```

Expected: Same pass/fail rate as before rename

---

## Phase 2: Rename Runtime Paths

### Task 2.1: Add centralized path constants to aletheon-abi

**Files:**
- Create: `crates/aletheon-abi/src/paths.rs`
- Modify: `crates/aletheon-abi/src/lib.rs`

- [ ] **Step 1: Create paths.rs**

```rust
// crates/aletheon-abi/src/paths.rs
//! Centralized path constants for Aletheon runtime.
//!
//! All filesystem paths used by the agent are defined here
//! to avoid scattered hardcoded values.

use std::path::PathBuf;

/// User config directory: ~/.aletheon/
pub fn config_dir() -> PathBuf {
    dirs_or_fallback().join(".aletheon")
}

/// System socket directory: /var/run/aletheon/
pub const SOCKET_DIR: &str = "/var/run/aletheon";

/// System snapshot directory: /var/lib/aletheon/snapshots
pub const SNAPSHOT_DIR: &str = "/var/lib/aletheon/snapshots";

/// System hooks directory: /etc/aletheon/hooks
pub const HOOKS_SYSTEM_DIR: &str = "/etc/aletheon/hooks";

/// Cgroup prefix for sandbox isolation
pub const CGROUP_PREFIX: &str = "aletheon";

/// XDG config: ~/.config/aletheon/
pub fn xdg_config_dir() -> PathBuf {
    dirs_or_fallback().join(".config").join("aletheon")
}

/// XDG data: ~/.local/share/aletheon/
pub fn xdg_data_dir() -> PathBuf {
    dirs_or_fallback().join(".local").join("share").join("aletheon")
}

/// User hooks directory: ~/.aletheon/hooks/
pub fn user_hooks_dir() -> PathBuf {
    config_dir().join("hooks")
}

/// Local hooks directory: .aletheon/hooks/
pub fn local_hooks_dir() -> PathBuf {
    PathBuf::from(".aletheon").join("hooks")
}

/// Skills directory: ~/.aletheon/skills/
pub fn skills_dir() -> PathBuf {
    config_dir().join("skills")
}

/// MCP tokens path: ~/.config/aletheon/mcp_tokens.json
pub fn mcp_tokens_path() -> PathBuf {
    xdg_config_dir().join("mcp_tokens.json")
}

/// Config file path: ~/.aletheon/config.toml
pub fn config_file() -> PathBuf {
    config_dir().join("config.toml")
}

/// Env file path: ~/.aletheon/.env
pub fn env_file() -> PathBuf {
    config_dir().join(".env")
}

fn dirs_or_fallback() -> PathBuf {
    std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/tmp"))
}
```

- [ ] **Step 2: Export from lib.rs**

Add to `crates/aletheon-abi/src/lib.rs`:
```rust
pub mod paths;
```

- [ ] **Step 3: Commit**

```bash
git add crates/aletheon-abi/src/paths.rs crates/aletheon-abi/src/lib.rs
git commit -m "feat(aletheon-abi): add centralized path constants"
```

---

### Task 2.2: Update daemon/mod.rs paths

**Files:**
- Modify: `crates/aletheon-runtime/src/impl/daemon/mod.rs`

- [ ] **Step 1: Replace hardcoded paths**

Replace:
```rust
let path = PathBuf::from(home).join(".argos/config.toml");
```
with:
```rust
let path = aletheon_abi::paths::config_file();
```

Replace:
```rust
let path = PathBuf::from(home).join(".argos/.env");
```
with:
```rust
let path = aletheon_abi::paths::env_file();
```

Replace:
```rust
format!("{}/.local/share/argos", home)
```
with:
```rust
aletheon_abi::paths::xdg_data_dir().to_string_lossy().to_string()
```

Also update the doc comment `/// Daemon configuration, migrated from argosd.` → `/// Daemon configuration.`

- [ ] **Step 2: Commit**

```bash
git add crates/aletheon-runtime/src/impl/daemon/mod.rs
git commit -m "refactor: use centralized paths in daemon config"
```

---

### Task 2.3: Update memory/vector_store.rs paths

**Files:**
- Modify: `crates/aletheon-runtime/src/impl/memory/vector_store.rs`

- [ ] **Step 1: Replace `.join(".argos")` with path constant**

Find the line with `.join(".argos")` and replace with:
```rust
.join(".aletheon")
```

Or better, use the centralized constant:
```rust
aletheon_abi::paths::config_dir()
```

- [ ] **Step 2: Commit**

```bash
git add crates/aletheon-runtime/src/impl/memory/vector_store.rs
git commit -m "refactor: use centralized paths in vector store"
```

---

### Task 2.4: Update hook/config.rs paths

**Files:**
- Modify: `crates/aletheon-self/src/impl/hook/config.rs`

- [ ] **Step 1: Replace all hook directory paths**

Replace:
```rust
hooks.extend(load_hooks_from_dir(Path::new("/etc/argos/hooks"))?);
```
with:
```rust
hooks.extend(load_hooks_from_dir(Path::new(aletheon_abi::paths::HOOKS_SYSTEM_DIR))?);
```

Replace:
```rust
hooks.extend(load_hooks_from_dir(&home.join(".argos/hooks"))?);
```
with:
```rust
hooks.extend(load_hooks_from_dir(&aletheon_abi::paths::user_hooks_dir())?);
```

Replace:
```rust
hooks.extend(load_hooks_from_dir(Path::new(".argos/hooks"))?);
```
with:
```rust
hooks.extend(load_hooks_from_dir(&aletheon_abi::paths::local_hooks_dir())?);
```

- [ ] **Step 2: Commit**

```bash
git add crates/aletheon-self/src/impl/hook/config.rs
git commit -m "refactor: use centralized paths in hook config"
```

---

### Task 2.5: Update ui/skill.rs paths

**Files:**
- Modify: `crates/aletheon-body/src/impl/ui/skill.rs`

- [ ] **Step 1: Replace skills directory path**

Replace:
```rust
/// Default skills directory: ~/.argos/skills/
```
with:
```rust
/// Default skills directory: ~/.aletheon/skills/
```

Replace `.join(".argos")` with `.join(".aletheon")` (or use `aletheon_abi::paths::skills_dir()`).

- [ ] **Step 2: Commit**

```bash
git add crates/aletheon-body/src/impl/ui/skill.rs
git commit -m "refactor: use centralized paths in skill loader"
```

---

### Task 2.6: Update rollback/mod.rs paths

**Files:**
- Modify: `crates/aletheon-self/src/impl/security/rollback/mod.rs`

- [ ] **Step 1: Replace snapshot directory**

Replace:
```rust
snapshot_dir: std::path::PathBuf::from("/var/lib/argos/snapshots"),
```
with:
```rust
snapshot_dir: std::path::PathBuf::from(aletheon_abi::paths::SNAPSHOT_DIR),
```

Replace `.join(".argos")` with `.join(".aletheon")`.

Update test paths:
```rust
// OLD:
paths: vec!["/tmp/argos-test-snapshot".to_string()],
std::fs::write("/tmp/argos-test-snapshot", "test data").unwrap();
let _ = std::fs::remove_file("/tmp/argos-test-snapshot");
// NEW:
paths: vec!["/tmp/aletheon-test-snapshot".to_string()],
std::fs::write("/tmp/aletheon-test-snapshot", "test data").unwrap();
let _ = std::fs::remove_file("/tmp/aletheon-test-snapshot");
```

- [ ] **Step 2: Commit**

```bash
git add crates/aletheon-self/src/impl/security/rollback/mod.rs
git commit -m "refactor: use centralized paths in rollback"
```

---

### Task 2.7: Update discovery.rs paths

**Files:**
- Modify: `crates/aletheon-body/src/impl/platform/awareness/discovery.rs`
- Modify: `crates/aletheon-body/src/impl/platform/awareness/mod.rs`

- [ ] **Step 1: Replace socket directory constant**

Replace:
```rust
pub const DEFAULT_SOCKET_DIR: &str = "/var/run/argos";
```
with:
```rust
pub const DEFAULT_SOCKET_DIR: &str = aletheon_abi::paths::SOCKET_DIR;
```

Update doc comments:
```rust
// OLD:
//! Scans `/var/run/argos/*.sock` to find running agents.
// NEW:
//! Scans `/var/run/aletheon/*.sock` to find running agents.
```

Update test path:
```rust
// OLD:
let discovery = AgentDiscovery::with_dir("/tmp/argos-test-nonexistent");
// NEW:
let discovery = AgentDiscovery::with_dir("/tmp/aletheon-test-nonexistent");
```

- [ ] **Step 2: Commit**

```bash
git add crates/aletheon-body/src/impl/platform/awareness/
git commit -m "refactor: use centralized paths in agent discovery"
```

---

### Task 2.8: Update sandbox_driver/mod.rs paths

**Files:**
- Modify: `crates/aletheon-body/src/impl/driver/sandbox_driver/mod.rs`

- [ ] **Step 1: Replace cgroup path**

Replace:
```rust
let path = PathBuf::from(format!("/sys/fs/cgroup/argos-{id}"));
```
with:
```rust
let path = PathBuf::from(format!("/sys/fs/cgroup/{}-{id}", aletheon_abi::paths::CGROUP_PREFIX));
```

Update doc comment:
```rust
// OLD:
/// Create a new cgroup named `argos-<id>`.
// NEW:
/// Create a new cgroup named `aletheon-<id>`.
```

- [ ] **Step 2: Commit**

```bash
git add crates/aletheon-body/src/impl/driver/sandbox_driver/mod.rs
git commit -m "refactor: use centralized paths in sandbox driver"
```

---

### Task 2.9: Update mcp/auth.rs paths

**Files:**
- Modify: `crates/aletheon-body/src/impl/mcp/auth.rs`

- [ ] **Step 1: Replace MCP tokens path**

Replace:
```rust
/// Default store at `~/.config/argos/mcp_tokens.json`.
Ok(base.join("argos").join("mcp_tokens.json"))
```
with:
```rust
/// Default store at `~/.config/aletheon/mcp_tokens.json`.
Ok(base.join("aletheon").join("mcp_tokens.json"))
```

Or use centralized path:
```rust
Ok(aletheon_abi::paths::mcp_tokens_path())
```

- [ ] **Step 2: Commit**

```bash
git add crates/aletheon-body/src/impl/mcp/auth.rs
git commit -m "refactor: use centralized paths in MCP auth"
```

---

### Task 2.10: Validate Phase 2

- [ ] **Step 1: Check compilation**

```bash
cargo check --workspace 2>&1
```

Expected: Compiles successfully

- [ ] **Step 2: Commit any remaining path fixes**

If any paths were missed, fix and commit.

---

## Phase 3: Rename Code Identifiers

### Task 3.1: Rename `ArgosBodyRuntime` → `AletheonBodyRuntime`

**Files:**
- Modify: `crates/aletheon-body/src/core/mod.rs`
- Modify: `crates/aletheon-body/src/lib.rs`

- [ ] **Step 1: Replace all occurrences**

In `core/mod.rs`:
```rust
// OLD:
pub struct ArgosBodyRuntime {
impl ArgosBodyRuntime {
/// Create a new ArgosBodyRuntime with default tools and security.
impl Subsystem for ArgosBodyRuntime {
impl BodyRuntime for ArgosBodyRuntime {
// NEW:
pub struct AletheonBodyRuntime {
impl AletheonBodyRuntime {
/// Create a new AletheonBodyRuntime with default tools and security.
impl Subsystem for AletheonBodyRuntime {
impl BodyRuntime for AletheonBodyRuntime {
```

In `lib.rs`:
```rust
// OLD:
pub use core::ArgosBodyRuntime;
// NEW:
pub use core::AletheonBodyRuntime;
```

In tests (core/mod.rs):
```rust
// OLD:
let rt = ArgosBodyRuntime::with_runner(
// NEW:
let rt = AletheonBodyRuntime::with_runner(
```

Also update Subsystem name:
```rust
// OLD:
"argos-body"
// NEW:
"aletheon-body"
```

And log message:
```rust
// OLD:
"ArgosBodyRuntime initialized with {} capabilities",
// NEW:
"AletheonBodyRuntime initialized with {} capabilities",
```

- [ ] **Step 2: Commit**

```bash
git add crates/aletheon-body/src/core/mod.rs crates/aletheon-body/src/lib.rs
git commit -m "refactor: rename ArgosBodyRuntime → AletheonBodyRuntime"
```

---

### Task 3.2: Rename `ArgosPermissionLevel` → `ToolPermissionLevel`

**Files:**
- Modify: `crates/aletheon-body/src/core/conversions.rs`

- [ ] **Step 1: Replace all occurrences**

```rust
// OLD:
use aletheon_abi::tool::{ToolResult, ToolResultMeta, PermissionLevel as ArgosPermissionLevel, ToolContext};
// NEW:
use aletheon_abi::tool::{ToolResult, ToolResultMeta, PermissionLevel as ToolPermissionLevel, ToolContext};
```

Replace all `ArgosPermissionLevel` → `ToolPermissionLevel` (6 occurrences).

- [ ] **Step 2: Commit**

```bash
git add crates/aletheon-body/src/core/conversions.rs
git commit -m "refactor: rename ArgosPermissionLevel → ToolPermissionLevel"
```

---

### Task 3.3: Rename `argos_to_abi_permission()` → `tool_to_abi_permission()`

**Files:**
- Modify: `crates/aletheon-body/src/core/conversions.rs`

- [ ] **Step 1: Replace function name and doc comments**

```rust
// OLD:
/// Convert argos PermissionLevel -> aletheon PermissionLevel
pub fn argos_to_abi_permission(level: ArgosPermissionLevel) -> AbiPermissionLevel {
// NEW:
/// Convert ToolPermissionLevel -> aletheon PermissionLevel
pub fn tool_to_abi_permission(level: ToolPermissionLevel) -> AbiPermissionLevel {
```

Replace all call sites:
```rust
// OLD:
level: argos_to_abi_permission(level),
assert_eq!(argos_to_abi_permission(ArgosPermissionLevel::L0), ...
// NEW:
level: tool_to_abi_permission(level),
assert_eq!(tool_to_abi_permission(ToolPermissionLevel::L0), ...
```

Also update other doc comments:
```rust
// OLD:
/// Convert aletheon Action -> argos Tool name + JSON input
/// Convert argos ToolResult -> aletheon ActionResult
/// Convert aletheon Context -> argos ToolContext
/// Convert argos Tool metadata -> aletheon Capability
// NEW:
/// Convert aletheon Action -> Tool name + JSON input
/// Convert ToolResult -> aletheon ActionResult
/// Convert aletheon Context -> ToolContext
/// Convert Tool metadata -> aletheon Capability
```

- [ ] **Step 2: Commit**

```bash
git add crates/aletheon-body/src/core/conversions.rs
git commit -m "refactor: rename argos_to_abi_permission → tool_to_abi_permission"
```

---

### Task 3.4: Update MCP client name

**Files:**
- Modify: `crates/aletheon-body/src/impl/mcp/transport.rs`
- Modify: `crates/aletheon-body/src/impl/mcp/client.rs`

- [ ] **Step 1: Replace client name**

```rust
// OLD:
"clientInfo": { "name": "argos", "version": "0.1.0" }
// NEW:
"clientInfo": { "name": "aletheon", "version": "0.1.0" }
```

- [ ] **Step 2: Commit**

```bash
git add crates/aletheon-body/src/impl/mcp/
git commit -m "refactor: rename MCP client argos → aletheon"
```

---

### Task 3.5: Update uinput device name

**Files:**
- Modify: `crates/aletheon-body/src/impl/driver/input/uinput.rs`

- [ ] **Step 1: Replace device name**

```rust
// OLD:
let s = b"argos-virtual-input";
// NEW:
let s = b"aletheon-virtual-input";
```

- [ ] **Step 2: Commit**

```bash
git add crates/aletheon-body/src/impl/driver/input/uinput.rs
git commit -m "refactor: rename uinput device argos → aletheon"
```

---

### Task 3.6: Update X11 atoms

**Files:**
- Modify: `crates/aletheon-body/src/impl/driver/display/clipboard_x11.rs`

- [ ] **Step 1: Replace atom names**

```rust
// OLD:
let result_atom = intern_atom(&conn, b"ARGOS_CLIP")?;
let data_atom = intern_atom(&conn, b"ARGOS_CLIPBOARD_DATA")?;
let test_text = "argos-x11-clipboard-test-42";
// NEW:
let result_atom = intern_atom(&conn, b"ALETHEON_CLIP")?;
let data_atom = intern_atom(&conn, b"ALETHEON_CLIPBOARD_DATA")?;
let test_text = "aletheon-x11-clipboard-test-42";
```

- [ ] **Step 2: Commit**

```bash
git add crates/aletheon-body/src/impl/driver/display/clipboard_x11.rs
git commit -m "refactor: rename X11 atoms ARGOS → ALETHEON"
```

---

### Task 3.7: Update remaining test paths and identifiers

**Files:**
- Modify: `crates/aletheon-body/src/impl/acix/experience.rs`
- Modify: `crates/aletheon-memory/src/self_memory.rs`

- [ ] **Step 1: Replace test paths**

In `experience.rs`:
```rust
// OLD:
let dir = std::env::temp_dir().join("argos_test_experience");
// NEW:
let dir = std::env::temp_dir().join("aletheon_test_experience");
```

In `self_memory.rs`:
```rust
// OLD:
mem.store(make_identity_change(b"renamed agent to Argos"))
// NEW:
mem.store(make_identity_change(b"renamed agent to Aletheon"))
```

- [ ] **Step 2: Commit**

```bash
git add crates/aletheon-body/src/impl/acix/experience.rs crates/aletheon-memory/src/self_memory.rs
git commit -m "refactor: rename test identifiers argos → aletheon"
```

---

### Task 3.8: Clean up origin doc comments

**Files:**
- Modify: all `.rs` files with `"Merged from argos-*"` or `"migrated from argos-*"` comments

- [ ] **Step 1: Replace or remove origin comments**

Replace patterns like:
```
//! Merged from argos-types message module into aletheon-abi.
```
with:
```
//! Message types for inter-agent communication.
```

Replace:
```
//! Migrated from argos-security into aletheon-body's unified impl layer.
```
with:
```
//! Security layer for aletheon-body.
```

Replace:
```
//! Migrated from argos-ipc. Provides Unix socket, io_uring, and shared memory
```
with:
```
//! IPC transport layer. Provides Unix socket, io_uring, and shared memory
```

Replace:
```
/// Bridges argos-core's LlmProvider for use by BrainCore.
```
with:
```
/// Bridges LlmProvider for use by BrainCore.
```

Files to update:
- `aletheon-abi/src/lib.rs` (2 lines)
- `aletheon-abi/src/message.rs` (1 line)
- `aletheon-abi/src/tool.rs` (1 line)
- `aletheon-abi/src/sandbox.rs` (1 line)
- `aletheon-abi/src/ipc_types.rs` (1 line)
- `aletheon-abi/src/llm_types.rs` (1 line)
- `aletheon-abi/src/capability.rs` (1 line)
- `aletheon-body/src/impl/tools/mod.rs` (1 line)
- `aletheon-body/src/impl/tools/executor.rs` (1 line)
- `aletheon-body/src/impl/driver/mod.rs` (1 line)
- `aletheon-body/src/impl/sandbox/mod.rs` (1 line)
- `aletheon-body/src/impl/security/mod.rs` (1 line)
- `aletheon-body/src/impl/acix/mod.rs` (1 line)
- `aletheon-body/src/impl/acix/grounding.rs` (2 lines)
- `aletheon-body/src/impl/acix/task.rs` (2 lines)
- `aletheon-body/src/impl/platform/awareness/mod.rs` (1 line)
- `aletheon-body/src/impl/platform/awareness/discovery.rs` (2 lines)
- `aletheon-body/src/impl/ui/mod.rs` (5 lines — update "argos" in UI strings)
- `aletheon-runtime/src/core/config.rs` (1 line)
- `aletheon-runtime/src/impl/engine/mod.rs` (1 line)
- `aletheon-runtime/src/lib.rs` (2 lines)
- `aletheon-runtime/src/bridge/mod.rs` (1 line)
- `aletheon-comm/src/impl/ipc/mod.rs` (1 line)
- `aletheon-brain/src/bridge/llm.rs` (1 line)
- `aletheon-brain/src/bridge/learning.rs` (2 lines)
- `aletheon-brain/src/bridge/inference.rs` (1 line)
- `aletheon-brain/src/bridge/mod.rs` (1 line)
- `aletheon-brain/src/config/mod.rs` (1 line)
- `aletheon-self/src/core/mod.rs` (1 line)
- `aletheon-self/src/lib.rs` (1 line)
- `aletheon-self/src/bridge/hook.rs` (1 line)
- `aletheon-self/src/bridge/policy.rs` (1 line)
- `aletheon-self/src/bridge/loop_detector.rs` (1 line)
- `aletheon-self/src/impl/security/mod.rs` (1 line)

Also update UI strings in `aletheon-body/src/impl/ui/mod.rs`:
```rust
// OLD:
"Cannot connect to daemon at {}: {}\n\nStart the daemon first:\n  argos daemon &"
"Welcome to argos! Type a message to get started. Ctrl+C to quit."
"  argos v0.1.0"
" argos".to_string()
format!(" argos  |  {model_display}")
// NEW:
"Cannot connect to daemon at {}: {}\n\nStart the daemon first:\n  aletheon daemon &"
"Welcome to aletheon! Type a message to get started. Ctrl+C to quit."
"  aletheon v0.1.0"
" aletheon".to_string()
format!(" aletheon  |  {model_display}")
```

- [ ] **Step 2: Commit**

```bash
git add -A crates/
git commit -m "refactor: clean up remaining argos references in doc comments and UI strings"
```

---

### Task 3.9: Validate Phase 3

- [ ] **Step 1: Check compilation**

```bash
cargo check --workspace 2>&1
```

- [ ] **Step 2: Run tests**

```bash
cargo test --workspace 2>&1
```

- [ ] **Step 3: Verify no argos references remain in .rs files**

```bash
grep -rn 'argos' crates/ --include='*.rs' | grep -v '//' | grep -v 'Migrat' | grep -v 'migrat'
```

Expected: No output (or only acceptable historical references)

---

## Phase 4: Update Documentation

### Task 4.1: Update README.md

**Files:**
- Modify: `README.md`

- [ ] **Step 1: Update project description**

Replace the current README content with an updated version that:
- Uses "Aletheon" instead of any "argos" references
- Reflects the current 10-crate architecture
- References the Nous architecture (Soul/Brain/Body)

- [ ] **Step 2: Commit**

```bash
git add README.md
git commit -m "docs: update README for Aletheon project"
```

---

### Task 4.2: Update design docs

**Files:**
- Modify: `docs/design/README.md`
- Modify: `docs/design/architecture-overview.md`
- Modify: `docs/design/execution/ipc.md`
- Modify: `docs/design/execution/tool-system.md`
- Modify: `docs/design/execution/sandbox.md`
- Modify: `docs/design/security/security-model.md`
- Modify: `docs/design/security/writable-root.md`
- Modify: `docs/design/platform/agent-awareness.md`
- Modify: `docs/design/testing/ci-pipeline.md`
- Modify: `docs/design/core/session-lifecycle.md`
- Modify: `docs/design/orchestration/hybrid-inference.md`
- Modify: `docs/design/perception/perception-layer.md`

- [ ] **Step 1: Replace all argos references**

In each file, replace:
- `argos/crates/agent-core/` → `crates/aletheon-*/`
- `/etc/argos/` → `/etc/aletheon/`
- `~/.argos/` → `~/.aletheon/`
- `.argos/` → `.aletheon/`
- `ghcr.io/aurobear/argos` → `ghcr.io/aurobear/aletheon`
- `docs/plans/2026-06-06-argos-design.md` → keep as-is (historical reference)

- [ ] **Step 2: Commit**

```bash
git add docs/design/
git commit -m "docs: update design docs with aletheon paths"
```

---

### Task 4.3: Clean up Cargo.toml comments

**Files:**
- Modify: `crates/aletheon-body/Cargo.toml`

- [ ] **Step 1: Update migration comments**

```toml
# OLD:
# TUI dependencies (migrated from argos-cli)
# Migrated from argos-driver
# NEW:
# TUI dependencies
# Driver dependencies
```

- [ ] **Step 2: Commit**

```bash
git add crates/aletheon-body/Cargo.toml
git commit -m "chore: clean up migration comments in Cargo.toml"
```

---

### Task 4.4: Validate Phase 4

- [ ] **Step 1: Verify no argos references remain**

```bash
grep -rn 'argos' --include='*.rs' --include='*.toml' --include='*.md' crates/ README.md docs/design/ | grep -v 'docs/plans/2026-06-14' | grep -v 'Migrat' | grep -v 'migrat'
```

Expected: Only migration plan docs (historical) have argos references

---

## Phase 5: Set Up CI

### Task 5.1: Create GitHub Actions workflow

**Files:**
- Create: `.github/workflows/ci.yml`

- [ ] **Step 1: Create CI workflow**

```yaml
name: CI

on:
  push:
    branches: [dev]
  pull_request:
    branches: [dev]

env:
  CARGO_TERM_COLOR: always

jobs:
  check:
    name: cargo check
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - uses: Swatinem/rust-cache@v2
      - run: cargo check --workspace

  test:
    name: cargo test
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - uses: Swatinem/rust-cache@v2
      - run: cargo test --workspace

  clippy:
    name: cargo clippy
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - uses: Swatinem/rust-cache@v2
      - run: cargo clippy --workspace -- -D warnings

  fmt:
    name: cargo fmt
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - run: cargo fmt --all -- --check
```

- [ ] **Step 2: Commit**

```bash
mkdir -p .github/workflows
git add .github/workflows/ci.yml
git commit -m "ci: add GitHub Actions workflow for check/test/clippy/fmt"
```

---

### Task 5.2: Final validation

- [ ] **Step 1: Full workspace check**

```bash
cargo check --workspace 2>&1
```

Expected: Compiles successfully

- [ ] **Step 2: Full test suite**

```bash
cargo test --workspace 2>&1
```

Expected: Tests pass (or same pre-existing failures)

- [ ] **Step 3: Final argos scan**

```bash
grep -rn 'argos' crates/ --include='*.rs' --include='*.toml' | wc -l
```

Expected: 0 (or only historical migration plan references in docs/)

- [ ] **Step 4: Push and create PR**

```bash
git push origin dev
```

---

## Summary

| Phase | Tasks | Key Changes |
|---|---|---|
| Phase 1 | 7 | Fix workspace build: rename crate names, update imports |
| Phase 2 | 10 | Add centralized path constants, update all filesystem paths |
| Phase 3 | 9 | Rename code identifiers, protocol names, clean up comments |
| Phase 4 | 4 | Update README, design docs, Cargo.toml comments |
| Phase 5 | 2 | Create CI workflow, final validation |

**Total: 32 tasks across 5 phases.**
