# M-B — Plugin Lifecycle Trait — Implementation Plan

> **For agentic workers:** Use `workflow-feature` or `plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking. **Design-only handoff — do not execute product changes until the design-only gate is lifted.**

**Goal:** Give plugins a long-lived `init/run/shutdown` lifecycle. Today capability plugins are surfaced only as execute-only `Tool`s; there is no hook a plugin can implement to run setup on load and teardown on unload. Add an additive `Plugin` trait in `base` and wire `init` on load / `shutdown` on unload into the existing `PluginManager`, tracked by the existing `PluginState`, without breaking `Tool`-only plugins.

**Architecture:** Additive trait + lifecycle wiring only. The trait lives in `base` (the interfaces-only crate). `runtime`'s `PluginManager` gains an optional per-plugin `Box<dyn Plugin>` and a `load_native` entry point that calls `init` and transitions the plugin to `PluginState::Active`; `unload` calls `shutdown` before transitioning to `PluginState::Unloaded`. Plugins with no `Plugin` instance (the current `cmd:`/`native:`/`wasm:` `Tool`-only path via `load_all`) are unchanged — they simply carry `plugin: None`.

**Tech Stack:** Rust (Cargo workspace), `async-trait`, `tokio`, `base::tool::Tool`, existing `PluginManager`/`PluginState`/`PluginRuntime`.

**Spec:** `docs/plans/2026-07-01-modules-roadmap-design.md` § "M-B. Plugin lifecycle trait".

**Branch:** `auro/feat/20260701-aletheon-plugin-lifecycle` (own branch per repo policy).

---

## Ground truth (verified 2026-07-01)

| Fact | Anchor |
|---|---|
| `PluginState` machine: `Discovered / Loaded / Active / Error(String) / Unloaded` | `crates/runtime/src/impl/plugin/manager.rs:14-21` |
| `ManagedPlugin { manifest, state, tools: Vec<Arc<dyn Tool>> }` (no `Plugin` field today) | `crates/runtime/src/impl/plugin/manager.rs:23-28` |
| `PluginManager { plugins: RwLock<HashMap<String, ManagedPlugin>>, loader }` | `crates/runtime/src/impl/plugin/manager.rs:30-34` |
| `load_all` inserts `ManagedPlugin{ state: Loaded, .. }` (and `Error` on runtime failure) — never `Active` | `crates/runtime/src/impl/plugin/manager.rs:45-99` (state set at `:73`, `:88`) |
| `get_tools` returns tools for `Loaded` **or** `Active` plugins | `crates/runtime/src/impl/plugin/manager.rs:102-109` |
| `unload` sets `PluginState::Unloaded` + clears tools; no teardown hook | `crates/runtime/src/impl/plugin/manager.rs:118-128` |
| `resolve_plugin_dir(&manifest) -> PathBuf` finds the plugin's dir | `crates/runtime/src/impl/plugin/manager.rs:132-147` |
| `create_plugin_tools(&manifest, runtime) -> Vec<Arc<dyn Tool>>` builds `PluginTool`s | `crates/runtime/src/impl/plugin/manager.rs:149-170` |
| Capability plugins are execute-only `Tool`s (`PluginTool`) | `crates/runtime/src/impl/plugin/manager.rs:183-255` (struct at `:184`) |
| Manifest carries `cmd:`/`native:`/`wasm:` entries; `PluginManifest` shape | `crates/runtime/src/impl/plugin/manifest.rs:47-80`; `EntryType` at `:4-12` |
| `PluginRuntime::from_entry` only supports `cmd:`; `native:`/`wasm:` return `Err` | `crates/runtime/src/impl/plugin/runtime.rs:22-51` |
| Plugin module path is `crates/runtime/src/impl/plugin/` (NOT `src/plugin/`) | `crates/runtime/src/impl/plugin/mod.rs:1-9` |
| `base` is the interfaces-only crate; subsystem traits live under `include/` | `crates/base/src/lib.rs:1-27`; `crates/base/src/include/mod.rs:5-12` |
| Sibling lifecycle trait style: `#[async_trait] pub trait Subsystem { init/health/shutdown/version }` | `crates/base/src/include/subsystem.rs:78-108` |
| `Version::new(major,minor,patch)` const ctor, re-exported as `base::Version` | `crates/base/src/include/subsystem.rs:24-30`; `crates/base/src/lib.rs:103` |
| `Tool` trait + `PermissionLevel` live in `base::types::tool` (re-exported `base::tool`) | `crates/base/src/types/tool.rs:87` (Tool), `:10` (PermissionLevel); `crates/base/src/lib.rs:60` |
| Cargo package names: base crate = `base`, runtime crate = `runtime` | `crates/base/Cargo.toml` `name = "base"`; `crates/runtime/Cargo.toml` `name = "runtime"` |
| Test deps present: `base` has `tokio` dev-dep; `runtime` has `tempfile` + `async-trait` | `crates/base/Cargo.toml:31`; `crates/runtime/Cargo.toml:48`, `:25` |

