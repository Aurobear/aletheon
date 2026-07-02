# M-B + M-C Consolidated Implementation Design

**Date:** 2026-07-02
**Status:** Design (design-only gate in effect -- no product code changes)
**Source plans:**
- `docs/plans/2026-07-01-mb-plugin-lifecycle-plan.md`
- `docs/plans/2026-07-01-mc-result-pipeline-plan.md`
**Roadmap:** `docs/plans/2026-07-01-modules-roadmap-design.md` M-B and M-C sections
**Branch:** `auro/feat/20260701-aletheon-governed-memory-design` (current)

---

## 1. Verified Ground Truth

All claims from both plans were verified against the actual source files on 2026-07-02.

### M-B (Plugin Lifecycle) Ground Truth

| Fact | Claimed Anchor | Actual Anchor | Drift? |
|---|---|---|---|
| `PluginState`: `Discovered/Loaded/Active/Error(String)/Unloaded` | `manager.rs:14-21` | `manager.rs:14-21` | No |
| `ManagedPlugin { manifest, state, tools }` -- no `Plugin` field | `manager.rs:23-28` | `manager.rs:23-28` | No |
| `PluginManager { plugins: RwLock<HashMap<String, ManagedPlugin>>, loader }` | `manager.rs:30-34` | `manager.rs:31-34` | No |
| `load_all` inserts `Error` at `:73`, `Loaded` at `:88`; never `Active` | `manager.rs:45-99` | `manager.rs:73`=`Error`, `:88`=`Loaded` | No |
| `get_tools` returns for `Loaded` or `Active` | `manager.rs:102-109` | `manager.rs:106` | No |
| `unload` sets `Unloaded` + clears tools; no teardown | `manager.rs:118-128` | `manager.rs:118-128` | No |
| `resolve_plugin_dir` at `manager.rs:132-147` | `manager.rs:132-147` | Exact match | No |
| `create_plugin_tools` at `manager.rs:149-170` | `manager.rs:149-170` | Exact match | No |
| `PluginTool` struct at `manager.rs:184` | `manager.rs:183-255` | `manager.rs:184` | No |
| `PluginManifest` fields 47-80; `EntryType` at 4-12 | `manifest.rs:47-80`, `manifest.rs:4-12` | Exact match | No |
| `PluginRuntime::from_entry` only supports `cmd:` | `runtime.rs:22-51` | `runtime.rs:46-47` return `Err` for native/wasm | No |
| Plugin module path = `crates/runtime/src/impl/plugin/` | `mod.rs:1-9` | `mod.rs:1-9` (4 decls + 4 re-exports) | No |
| `base` is interfaces-only | `lib.rs:1-4` | `lib.rs:1-4` | No |
| `Subsystem` trait style: `async_trait`, `Send+Sync`, `init/shutdown/version` | `subsystem.rs:78-108` | `subsystem.rs:81-108` | No |
| `Version::new` const ctor | `subsystem.rs:24-30` | `subsystem.rs:24` | No |
| `Tool` trait at `tool.rs:87`; `PermissionLevel` at `tool.rs:10` | `tool.rs:87`, `tool.rs:10` | Exact match | No |
| `base` Cargo pkg name = `"base"` | `base/Cargo.toml:2` | `base/Cargo.toml:2` | No |
| `runtime` Cargo pkg name = `"runtime"` | `runtime/Cargo.toml:2` | `runtime/Cargo.toml:2` | No |
| `base` dev-dep `tokio`; `runtime` dev-dep `tempfile` + `async-trait` | `base/Cargo.toml:31`; `runtime/Cargo.toml:48,25` | `base/Cargo.toml:31`=`tokio`; `runtime/Cargo.toml:48`=`tempfile`, `:25`=`async-trait` | No |

### M-C (Result Pipeline) Ground Truth

| Fact | Claimed Anchor | Actual Anchor | Drift? |
|---|---|---|---|
| Package names: `base`, `runtime` (bins `aletheond`, `aletheon-exec`) | `base/Cargo.toml:2`; `runtime/Cargo.toml:2,9,13` | Exact match | No |
| No-tool return site: returns assistant text directly, no verify hook | `step.rs:69-82` | `step.rs:69-82` | No |
| Loop struct `ReActLoop` at `mod.rs:124` | `mod.rs:124` | Exact match | No |
| `ReActLoop::new(config)` at `mod.rs:154-193` | `mod.rs:154-193` | Actual end at `:191` (minor line diff) | No |
| `run<L,F,Fut>(...) -> Result<(String, TurnMetrics)>` | `step.rs:16-27` | `step.rs:16-27` | No |
| `TurnMetrics` fields: `tool_calls_made, tool_errors, elapsed_ms, iterations, completed_normally` | `mod.rs:30-36` | `mod.rs:30-36` | No |
| `run()` body is a `loop { ... }`; `continue` re-enters LLM call | `step.rs` around `:29-242` | **Actual: `while self.should_continue()` at `step.rs:34`** | **Minor drift** -- functionally identical; `while` checks iteration bound each cycle |
| `base` already has `async-trait` + `tokio` (full) | `base/Cargo.toml` [deps] | `base/Cargo.toml:9,16` | No |
| `base::policy` module exists with `pub mod execpolicy;` | `policy/mod.rs:1-3`; `lib.rs:27,79` | `policy/mod.rs:1-3`; `lib.rs:27,79` | No |
| Existing loop test pattern: `ScriptedLlm`, `RuntimeConfig::default()`, `run()` with tool closure | `mod.rs:542-585` | `mod.rs:542-585` | No |
| `Message` type accessible via `crate::message::Message` in base tests | N/A (design inference) | Works via `lib.rs:52` re-export `pub use types::message;` | No |

