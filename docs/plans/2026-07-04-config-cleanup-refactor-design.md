# Design: Config Foundation (T1) + Dead-Code Cleanup (T2) + Refactoring Roadmap

Date: 2026-07-04
Status: Approved (design), pending implementation plan
Branch: `auro/refactor/config-cleanup` (to be created; never commit to `dev`)

## 1. Context & Problem

Aletheon carries an **unfinished refactoring**: an `Engine` abstraction was removed
and its consumers were never rewired. The result is a mix of orphaned subsystems,
duplicated modules, and three diverging config files. This surfaced when the agent
gave a wrong self-report (hallucinated a "Storm Breaker" cause and read a stale
config file it wasn't actually running on).

This design scopes **two tracks for immediate implementation** (T1 config foundation,
T2 dead-code cleanup) and **documents the remaining tracks (T3–T7) as a roadmap** for
later, independent spec→plan cycles.

### 1.1 Full incomplete-refactoring map (reference)

Verified via code audit; anchors are `file:line`.

**Orphaned / broken (Engine-removal fallout):**
- Old `PerceptionBridge` (mpsc→engine): receiver dropped at
  `crates/runtime/src/impl/daemon/handler/init.rs:98` (`_perception_rx`). Constructed at
  `crates/runtime/src/core/runtime_core.rs:174,194-207`. Every Critical/High perception
  event fails `engine_tx.send()` → logs `Engine receiver dropped, buffering event`
  every ~5s (`crates/dasein/src/impl/perception/bridge.rs:74`) and pushes to an
  **unbounded** buffer (leak; Critical/High path skips `buffer_max`).
- Newer bus-based `PerceptionModule`
  (`crates/runtime/src/impl/engine/modules/perception_module.rs`) was written to
  *replace* the bridge (publishes to topic `perception.events`) but is **never spawned**
  and has **zero subscribers**. Perception is orphaned on both old and new paths.
- Dead fields marked `#[allow(dead_code)]` "unused after Engine removal":
  `agent_registry`, `checkpoint_store`, `agent_loader`
  (`crates/runtime/src/impl/daemon/handler/mod.rs:97,145,150`).
- `Controller` scaffold (`crates/runtime/src/core/controller.rs`): "will be wired into
  TUI/HTTP in a future phase"; never instantiated in production.
- Duplicate `PerceptionBridge` file: `crates/dasein/src/bridge/perception.rs` vs the
  live `crates/dasein/src/impl/perception/bridge.rs`.

**Wired & working (do not touch):** `awareness_signal` → evolution
(`crates/runtime/src/core/react_loop/awareness.rs`, `evolution_coordinator.rs:256`),
`GoalTracker` (steers prompt + constraints), `FactStore` recall
(`crates/runtime/src/impl/daemon/handler/chat.rs:125-164`), dasein prompt injection
(`chat.rs:499`). All three execution paths (daemon / exec / TUI) work — the old
"can't run" note is **outdated**.

**Half-wired (cosmetic / write-only) — deferred to roadmap:** mood updates discard
`_mood` (`crates/dasein/.../mod.rs:136`, called `handler/mod.rs:269`); Reflection
deviation classification never runs because context fields are always empty
(`crates/runtime/src/core/react_loop/tool_exec.rs:418-420`); episodic memory is
write-only; evolution/metacog fully wired but disabled by default.

**Duplicated-but-live modules (careful, not dead):** `loop_detector.rs` and
`circuit_breaker.rs` exist byte-identically in `crates/corpus/src/security/security/`
(used by `corpus/.../runner.rs`, which the runtime tool-runner uses) **and**
`crates/dasein/src/impl/security/` (used by dasein's `LoopBridge`). Both are live —
consolidation is a refactor, not a deletion.

**Config divergence (3-way) + bug:**
- `config/default.toml` (anthropic/sonnet), `/etc/aletheon/config.toml`
  (leju / deepseek-v4-pro / max_iter 50), `~/.aletheon/config.toml`
  (deepseek / v4-flash / max_iter 25, stale).
- Two `AppConfig` types: `cognit::config::AppConfig` (no `hooks`) and
  `runtime::core::config::AppConfig` (has `hooks`).
- **Hooks bug:** `crates/runtime/src/core/runtime_core.rs:105-108` loads hooks via
  `runtime::AppConfig::load_layered(None)` → only `~/.aletheon`, **ignoring `--config`**.
- `load_layered()` (`crates/cognit/src/config/mod.rs:417-441`) layers only
  compiled-defaults → `~/.aletheon` → project; **no `/etc/aletheon` layer**.
- `merge()` (`mod.rs:326-411`) **skips** `perception`, `evolution`, `system_prompt`,
  `compaction_enabled` → those cannot be overridden by layering.

## 2. Decisions (locked)

- **Direction:** implement T1 then T2; document T3–T7 as roadmap.
- **max_iterations:** `0` means **unlimited**; default becomes `0`. Runaway protection
  relies on the existing `CircuitBreaker` + repeated-call detection + `ToolBudget`.
- **Config authority:** `~/.aletheon/config.toml` is the day-to-day authoritative file.
  `/etc/aletheon` holds packaged service defaults only.
- **Orphans (C-class):** keep all, add explanatory comments, reference from roadmap.
  Do not delete.

## 3. T1 — Config Foundation

### 3.1 Unified layered loading
Single canonical loader; the merged result is passed everywhere (no re-loading).
Precedence (low → high):

```
Layer 0  compiled defaults
Layer 1  /etc/aletheon/config.toml        (system defaults; NEW layer)
Layer 2  ~/.aletheon/config.toml          (user; authoritative for daily edits)
Layer 3  <project>/.aletheon/config.toml  (project-local)
Layer 4  --config <file>                  (explicit; highest)
```

- Extend `AppConfig::load_layered` to insert the `/etc/aletheon` layer before the
  user layer.
- When `--config` is given, load it as the top layer over the standard search
  (still merges on top of compiled defaults), instead of replacing everything.

### 3.2 Fix the hooks bug
`runtime_core.rs:105-108`: use the already-loaded config's `hooks` instead of a
separate `load_layered(None)` call. Hooks then honor `--config` and all layers.

### 3.3 Complete `merge()`
Add missing sections so layering actually works: `perception`, `evolution`,
`agent.system_prompt`, `agent.compaction_enabled`, full `sandbox`, full `daemon`.
Keep list semantics explicit (providers merge-by-name; mcp_servers/plugins append).

### 3.4 Dual AppConfig — minimal fix only
Do **not** fully merge the two structs (that is a larger, riskier refactor → roadmap).
For T1: make the runtime side consume the single loaded config for hooks, and add
doc comments on both structs explaining the relationship and why they're separate
(cyclic-dependency break). Full unification tracked in roadmap.

### 3.5 max_iterations = 0 → unlimited
- `should_continue()` (`crates/runtime/src/core/react_loop/mod.rs:160-162`): treat
  `max_iterations == 0` as "no iteration cap" (always continue on the iteration check;
  termination then comes from LLM stop, CircuitBreaker, repeated-call detection, or
  ToolBudget).
- `default_max_iterations()` (`crates/cognit/src/config/mod.rs:86`) → `0`.
- Update the "hit max_iterations" branch (`step.rs:318-332`) to be unreachable when
  unlimited; ensure the loop can still exit via the other guards.

### 3.6 Grounding hardening ("笨/编造" fix)
Add a directive to the system prompt: before stating conclusions about its own runtime
state or configuration, the agent must read the actual logs and the actually-effective
config file — no guessing. (Config value `agent.system_prompt` / default prompt.)

### 3.7 Config content + deployment alignment
- Correct `~/.aletheon/config.toml` content to the good values
  (provider `leju`, model `deepseek/deepseek-v4-pro`, `max_iterations = 0`).
- Run the daemon **as the user** (user systemd service) so `~` resolves to
  `/home/aurobear`; the current root system service resolves `~` to `/root`.
  Reconcile `config/aletheon.service` / `config/aletheon.user.service` / `setup.sh`
  accordingly.
- Make `config/default.toml` consistent with compiled defaults (remove the
  anthropic/sonnet mismatch) so the three sources no longer contradict.

## 4. T2 — Dead-Code Cleanup

### 4.1 Class A — safe delete (verify no references first)
- `crates/dasein/src/bridge/perception.rs` (duplicate of the live impl bridge).
- Dead constants/fields: `MAX_EXTRACTION_TOKENS`
  (`crates/runtime/src/impl/memory/auto_memory.rs:25`), `DEFAULT_TRUST`
  (`crates/runtime/src/impl/memory/fact_store/mod.rs:14`), `ActiveRecording.id`
  (`crates/runtime/src/impl/daemon/debug_handler.rs:49`), `CheckpointStore.session_dir`
  (`crates/runtime/src/core/checkpoint.rs:31`).
- Goal: `cargo build` warnings → 0 (for the items removed).

### 4.2 Class B — perception: stop the bleeding, keep the parts
- Stop spawning the old `PerceptionBridge` and drop the dead injection channel in
  `runtime_core.rs:174,194-207`; remove the `_perception_rx` param plumbing
  (`init.rs:98`).
- Add `enabled: bool` to `PerceptionConfig`
  (`crates/cognit/src/config/mod.rs:287`), default `false`, so `PerceptionManager`
  is not spawned (no 5s journald/inotify polling) until T3 turns it on.
- **Keep** `PerceptionModule` (bus) and `PerceptionManager` intact for T3.
- Result: `Engine receiver dropped` spam gone; buffer leak gone.

### 4.3 Class C — orphans: keep + comment (per decision)
Keep and annotate, cross-referencing the roadmap track that will consume each:
- `agent_registry`, `agent_loader` → T3-adjacent (multi-agent orchestration).
- `checkpoint_store` / `CheckpointStore` → future file-edit rewind.
- `Controller` scaffold → future multi-frontend (TUI/HTTP).
- `DeliveryManager.clients` → future automation/delivery.
Each keeps `#[allow(dead_code)]` with a comment: "Parked — see roadmap §<track>".

### 4.4 Class D — security duplicate consolidation → roadmap
The two live `loop_detector`/`circuit_breaker` copies (corpus vs dasein) should be
consolidated to a single shared module with both consumers updated, with tests. This
is a refactor (medium risk), not dead-code removal → **deferred to roadmap §T-sec**;
T2 only adds a doc note marking the duplication.

## 5. Roadmap (deferred tracks — future spec→plan each)

- **T3 — Perception → behavior closed loop.** Spawn `PerceptionModule`, add a
  subscriber that injects throttled/prioritized perception events into the ReAct turn;
  set `perception.enabled = true`. Basis: module + manager already exist. Risk: prompt
  noise / cost. Acceptance: a watched event demonstrably alters a turn.
- **T4 — Memory closed loop.** Make episodic memory read-back (recall past
  experiences/reflections into context); populate Reflection deviation context fields
  (`tool_exec.rs:418-420`); optionally wire ExperienceSummarizer / SelfAwareness seed.
  Risk: context bloat / cost.
- **T5 — Mood drives behavior.** Stop discarding `_mood`; let mood influence real
  decisions (caution level / whether to ask a clarifying question). Risk: gimmicky.
- **T6 — Enable evolution (metacog).** Turn on self-mutation; validate morphogenesis
  pipeline + rollback safety. **High risk (autonomous self-modification); do last,
  with strong observability.**
- **T7 — TUI redraw corruption.** Investigate streaming-output vs input-box redraw
  interleaving (`crates/interact/src/tui/{streaming,response,render}`); confirm whether
  it is a real bug or a paste artifact first.
- **T-sec — Security module consolidation.** Merge the duplicate
  `loop_detector`/`circuit_breaker` into one shared module (see §4.4).
- **T-cfg2 — Full AppConfig unification.** Collapse `cognit` vs `runtime` `AppConfig`
  (see §3.4).

## 6. Validation

- `cargo build` warnings → 0; `cargo test` green (incl. config merge/layer tests and
  a `max_iterations == 0` loop-termination test driven by the circuit breaker).
- Start daemon → journal shows **no** `Engine receiver dropped`; RSS stable over time.
- Change one value in `~/.aletheon/config.toml` → observe it take effect (proves
  layering + authority).
- Run a long task with `max_iterations = 0` → confirm termination comes from a guard,
  not an iteration cap.
- Confirm hooks defined in the authoritative config are actually loaded (proves the
  hooks-bug fix).

## 7. Rollout & risk

- Work on branch `auro/refactor/config-cleanup`; PR to `dev` after validation (never
  commit directly to `dev`).
- Deployment change (root service → user service) is the highest-risk item: stage it,
  verify the daemon starts as the user and reads `~/.aletheon`, keep the old unit until
  confirmed.
- New config `enabled` flags default OFF, so behavior is conservative by default.

## 8. New files / key files touched (T1+T2)

- `crates/cognit/src/config/mod.rs` — `load_layered` (+/etc layer), `merge` (complete),
  `default_max_iterations` → 0, `PerceptionConfig.enabled`.
- `crates/runtime/src/core/runtime_core.rs` — hooks fix, stop spawning bridge,
  drop injection channel.
- `crates/runtime/src/impl/daemon/handler/{init.rs,mod.rs}` — remove `_perception_rx`
  plumbing; annotate C-class orphans.
- `crates/runtime/src/core/react_loop/{mod.rs,step.rs}` — `max_iterations == 0`
  handling.
- Deleted: `crates/dasein/src/bridge/perception.rs` + dead constants/fields.
- Config/deploy: `~/.aletheon/config.toml`, `config/default.toml`,
  `config/aletheon*.service`, `setup.sh`.
- Default system prompt (grounding directive).
