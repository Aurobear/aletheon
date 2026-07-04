# Config Foundation (T1) + Dead-Code Cleanup (T2) Implementation Plan

> **For agentic workers:** Use `workflow-feature` or `plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Unify Aletheon's config layering (add `/etc` layer, fix the hooks bug, complete `merge()`, make `max_iterations=0` mean unlimited, harden the self-report prompt) and stop the orphaned perception subsystem from spamming logs / leaking memory — without deleting parked subsystems.

**Architecture:** Two `AppConfig` types (`cognit` and `runtime`) both gain a `/etc/aletheon` layer and complete `merge()`; the daemon loads one layered config and reuses it for hooks. The old `PerceptionBridge` (whose engine receiver was dropped after the Engine refactor) stops being spawned; `PerceptionManager` is gated behind a new `perception.enabled` flag (default off). The bus-based `PerceptionModule` and manager code are kept for the future T3 track.

**Tech Stack:** Rust (workspace crates: `cognit`, `runtime`, `dasein`), `toml`, `serde`, `tokio`, systemd user service.

Reference design: `docs/plans/2026-07-04-config-cleanup-refactor-design.md`
Branch: `auro/refactor/config-cleanup` (never commit to `dev`).

---

## Part T1 — Config Foundation

### Task 1: `max_iterations = 0` means unlimited

**Files:**
- Modify: `crates/cognit/src/config/mod.rs:86-88` (default), `crates/cognit/src/config/mod.rs:444-466` (tests)
- Modify: `crates/runtime/src/core/react_loop/mod.rs:159-162` (`should_continue`)
- Test: `crates/runtime/src/core/react_loop/mod.rs` (`#[cfg(test)]`)

- [ ] **Step 1: Change the compiled default to 0**

In `crates/cognit/src/config/mod.rs`, replace:

```rust
fn default_max_iterations() -> usize {
    25
}
```

with:

```rust
/// 0 means "no iteration cap" — termination then relies on the LLM stopping,
/// the circuit breaker, repeated-call detection, and the tool budget.
fn default_max_iterations() -> usize {
    0
}
```

- [ ] **Step 2: Write the failing test for `should_continue`**

In `crates/runtime/src/core/react_loop/mod.rs`, inside the existing `#[cfg(test)] mod tests`, add:

```rust
#[test]
fn max_iterations_zero_means_unlimited() {
    let mut loop_ = ReActLoop::new_for_test(); // existing test constructor
    loop_.config.max_iterations = 0;
    loop_.iteration = 10_000;
    assert!(
        loop_.should_continue(),
        "max_iterations=0 must never stop on the iteration check"
    );

    loop_.config.max_iterations = 5;
    loop_.iteration = 5;
    assert!(
        !loop_.should_continue(),
        "finite cap still stops when reached"
    );
}
```

> If there is no `new_for_test()` helper, construct `ReActLoop` the same way the
> other tests in this module do (see `max_iterations: 5` usages at
> `crates/runtime/src/core/react_loop/mod.rs:311,470,567`) and set the two fields
> directly.

- [ ] **Step 3: Run test to verify it fails**

Run: `cargo test -p runtime max_iterations_zero_means_unlimited -- --nocapture`
Expected: FAIL — with the current `self.iteration < self.config.max_iterations`, `10_000 < 0` is false.

- [ ] **Step 4: Implement the unlimited semantics**

In `crates/runtime/src/core/react_loop/mod.rs`, replace:

```rust
    /// Check if we've hit the max iterations
    pub fn should_continue(&self) -> bool {
        self.iteration < self.config.max_iterations
    }
```

with:

```rust
    /// Check if we've hit the max iterations.
    /// `max_iterations == 0` means unlimited: the loop then terminates only via
    /// LLM stop, circuit breaker, repeated-call detection, or the tool budget.
    pub fn should_continue(&self) -> bool {
        self.config.max_iterations == 0 || self.iteration < self.config.max_iterations
    }
```

- [ ] **Step 5: Run test to verify it passes**

Run: `cargo test -p runtime max_iterations_zero_means_unlimited`
Expected: PASS

- [ ] **Step 6: Commit**

```bash
git add crates/cognit/src/config/mod.rs crates/runtime/src/core/react_loop/mod.rs
git commit -m "feat(config): max_iterations=0 means unlimited (default 0)"
```

---