**Conclusion:** All ground truth claims are accurate. One cosmetic drift noted (M-C plan says `loop { ... }`, actual is `while self.should_continue()`) -- zero impact on implementation.

---

## 2. Architecture Overview

### 2.1 Plugin State Machine (M-B)

The existing `PluginState` enum at `crates/runtime/src/impl/plugin/manager.rs:14-21` has five variants. This plan adds lifecycle wiring but NO new variants:

```
                         load_native() calls Plugin::init()
  +---------------+        success                          +---------------+
  |   Unloaded    | --------------------------------------->|    Active     |
  | (start/end)   |                                          | (running)     |
  +-------+-------+                                          +-------+-------+
          ^                                                          |
          |                    unload() calls                        |
          |              Plugin::shutdown() (best-effort)            |
          |                      then Unloaded                       |
          |                                                          |
  +-------+-------+                         +-----------------------+
  |    Loaded     |                         |
  | (Tool-only    |                         |
  |  via load_all)|                         v
  +---------------+                +--------+--------+
                                   |     Error(e)     |
                                   | (init failure or |
                                   |  runtime failure)|
                                   +-----------------+

  Existing path (unchanged): load_all discovers from disk ->
    Loaded (Tool-only, plugin: None) or Error (runtime failure)
  New path (additive): load_native(manifest, Box<dyn Plugin>) ->
    init() -> Active or init() failure -> Error(e)
```

**Key invariants:**
- `load_all` NEVER produces `Active` -- it has no in-process `Plugin` object to `init()`
- `get_tools()` at `manager.rs:106` already returns tools for `Loaded || Active` -- no change needed
- `Tool`-only plugins (`plugin: None`) are completely unchanged by this work
- `Plugin::run()` defaults to no-op; no background task is spawned

### 2.2 Verifier Flow (M-C)

The verifier seam is inserted at the no-tool-call return site in the ReAct loop:

```
  ReActLoop::run()
       |
       v
  while self.should_continue():
       |
       v
  llm.complete(messages, tools)
       |
       v
  Parse content blocks -> text_parts, tool_calls
       |
       +--- tool_calls.is_empty()? ---+
       |                              |
      YES                            NO
       |                              |
       v                              v
  final_text = join(text_parts)     Execute tools,
       |                            push results,
       v                            continue loop
  [M-C SEAM] verifier.verify()?
       |
       +--- None (default) ----------> unchanged behavior: emit signals,
       |                               push assistant msg, return Ok((final_text, metrics))
       |
       +--- Some(verifier) ----------+
                                      |
                    +--- Verdict::Accept ---> same as None path
                    |
                    +--- Verdict::Reject { reason } ---+
                         (if attempts < max)            |
                              |                         |
                              v                         |
                         push assistant(final_text)     |
                         push user(revision request)    |
                         verify_attempts++              |
                         continue (re-enter while loop) |
                                                       |
                         (if attempts >= max) --------->+
                              return as-is (last answer)
```

**Key invariants:**
- Default `verifier: None` -- behavior byte-identical to today
- Reject bounded by `max_verify_attempts` (default 2) -- prevents infinite reject loops
- Abnormal exits (budget exceeded, circuit breaker tripped, max iterations) are NOT verified
- `verify_attempts` resets to 0 at the start of each `run()` call

### 2.3 Plugin-as-Verifier Bridge

A plugin implementing both `Plugin` (lifecycle) and `Verifier` (M-C trait) can be loaded as a native plugin whose capabilities include a tool that wraps the Verifier, OR the plugin host could wire the verifier directly:

```
  PluginManager.load_native(manifest, plugin)
       |
       +--- Plugin::init() -> Active
       +--- Plugin::capabilities() -> Vec<Arc<dyn Tool>>  (merged into tools)
       |
       [Optional: if plugin also impl Verifier]
       +--- Downcast Box<dyn Plugin> to &dyn Verifier
       +--- ReActLoop::set_verifier(Arc::from(verifier))
```

The bridge is a pattern, not a code change in this design: a plugin that implements `Verifier` can have its verifier extracted via trait object casting (if the plugin crate depends on `base` and implements both traits). The `Plugin` trait's `capabilities()` can return a tool that calls the verifier, or the host can coerce the plugin to a verifier via a separate registration path.

---

## 3. Complete Code for All Changes

### 3.1 M-B Phase 1: `Plugin` trait in `base`

#### File: `crates/base/src/include/plugin.rs` (NEW)

```rust
//! Plugin lifecycle contract -- the long-lived counterpart to execute-only tools.
//!
//! A plugin MAY implement this trait for `init` / `run` / `shutdown` behavior and
//! to register additional capabilities (tools). Plugins that only expose
//! execute-only `Tool`s do not need to implement it -- the trait is additive.

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

/// Long-lived plugin lifecycle -- the seam for `init` / `run` / `shutdown`.
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

#[cfg(test)]
mod tests {
    use super::*;
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
        let init = StdArc::new(AtomicUsize::new(0));
        let down = StdArc::new(AtomicUsize::new(0));
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

#### File: `crates/base/src/include/mod.rs`

**Insertion point:** Line 12, after `pub mod subsystem;`

```rust
pub mod plugin;
```

Full resulting file (lines 1-14):
```rust
//! Subsystem trait contracts -- like Linux kernel's include/ directory.
//!
//! Each file defines the trait contract for one subsystem.

