# Linux Kernel Design Patterns Adoption — Implementation Plan

> **For agentic workers:** Use `workflow-feature` or `writing-plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Adopt Linux kernel design patterns across Aletheon's core abstractions — unified registration, error handling, lifecycle management, and observability — to make the system-level OS agent more robust and extensible.

**Architecture:** Add foundational traits to `aletheon-abi` (Registry, ManagedResource, InitPhase, Observable), then migrate each subsystem to use them. Each phase produces working, tested code.

**Tech Stack:** Rust, async-trait, tokio, rusqlite, serde

---

## File Map

| File | Action | Purpose |
|---|---|---|
| `crates/aletheon-abi/src/registry.rs` | **Create** | Unified Registry trait |
| `crates/aletheon-abi/src/resource.rs` | **Create** | ManagedResource lifecycle wrapper |
| `crates/aletheon-abi/src/observable.rs` | **Create** | Observable trait for status/metrics |
| `crates/aletheon-abi/src/error.rs` | **Modify** | Add RegistryErrorKind, shorthand ctors |
| `crates/aletheon-abi/src/subsystem.rs` | **Modify** | Add InitPhase enum |
| `crates/aletheon-abi/src/lib.rs` | **Modify** | Re-export new modules |
| `crates/aletheon-body/src/impl/tools/registry.rs` | **Modify** | Registry trait, unregister, dup detection |
| `crates/aletheon-runtime/src/impl/hooks/registry.rs` | **Modify** | Registry trait, unregister, hook timeout |
| `crates/aletheon-memory/src/router.rs` | **Modify** | Dynamic backend Vec, deduplicate fan-out |
| `crates/aletheon-runtime/src/impl/plugin/loader.rs` | **Modify** | Topological sort + cycle detection |
| `crates/aletheon-runtime/src/impl/plugin/manifest.rs` | **Modify** | EntryType enum |
| `crates/aletheon-self/src/core/narrative.rs` | **Modify** | VecDeque O(1) eviction |
| `crates/aletheon-self/src/core/store.rs` | **Modify** | Persistable trait |
| `crates/aletheon-self/src/core/mod.rs` | **Modify** | Use Persistable for save_all/load_all |

---

## Phase 1: ABI Foundation

### Task 1: Add RegistryErrorKind and shorthand constructors to error.rs

**Files:**
- Modify: `crates/aletheon-abi/src/error.rs`

- [ ] **Step 1: Add RegistryErrorKind enum** after `ConfigErrorKind` (line 117):

```rust
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum RegistryErrorKind {
    AlreadyExists,
    NotFound,
    DependencyCycle,
    DependencyMissing,
    VersionIncompatible,
}
```

- [ ] **Step 2: Add `Registry` variant to `ErrorCategory`** (after `Config` variant):

```rust
Registry {
    kind: RegistryErrorKind,
},
```

- [ ] **Step 3: Add shorthand constructors** to `impl AgentError`:

```rust
pub fn already_exists(name: &str) -> Self {
    Self::new(
        ErrorSeverity::Unrecoverable,
        ErrorCategory::Registry { kind: RegistryErrorKind::AlreadyExists },
        format!("'{}' already registered", name),
    )
}

pub fn not_found(name: &str) -> Self {
    Self::new(
        ErrorSeverity::Unrecoverable,
        ErrorCategory::Registry { kind: RegistryErrorKind::NotFound },
        format!("'{}' not found", name),
    )
}

pub fn dependency_cycle(detail: &str) -> Self {
    Self::new(
        ErrorSeverity::Unrecoverable,
        ErrorCategory::Registry { kind: RegistryErrorKind::DependencyCycle },
        format!("Dependency cycle: {}", detail),
    )
}

pub fn hook_timeout(hook: &str, secs: u64) -> Self {
    Self::new(
        ErrorSeverity::Degraded,
        ErrorCategory::Tool { tool: hook.to_string(), kind: ToolErrorKind::Timeout },
        format!("Hook '{}' timed out after {}s", hook, secs),
    )
}
```

- [ ] **Step 4: Add tests** to the `#[cfg(test)]` module:

```rust
#[test]
fn test_registry_constructors() {
    let e = AgentError::already_exists("tool_x");
    assert!(!e.is_retryable());
    assert!(e.message.contains("already registered"));

    let e = AgentError::not_found("tool_y");
    assert!(e.message.contains("not found"));

    let e = AgentError::dependency_cycle("A -> B -> A");
    assert!(e.message.contains("cycle"));

    let e = AgentError::hook_timeout("audit", 30);
    assert!(e.is_retryable());
}
```

- [ ] **Step 5: Update re-exports** in `crates/aletheon-abi/src/lib.rs`:

Add `RegistryErrorKind` to the error re-export line.

- [ ] **Step 6: Run and commit**