### Task 2: Complete `cognit::AppConfig::merge()`

**Files:**
- Modify: `crates/cognit/src/config/mod.rs:326-411` (`merge`)
- Test: `crates/cognit/src/config/mod.rs` (`#[cfg(test)] mod tests`, line ~444)

- [ ] **Step 1: Write the failing test**

Add to `crates/cognit/src/config/mod.rs` tests:

```rust
#[test]
fn merge_covers_perception_evolution_and_prompt() {
    let mut base = AppConfig::default();
    let mut other = AppConfig::default();
    other.perception.enabled = true;               // field added in Task 9
    other.agent.system_prompt = "OVERRIDDEN".into();
    other.agent.compaction_enabled = false;

    base.merge(other);

    assert!(base.perception.enabled, "perception must merge");
    assert_eq!(base.agent.system_prompt, "OVERRIDDEN");
    assert!(!base.agent.compaction_enabled);
}
```

> This test depends on `PerceptionConfig.enabled` from Task 9. If implementing
> strictly in order, temporarily assert only `system_prompt`/`compaction_enabled`
> here and add the `perception.enabled` assertion after Task 9.

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p cognit merge_covers_perception_evolution_and_prompt`
Expected: FAIL — `system_prompt`/`compaction_enabled`/`perception` are not merged today.

- [ ] **Step 3: Extend `merge()`**

In `crates/cognit/src/config/mod.rs`, inside `pub fn merge(&mut self, other: AppConfig)`, in the Agent block (after the `compaction_threshold` merge at line ~345) add:

```rust
        if other.agent.system_prompt != default_system_prompt() {
            self.agent.system_prompt = other.agent.system_prompt;
        }
        if !other.agent.compaction_enabled {
            self.agent.compaction_enabled = other.agent.compaction_enabled;
        }
```

Then, before the closing brace of `merge()` (after the Daemon block at line ~410) add:

```rust
        // Perception: override if non-default
        if other.perception.enabled {
            self.perception.enabled = other.perception.enabled;
        }
        if other.perception.watch_paths != default_perception_watch_paths() {
            self.perception.watch_paths = other.perception.watch_paths;
        }
        if !other.perception.enable_journald {
            self.perception.enable_journald = other.perception.enable_journald;
        }

        // Evolution: override if enabled downstream
        if other.evolution.enabled {
            self.evolution.enabled = other.evolution.enabled;
        }
```

> If `EvolutionSettings` field names differ from `enabled`, adjust to the actual
> field (check `crates/cognit/src/config/mod.rs` `EvolutionSettings`). Keep the
> "non-default wins" pattern.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p cognit merge_covers_perception_evolution_and_prompt`
Expected: PASS (add `perception.enabled` assertion once Task 9 is done)

- [ ] **Step 5: Commit**

```bash
git add crates/cognit/src/config/mod.rs
git commit -m "feat(config): merge perception/evolution/system_prompt/compaction layers"
```

---

### Task 3: Add `/etc/aletheon` layer to `cognit::load_layered`

**Files:**
- Modify: `crates/cognit/src/config/mod.rs:413-441`
- Test: `crates/cognit/src/config/mod.rs` tests

- [ ] **Step 1: Implement the system layer**

In `crates/cognit/src/config/mod.rs`, replace the body of `load_layered` (lines 417-441) so the layer order becomes defaults → `/etc` → user → project:

```rust
    /// Load config with layer merging (low → high precedence):
    /// - Layer 0: compiled defaults
    /// - Layer 1: /etc/aletheon/config.toml   (system defaults)
    /// - Layer 2: ~/.aletheon/config.toml     (user; authoritative for daily edits)
    /// - Layer 3: <project>/.aletheon/config.toml (project-local)
    pub fn load_layered(project_dir: Option<&Path>) -> Self {
        let mut config = Self::default();

        // Layer 1: system
        let etc_path = Path::new("/etc/aletheon/config.toml");
        if etc_path.exists() {
            if let Ok(sys_config) = Self::from_file(etc_path) {
                config.merge(sys_config);
            }
        }

        // Layer 2: user global
        let global_path = dirs::home_dir()
            .map(|h| h.join(".aletheon/config.toml"))
            .filter(|p| p.exists());
        if let Some(path) = global_path {
            if let Ok(user_config) = Self::from_file(&path) {
                config.merge(user_config);
            }
        }

        // Layer 3: project local
        if let Some(dir) = project_dir {
            let project_path = dir.join(".aletheon/config.toml");
            if project_path.exists() {
                if let Ok(project_config) = Self::from_file(&project_path) {
                    config.merge(project_config);
                }
            }
        }

        config
    }
```