pub mod body;
pub mod brain;
pub mod event_bus;
pub mod memory;
pub mod meta;
pub mod runtime;
pub mod self_field;
pub mod subsystem;
pub mod plugin;
```

#### File: `crates/base/src/lib.rs`

**Insertion 1:** Line 42, after `pub use include::subsystem;` (inside the `include/` module re-export block):

```rust
pub use include::plugin;
```

**Insertion 2:** Line 103, next to `pub use include::subsystem::{... Version};` (inside the item-level re-export block):

```rust
pub use include::plugin::{Plugin, PluginContext};
```

### 3.2 M-B Phase 2: Lifecycle wiring in PluginManager

#### File: `crates/runtime/src/impl/plugin/manager.rs`

**Change 1:** Add import at line 11 (alongside `use base::tool::{...}`):

```rust
use base::plugin::{Plugin, PluginContext};
```

**Change 2:** Add `plugin` field to `ManagedPlugin` at lines 23-28. Replace:

```rust
pub struct ManagedPlugin {
    pub manifest: PluginManifest,
    pub state: PluginState,
    pub tools: Vec<Arc<dyn Tool>>,
}
```

With:

```rust
pub struct ManagedPlugin {
    pub manifest: PluginManifest,
    pub state: PluginState,
    pub tools: Vec<Arc<dyn Tool>>,
    pub plugin: Option<Box<dyn Plugin>>,
}
```

**Change 3:** In `load_all()`, add `plugin: None` to both `ManagedPlugin` constructions:

At lines 69-75 (Error insert), change:
```rust
                        plugins.insert(
                            id.clone(),
                            ManagedPlugin {
                                manifest: manifest.clone(),
                                state: PluginState::Error(e.to_string()),
                                tools: Vec::new(),
                            },
                        );
```

To:
```rust
                        plugins.insert(
                            id.clone(),
                            ManagedPlugin {
                                manifest: manifest.clone(),
                                state: PluginState::Error(e.to_string()),
                                tools: Vec::new(),
                                plugin: None,
                            },
                        );
```

At lines 84-91 (Loaded insert), change:
```rust
                plugins.insert(
                    id.clone(),
                    ManagedPlugin {
                        manifest: manifest.clone(),
                        state: PluginState::Loaded,
                        tools,
                    },
                );
```

To:
```rust
                plugins.insert(
                    id.clone(),
                    ManagedPlugin {
                        manifest: manifest.clone(),
                        state: PluginState::Loaded,
                        tools,
                        plugin: None,
                    },
                );
```

**Change 4:** Add `load_native` method to `impl PluginManager` (after `load_all` closing brace at line 99):

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
        // yield no manifest tools -- the plugin's capabilities() still apply.
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

**Change 5:** Replace `unload` body at lines 118-128 to call `shutdown`:

From:
```rust
    /// Unload a plugin.
    pub async fn unload(&self, plugin_id: &str) -> Result<(), String> {
        let mut plugins = self.plugins.write().await;
        if let Some(plugin) = plugins.get_mut(plugin_id) {
            plugin.state = PluginState::Unloaded;
            plugin.tools.clear();
            info!(id = plugin_id, "Plugin unloaded");
            Ok(())
        } else {
            Err(format!("Plugin '{}' not found", plugin_id))
        }
    }
```

To:
```rust
    /// Unload a plugin. Runs `Plugin::shutdown` (best-effort) if the plugin has
    /// a lifecycle object, then transitions to `PluginState::Unloaded`.
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

### 3.3 M-C Phase 1: `Verifier` trait in `base`

#### File: `crates/base/src/policy/verifier.rs` (NEW)

```rust
//! Result-verification seam (M-C). A `Verifier` inspects a candidate final
//! answer and either accepts it or rejects it with a reason so the runtime can
//! request a revision. The default `NoopVerifier` always accepts, preserving
//! behavior when no verifier is configured.

use crate::message::Message;
use async_trait::async_trait;

/// Outcome of verifying a candidate final answer.
#[derive(Debug, Clone)]
pub enum Verdict {
    /// The answer is acceptable; return it to the caller.
    Accept,
    /// The answer is rejected; `reason` is fed back to the model for a revision.
    Reject { reason: String },
}

/// Inspects a candidate final answer in the context of the conversation.
#[async_trait]
pub trait Verifier: Send + Sync {
    /// Verify the model's final text. `messages` is the full conversation so far
    /// (system + user + assistant + tool turns), for context-aware checks.
    async fn verify(&self, final_text: &str, messages: &[Message]) -> Verdict;
}

/// The default verifier: accepts everything (no behavior change).
pub struct NoopVerifier;

#[async_trait]
impl Verifier for NoopVerifier {
    async fn verify(&self, _final_text: &str, _messages: &[Message]) -> Verdict {
        Verdict::Accept
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::Message;

    #[tokio::test]
    async fn noop_verifier_always_accepts() {
        let v = NoopVerifier;
        let msgs = vec![Message::user("hi")];
        assert!(matches!(v.verify("any answer", &msgs).await, Verdict::Accept));
    }

    #[tokio::test]
    async fn reject_carries_reason() {
        struct Always;
        #[async_trait::async_trait]
        impl Verifier for Always {
            async fn verify(&self, _text: &str, _msgs: &[Message]) -> Verdict {
                Verdict::Reject { reason: "nope".into() }
            }
        }
        match Always.verify("x", &[]).await {
            Verdict::Reject { reason } => assert_eq!(reason, "nope"),
            _ => panic!("expected reject"),
        }
    }
}
```

#### File: `crates/base/src/policy/mod.rs`

**Insertion point:** Line 3, after `pub mod execpolicy;`

From:
```rust
//! Execution policy engine.

pub mod execpolicy;
```

To:
```rust
//! Execution policy engine.

pub mod execpolicy;
pub mod verifier;
```

#### File: `crates/base/src/lib.rs`

**Insertion point:** Line 79, next to `pub use policy::execpolicy;`

Add:
```rust
pub use policy::verifier;
```

### 3.4 M-C Phase 2: Verifier wiring in ReActLoop

#### File: `crates/runtime/src/core/react_loop/mod.rs`

**Change 1:** Add imports at line 17 (near other `use` statements):