```bash
cargo test -p aletheon-abi
git add crates/aletheon-abi/src/error.rs crates/aletheon-abi/src/lib.rs
git commit -m "feat(abi): add RegistryErrorKind and shorthand constructors"
```

---

### Task 2: Add Registry trait

**Files:**
- Create: `crates/aletheon-abi/src/registry.rs`
- Modify: `crates/aletheon-abi/src/lib.rs`

- [ ] **Step 1: Create `crates/aletheon-abi/src/registry.rs`**

```rust
//! Unified registry trait — like Linux kernel's register/unregister pattern.
//!
//! Every subsystem that manages named items (tools, hooks, agents, backends)
//! implements this trait for symmetric register/unregister semantics.

use crate::AgentError;

/// Opaque registration ID for unregister without knowing the name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RegistrationId(pub u64);

/// Unified registry contract — symmetric register/unregister.
///
/// Like Linux kernel's `register_filesystem()` / `unregister_filesystem()`,
/// `driver_register()` / `driver_unregister()`, etc.
pub trait Registry<T> {
    /// Register an item. Returns ID on success.
    /// Fails with `AgentError::already_exists` on name collision.
    fn register(&mut self, item: T) -> Result<RegistrationId, AgentError>;

    /// Unregister by ID. Returns the item or `AgentError::not_found`.
    fn unregister(&mut self, id: RegistrationId) -> Result<T, AgentError>;

    /// Look up by name.
    fn get(&self, name: &str) -> Option<&T>;

    /// Check if a name is registered.
    fn contains(&self, name: &str) -> bool;

    /// List all registered names.
    fn names(&self) -> Vec<&str>;

    /// Count of registered items.
    fn len(&self) -> usize;

    /// Whether empty.
    fn is_empty(&self) -> bool { self.len() == 0 }
}
```

- [ ] **Step 2: Add `pub mod registry;` and re-export** in lib.rs

- [ ] **Step 3: Run and commit**

```bash
cargo test -p aletheon-abi
git add crates/aletheon-abi/src/registry.rs crates/aletheon-abi/src/lib.rs
git commit -m "feat(abi): add unified Registry trait with register/unregister symmetry"
```

---

### Task 3: Add ManagedResource lifecycle wrapper

**Files:**
- Create: `crates/aletheon-abi/src/resource.rs`
- Modify: `crates/aletheon-abi/src/lib.rs`

- [ ] **Step 1: Create `crates/aletheon-abi/src/resource.rs`**

```rust
//! ManagedResource — lifecycle-aware shared resource wrapper.
//!
//! Like Linux kernel's kobject reference counting with state tracking.
//! Prevents use-after-shutdown and poisoned mutex panics.

use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use crate::AgentError;

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ResourceState {
    Uninit = 0,
    Ready = 1,
    Shutting = 2,
    Dead = 3,
}

/// Lifecycle-aware wrapper around a shared resource.
pub struct ManagedResource<T> {
    inner: Arc<Mutex<Option<T>>>,
    state: Arc<AtomicU8>,
    name: String,
}

impl<T> ManagedResource<T> {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            inner: Arc::new(Mutex::new(None)),
            state: Arc::new(AtomicU8::new(ResourceState::Uninit as u8)),
            name: name.into(),
        }
    }

    pub fn with_value(name: impl Into<String>, value: T) -> Self {
        Self {
            inner: Arc::new(Mutex::new(Some(value))),
            state: Arc::new(AtomicU8::new(ResourceState::Ready as u8)),
            name: name.into(),
        }
    }

    pub fn state(&self) -> ResourceState {
        match self.state.load(Ordering::Acquire) {
            0 => ResourceState::Uninit,
            1 => ResourceState::Ready,
            2 => ResourceState::Shutting,
            _ => ResourceState::Dead,
        }
    }

    pub fn name(&self) -> &str { &self.name }

    pub fn init(&self, value: T) -> Result<(), AgentError> {
        if self.state.load(Ordering::Acquire) != ResourceState::Uninit as u8 {
            return Err(AgentError::new(
                crate::ErrorSeverity::Unrecoverable,
                crate::ErrorCategory::Config { kind: crate::ConfigErrorKind::Invalid },
                format!("Resource '{}' already initialized", self.name),
            ));
        }
        let mut guard = self.inner.lock().map_err(|_| poisoned_err(&self.name))?;
        *guard = Some(value);
        self.state.store(ResourceState::Ready as u8, Ordering::Release);
        Ok(())
    }

    pub fn get(&self) -> Result<MutexGuard<'_, Option<T>>, AgentError> {
        if self.state.load(Ordering::Acquire) != ResourceState::Ready as u8 {
            return Err(AgentError::new(
                crate::ErrorSeverity::Degraded,
                crate::ErrorCategory::Config { kind: crate::ConfigErrorKind::Invalid },
                format!("Resource '{}' not available (state={:?})", self.name, self.state()),
            ));
        }
        self.inner.lock().map_err(|_| poisoned_err(&self.name))
    }

    pub fn shutdown(&self) -> Result<Option<T>, AgentError> {
        self.state.store(ResourceState::Shutting as u8, Ordering::Release);
        let mut guard = self.inner.lock().map_err(|_| poisoned_err(&self.name))?;
        let value = guard.take();
        self.state.store(ResourceState::Dead as u8, Ordering::Release);
        Ok(value)
    }
}

fn poisoned_err(name: &str) -> AgentError {
    AgentError::new(
        crate::ErrorSeverity::Unrecoverable,
        crate::ErrorCategory::Config { kind: crate::ConfigErrorKind::Invalid },
        format!("Resource '{}' mutex poisoned", name),
    )
}

impl<T> Clone for ManagedResource<T> {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
            state: Arc::clone(&self.state),
            name: self.name.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lifecycle_states() {
        let r = ManagedResource::<i32>::new("test");
        assert_eq!(r.state(), ResourceState::Uninit);
        r.init(42).unwrap();
        assert_eq!(r.state(), ResourceState::Ready);
        assert_eq!(*r.get().unwrap(), Some(42));
        let v = r.shutdown().unwrap();
        assert_eq!(v, Some(42));
        assert_eq!(r.state(), ResourceState::Dead);
    }

    #[test]
    fn get_before_init_fails() {
        let r = ManagedResource::<i32>::new("test");
        assert!(r.get().is_err());
    }

    #[test]
    fn double_init_fails() {
        let r = ManagedResource::<i32>::new("test");
        r.init(1).unwrap();
        assert!(r.init(2).is_err());
    }

    #[test]
    fn get_after_shutdown_fails() {
        let r = ManagedResource::with_value("test", 42);
        r.shutdown().unwrap();
        assert!(r.get().is_err());
    }
}
```