- [ ] **Step 2: Add a test proving user overrides /etc**

Add to `crates/cognit/src/config/mod.rs` tests:

```rust
#[test]
fn merge_precedence_user_over_system() {
    // Unit-level proxy for layer precedence: later merge wins.
    let mut config = AppConfig::default();
    let mut system = AppConfig::default();
    system.agent.default_model = Some("system-model".into());
    let mut user = AppConfig::default();
    user.agent.default_model = Some("user-model".into());

    config.merge(system);
    config.merge(user);

    assert_eq!(config.agent.default_model.as_deref(), Some("user-model"));
}
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p cognit merge_precedence_user_over_system`
Expected: PASS

- [ ] **Step 4: Commit**

```bash
git add crates/cognit/src/config/mod.rs
git commit -m "feat(config): add /etc/aletheon system layer to cognit load_layered"
```

---

### Task 4: Mirror `/etc` layer + hooks merge in `runtime::AppConfig`

**Files:**
- Modify: `crates/runtime/src/core/config/mod.rs:71-140` (`merge`), `:164-192` (`load_layered`)
- Test: `crates/runtime/src/core/config/mod.rs` tests (~line 479)

- [ ] **Step 1: Ensure `merge()` merges hooks**

In `crates/runtime/src/core/config/mod.rs` `merge()`, add (if not present) a hooks merge. Inspect `HooksConfig`; if it is a list of hooks, append; if keyed, insert-overwrite. Example for a `Vec`-backed `hooks.entries`:

```rust
        // Hooks: append entries from higher layers
        self.hooks.entries.extend(other.hooks.entries);
```

> Adjust to the real `HooksConfig` shape. The requirement: hooks defined in any
> layer are present after merge (this is what makes the Task 5 fix meaningful).

- [ ] **Step 2: Add the `/etc` layer to runtime `load_layered`**

Apply the same three-layer body as Task 3 (defaults → `/etc` → user → project) to `crates/runtime/src/core/config/mod.rs:168-192`.

- [ ] **Step 3: Add a hooks-layering test**

```rust
#[test]
fn hooks_merge_from_layers() {
    let mut base = AppConfig::default();
    let other = AppConfig::from_str_for_test(/* toml with one hook */);
    let before = base.hooks.entries.len();
    base.merge(other);
    assert!(base.hooks.entries.len() > before, "hooks must merge across layers");
}
```

> Use whatever test constructor the module already uses; if none, parse a small
> TOML string with `toml::from_str`.

- [ ] **Step 4: Run tests**

Run: `cargo test -p runtime hooks_merge_from_layers`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add crates/runtime/src/core/config/mod.rs
git commit -m "feat(config): /etc layer + hooks merge in runtime AppConfig"
```

---

### Task 5: Fix the hooks bug (honor `--config`)

**Files:**
- Modify: `crates/runtime/src/core/runtime_core.rs:105-108`

- [ ] **Step 1: Load runtime hooks with the same path logic as the main config**

In `crates/runtime/src/core/runtime_core.rs`, replace:

```rust
            hooks: {
                let rt_config = crate::core::config::AppConfig::load_layered(None);
                rt_config.hooks
            },
```

with:

```rust
            hooks: {
                // Honor --config: hooks must come from the same file(s) as the
                // main config, not always ~/.aletheon. (Fixes the hooks bug.)
                let rt_config = if let Some(ref path) = config_path {
                    crate::core::config::AppConfig::load_or_default(path)
                } else {
                    crate::core::config::AppConfig::load_layered(None)
                };
                rt_config.hooks
            },
```

- [ ] **Step 2: Build**

Run: `cargo build -p runtime`
Expected: compiles clean.

- [ ] **Step 3: Manual verification (deferred to Task 14)** — noted; verified end-to-end there.

- [ ] **Step 4: Commit**

```bash
git add crates/runtime/src/core/runtime_core.rs
git commit -m "fix(config): hooks honor --config instead of always reading ~/.aletheon"
```

---

### Task 6: Treat `--config` as the top layer (over the standard search)

**Files:**
- Modify: `crates/runtime/src/core/runtime_core.rs:56-61`

- [ ] **Step 1: Layer the explicit file on top of the layered search**

Replace:

```rust
        let app_config = if let Some(ref path) = config_path {
            cognit::config::AppConfig::load_or_default(path)
        } else {
            cognit::config::AppConfig::load_layered(None)
        };