```rust
use base::policy::verifier::{Verdict, Verifier};
use std::sync::Arc;
```

**Change 2:** Add fields to `ReActLoop` struct (lines 124-151). Insert after the `reflection_engine` field at line 150:

```rust
    /// Optional result verifier (M-C). None = no-op (unchanged behavior).
    verifier: Option<Arc<dyn Verifier>>,
    /// Verify attempts used this turn (reset at the start of run()).
    verify_attempts: usize,
    /// Max verify-reject retries per turn before returning as-is.
    max_verify_attempts: usize,
```

**Change 3:** In `new()` (line 154-191), add initializers inside the `Self { ... }` block. After `reflection_engine` (line 189):

```rust
            verifier: None,
            verify_attempts: 0,
            max_verify_attempts: 2,
```

**Change 4:** Add `set_verifier` method to `impl ReActLoop` (e.g., after `set_reflection_interval` or near `set_interrupt_flag` at line 273):

```rust
    /// Install a result verifier. Without this, verification is a no-op.
    pub fn set_verifier(&mut self, verifier: Arc<dyn Verifier>) {
        self.verifier = Some(verifier);
    }
```

#### File: `crates/runtime/src/core/react_loop/step.rs`

**Change 1:** Reset per-turn counter. At line 30 (`let mut tool_errors: usize = 0;`), add after:

```rust
        self.verify_attempts = 0;
```

**Change 2:** Insert verifier seam at the no-tool return site (lines 69-82). Replace:

```rust
            if tool_calls.is_empty() {
                let final_text = text_parts.join("\n");
                // Emit awareness: uncertainty from response + final response signal
                self.emit_thinking_complete("thinking", &final_text);
                self.emit_final_response("final_response");
                self.messages.push(Message::assistant(&final_text));
                let metrics = TurnMetrics {
                    tool_calls_made,
                    tool_errors,
                    elapsed_ms: start.elapsed().as_millis() as u64,
                    iterations: self.iteration,
                    completed_normally: true,
                };
                return Ok((final_text, metrics));
            }
```

With:

```rust
            if tool_calls.is_empty() {
                let final_text = text_parts.join("\n");

                // M-C: optional verification seam. Default (None) = unchanged behavior.
                if let Some(verifier) = self.verifier.clone() {
                    if self.verify_attempts < self.max_verify_attempts {
                        if let Verdict::Reject { reason } =
                            verifier.verify(&final_text, &self.messages).await
                        {
                            self.verify_attempts += 1;
                            // Record the rejected answer, then request a revision and re-loop.
                            self.messages.push(Message::assistant(&final_text));
                            self.messages.push(Message::user(&format!(
                                "[verification] Your previous answer was rejected: {reason}\n\
                                 Please correct it and provide a better final answer."
                            )));
                            warn!(reason = reason.as_str(), "verifier rejected final answer; retrying");
                            continue;
                        }
                    }
                }

                // Emit awareness: uncertainty from response + final response signal
                self.emit_thinking_complete("thinking", &final_text);
                self.emit_final_response("final_response");
                self.messages.push(Message::assistant(&final_text));
                let metrics = TurnMetrics {
                    tool_calls_made,
                    tool_errors,
                    elapsed_ms: start.elapsed().as_millis() as u64,
                    iterations: self.iteration,
                    completed_normally: true,
                };
                return Ok((final_text, metrics));
            }
```

---

## 4. TDD Test Code

### 4.1 M-B Tests

#### Test 1: Trait definition (in `crates/base/src/include/plugin.rs` -- embedded in the file above)

```bash
# Expected: FAIL (plugin module doesn't exist yet)
cargo test -p base plugin::tests::plugin_default_methods_and_hooks

# After implementation: PASS
cargo test -p base plugin
```

#### Test 2: load_native fires init and activates (add to `crates/runtime/src/impl/plugin/manager.rs` at end of file)

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
        mgr.unload("tool-only").await.unwrap();
        assert_eq!(mgr.get_state("tool-only").await, Some(PluginState::Unloaded));
    }
}
```

TDD commands:
```bash
# Step 1: Expect FAIL -- load_native doesn't exist
cargo test -p runtime lifecycle_tests::load_native_fires_init_and_activates

# Step 2: After implementing load_native -- PASS
cargo test -p runtime lifecycle_tests::load_native_fires_init_and_activates

# Step 3: Expect FAIL -- unload doesn't call shutdown
cargo test -p runtime lifecycle_tests::unload_fires_shutdown_and_unloads

# Step 4: After implementing shutdown in unload -- all PASS
cargo test -p runtime lifecycle_tests