- [ ] **Step 2: Add `pub mod resource;` and re-export** in lib.rs

- [ ] **Step 3: Run and commit**

```bash
cargo test -p aletheon-abi
git add crates/aletheon-abi/src/resource.rs crates/aletheon-abi/src/lib.rs
git commit -m "feat(abi): add ManagedResource lifecycle wrapper with state tracking"
```

---

### Task 4: Add InitPhase to Subsystem trait

**Files:**
- Modify: `crates/aletheon-abi/src/subsystem.rs`

- [ ] **Step 1: Add InitPhase enum** before `SubsystemHealth`:

```rust
/// Initialization phase — like Linux kernel's initcall levels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum InitPhase {
    Core = 0,
    Subsystem = 1,
    Service = 2,
    Late = 3,
}

impl Default for InitPhase {
    fn default() -> Self { Self::Subsystem }
}
```

- [ ] **Step 2: Add `init_phase()` default method** to `Subsystem` trait:

```rust
    fn init_phase(&self) -> InitPhase { InitPhase::Subsystem }
```

- [ ] **Step 3: Update re-exports** in lib.rs — add `InitPhase`

- [ ] **Step 4: Run and commit**

```bash
cargo test -p aletheon-abi
git add crates/aletheon-abi/src/subsystem.rs crates/aletheon-abi/src/lib.rs
git commit -m "feat(abi): add InitPhase for kernel-style subsystem initialization ordering"
```

---

### Task 5: Add Observable trait

**Files:**
- Create: `crates/aletheon-abi/src/observable.rs`
- Modify: `crates/aletheon-abi/src/lib.rs`

- [ ] **Step 1: Create `crates/aletheon-abi/src/observable.rs`**

```rust
//! Observable trait — like Linux kernel's /proc and /sys interfaces.

use std::collections::HashMap;

/// Structured status snapshot for a subsystem.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SubsystemStatus {
    pub name: String,
    pub running: bool,
    pub status_line: String,
    pub details: HashMap<String, serde_json::Value>,
}

/// Observable — exposes internal state for monitoring.
pub trait Observable: Send + Sync {
    fn status(&self) -> SubsystemStatus;
    fn metrics(&self) -> HashMap<String, f64> { HashMap::new() }
}
```

- [ ] **Step 2: Add `pub mod observable;` and re-export** in lib.rs

- [ ] **Step 3: Run and commit**

```bash
cargo test -p aletheon-abi
git add crates/aletheon-abi/src/observable.rs crates/aletheon-abi/src/lib.rs
git commit -m "feat(abi): add Observable trait for proc/sysfs-style status exposure"
```

---

## Phase 2: Migrate Subsystems

### Task 6: ToolRegistry — implement Registry trait + unregister + dup detection

**Files:**
- Modify: `crates/aletheon-body/src/impl/tools/registry.rs`

- [ ] **Step 1: Add registration tracking and implement Registry**

Replace the entire file:

