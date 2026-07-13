# Clock Unification — Remaining Work Implementation Plan

> **For agentic workers (DeepSeek/Claude):** Execute per-crate, one crate per PR.
> Steps use checkbox (`- [ ]`) syntax. TDD not required for mechanical swaps, but
> every crate must end green: `cargo test -p <crate>` + `cargo clippy -p <crate> --all-targets` + `cargo fmt --all --check`.

**Goal:** Replace the remaining ~139 direct wall/mono/timer calls with the kernel `Clock`/`Timer` abstraction so time is a single injected source (enables deterministic `TestClock` tests, removes flaky timing).

**Background:** Phase 1 (commit `4d4ab99`) injected `Arc<dyn Clock>` into every domain crate and converted the easy sites. What remains are the harder sites that need a struct field type change or thread a clock into a leaf. This plan finishes them.

**Foundation already in place (do NOT re-add):**
- `fabric::Clock` trait: `wall_now() -> WallTime`, `mono_now() -> MonoTime`.
- `fabric::wall_to_datetime(WallTime) -> chrono::DateTime<Utc>` — the chrono bridge.
- `aletheon_kernel::chronos::{SystemClock, TestClock}`; `Timer::sleep(clock, dur)`, `Timer::timeout(clock, dur, fut)`.
- Every domain crate already receives an `Arc<dyn Clock>` at construction (Phase 1). Find the existing clock handle in each type before threading a new one.

---

## Remaining count by crate (grep baseline — production only)

Run this to see the live list before/after each crate:
```bash
grep -rn "Utc::now\|Instant::now\|SystemTime::now\|tokio::time::sleep\|tokio::time::timeout" crates/<crate>/src | grep -v test
```

| Crate | Remaining | Nature |
|---|---|---|
| mnemosyne | ~33 | timestamps + duration measures in backends |
| executive | ~32 | impl/ legacy: supervisor, agent/process, automation |
| dasein | ~29 | watchdog/safe_mode Instant fields, perception source timestamps, sorge/inotify sleeps |
| corpus | ~19 | drivers (boot, clipboard_x11, android), sandbox timing |
| fabric | ~17 | mostly type-layer defaults/log timestamps |
| kernel | ~8 | internal (some are the Clock impls themselves — verify before touching) |
| cognit | ~1 | single residual |

> Note: some `kernel` hits are inside `SystemClock`/`TestClock` themselves — those are the source of truth and must stay. Exclude `crates/kernel/src/chronos/`.

---

## Replacement recipes (apply by category)

### Category A — wall-clock timestamp (`chrono::Utc::now()`)
Used for event/record timestamps, dates.
```rust
// BEFORE
timestamp: chrono::Utc::now(),
// AFTER (clock is an Arc<dyn Clock> already held by the struct/param)
timestamp: fabric::wall_to_datetime(clock.wall_now()),
```
If the struct doesn't yet hold a clock, add a `clock: Arc<dyn Clock>` field and thread it from the constructor (the crate root already has one from Phase 1 — pass it down). Prefer field injection over a parameter on every method.

### Category B — monotonic duration / deadline (`std::time::Instant::now()`)
Used for elapsed-time measurement, heartbeats, `last_beat`.
```rust
// BEFORE
let start = Instant::now();
// ... later
let elapsed = start.elapsed();
// AFTER
let start = clock.mono_now();
let elapsed = clock.mono_now().saturating_duration_since(start); // use MonoTime's duration API
```
For **struct fields** typed `Instant` (e.g. `dasein watchdog.rs:28 last_beat: Mutex<Instant>`, `safe_mode.rs:48 entered_at: Option<Instant>`): change the field type to `MonoTime`, update all reads/writes, and inject a clock into the owning struct. This is the bulk of the "hard" work.

### Category C — async sleep / timeout (`tokio::time::sleep|timeout`)
```rust
// BEFORE
tokio::time::sleep(dur).await;
tokio::time::timeout(dur, fut).await
// AFTER
aletheon_kernel::chronos::Timer::sleep(&clock, dur).await;
aletheon_kernel::chronos::Timer::timeout(&clock, dur, fut).await
```
Note `select!` arms (e.g. `dasein/src/dasein/sorge.rs:70,154`): the `Timer::sleep(&clock, ..)` future can be used directly in a `select!` arm. Verify the future is `Unpin`/boxed as the existing arm expects.