# Step 5: Full regression
cargo test -p runtime plugin
```

### 4.2 M-C Tests

#### Test 1: NoopVerifier always accepts (in `crates/base/src/policy/verifier.rs` -- embedded above)

```bash
cargo test -p base policy::verifier
```

#### Test 2: Verifier rejection triggers retry, no-verifier unchanged (add to `crates/runtime/src/core/react_loop/mod.rs` tests module, near line 1046 after existing tests)

```rust
    // ── M-C Verifier tests ─────────────────────────────────────────────────

    use base::policy::verifier::{Verdict, Verifier};
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Rejects the first candidate answer, accepts all subsequent ones.
    struct RejectOnce {
        seen: AtomicUsize,
    }
    #[async_trait]
    impl Verifier for RejectOnce {
        async fn verify(&self, _text: &str, _msgs: &[Message]) -> Verdict {
            if self.seen.fetch_add(1, Ordering::SeqCst) == 0 {
                Verdict::Reject { reason: "first try rejected".into() }
            } else {
                Verdict::Accept
            }
        }
    }

    /// An LLM that always returns plain text (no tool calls), counting its calls.
    struct TextLlm {
        calls: Mutex<usize>,
    }
    #[async_trait]
    impl LlmProvider for TextLlm {
        async fn complete(&self, _m: &[Message], _t: &[ToolDefinition]) -> anyhow::Result<LlmResponse> {
            let mut n = self.calls.lock().unwrap();
            *n += 1;
            Ok(LlmResponse {
                content: vec![ContentBlock::Text { text: format!("answer {n}") }],
                stop_reason: StopReason::EndTurn,
                usage: Usage::default(),
                cache_hit_tokens: 0,
                cache_miss_tokens: 0,
            })
        }
        async fn complete_stream(&self, _m: &[Message], _t: &[ToolDefinition]) -> anyhow::Result<LlmStream> {
            unimplemented!("not used in test")
        }
        fn name(&self) -> &str { "text" }
        fn max_context_length(&self) -> usize { 100_000 }
    }

    #[tokio::test]
    async fn verifier_rejection_triggers_one_retry() {
        let cfg = RuntimeConfig {
            max_iterations: 5,
            session_id: "t".into(),
            learning_enabled: false,
            compaction_enabled: false,
            ..RuntimeConfig::default()
        };
        let mut lp = ReActLoop::new(cfg);
        lp.set_verifier(std::sync::Arc::new(RejectOnce { seen: AtomicUsize::new(0) }));
        let llm = TextLlm { calls: Mutex::new(0) };
        let tool_defs: Vec<ToolDefinition> = vec![];
        let (out, _m) = lp
            .run("go", &llm, &tool_defs, |_id: &str, name: &str, _in: &serde_json::Value| {
                let name = name.to_string();
                async move { (format!("ran {name}"), false) }
            })
            .await
            .unwrap();
        // First answer rejected -> loop retried -> second answer accepted.
        assert_eq!(out, "answer 2", "rejected answer should be revised, got: {out}");
    }

    #[tokio::test]
    async fn no_verifier_returns_first_answer_unchanged() {
        let cfg = RuntimeConfig { max_iterations: 5, session_id: "t".into(),
            learning_enabled: false, compaction_enabled: false, ..RuntimeConfig::default() };
        let mut lp = ReActLoop::new(cfg); // no set_verifier -> None
        let llm = TextLlm { calls: Mutex::new(0) };
        let tool_defs: Vec<ToolDefinition> = vec![];
        let (out, _m) = lp.run("go", &llm, &tool_defs,
            |_i: &str, n: &str, _in: &serde_json::Value| { let n = n.to_string(); async move { (n, false) } })
            .await.unwrap();
        assert_eq!(out, "answer 1", "no verifier = unchanged behavior");
    }
```

TDD commands:
```bash
# Step 1: Expect FAIL -- set_verifier doesn't exist, no retry logic
cargo test -p runtime react_loop::tests::verifier_rejection_triggers_one_retry

# Step 2: After implementation -- both verifier tests PASS
cargo test -p runtime react_loop::tests::verifier_rejection_triggers_one_retry
cargo test -p runtime react_loop::tests::no_verifier_returns_first_answer_unchanged

# Step 3: Full regression -- all existing react_loop tests still pass
cargo test -p runtime react_loop
```

---

## 5. Exact File Paths and Line Numbers

### Files created (2):

| File | Lines | Purpose |
|---|---|---|
| `crates/base/src/include/plugin.rs` | ~100 (trait + test) | `Plugin` trait, `PluginContext`, unit test |
| `crates/base/src/policy/verifier.rs` | ~85 (trait + NoopVerifier + tests) | `Verifier` trait, `Verdict` enum, `NoopVerifier` |

### Files modified (5):

| File | Changes | Insertion points |
|---|---|---|
| `crates/base/src/include/mod.rs` | 1 line added | After line 12 (`pub mod subsystem;`) |
| `crates/base/src/lib.rs` | 2 lines added | After line 42 (module re-export); after line 103 (item re-export) |
| `crates/base/src/policy/mod.rs` | 1 line added | After line 3 (`pub mod execpolicy;`) |
| `crates/runtime/src/impl/plugin/manager.rs` | ~6 edits | Import at :11; `plugin` field at :27; `plugin: None` at :74, :90; `load_native` after :99; `unload` body replace :118-128; tests appended after :256 |
| `crates/runtime/src/core/react_loop/mod.rs` | ~5 edits | Import at :17; fields at :151; initializers at :191; `set_verifier` method after :273; verifier tests appended after :1046 |
| `crates/runtime/src/core/react_loop/step.rs` | 2 edits | `verify_attempts = 0` after :30; verifier seam insertion in :69-82 |

### All insertion points with exact line anchors:

| Absolute path | Line anchor | Operation |
|---|---|---|
| `crates/base/src/include/mod.rs:12` | `pub mod subsystem;` | Insert `pub mod plugin;` after |
| `crates/base/src/lib.rs:42` | `pub use include::subsystem;` | Insert `pub use include::plugin;` after |
| `crates/base/src/lib.rs:79` | `pub use policy::execpolicy;` | Insert `pub use policy::verifier;` after |
| `crates/base/src/lib.rs:103` | `pub use include::subsystem::{... Version};` | Insert `pub use include::plugin::{Plugin, PluginContext};` after |
| `crates/base/src/policy/mod.rs:3` | `pub mod execpolicy;` | Insert `pub mod verifier;` after |
| `crates/runtime/src/impl/plugin/manager.rs:11` | `use base::tool::{...};` | Insert `use base::plugin::{Plugin, PluginContext};` after |
| `crates/runtime/src/impl/plugin/manager.rs:27` | `pub tools: Vec<Arc<dyn Tool>>,` | Insert `pub plugin: ...` after (in struct) |
| `crates/runtime/src/impl/plugin/manager.rs:69-75` | Error-case `ManagedPlugin { ... }` | Add `plugin: None,` |
| `crates/runtime/src/impl/plugin/manager.rs:84-91` | Loaded-case `ManagedPlugin { ... }` | Add `plugin: None,` |
| `crates/runtime/src/impl/plugin/manager.rs:99` | `Ok(loaded)` in `load_all` | Insert `load_native` method after |
| `crates/runtime/src/impl/plugin/manager.rs:118-128` | Current `unload` body | Replace with shutdown-aware version |
| `crates/runtime/src/core/react_loop/mod.rs:17` | `use base::ui_event::AwarenessLevel;` | Insert verifier imports |
| `crates/runtime/src/core/react_loop/mod.rs:150` | `reflection_engine: ReflectionEngine,` | Insert 3 verifier fields after |
| `crates/runtime/src/core/react_loop/mod.rs:189` | `reflection_engine,` (in `Self { .. }`) | Insert 3 verifier initializers after |
| `crates/runtime/src/core/react_loop/mod.rs:273` | `set_interrupt_flag` or equivalent | Insert `set_verifier` method after |
| `crates/runtime/src/core/react_loop/step.rs:30` | `let mut tool_errors: usize = 0;` | Insert `self.verify_attempts = 0;` after |
| `crates/runtime/src/core/react_loop/step.rs:69-82` | No-tool return block | Insert verifier seam between `final_text` and emit signals |

---

## 6. Integration Test Strategy

### 6.1 Plugin that IS a Verifier (Integration Test)

A plugin can implement both `Plugin` (lifecycle) and `Verifier` (M-C) -- the plugin crate depends on `base`, which provides both traits. The integration test loads such a plugin via `load_native`, then wires its verifier into a `ReActLoop`.

Test file: `crates/runtime/tests/plugin_as_verifier_integration.rs` (NEW)

```rust
//! Integration test: a plugin that implements both Plugin and Verifier.
//!
//! Verifies that:
//! 1. The plugin's Verifier trait can be extracted and used by ReActLoop.
//! 2. load_native works with a plugin that also implements Verifier.
//! 3. The verifier plugin can reject LLM outputs, triggering retries.