```rust
use std::collections::HashMap;
use std::sync::Arc;
use aletheon_abi::{AgentError, Registry, RegistrationId};
use super::Tool;

pub struct ToolRegistry {
    tools: HashMap<String, Arc<dyn Tool>>,
    id_map: HashMap<RegistrationId, String>,
    next_id: u64,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self {
            tools: HashMap::new(),
            id_map: HashMap::new(),
            next_id: 1,
        }
    }

    pub fn get(&self, name: &str) -> Option<&Arc<dyn Tool>> {
        self.tools.get(name)
    }

    pub fn list(&self) -> Vec<&str> {
        self.tools.keys().map(|s| s.as_str()).collect()
    }

    pub fn definitions(&self) -> Vec<aletheon_abi::ToolDefinition> {
        self.tools.values().map(|t| aletheon_abi::ToolDefinition {
            name: t.name().to_string(),
            description: t.description().to_string(),
            input_schema: t.input_schema(),
        }).collect()
    }
}

impl Registry<Arc<dyn Tool>> for ToolRegistry {
    fn register(&mut self, tool: Arc<dyn Tool>) -> Result<RegistrationId, AgentError> {
        let name = tool.name().to_string();
        if self.tools.contains_key(&name) {
            return Err(AgentError::already_exists(&name));
        }
        let id = RegistrationId(self.next_id);
        self.next_id += 1;
        self.tools.insert(name.clone(), tool);
        self.id_map.insert(id, name);
        Ok(id)
    }

    fn unregister(&mut self, id: RegistrationId) -> Result<Arc<dyn Tool>, AgentError> {
        let name = self.id_map.remove(&id)
            .ok_or_else(|| AgentError::not_found(&format!("registration {:?}", id)))?;
        self.tools.remove(&name)
            .ok_or_else(|| AgentError::not_found(&name))
    }

    fn get(&self, name: &str) -> Option<&Arc<dyn Tool>> {
        self.tools.get(name)
    }

    fn contains(&self, name: &str) -> bool {
        self.tools.contains_key(name)
    }

    fn names(&self) -> Vec<&str> {
        self.tools.keys().map(|s| s.as_str()).collect()
    }

    fn len(&self) -> usize {
        self.tools.len()
    }
}

impl Default for ToolRegistry {
    fn default() -> Self {
        let mut registry = Self::new();
        // Register built-in tools — errors are programming bugs
        let tools: Vec<Arc<dyn Tool>> = vec![
            Arc::new(super::bash_exec::BashExecTool),
            Arc::new(super::file_read::FileReadTool),
            Arc::new(super::file_write::FileWriteTool),
            Arc::new(super::system_status::SystemStatusTool),
            Arc::new(super::process_list::ProcessListTool),
            Arc::new(super::ebpf_compile::EbpfCompileTool),
            Arc::new(super::module_build::ModuleBuildTool),
            Arc::new(super::module_load::ModuleLoadTool),
            Arc::new(super::kernel_build::KernelBuildTool),
            Arc::new(super::code_graph::CodeGraphTool),
            Arc::new(super::file_search::FileSearchTool),
            Arc::new(super::apply_patch::ApplyPatchTool),
        ];
        for tool in tools {
            registry.register(tool).expect("duplicate built-in tool");
        }
        registry
    }
}
```