```

with:

```rust
        // Layered base (defaults → /etc → user → project), then --config on top.
        let mut app_config = cognit::config::AppConfig::load_layered(None);
        if let Some(ref path) = config_path {
            app_config.merge(cognit::config::AppConfig::load_or_default(path));
        }
```

> Note: with this change, `systemd --config /etc/...` still works (it becomes the
> top layer). Per the deployment decision (Task 8) the user service will *not*
> pass `--config`, letting `/etc` → `~/.aletheon` layering apply with the user
> file authoritative.

- [ ] **Step 2: Build**

Run: `cargo build -p runtime`
Expected: compiles clean.

- [ ] **Step 3: Commit**

```bash
git add crates/runtime/src/core/runtime_core.rs
git commit -m "feat(config): --config is the highest layer over the standard search"
```

---

### Task 7: Grounding directive in the default system prompt

**Files:**
- Modify: `crates/cognit/src/config/mod.rs:102-105` (`default_system_prompt`)

- [ ] **Step 1: Add the self-report grounding rule**

Replace:

```rust
fn default_system_prompt() -> String {
    "You are a helpful AI assistant with tools. Use tools when appropriate to help the user."
        .to_string()
}
```

with:

```rust
fn default_system_prompt() -> String {
    "You are a helpful AI assistant with tools. Use tools when appropriate to help the user. \
     Before stating any conclusion about your own runtime state, logs, or configuration, \
     you MUST read the actual logs and the actually-effective config file first — never guess \
     or invent an explanation."
        .to_string()
}
```

- [ ] **Step 2: Build + existing tests**

Run: `cargo test -p cognit`
Expected: PASS (adjust any test that asserts the exact old prompt string).

- [ ] **Step 3: Commit**

```bash
git add crates/cognit/src/config/mod.rs
git commit -m "feat(agent): system prompt must ground self-reports in logs/config"
```

---

### Task 8: Config content + deployment alignment (`~/.aletheon` authoritative, user service)

**Files:**
- Modify: `config/default.toml` (remove anthropic/sonnet mismatch; match compiled defaults)
- Modify: `config/aletheon.user.service` (do not pass `--config`)
- Modify: `setup.sh` (install/enable user service; write correct `~/.aletheon/config.toml`)
- Deploy-time: `~/.aletheon/config.toml` (corrected content)

- [ ] **Step 1: Make `config/default.toml` consistent with compiled defaults**

Set `[agent]` to `default_provider`/`default_model` that match the intended default
(leave provider list intact) and `max_iterations = 0`. Remove the
`claude-sonnet-4` default-model line that contradicts `/etc` and `~/.aletheon`.

- [ ] **Step 2: User systemd service must rely on layering (no `--config`)**

In `config/aletheon.user.service`, set `ExecStart` to the unified binary daemon
WITHOUT `--config`, so `load_layered` applies `/etc` then `~/.aletheon`:

```ini
[Service]
ExecStart=/usr/bin/aletheon daemon
Restart=on-failure
```

- [ ] **Step 3: `setup.sh` installs & enables the user service and writes correct user config**

Ensure `setup.sh`:
- installs `config/aletheon.user.service` to `~/.config/systemd/user/aletheon.service`
- runs `systemctl --user daemon-reload && systemctl --user enable --now aletheon`
- writes `~/.aletheon/config.toml` with the authoritative values if missing:

```toml
[agent]
default_provider = "leju"
default_model = "deepseek/deepseek-v4-pro"
max_iterations = 0
```

- [ ] **Step 4: Correct the live user config (deploy action)**

Update `~/.aletheon/config.toml` so `default_model = "deepseek/deepseek-v4-pro"`,
`default_provider = "leju"`, `max_iterations = 0` (replacing the stale flash/25 values).

- [ ] **Step 5: Verify config resolution**

```bash
aletheon daemon &   # user service style
journalctl --user -u aletheon -n 20 --no-pager | grep -i "Loaded config"
```
Expected: providers count reflects the merged config; model resolves to
`deepseek/deepseek-v4-pro`.

- [ ] **Step 6: Commit**

```bash
git add config/default.toml config/aletheon.user.service setup.sh
git commit -m "chore(deploy): user systemd service + ~/.aletheon authoritative config"
```

---

## Part T2 — Dead-Code Cleanup

### Task 9: Add `perception.enabled` flag (default false)

**Files:**
- Modify: `crates/cognit/src/config/mod.rs:285-307` (`PerceptionConfig`)

- [ ] **Step 1: Add the field with a default-false**

Replace the struct + default:

```rust
/// Perception subsystem configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PerceptionConfig {
    /// Master switch. Off by default: the perception→behavior loop is not yet
    /// wired (see roadmap §T3). When false, no watchers are spawned.
    #[serde(default)]
    pub enabled: bool,
    /// Filesystem paths to watch with inotify.
    #[serde(default = "default_perception_watch_paths")]
    pub watch_paths: Vec<String>,
    /// Whether to enable journald log monitoring.
    #[serde(default = "default_true")]
    pub enable_journald: bool,
}