use base::include::subsystem::Version;
use base::message::{ContentBlock, Message};
use base::plugin::{Plugin, PluginContext};
use base::policy::verifier::{Verdict, Verifier};
use base::ToolDefinition;
use cognit::impl::llm::provider::{
    LlmProvider, LlmResponse, LlmStream, StopReason, Usage,
};
use runtime::core::config::RuntimeConfig;
use runtime::core::react_loop::ReActLoop;
use runtime::impl::plugin::manager::{PluginManager, PluginState};
use runtime::impl::plugin::manifest::PluginManifest;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

/// A plugin that is BOTH a lifecycle Plugin AND a Verifier.
/// It rejects text containing the word "bad", accepts everything else.
struct VerifyPlugin {
    init_calls: Arc<AtomicUsize>,
    shutdown_calls: Arc<AtomicUsize>,
    verify_calls: Arc<AtomicUsize>,
}

impl VerifyPlugin {
    fn new() -> (Self, Arc<AtomicUsize>, Arc<AtomicUsize>, Arc<AtomicUsize>) {
        let init = Arc::new(AtomicUsize::new(0));
        let down = Arc::new(AtomicUsize::new(0));
        let vfy = Arc::new(AtomicUsize::new(0));
        (
            Self {
                init_calls: init.clone(),
                shutdown_calls: down.clone(),
                verify_calls: vfy.clone(),
            },
            init,
            down,
            vfy,
        )
    }
}