---

## Design decisions (made for this plan)

1. **Trait is additive and optional.** A plugin *may* implement `Plugin` for long-lived behavior. `Tool`-only plugins (today's `load_all` path) keep working untouched; they carry `plugin: None`.
2. **New `load_native` entry point rather than overloading `load_all`.** `load_all` discovers external `cmd:`/`native:`/`wasm:` manifests from disk and has no in-process Rust object to call `init` on. A dedicated `load_native(manifest, Box<dyn Plugin>)` is the clean, testable seam for an in-process `Plugin` and keeps the disk-discovery path unchanged. Registering native plugins into `load_all`'s discovery flow is an explicit follow-up (out of scope).
3. **Lifecycle maps onto the existing `PluginState`.** `init` success → `PluginState::Active`; `init` failure → `PluginState::Error(e)`; `unload` runs `shutdown` (best-effort, logged on error) then → `PluginState::Unloaded`. No new state variants.
4. **`capabilities()` merges into `tools`.** A `Plugin` may register additional `Arc<dyn Tool>`s via `capabilities()`; `load_native` appends them to the manifest-declared tools so `get_tools` surfaces both. Default impl returns empty.
5. **`run` is a default no-op.** The `run` hook is defined for future long-lived plugins but defaults to `Ok(())`; wiring a background `run` task is out of scope (no daemon task spawn in this plan).

---

## File map

| File | Change |
|---|---|
| `crates/base/src/include/plugin.rs` | **new** — `Plugin` trait + `PluginContext` |
| `crates/base/src/include/mod.rs` | add `pub mod plugin;` |
| `crates/base/src/lib.rs` | add `pub use include::plugin;` + `pub use include::plugin::{Plugin, PluginContext};` |
| `crates/runtime/src/impl/plugin/manager.rs` | add `plugin: Option<Box<dyn Plugin>>` to `ManagedPlugin`; add `load_native`; call `shutdown` in `unload`; set `plugin: None` in `load_all`'s two inserts |

Default checks per phase: `cargo build -p base` / `cargo test -p base`, then `cargo build -p runtime` / `cargo test -p runtime`. Each phase ends with a commit.

---

## Phase 1 — Define the `Plugin` trait in `base`

### Task 1: Add `Plugin` trait + `PluginContext`

**Files:** Create `crates/base/src/include/plugin.rs`; modify `crates/base/src/include/mod.rs`, `crates/base/src/lib.rs`.

- [ ] **Step 1: Write the failing test**

Add a test module at the bottom of the new `crates/base/src/include/plugin.rs` that exercises a sample `Plugin` impl (this pins the trait's method set + default methods):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    struct SamplePlugin {
        init_calls: Arc<AtomicUsize>,
        shutdown_calls: Arc<AtomicUsize>,
    }

    #[async_trait::async_trait]
    impl Plugin for SamplePlugin {
        fn id(&self) -> &str {
            "sample"
        }
        fn version(&self) -> crate::include::subsystem::Version {
            crate::include::subsystem::Version::new(0, 1, 0)
        }
        async fn init(&mut self, _ctx: &PluginContext) -> anyhow::Result<()> {
            self.init_calls.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
        async fn shutdown(&mut self) -> anyhow::Result<()> {
            self.shutdown_calls.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    #[tokio::test]
    async fn plugin_default_methods_and_hooks() {
        let init = Arc::new(AtomicUsize::new(0));
        let down = Arc::new(AtomicUsize::new(0));
        let mut p = SamplePlugin {
            init_calls: init.clone(),
            shutdown_calls: down.clone(),
        };
        let ctx = PluginContext {
            plugin_id: "sample".into(),
            working_dir: std::path::PathBuf::from("."),
            config: serde_json::Value::Null,
        };
        // default run() is a no-op; capabilities() defaults empty
        assert!(p.run().await.is_ok());
        assert!(p.capabilities().is_empty());
        p.init(&ctx).await.unwrap();
        p.shutdown().await.unwrap();
        assert_eq!(init.load(Ordering::SeqCst), 1);
        assert_eq!(down.load(Ordering::SeqCst), 1);
        assert_eq!(p.id(), "sample");
    }
}
```

- [ ] **Step 2: Run — expected FAIL** (module `plugin`, trait `Plugin`, and `PluginContext` do not exist yet → does not compile).

Run: `cargo test -p base plugin::tests::plugin_default_methods_and_hooks`
Expected: FAIL (compile error: unresolved `Plugin` / `PluginContext`).

- [ ] **Step 3: Implement the trait**

Write the trait at the top of `crates/base/src/include/plugin.rs` (above the test module), mirroring the `Subsystem` style at `include/subsystem.rs:78-108`:

```rust
//! Plugin lifecycle contract — the long-lived counterpart to execute-only tools.
//!
//! A plugin MAY implement this trait for `init` / `run` / `shutdown` behavior and
//! to register additional capabilities (tools). Plugins that only expose
//! execute-only `Tool`s do not need to implement it — the trait is additive.

use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;

use crate::include::subsystem::Version;
use crate::types::tool::Tool;

/// Context handed to a plugin at `init`.
///
/// Kept intentionally small: the plugin's id, the directory its manifest lives
/// in, and its parsed configuration (from the manifest / host).
pub struct PluginContext {
    pub plugin_id: String,
    pub working_dir: std::path::PathBuf,
    pub config: serde_json::Value,
}

/// Long-lived plugin lifecycle — the seam for `init` / `run` / `shutdown`.
///
/// The host (`PluginManager`) calls `init` on load and `shutdown` on unload,
/// tracked by the existing `PluginState`. `run` is an optional long-lived hook
/// that defaults to a no-op.
#[async_trait]
pub trait Plugin: Send + Sync {
    /// Stable plugin identifier (matches the manifest `id`).
    fn id(&self) -> &str;

    /// Plugin version, for ABI/compatibility checks.
    fn version(&self) -> Version;

    /// Called once when the plugin is loaded. Set up resources here.
    async fn init(&mut self, ctx: &PluginContext) -> Result<()>;

    /// Optional long-lived behavior. Defaults to a no-op so `Tool`-only and
    /// short-lived plugins need not implement it.
    async fn run(&mut self) -> Result<()> {
        Ok(())
    }

    /// Called once when the plugin is unloaded. Flush and release resources here.
    async fn shutdown(&mut self) -> Result<()>;

    /// Additional capabilities (tools) this plugin registers. Defaults to none;
    /// the host merges these into the plugin's execute-only tool set.
    fn capabilities(&self) -> Vec<Arc<dyn Tool>> {
        Vec::new()
    }
}
```

Register the module in `crates/base/src/include/mod.rs` (alongside the existing `pub mod` entries at `:5-12`):

```rust
pub mod plugin;
```

Re-export from `crates/base/src/lib.rs`. Add to the `pub use include::*;` block (near `:35-42`):

```rust
pub use include::plugin;
```

and to the item-level re-export block (near `:103`, next to the `subsystem` re-export):

```rust
pub use include::plugin::{Plugin, PluginContext};
```

- [ ] **Step 4: Run — expected PASS.**

Run: `cargo test -p base plugin`
Expected: PASS. Also `cargo build -p base` compiles.

- [ ] **Step 5: Commit**

```bash
git add crates/base/src/include/plugin.rs crates/base/src/include/mod.rs crates/base/src/lib.rs
git commit -m "feat(base): add Plugin lifecycle trait (init/run/shutdown + capabilities)"
```

---

## Phase 2 — Wire lifecycle into `PluginManager`

### Task 2: Add `load_native` — `init` fires on load, plugin becomes `Active`, capabilities surface

**Files:** Modify `crates/runtime/src/impl/plugin/manager.rs`.

- [ ] **Step 1: Write the failing test**

Add a test module at the bottom of `crates/runtime/src/impl/plugin/manager.rs`:

```rust
#[cfg(test)]
mod lifecycle_tests {
    use super::*;
    use base::plugin::{Plugin, PluginContext};
    use base::include::subsystem::Version;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc as StdArc;

    struct SamplePlugin {
        init_calls: StdArc<AtomicUsize>,
        shutdown_calls: StdArc<AtomicUsize>,
    }

    #[async_trait::async_trait]
    impl Plugin for SamplePlugin {
        fn id(&self) -> &str {
            "sample"
        }
        fn version(&self) -> Version {
            Version::new(0, 1, 0)
        }
        async fn init(&mut self, _ctx: &PluginContext) -> anyhow::Result<()> {
            self.init_calls.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
        async fn shutdown(&mut self) -> anyhow::Result<()> {
            self.shutdown_calls.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    fn sample_manifest() -> PluginManifest {
        PluginManifest {
            id: "sample".into(),
            name: "Sample".into(),
            version: "0.1.0".into(),
            description: String::new(),
            author: String::new(),
            entry: "cmd:./noop.sh".into(),
            tools: vec![],
            hooks: vec![],
            dependencies: vec![],
            min_agent_version: None,
            permissions: vec![],
            plugin_permissions: None,
        }
    }

    #[tokio::test]
    async fn load_native_fires_init_and_activates() {
        let mgr = PluginManager::new(vec![std::env::temp_dir()]);
        let init = StdArc::new(AtomicUsize::new(0));
        let down = StdArc::new(AtomicUsize::new(0));
        let plugin = Box::new(SamplePlugin {
            init_calls: init.clone(),
            shutdown_calls: down.clone(),
        });
        mgr.load_native(sample_manifest(), plugin).await.unwrap();
        assert_eq!(init.load(Ordering::SeqCst), 1, "init must fire once on load");
        assert_eq!(mgr.get_state("sample").await, Some(PluginState::Active));
    }
}
```

- [ ] **Step 2: Run — expected FAIL** (`load_native` does not exist).

Run: `cargo test -p runtime lifecycle_tests::load_native_fires_init_and_activates`
Expected: FAIL (no method `load_native`).

- [ ] **Step 3: Implement `load_native` + the `plugin` field**

Add the trait imports near the top of `manager.rs` (alongside `use base::tool::{...}` at `:11`):

```rust
use base::plugin::{Plugin, PluginContext};
```

Add the field to `ManagedPlugin` (`:23-28`):

```rust
pub struct ManagedPlugin {
    pub manifest: PluginManifest,
    pub state: PluginState,
    pub tools: Vec<Arc<dyn Tool>>,
    pub plugin: Option<Box<dyn Plugin>>, // NEW: long-lived lifecycle object, if any
}
```

Set `plugin: None` in **both** `ManagedPlugin` constructions inside `load_all` (the `Error` insert at `:69-76` and the `Loaded` insert at `:84-91`) — this keeps the disk-discovery `Tool`-only path unchanged.

Add the new method to `impl PluginManager` (e.g. after `load_all`):

```rust
/// Load an in-process plugin that implements the `Plugin` lifecycle trait.
/// Calls `init` and, on success, tracks it as `PluginState::Active`, merging
/// any capabilities it registers into its tool set. `Tool`-only plugins keep
/// using `load_all` and are unaffected.
pub async fn load_native(
    &self,
    manifest: PluginManifest,
    mut plugin: Box<dyn Plugin>,
) -> Result<(), anyhow::Error> {
    let plugin_dir = self.resolve_plugin_dir(&manifest);

    // Manifest-declared execute-only tools still work (best-effort; a missing
    // command only warns inside from_entry). Native/unsupported runtimes just
    // yield no manifest tools — the plugin's capabilities() still apply.
    let mut tools: Vec<Arc<dyn Tool>> =
        match PluginRuntime::from_entry(&manifest.entry, &plugin_dir) {
            Ok(rt) => self.create_plugin_tools(&manifest, rt),
            Err(_) => Vec::new(),
        };

    let ctx = PluginContext {
        plugin_id: manifest.id.clone(),
        working_dir: plugin_dir,
        config: serde_json::Value::Null,
    };

    let id = manifest.id.clone();
    let mut plugins = self.plugins.write().await;

    match plugin.init(&ctx).await {
        Ok(()) => {
            tools.extend(plugin.capabilities());
            info!(id = id.as_str(), "Plugin init complete; activating");
            plugins.insert(
                id,
                ManagedPlugin {
                    manifest,
                    state: PluginState::Active,
                    tools,
                    plugin: Some(plugin),
                },
            );
            Ok(())
        }
        Err(e) => {
            warn!(id = id.as_str(), error = %e, "Plugin init failed");
            plugins.insert(
                id,
                ManagedPlugin {
                    manifest,
                    state: PluginState::Error(e.to_string()),
                    tools: Vec::new(),
                    plugin: None,
                },
            );
            Err(e)
        }
    }
}
```

- [ ] **Step 4: Run — expected PASS.**

Run: `cargo test -p runtime lifecycle_tests::load_native_fires_init_and_activates`
Expected: PASS. Also `cargo build -p runtime` and `cargo test -p runtime plugin` compile/pass (existing manifest/loader tests unaffected).

- [ ] **Step 5: Commit**

```bash
git add crates/runtime/src/impl/plugin/manager.rs
git commit -m "feat(plugin): load_native runs Plugin::init and activates lifecycle plugins"
```

### Task 3: `shutdown` fires on unload; `Tool`-only plugins unaffected

**Files:** Modify `crates/runtime/src/impl/plugin/manager.rs`.

- [ ] **Step 1: Write the failing tests** (append to the `lifecycle_tests` module from Task 2)

```rust
#[tokio::test]
async fn unload_fires_shutdown_and_unloads() {
    let mgr = PluginManager::new(vec![std::env::temp_dir()]);
    let init = StdArc::new(AtomicUsize::new(0));
    let down = StdArc::new(AtomicUsize::new(0));
    let plugin = Box::new(SamplePlugin {
        init_calls: init.clone(),
        shutdown_calls: down.clone(),
    });
    mgr.load_native(sample_manifest(), plugin).await.unwrap();
    mgr.unload("sample").await.unwrap();
    assert_eq!(down.load(Ordering::SeqCst), 1, "shutdown must fire once on unload");
    assert_eq!(mgr.get_state("sample").await, Some(PluginState::Unloaded));
}

#[tokio::test]
async fn tool_only_plugin_unaffected_by_lifecycle() {
    // A genuine Tool-only plugin loaded via the unchanged disk-discovery path:
    // no Plugin instance, so unload must NOT attempt shutdown and must still work.
    let dir = tempfile::tempdir().unwrap();
    let pdir = dir.path().join("tool-only");
    std::fs::create_dir_all(&pdir).unwrap();
    std::fs::write(
        pdir.join("plugin.toml"),
        r#"
id = "tool-only"
name = "Tool Only"
version = "0.1.0"
entry = "cmd:./run.sh"

[[tools]]
name = "echo"
description = "echo tool"
input_schema = {}
permission_level = "L0"
"#,
    )
    .unwrap();

    let mgr = PluginManager::new(vec![dir.path().to_path_buf()]);
    mgr.load_all().await.unwrap();
    assert_eq!(mgr.get_state("tool-only").await, Some(PluginState::Loaded));
    assert!(
        mgr.get_tools().await.iter().any(|t| t.name() == "echo"),
        "Tool-only plugin's tool must still surface"
    );
    // Unload must succeed with no Plugin instance (no shutdown to run).
    mgr.unload("tool-only").await.unwrap();
    assert_eq!(mgr.get_state("tool-only").await, Some(PluginState::Unloaded));
}
```

- [ ] **Step 2: Run — expected FAIL** (`unload` does not run `shutdown`; `shutdown_calls` stays 0).

Run: `cargo test -p runtime lifecycle_tests::unload_fires_shutdown_and_unloads`
Expected: FAIL (`assert_eq!(down, 1)` fails — shutdown never called). The `tool_only_plugin_unaffected_by_lifecycle` test already passes with the Task-2 `plugin: None` wiring; running it here guards against regression.

- [ ] **Step 3: Call `shutdown` in `unload`**

Replace the body of `unload` (`:118-128`) so it runs the lifecycle hook before transitioning state:

```rust
/// Unload a plugin. Runs `Plugin::shutdown` (best-effort) if the plugin has a
/// lifecycle object, then transitions to `PluginState::Unloaded`.
pub async fn unload(&self, plugin_id: &str) -> Result<(), String> {
    let mut plugins = self.plugins.write().await;
    if let Some(plugin) = plugins.get_mut(plugin_id) {
        if let Some(mut lifecycle) = plugin.plugin.take() {
            if let Err(e) = lifecycle.shutdown().await {
                warn!(id = plugin_id, error = %e, "Plugin shutdown failed");
            }
        }
        plugin.state = PluginState::Unloaded;
        plugin.tools.clear();
        info!(id = plugin_id, "Plugin unloaded");
        Ok(())
    } else {
        Err(format!("Plugin '{}' not found", plugin_id))
    }
}
```

> `plugin.plugin.take()` moves the `Box<dyn Plugin>` out so `shutdown` gets `&mut self`; a `Tool`-only plugin (`plugin: None`) skips the call entirely, preserving current behavior.

- [ ] **Step 4: Run — expected PASS.**

Run: `cargo test -p runtime lifecycle_tests`
Expected: all three lifecycle tests PASS. Also `cargo test -p runtime plugin` (manifest/loader tests) and `cargo build -p runtime`.

- [ ] **Step 5: Commit**

```bash
git add crates/runtime/src/impl/plugin/manager.rs
git commit -m "feat(plugin): run Plugin::shutdown on unload; Tool-only plugins unaffected"
```

---

## Self-review checklist (done at plan-write time)

- **Spec coverage:** trait in `base` (Task 1) ↔ M-B "Add a `Plugin` trait in `base` (`init/run/shutdown` + capability registration)"; `init` on load / `shutdown` on unload around `PluginState` (Tasks 2–3) ↔ M-B "the trait is additive; `PluginRuntime` calls `init` on load, `shutdown` on unload, tracked by the existing `PluginState`"; `Tool`-only unaffected + init/shutdown fire (Task 3 tests) ↔ M-B test criterion.
- **Non-goals honored:** no WASM host work (`native:`/`wasm:` still return `Err` from `PluginRuntime::from_entry:46-47`); no new plugins shipped; `run` left as a default no-op (no background task spawn).
- **Type consistency:** `Plugin::capabilities() -> Vec<Arc<dyn Tool>>` matches `ManagedPlugin.tools: Vec<Arc<dyn Tool>>` (`manager.rs:27`) and `PluginTool: Tool` (`manager.rs:194`); `Version::new` const ctor exists (`subsystem.rs:24-30`); `PluginManifest` literal in tests matches its real fields (`manifest.rs:47-80`).
- **Package names:** trait crate = `base` (`cargo test -p base`), wiring crate = `runtime` (`cargo test -p runtime`) — verified against each `Cargo.toml` `[package] name`.
- **Placeholder scan:** none — real trait, real manager methods, real tests, exact `cargo` commands.

## Risks / notes for the implementer

- **`load_all` discovery is NOT wired to native plugins.** This plan adds `load_native` as the in-process seam; registering a `Box<dyn Plugin>` from the disk-discovery flow (matching a manifest `id` to a native factory) is a deliberate follow-up. Do not try to instantiate `dyn Plugin` inside `load_all` — there is no in-process object for `cmd:`/`native:`/`wasm:` manifests.
- **`base` must stay implementation-free.** `base` is interfaces-only (`lib.rs:1-4`). Add only the trait + `PluginContext` (a plain data struct) to `plugin.rs`; do not pull runtime logic into `base`.
- **`ManagedPlugin` gained a field.** Any other constructor of `ManagedPlugin` (currently only the two in `load_all`) must set `plugin:` — grep `ManagedPlugin {` before building to catch new call sites.
- **`shutdown` is best-effort.** `unload` logs and continues on `shutdown` error so a misbehaving plugin cannot wedge unload; if a stricter contract is wanted later, surface the error through the return type (out of scope here).
- **`get_tools` already includes `Active`.** No change needed there — it returns tools for `Loaded` **or** `Active` (`manager.rs:106`), so `load_native`'s `Active` plugins surface their tools automatically.
- **`config` is `Value::Null` for now.** `load_native` passes an empty config; threading manifest-derived config into `PluginContext.config` is a natural follow-up once a config source is defined.