impl Default for PerceptionConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            watch_paths: default_perception_watch_paths(),
            enable_journald: true,
        }
    }
}
```

- [ ] **Step 2: Build**

Run: `cargo build -p cognit`
Expected: compiles clean.

- [ ] **Step 3: Commit**

```bash
git add crates/cognit/src/config/mod.rs
git commit -m "feat(perception): add enabled flag (default off) to PerceptionConfig"
```

---

### Task 10: Stop spawning the orphaned bridge; gate the manager

**Files:**
- Modify: `crates/runtime/src/core/runtime_core.rs:172-211`
- Modify: `crates/runtime/src/impl/daemon/handler/init.rs:93-101` (remove `_perception_rx` param)
- Modify: `crates/runtime/src/impl/daemon/handler/mod.rs:68` (stale comment)

- [ ] **Step 1: Gate manager behind `enabled`, remove bridge + dead channel**

In `crates/runtime/src/core/runtime_core.rs`, replace the whole
"Perception manager + bridge" block (lines 172-211) with:

```rust
        // ── Perception manager (gated) ──────────────────────────────
        // The old PerceptionBridge fed an "Engine" that was removed; its
        // injection receiver was dropped, which caused endless
        // "Engine receiver dropped" warnings and an unbounded buffer.
        // Until the perception→behavior loop is rewired (roadmap §T3), only
        // spawn the manager when explicitly enabled, and do not spawn the
        // bridge at all.
        if app_config.perception.enabled {
            let (event_tx, mut event_rx) = mpsc::channel::<PerceptionEvent>(256);
            let perception_config = &app_config.perception;
            let watch_paths: Vec<PathBuf> = perception_config
                .watch_paths
                .iter()
                .map(PathBuf::from)
                .collect();
            let enable_journald = perception_config.enable_journald;
            tokio::spawn(async move {
                let mut manager = dasein::r#impl::perception::manager::PerceptionManager::new(
                    event_tx,
                    watch_paths,
                    enable_journald,
                );
                if let Err(e) = manager.start().await {
                    tracing::error!(error = %e, "Perception manager failed");
                }
            });
            // Drain-and-drop until §T3 wires a real consumer, so the manager's
            // sender does not back-pressure. (No behavior injection yet.)
            tokio::spawn(async move { while event_rx.recv().await.is_some() {} });
        }

        // ── RequestHandler ──────────────────────────────────────────
        info!("Creating request handler...");
        let request_handler = RequestHandler::new(
            &config,
            &registry,
            app_config.model_routing.clone(),
            app_config.evolution.enabled,
            Some(kernel_bus),
            cancel_token.clone(),
        )
        .await?;
```

> This removes `injection_tx`/`injection_rx`, the `PerceptionBridge::new`/spawn,
> and the `injection_rx` argument to `RequestHandler::new`. Remove the now-unused
> `use dasein::r#impl::perception::bridge::PerceptionInjection;` at
> `runtime_core.rs:32` if the compiler flags it. Keep the `PerceptionEvent` import.

- [ ] **Step 2: Remove the dropped `_perception_rx` parameter**