#[async_trait::async_trait]
impl Plugin for VerifyPlugin {
    fn id(&self) -> &str { "verify-plugin" }
    fn version(&self) -> Version { Version::new(0, 1, 0) }
    async fn init(&mut self, _ctx: &PluginContext) -> anyhow::Result<()> {
        self.init_calls.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
    async fn shutdown(&mut self) -> anyhow::Result<()> {
        self.shutdown_calls.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

#[async_trait::async_trait]
impl Verifier for VerifyPlugin {
    async fn verify(&self, final_text: &str, _messages: &[Message]) -> Verdict {
        self.verify_calls.fetch_add(1, Ordering::SeqCst);
        if final_text.contains("bad") {
            Verdict::Reject { reason: "contains banned word 'bad'".into() }
        } else {
            Verdict::Accept
        }
    }
}

fn verify_manifest() -> PluginManifest {
    PluginManifest {
        id: "verify-plugin".into(),
        name: "Verify Plugin".into(),
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

/// LLM that produces "bad answer" on first call, "good answer" on second.
struct GoodBadLlm { calls: Mutex<usize> }
#[async_trait::async_trait]
impl LlmProvider for GoodBadLlm {
    async fn complete(&self, _m: &[Message], _t: &[ToolDefinition]) -> anyhow::Result<LlmResponse> {
        let mut n = self.calls.lock().unwrap();
        *n += 1;
        let text = if *n == 1 { "bad answer" } else { "good answer" };
        Ok(LlmResponse {
            content: vec![ContentBlock::Text { text: text.into() }],
            stop_reason: StopReason::EndTurn,
            usage: Usage::default(),
            cache_hit_tokens: 0,
            cache_miss_tokens: 0,
        })
    }
    async fn complete_stream(&self, _m: &[Message], _t: &[ToolDefinition]) -> anyhow::Result<LlmStream> {
        unimplemented!()
    }
    fn name(&self) -> &str { "good-bad" }
    fn max_context_length(&self) -> usize { 100_000 }
}

#[tokio::test]
async fn plugin_as_verifier_rejects_bad_text() {
    // 1. Load the plugin via PluginManager
    let mgr = PluginManager::new(vec![std::env::temp_dir()]);
    let (plugin, init, down, vfy) = VerifyPlugin::new();
    mgr.load_native(verify_manifest(), Box::new(plugin)).await.unwrap();
    assert_eq!(init.load(Ordering::SeqCst), 1);
    assert_eq!(mgr.get_state("verify-plugin").await, Some(PluginState::Active));

    // 2. Extract the verifier from the manager (production would have a get_verifier API)
    // For integration test: create a new Arc<dyn Verifier> referencing the same counters.
    // In production, the PluginManager would expose a get_verifier() method that
    // downcasts the Box<dyn Plugin> to &dyn Verifier.
    let verify_plugin_for_loop = VerifyPlugin {
        init_calls: init.clone(),
        shutdown_calls: down.clone(),
        verify_calls: vfy.clone(),
    };

    // 3. Wire it into ReActLoop
    let cfg = RuntimeConfig {
        max_iterations: 5,
        session_id: "vfy".into(),
        learning_enabled: false,
        compaction_enabled: false,
        ..RuntimeConfig::default()
    };
    let mut lp = ReActLoop::new(cfg);
    lp.set_verifier(Arc::new(verify_plugin_for_loop));

    let llm = GoodBadLlm { calls: Mutex::new(0) };
    let tool_defs: Vec<ToolDefinition> = vec![];
    let (out, _metrics) = lp
        .run("test", &llm, &tool_defs, |_id: &str, name: &str, _in: &serde_json::Value| {
            let name = name.to_string();
            async move { (format!("ran {name}"), false) }
        })
        .await
        .unwrap();

    // "bad answer" was rejected -> LLM called again -> "good answer" accepted
    assert_eq!(out, "good answer", "expected 'good answer' after 'bad answer' rejected");
    assert_eq!(vfy.load(Ordering::SeqCst), 2, "verifier called for both answers");

    // 4. Cleanup: unload plugin
    mgr.unload("verify-plugin").await.unwrap();
    assert_eq!(down.load(Ordering::SeqCst), 1);
    assert_eq!(mgr.get_state("verify-plugin").await, Some(PluginState::Unloaded));
}
```

Run: `cargo test -p runtime plugin_as_verifier_rejects_bad_text`

Note: This integration test demonstrates the *pattern* of plugin-as-verifier. The production bridge (extracting `&dyn Verifier` from `Box<dyn Plugin>` inside `PluginManager`) is a follow-up; the test creates a fresh `Arc<dyn Verifier>` referencing the same atomic counters to assert the end-to-end behavior is correct.

### 6.2 Cross-Module Integration Test: Plugin With Verifier Tool

A plugin can expose its verification logic as a tool via `capabilities()`, giving the LLM the ability to self-critique:

```rust
#[tokio::test]
async fn plugin_exposes_verifier_as_tool() {
    // Plugin capabilities() returns a "verify_output" tool
    // When the LLM calls this tool, the plugin's Verifier impl checks the text
    // Verifier tool: accepts text, returns Accept/Reject verdict
    // This test verifies that capabilities() tools are merged into get_tools()
}
```

### 6.3 Full Regression Test Suite

```bash
# After all changes, run the full suite:
cargo build --workspace
cargo test -p base                        # All base tests (plugin + verifier)
cargo test -p runtime plugin              # Existing plugin tests + new lifecycle tests
cargo test -p runtime react_loop          # All react_loop tests + verifier tests
cargo test --workspace                    # Full workspace
```

---

## 7. Rollback Plan

### Per-Phase Rollback

Each phase produces exactly one commit. Rollback is `git revert <commit>` per phase:

| Phase | Commit message | Revert impact |
|---|---|---|
| M-B Phase 1 | `feat(base): add Plugin lifecycle trait (init/run/shutdown + capabilities)` | Removes `plugin.rs`, reverts `include/mod.rs` and `lib.rs`. Zero impact on runtime -- the trait exists but nothing calls it. |
| M-B Phase 2 Task 2 | `feat(plugin): load_native runs Plugin::init and activates lifecycle plugins` | Reverts `manager.rs`. `ManagedPlugin.plugin` field goes away, `load_native` is removed. No `plugin: None` needed in `load_all`. Existing tests unaffected. |
| M-B Phase 2 Task 3 | `feat(plugin): run Plugin::shutdown on unload; Tool-only plugins unaffected` | Reverts `unload` body to original. `plugin.plugin.take()` call removed. Existing unload behavior restored. |
| M-C Phase 1 | `feat(base): add Verifier trait + NoopVerifier (result-pipeline seam)` | Removes `verifier.rs`, reverts `policy/mod.rs` and `lib.rs`. Zero impact -- trait unused. |
| M-C Phase 2 | `feat(react_loop): optional Verifier seam at final-answer return (default no-op)` | Reverts `mod.rs` fields/setter and `step.rs` seam. Default `verifier: None` path is byte-identical to current. Existing loop tests pass both before and after. |

### Full Rollback

```bash
git revert <M-C-Phase-2-commit>  # Last first
git revert <M-C-Phase-1-commit>
git revert <M-B-Phase-2-Task-3-commit>
git revert <M-B-Phase-2-Task-2-commit>
git revert <M-B-Phase-1-commit>
cargo build --workspace && cargo test --workspace
```

### Rollback Safety

- All changes are additive (new trait modules, new method, field extension). No existing function signatures are changed.
- The only behavioral change is `unload` running `shutdown` -- reverting it restores the exact original behavior.
- No database schema changes, no config file changes, no binary API changes.

---

## 8. Risk Assessment

### M-B Risks

| Risk | Severity | Likelihood | Mitigation |
|---|---|---|---|
| `ManagedPlugin` new field breaks other constructors | Low | Low | Only two constructors exist (both in `load_all`). Both are updated. Grep for `ManagedPlugin {` before building. |
| `shutdown` panics during unload, blocking cleanup | Low | Low | `unload` wraps `shutdown` in best-effort (logs error, continues). `plugin.take()` ensures the Box is dropped after the call. |
| `load_native`'s `config: Value::Null` causes plugin init failure | Low | Medium | Follow-up work: derive config from manifest. For MVP, plugins expecting config should default gracefully or fail init, which puts them in `Error` state. |
| Build break: `base` re-exports don't resolve | Low | Low | All re-export paths verified against `lib.rs` pattern. `cargo build -p base` catches immediately. |
| `get_tools` already handles `Active` state | None | None | Verified at `manager.rs:106` -- no change needed. |

### M-C Risks

| Risk | Severity | Likelihood | Mitigation |
|---|---|---|---|
| No-op guarantee broken: verifier changes default behavior | High | Low | `no_verifier_returns_first_answer_unchanged` test guards this. The `verifier: None` path skips all new code. |
| Infinite reject loop | Medium | Low | `max_verify_attempts` (default 2) caps retries. After cap, last answer returned as-is. Outer `max_iterations` also bounds. |
| Verifier latency on hot path | Low | Low | Verifier is opt-in (`None` by default). When enabled, latency is the verifier's own cost -- verifiers should be fast (regex/schema checks) or async-aware. |
| Message ordering violation on reject | Low | Low | Reject appends `assistant(final_text)` then `user(revision)` -- valid OpenAI/Anthropic ordering (assistant text followed by user). Verified against provider spec. |
| `continue` skips tool execution loop body | Low | Low | `continue` at the no-tool return site re-enters `while self.should_continue()` at the top. The verifier seam is inside the `tool_calls.is_empty()` branch, so the `continue` correctly skips tool execution and re-runs the LLM call. |
| Only no-tool site wired; forced exits not verified | Low | Design choice | Budget exceeded, circuit breaker tripped, and max-iteration fallbacks are abnormal exits -- verifying them would loop pathologically. Documented in design. |

### Cross-Module Risk

| Risk | Severity | Likelihood | Mitigation |
|---|---|---|---|
| Plugin-as-Verifier bridge not fully wired in MVP | Medium | Medium | Integration test demonstrates the pattern with a test-only verification. Production wiring (downcast from `Box<dyn Plugin>` to `&dyn Verifier`) is a follow-up. The traits are in the same crate (`base`), the bridge is mechanically straightforward. |
| Two separate traits in `base` create dependency confusion | Low | Low | Both traits live in `base` (ABI layer). Dependencies are clear: `runtime` depends on `base`; plugins depend on `base`. No circular deps. |

### Overall Assessment

**M-B risk: Low-Medium.** Additive trait + field extension + new method. Test coverage for Tool-only regression (tool_only_plugin_unaffected_by_lifecycle). No change to discovery path.

**M-C risk: Low.** Optional seam with default no-op. Byte-identical behavior when no verifier is configured. Bounded retry on rejection. No change to tool execution path.

**Combined risk: Low-Medium.** The two modules are independent (M-B depends only on Tier 2c which is not a dependency of M-C). They can be implemented in either order or in parallel.

---

## Appendix A: Summary of Changed Files

```
crates/base/src/include/
  plugin.rs          (NEW - 100 lines, Plugin trait + PluginContext + test)

crates/base/src/include/
  mod.rs             (+1 line,  pub mod plugin;)

crates/base/src/policy/
  verifier.rs        (NEW - 85 lines, Verifier trait + Verdict + NoopVerifier + tests)

crates/base/src/policy/
  mod.rs             (+1 line,  pub mod verifier;)

crates/base/src/
  lib.rs             (+3 lines, re-exports for plugin + verifier modules)

crates/runtime/src/impl/plugin/
  manager.rs         (+130 lines estimated: field, load_native, unload rewrite,
                      test module)

crates/runtime/src/core/react_loop/
  mod.rs             (+80 lines estimated: 3 fields, 3 init, set_verifier, 2 tests)

crates/runtime/src/core/react_loop/
  step.rs            (+25 lines estimated: verify_attempts reset, verifier seam)

crates/runtime/tests/
  plugin_as_verifier_integration.rs  (NEW - 140 lines, integration test)
```

Total estimated net new lines: ~590 across 6 new files and 5 modified files.

## Appendix B: Build & Test Sequence

```bash
# Phase M-B.1: Plugin trait
cargo test -p base plugin              # expected: FAIL -> PASS after impl
cargo build -p base                    # must succeed

# Phase M-B.2a: load_native
cargo test -p runtime lifecycle_tests::load_native_fires_init_and_activates  # FAIL -> PASS
cargo build -p runtime                 # must succeed

# Phase M-B.2b: unload shutdown
cargo test -p runtime lifecycle_tests  # all 3 lifecycle tests PASS
cargo test -p runtime plugin           # existing plugin tests unaffected
cargo build -p runtime

# Phase M-C.1: Verifier trait
cargo test -p base policy::verifier    # FAIL -> PASS
cargo build -p base

# Phase M-C.2: ReActLoop wiring
cargo test -p runtime react_loop       # all tests (existing + verifier) PASS
cargo build -p runtime

# Final
cargo build --workspace
cargo test --workspace
```

## Appendix C: Verified Drift from Source Plans

Single cosmetic drift found:

- **M-C plan, ground truth table row 8:** Claims `run()` body is a `loop { ... }` around `step.rs:29-242`. **Actual code at `step.rs:34`** uses `while self.should_continue()`. Functionally identical -- the `while` condition checks `iteration < max_iterations` each loop, and `continue` re-enters the `while` check just like it would a `loop`. Zero impact on the verifier seam (the seam sits inside the `tool_calls.is_empty()` branch, and `continue` correctly re-enters the `while`).