---

## Tasks (one per crate; order easiest → hardest)

### Task 1: cognit (~1) — warm-up
- [ ] Locate the single site; apply the matching recipe using cognit's existing `CognitCore` clock.
- [ ] `cargo test -p cognit`; grep shows 0. Commit: `refactor(cognit): finish Clock unification`.

### Task 2: fabric (~17)
- [ ] Most are type-layer. For any that are pure defaults with no clock in scope, evaluate whether they belong to `contract` (leave a `WallTime`-typed field and let the caller set it) vs. genuinely needing a clock. Do NOT add a Clock dependency to pure contract types — instead accept a `WallTime`/`MonoTime` argument.
- [ ] `cargo test -p fabric`; commit: `refactor(fabric): finish Clock unification`.

### Task 3: corpus (~19) — drivers + sandbox
- [ ] `ToolContext.clock` already exists (Phase 1). Thread it into `boot.rs`, `clipboard_x11.rs`, `android.rs`, `sandbox/executor.rs`, `socket_approval.rs`. Some sandbox timing measures are test-only parallelism checks — leave those in `#[cfg(test)]`.
- [ ] `cargo test -p corpus`; commit: `refactor(corpus): finish Clock unification (drivers/sandbox)`.

### Task 4: mnemosyne (~33) — backends
- [ ] `MemoryPipeline` already carries a clock (Phase 1). Thread into `tools.rs`, `core_memory/mod.rs`, and each backend's timestamp/duration sites via Category A/B.
- [ ] `cargo test -p mnemosyne`; commit: `refactor(mnemosyne): finish Clock unification (backends)`.

### Task 5: dasein (~29) — watchdog/safe_mode/perception/sorge
- [ ] Category B struct-field changes: `resilience/watchdog.rs` (`last_beat`), `resilience/safe_mode.rs` (`entered_at`), `perception/aggregator.rs`. Inject clock into these structs from `SelfField`'s clock (Phase 1).
- [ ] Category A: perception sources (`inotify_source.rs`, `journald_source.rs`, `proc_source.rs`, `ebpf_source.rs`, `bridge.rs`) event timestamps.
- [ ] Category C: `sorge.rs:70,154`, `inotify_source.rs:53`, `hook/dispatcher.rs:92 timeout`.
- [ ] `cargo test -p dasein`; commit: `refactor(dasein): finish Clock unification (watchdog/perception/sorge)`.

### Task 6: executive (~32) — impl/ legacy layer
- [ ] `ServicePorts.clock` is available. Thread into `impl/kernel/supervisor.rs`, `impl/agent/process.rs`, automation/orchestration legacy sites. These are the oldest code; expect `Instant` field changes.
- [ ] `cargo test -p executive`; commit: `refactor(executive): finish Clock unification (impl/ legacy)`.

### Task 7: kernel (~8) — residual only
- [ ] EXCLUDE `crates/kernel/src/chronos/` (the Clock impls). Convert any remaining consumer sites.
- [ ] `cargo test -p aletheon-kernel`; commit: `refactor(kernel): finish Clock unification (residual)`.

### Task 8: workspace gate
- [ ] `grep -rn "Utc::now\|Instant::now\|SystemTime::now\|tokio::time::sleep\|tokio::time::timeout" crates/*/src | grep -v test | grep -v "chronos/"` → **0 lines**.
- [ ] `cargo test --workspace`; `cargo clippy --workspace --all-targets`; `cargo fmt --all --check`.
- [ ] Add a `TestClock`-based deterministic test in one crate (e.g. dasein watchdog) proving `TestClock::advance()` drives the timeout without wall-clock waiting — this is the payoff and guards against regression.
- [ ] Commit: `test: deterministic TestClock coverage after Clock unification`.

---

## Guardrails
- Do NOT touch `crates/kernel/src/chronos/` (Clock source of truth) or convert time inside `#[cfg(test)]` that intentionally measures real wall-clock parallelism (leave those, they use real time deliberately — annotate why).
- Prefer struct-field clock injection over adding a `clock` parameter to many method signatures.
- One crate per commit; keep each green before moving on.
```