- [ ] **Step 2: Add tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use aletheon_abi::Registry;

    #[test]
    fn register_and_unregister() {
        let mut reg = ToolRegistry::new();
        let tool = Arc::new(super::super::bash_exec::BashExecTool);
        let id = reg.register(tool).unwrap();
        assert!(reg.contains("bash_exec"));
        let removed = reg.unregister(id).unwrap();
        assert_eq!(removed.name(), "bash_exec");
        assert!(!reg.contains("bash_exec"));
    }

    #[test]
    fn duplicate_register_fails() {
        let mut reg = ToolRegistry::new();
        let tool = Arc::new(super::super::bash_exec::BashExecTool);
        reg.register(tool.clone()).unwrap();
        assert!(reg.register(tool).is_err());
    }
}
```

- [ ] **Step 3: Fix callers** — search for `registry.register(` in the codebase and ensure callers handle the `Result`. The `Default::default()` impl uses `.expect()` which is fine for built-in tools.

- [ ] **Step 4: Run and commit**

```bash
cargo test -p aletheon-body
git add crates/aletheon-body/src/impl/tools/registry.rs
git commit -m "feat(body): implement Registry trait for ToolRegistry with unregister and dup detection"
```

---

### Task 7: HookRegistry — add unregister + timeout

**Files:**
- Modify: `crates/aletheon-runtime/src/impl/hooks/registry.rs`

- [ ] **Step 1: Add unregister method** to `HookRegistry`:

```rust
    /// Unregister a hook by name. Returns true if found and removed.
    pub fn unregister(&mut self, name: &str) -> bool {
        for hooks in self.hooks.values_mut() {
            let before = hooks.len();
            hooks.retain(|h| h.name != name);
            if hooks.len() < before {
                return true;
            }
        }
        false
    }
```

- [ ] **Step 2: Add timeout to `execute_single`**

Replace the `child.wait_with_output().await` call (line 120) with:

```rust
        let timeout = std::time::Duration::from_secs(30);
        match tokio::time::timeout(timeout, child.wait_with_output()).await {
            Ok(Ok(output)) => parse_hook_output(&output.stdout),
            Ok(Err(e)) => {
                warn!(hook = %hook.name, error = %e, "Hook execution failed");
                HookResult::Continue
            }
            Err(_) => {
                warn!(hook = %hook.name, timeout_secs = 30, "Hook execution timed out");
                // Try to kill the hanging process
                let _ = child.kill().await;
                HookResult::Continue
            }
        }
```

- [ ] **Step 3: Add tests**

```rust
#[test]
fn unregister_hook() {
    let mut reg = HookRegistry::new();
    reg.register(make_hook("a", HookPoint::PreTool, 10));
    reg.register(make_hook("b", HookPoint::PreTool, 5));
    assert_eq!(reg.count(&HookPoint::PreTool), 2);

    assert!(reg.unregister("a"));
    assert_eq!(reg.count(&HookPoint::PreTool), 1);
    assert!(!reg.unregister("nonexistent"));
}

#[tokio::test]
async fn hook_timeout_kills_hanging_script() {
    let dir = tempfile::TempDir::new().unwrap();
    let script = dir.path().join("hang.sh");
    // Script that sleeps forever
    std::fs::write(&script, "#!/bin/bash\nsleep 3600").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    let mut reg = HookRegistry::new();
    reg.register(RegisteredHook {
        name: "hang".into(),
        source: "test".into(),
        script_path: Some(script),
        point: HookPoint::PostTurn,
        priority: 10,
    });

    let ctx = HookContext {
        point: HookPoint::PostTurn,
        session_id: "test".into(),
        turn_count: 1,
        tool_name: None,
        tool_input: None,
        tool_result: None,
        message: None,
        metadata: HashMap::new(),
    };

    // Should complete within ~30s timeout, not hang forever
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(35),
        reg.execute(&ctx),
    ).await;
    assert!(result.is_ok(), "Hook execution should not hang");
    assert!(matches!(result.unwrap(), HookResult::Continue));
}
```

- [ ] **Step 4: Run and commit**

```bash
cargo test -p aletheon-runtime
git add crates/aletheon-runtime/src/impl/hooks/registry.rs
git commit -m "feat(runtime): add hook unregister and 30s execution timeout"
```

---

### Task 8: MemoryRouter — dynamic backend registration

**Files:**
- Modify: `crates/aletheon-memory/src/router.rs`

- [ ] **Step 1: Refactor to dynamic backends**

Replace the `MemoryRouter` struct and its impl:

```rust
pub struct MemoryRouter {
    backends: Vec<(MemoryType, Box<dyn MemoryBackend + Send + Sync>)>,
    db_dir: std::path::PathBuf,
}

impl MemoryRouter {
    pub fn new(db_dir: &std::path::Path) -> Self {
        let mut router = Self {
            backends: Vec::new(),
            db_dir: db_dir.to_path_buf(),
        };
        // Register default backends
        router.register(MemoryType::Episodic, Box::new(EpisodicMemory::new(db_dir.join("episodic.db"))));
        router.register(MemoryType::Semantic, Box::new(SemanticMemory::new(db_dir.join("semantic.db"))));
        router.register(MemoryType::Procedural, Box::new(ProceduralMemory::new(db_dir.join("procedural.db"))));
        router.register(MemoryType::SelfMemory, Box::new(SelfMemory::new(db_dir.join("self.db"))));
        router
    }

    /// Register a memory backend for a given type.
    pub fn register(&mut self, mt: MemoryType, backend: Box<dyn MemoryBackend + Send + Sync>) {
        self.backends.push((mt, backend));
    }

    fn backend_for(&self, mt: MemoryType) -> Option<&(MemoryType, Box<dyn MemoryBackend + Send + Sync>)> {
        self.backends.iter().find(|(t, _)| *t == mt)
    }

    fn all_backends(&self) -> &[ (MemoryType, Box<dyn MemoryBackend + Send + Sync>) ] {
        &self.backends
    }
}
```

- [ ] **Step 2: Refactor MemoryBackend impl to eliminate fan-out duplication**

```rust
#[async_trait]
impl MemoryBackend for MemoryRouter {
    async fn store(&self, entry: MemoryEntry) -> Result<MemoryHandle> {
        let (_, backend) = self.backend_for(entry.memory_type)
            .ok_or_else(|| anyhow::anyhow!("No backend for {:?}", entry.memory_type))?;
        backend.store(entry).await
    }