In `crates/runtime/src/impl/daemon/handler/init.rs`, delete line 98
(`_perception_rx: mpsc::Receiver<PerceptionInjection>,`) from `pub async fn new(`.
Remove the now-unused `use ... PerceptionInjection;` at `init.rs:43` if flagged.

- [ ] **Step 3: Update the stale comment**

In `crates/runtime/src/impl/daemon/handler/mod.rs:68`, update the comment that
references `set_perception_rx` to note perception is gated off pending roadmap §T3.

- [ ] **Step 4: Build**

Run: `cargo build -p runtime`
Expected: compiles clean; no unused-import warnings.

- [ ] **Step 5: Commit**

```bash
git add crates/runtime/src/core/runtime_core.rs crates/runtime/src/impl/daemon/handler/init.rs crates/runtime/src/impl/daemon/handler/mod.rs
git commit -m "fix(perception): stop spawning orphaned bridge; gate manager behind enabled flag"
```

---

### Task 11: Delete the duplicate perception bridge file

**Files:**
- Delete: `crates/dasein/src/bridge/perception.rs`
- Modify: `crates/dasein/src/bridge/mod.rs` (remove the `pub mod perception;` if present)

- [ ] **Step 1: Verify it is unreferenced**

Run:
```bash
grep -rn "bridge::perception\|bridge::PerceptionBridge" crates/ --include=*.rs | grep -v "impl/perception/bridge"
grep -rn "mod perception" crates/dasein/src/bridge/mod.rs
```
Expected: no consumer references (the live one is `impl::perception::bridge`).
If `bridge/mod.rs` declares `pub mod perception;`, that line is the only
reference and will be removed in Step 2.

- [ ] **Step 2: Delete and de-register**

```bash
git rm crates/dasein/src/bridge/perception.rs
```
Remove the `pub mod perception;` line from `crates/dasein/src/bridge/mod.rs` if present.

- [ ] **Step 3: Build**

Run: `cargo build -p dasein`
Expected: compiles clean.

- [ ] **Step 4: Commit**

```bash
git add -A crates/dasein/src/bridge/
git commit -m "refactor(dasein): remove duplicate perception bridge (dead copy)"
```

---

### Task 12: Delete the two truly-dead constants

**Files:**
- Modify: `crates/runtime/src/impl/memory/auto_memory.rs:23-26`
- Modify: `crates/runtime/src/impl/memory/fact_store/mod.rs:13-15`

- [ ] **Step 1: Remove `MAX_EXTRACTION_TOKENS`**

Delete these lines from `auto_memory.rs`:

```rust
/// Maximum tokens for the extraction LLM call.
#[allow(dead_code)]
const MAX_EXTRACTION_TOKENS: u32 = 500;
```

- [ ] **Step 2: Remove `DEFAULT_TRUST`**

Delete these lines from `fact_store/mod.rs` (leave `TRUST_MAX` and
`DEFAULT_MIN_TRUST`, which are used):

```rust
/// Default trust score for new facts — reserved for future trust-weighted retrieval.
#[allow(dead_code)]
const DEFAULT_TRUST: f64 = 0.5;
```

- [ ] **Step 3: Build**

Run: `cargo build -p runtime`
Expected: compiles clean; two fewer `dead_code` allowances.

- [ ] **Step 4: Commit**

```bash
git add crates/runtime/src/impl/memory/auto_memory.rs crates/runtime/src/impl/memory/fact_store/mod.rs
git commit -m "refactor(memory): remove unused MAX_EXTRACTION_TOKENS and DEFAULT_TRUST"
```

---

### Task 13: Annotate parked (C-class) orphans

**Files:**
- Modify: `crates/runtime/src/impl/daemon/handler/mod.rs:97,145,150` (`agent_registry`, `checkpoint_store`, `agent_loader`)
- Modify: `crates/runtime/src/core/controller.rs:6-48` (scaffold header)
- Modify: `crates/runtime/src/impl/automation/delivery.rs:13-14` (`clients`)
- Modify: `crates/runtime/src/core/checkpoint.rs:31` (`session_dir`) and `crates/runtime/src/impl/daemon/debug_handler.rs:49` (`ActiveRecording.id`)

- [ ] **Step 1: Replace each `#[allow(dead_code)]` comment with a roadmap pointer**

For each field/struct above, keep `#[allow(dead_code)]` and set the doc comment to
point at the roadmap track that will consume it, e.g. for `agent_registry`:

```rust
    /// Parked — multi-agent orchestration is unwired after the Engine removal.
    /// See docs/plans/2026-07-04-config-cleanup-refactor-design.md §5 (roadmap T3).
    #[allow(dead_code)]
    agent_registry: Arc<AgentRegistry>,
```

Use the matching track in the comment:
- `agent_registry`, `agent_loader` → roadmap T3 (multi-agent orchestration)
- `checkpoint_store`, `checkpoint.rs::session_dir` → future file-edit rewind
- `Controller` scaffold → future multi-frontend (TUI/HTTP)
- `delivery.rs::clients` → future automation/delivery
- `debug_handler.rs::ActiveRecording.id` → future debug-recording correlation

- [ ] **Step 2: Build**

Run: `cargo build -p runtime`
Expected: compiles clean; behavior unchanged.

- [ ] **Step 3: Commit**

```bash
git add crates/runtime/src/impl/daemon/handler/mod.rs crates/runtime/src/core/controller.rs crates/runtime/src/impl/automation/delivery.rs crates/runtime/src/core/checkpoint.rs crates/runtime/src/impl/daemon/debug_handler.rs
git commit -m "docs(runtime): annotate parked orphans with roadmap pointers"
```

---

### Task 14: Full validation

**Files:** none (verification only)

- [ ] **Step 1: Workspace build with warnings surfaced**

Run: `cargo build --workspace 2>&1 | tee /tmp/aletheon-build.log; grep -c "warning:" /tmp/aletheon-build.log`
Expected: build succeeds; warning count reduced vs. baseline (0 new warnings; removed `dead_code` items gone).

- [ ] **Step 2: Test suite**

Run: `cargo test --workspace`
Expected: all green (incl. `max_iterations_zero_means_unlimited`, `merge_*`, `hooks_merge_from_layers`).

- [ ] **Step 3: Clippy**

Run: `cargo clippy --workspace -- -D warnings`
Expected: no errors.

- [ ] **Step 4: Runtime smoke — no perception spam, no leak**

```bash
systemctl --user restart aletheon   # or run `aletheon daemon`
sleep 60
journalctl --user -u aletheon --since "1 min ago" --no-pager | grep -c "Engine receiver dropped"
```
Expected: `0` matches. RSS stable across the minute (`ps -o rss= -C aletheon` twice, ~unchanged).

- [ ] **Step 5: Config authority check**

Change one value in `~/.aletheon/config.toml` (e.g. `max_tokens`), restart, confirm
it takes effect in the `Loaded config` / model-router logs. Confirm a hook defined in
the authoritative config is loaded (`Loaded hooks from all layers count=N`, N>0).

- [ ] **Step 6: `max_iterations=0` behavior**

Send a multi-step task; confirm the turn ends via the circuit breaker / repeated-call
detection / natural stop, and the log does NOT show `ReActLoop hit max_iterations`.

- [ ] **Step 7: Final commit / PR**

```bash
git push -u origin auro/refactor/config-cleanup
gh pr create --base dev --title "Config foundation (T1) + dead-code cleanup (T2)" --body "Implements docs/plans/2026-07-04-config-cleanup-refactor-design.md (T1+T2). T3-T7 tracked as roadmap."
```

---

## Self-Review

- **Spec coverage:** T1 §3.1 (Tasks 3,4,6), §3.2 hooks (Task 5), §3.3 merge (Tasks 2,4),
  §3.4 dual-config minimal (Tasks 4,5 + comments), §3.5 max_iter=0 (Task 1), §3.6 prompt
  (Task 7), §3.7 deploy/content (Task 8). T2 §4.1 deletes (Tasks 11,12), §4.2 perception
  (Tasks 9,10), §4.3 parked orphans (Task 13), §4.4 security dup → roadmap (not in plan,
  by decision). Validation §6 (Task 14).
- **Placeholder scan:** No TBD/TODO. The two `> note` blocks flag real shape-dependent
  spots (`HooksConfig`, `EvolutionSettings`, test constructor) the implementer confirms
  against the code — each gives the exact pattern to apply.
- **Type consistency:** `PerceptionConfig.enabled: bool` added in Task 9 and consumed in
  Tasks 2 and 10; `should_continue` change matches the `max_iterations: usize` field;
  `config_path: Option<PathBuf>` reused consistently in Tasks 5 and 6.