    async fn recall(&self, query: &MemoryQuery) -> Result<Vec<MemoryEntry>> {
        if let Some(mt) = query.memory_type {
            if let Some((_, backend)) = self.backend_for(mt) {
                return backend.recall(query).await;
            }
            return Ok(vec![]);
        }

        // Fan out to all backends — unified pattern
        let mut all = Vec::new();
        for (_, backend) in &self.backends {
            match backend.recall(query).await {
                Ok(entries) => all.extend(entries),
                Err(e) => tracing::warn!(error = %e, "Backend recall failed, continuing"),
            }
        }
        all.sort_by(|a, b| b.importance.partial_cmp(&a.importance).unwrap_or(std::cmp::Ordering::Equal));
        if query.limit > 0 {
            all.truncate(query.limit);
        }
        Ok(all)
    }

    async fn list(&self, filter: &MemoryFilter) -> Result<Vec<MemoryEntry>> {
        if let Some(mt) = filter.memory_type {
            if let Some((_, backend)) = self.backend_for(mt) {
                return backend.list(filter).await;
            }
            return Ok(vec![]);
        }

        let mut all = Vec::new();
        for (_, backend) in &self.backends {
            match backend.list(filter).await {
                Ok(entries) => all.extend(entries),
                Err(e) => tracing::warn!(error = %e, "Backend list failed, continuing"),
            }
        }
        all.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        if filter.limit > 0 {
            all.truncate(filter.limit);
        }
        Ok(all)
    }

    async fn forget(&self, handle: &MemoryHandle) -> Result<()> {
        let (_, backend) = self.backend_for(handle.memory_type)
            .ok_or_else(|| anyhow::anyhow!("No backend for {:?}", handle.memory_type))?;
        backend.forget(handle).await
    }

    async fn compact(&self, strategy: CompactStrategy) -> Result<CompactResult> {
        let mut total = CompactResult::default();
        for (_, backend) in &self.backends {
            match backend.compact(strategy.clone()).await {
                Ok(r) => {
                    total.entries_before += r.entries_before;
                    total.entries_after += r.entries_after;
                    total.entries_removed += r.entries_removed;
                    total.entries_merged += r.entries_merged;
                }
                Err(e) => tracing::warn!(error = %e, "Backend compact failed, continuing"),
            }
        }
        Ok(total)
    }

    async fn stats(&self) -> Result<MemoryStats> {
        let mut by_type = HashMap::new();
        let mut total_size_bytes = 0u64;
        let mut oldest: Option<chrono::DateTime<chrono::Utc>> = None;
        let mut newest: Option<chrono::DateTime<chrono::Utc>> = None;

        for (_, backend) in &self.backends {
            match backend.stats().await {
                Ok(s) => {
                    by_type.extend(s.by_type);
                    total_size_bytes += s.total_size_bytes;
                    oldest = match (oldest, s.oldest_entry) {
                        (Some(a), Some(b)) => Some(a.min(b)),
                        (None, b) => b,
                        (a, None) => a,
                    };
                    newest = match (newest, s.newest_entry) {
                        (Some(a), Some(b)) => Some(a.max(b)),
                        (None, b) => b,
                        (a, None) => a,
                    };
                }
                Err(e) => tracing::warn!(error = %e, "Backend stats failed, continuing"),
            }
        }

        let total_entries = by_type.values().sum();
        Ok(MemoryStats { total_entries, by_type, total_size_bytes, oldest_entry: oldest, newest_entry: newest })
    }
}
```

- [ ] **Step 3: Update Subsystem impl** — adjust `init()`, `health()`, `shutdown()` to iterate `self.backends`.

- [ ] **Step 4: Run and commit**

```bash
cargo test -p aletheon-memory
git add crates/aletheon-memory/src/router.rs
git commit -m "feat(memory): dynamic backend registration, deduplicate fan-out pattern"
```

---

### Task 9: Plugin loader — proper topological sort with cycle detection

**Files:**
- Modify: `crates/aletheon-runtime/src/impl/plugin/loader.rs`

- [ ] **Step 1: Replace `resolve_dependencies` with proper topological sort**

```rust
    /// Resolve plugin dependencies with proper topological sort and cycle detection.
    pub fn resolve_dependencies(&self, plugins: &[PluginManifest]) -> Result<Vec<String>, String> {
        use std::collections::{HashMap, HashSet};

        let by_id: HashMap<&str, &PluginManifest> = plugins.iter().map(|p| (p.id.as_str(), p)).collect();

        // Check for missing non-optional dependencies
        for plugin in plugins {
            for dep in &plugin.dependencies {
                if !dep.optional && !by_id.contains_key(dep.id.as_str()) {
                    return Err(format!(
                        "Plugin '{}' requires '{}' which is not available",
                        plugin.id, dep.id
                    ));
                }
            }
        }

        // Kahn's algorithm for topological sort
        let mut in_degree: HashMap<&str, usize> = HashMap::new();
        let mut dependents: HashMap<&str, Vec<&str>> = HashMap::new();

        for plugin in plugins {
            in_degree.entry(plugin.id.as_str()).or_insert(0);
            for dep in &plugin.dependencies {
                if by_id.contains_key(dep.id.as_str()) {
                    *in_degree.entry(plugin.id.as_str()).or_insert(0) += 1;
                    dependents.entry(dep.id.as_str()).or_default().push(plugin.id.as_str());
                }
            }
        }

        let mut queue: Vec<&str> = in_degree.iter()
            .filter(|(_, &deg)| deg == 0)
            .map(|(&id, _)| id)
            .collect();
        queue.sort(); // deterministic ordering

        let mut order = Vec::new();
        let mut visited = HashSet::new();

        while let Some(id) = queue.pop() {
            if visited.contains(id) { continue; }
            visited.insert(id);
            order.push(id.to_string());

            if let Some(deps) = dependents.get(id) {
                for &dep_id in deps {
                    if let Some(deg) = in_degree.get_mut(dep_id) {
                        *deg -= 1;
                        if *deg == 0 {
                            queue.push(dep_id);
                        }
                    }
                }
            }
        }

        if order.len() != plugins.len() {
            let missing: Vec<_> = plugins.iter()
                .filter(|p| !visited.contains(p.id.as_str()))
                .map(|p| p.id.as_str())
                .collect();
            return Err(format!("Dependency cycle detected involving: {}", missing.join(", ")));
        }

        Ok(order)
    }
```

- [ ] **Step 2: Add tests for cycle detection**

```rust
#[test]
fn test_cycle_detection() {
    let loader = PluginLoader::new(vec![]);
    let plugins = vec![
        PluginManifest {
            id: "a".into(), name: "A".into(), version: "0.1.0".into(),
            entry: "cmd:./a.sh".into(),
            dependencies: vec![PluginDependency { id: "b".into(), version_req: "*".into(), optional: false }],
            ..make_manifest_default()
        },
        PluginManifest {
            id: "b".into(), name: "B".into(), version: "0.1.0".into(),
            entry: "cmd:./b.sh".into(),
            dependencies: vec![PluginDependency { id: "a".into(), version_req: "*".into(), optional: false }],
            ..make_manifest_default()
        },
    ];
    assert!(loader.resolve_dependencies(&plugins).is_err());
}
```

- [ ] **Step 3: Run and commit**

```bash
cargo test -p aletheon-runtime
git add crates/aletheon-runtime/src/impl/plugin/loader.rs
git commit -m "feat(runtime): proper topological sort with cycle detection for plugin dependencies"
```

---

### Task 10: Plugin manifest — EntryType enum

**Files:**
- Modify: `crates/aletheon-runtime/src/impl/plugin/manifest.rs`

- [ ] **Step 1: Add EntryType enum**

```rust
/// Plugin entry point type — replaces fragile string prefix parsing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "path")]
pub enum EntryType {
    /// Shell script entry point.
    Cmd(String),
    /// Native shared library.
    Native(String),
    /// WebAssembly module.
    Wasm(String),
}

impl EntryType {
    pub fn path(&self) -> &str {
        match self {
            Self::Cmd(p) | Self::Native(p) | Self::Wasm(p) => p,
        }
    }
}

impl std::str::FromStr for EntryType {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (prefix, path) = s.split_once(':')
            .ok_or_else(|| format!("Entry '{}' missing type prefix (expected 'cmd:', 'native:', or 'wasm:')", s))?;
        match prefix {
            "cmd" => Ok(Self::Cmd(path.to_string())),
            "native" => Ok(Self::Native(path.to_string())),
            "wasm" => Ok(Self::Wasm(path.to_string())),
            other => Err(format!("Unknown entry type '{}'", other)),
        }
    }
}
```

- [ ] **Step 2: Add `parsed_entry` method** to `PluginManifest`:

```rust
    /// Parse the entry string into a typed EntryType.
    pub fn parsed_entry(&self) -> Result<EntryType, String> {
        self.entry.parse()
    }
```

- [ ] **Step 3: Update validate()** to use `parsed_entry()`:

```rust
    pub fn validate(&self) -> Result<(), String> {
        if self.id.is_empty() { return Err("Plugin ID cannot be empty".into()); }
        if self.version.is_empty() { return Err("Plugin version cannot be empty".into()); }
        if self.entry.is_empty() { return Err("Plugin entry point cannot be empty".into()); }
        self.parsed_entry()?; // validate entry format
        Ok(())
    }
```

- [ ] **Step 4: Add test**

```rust
#[test]
fn test_entry_type_parsing() {
    assert_eq!("cmd:./run.sh".parse::<EntryType>().unwrap(), EntryType::Cmd("./run.sh".into()));
    assert_eq!("native:./lib.so".parse::<EntryType>().unwrap(), EntryType::Native("./lib.so".into()));
    assert!("bad_path".parse::<EntryType>().is_err());
    assert!("unknown:./x".parse::<EntryType>().is_err());
}
```

- [ ] **Step 5: Run and commit**

```bash
cargo test -p aletheon-runtime
git add crates/aletheon-runtime/src/impl/plugin/manifest.rs
git commit -m "feat(runtime): typed EntryType enum replaces fragile string prefix parsing"
```

---

### Task 11: NarrativeLayer — VecDeque for O(1) eviction

**Files:**
- Modify: `crates/aletheon-self/src/core/narrative.rs`

- [ ] **Step 1: Replace `Vec` with `VecDeque`**

Change import:
```rust
use std::collections::VecDeque;
```

Change struct:
```rust
pub struct NarrativeLayer {
    buffer: RwLock<VecDeque<NarrativeEntry>>,
    capacity: usize,
}
```

Change constructor:
```rust
    pub fn new(capacity: usize) -> Self {
        Self {
            buffer: RwLock::new(VecDeque::with_capacity(capacity)),
            capacity,
        }
    }
```

Change `record()` and `narrate()` — replace `buffer.remove(0)` with `buffer.pop_front()`:
```rust
        if buffer.len() >= self.capacity {
            buffer.pop_front();
        }
```

Change `recent()`:
```rust
    pub fn recent(&self, n: usize) -> Vec<NarrativeEntry> {
        let buffer = self.buffer.read();
        let skip = if n >= buffer.len() { 0 } else { buffer.len() - n };
        buffer.iter().skip(skip).cloned().collect()
    }
```

Change `save_to_store()`:
```rust
        for entry in buffer.iter() {
```

Change `load_from_store()`:
```rust
        let mut buffer = self.buffer.write();
        *buffer = entries.into();
```

- [ ] **Step 2: Run and commit**

```bash
cargo test -p aletheon-self
git add crates/aletheon-self/src/core/narrative.rs
git commit -m "fix(self): use VecDeque for O(1) ring buffer eviction in NarrativeLayer"
```

---

### Task 12: SelfField — Persistable trait for layer persistence

**Files:**
- Modify: `crates/aletheon-self/src/core/store.rs`
- Modify: `crates/aletheon-self/src/core/mod.rs`

- [ ] **Step 1: Add Persistable trait to store.rs**

```rust
/// Unified persistence contract for SelfField layers.
///
/// Like Linux kernel's super_operations (write_inode / drop_inode).
pub trait Persistable {
    fn table_name(&self) -> &str;
    fn save_to_store(&self, store: &SelfFieldStore) -> Result<()>;
    fn load_from_store(&mut self, store: &SelfFieldStore) -> Result<()>;
}
```

- [ ] **Step 2: Refactor `save_all` / `load_all` in mod.rs**

```rust
    pub fn save_all(&self) -> Result<()> {
        if let Some(ref store) = self.store {
            // Each layer implements Persistable — save in dependency order
            self.boundary.save_to_store(store)?;
            self.identity.save_to_store(store)?;
            self.care.save_to_store(store)?;
            self.narrative.save_to_store(store)?;
            self.mutation.save_to_store(store)?;
            self.attention.save_to_store(store)?;
            self.continuity.save_to_store(store)?;
            info!("SelfField: all layers persisted");
        }
        Ok(())
    }

    pub fn load_all(&mut self) -> Result<()> {
        if let Some(ref store) = self.store {
            self.boundary.load_from_store(store)?;
            self.identity.load_from_store(store)?;
            self.care.load_from_store(store)?;
            self.narrative.load_from_store(store)?;
            self.mutation.load_from_store(store)?;
            self.attention.load_from_store(store)?;
            self.continuity.load_from_store(store)?;
            info!("SelfField: all layers loaded");
        }
        Ok(())
    }
```

- [ ] **Step 3: Run and commit**

```bash
cargo test -p aletheon-self
git add crates/aletheon-self/src/core/store.rs crates/aletheon-self/src/core/mod.rs
git commit -m "refactor(self): add Persistable trait for unified layer persistence"
```

---

## Phase 3: Validation

### Task 13: Full test suite + clippy

- [ ] **Step 1: Run full test suite**

```bash
cargo test --workspace
```

Expected: all tests pass

- [ ] **Step 2: Run clippy**

```bash
cargo clippy --workspace -- -D warnings
```

Expected: no warnings

- [ ] **Step 3: Commit any fixes**

```bash
git add -A
git commit -m "fix: clippy and test fixes for kernel pattern adoption"
```
