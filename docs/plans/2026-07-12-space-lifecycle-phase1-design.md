# Space Lifecycle Fix — Phase 1 (Stop the Bleed)

> Date: 2026-07-12
> Status: design approved, pending implementation plan
> Scope: **Phase 1 only** — eliminate the per-turn ContextSpace memory leak.
> Follow-up: Phase 2 (space-model correctness) is a separate spec — see §7.

---

## 1. Problem

`InMemorySpaceManager.spaces` (`crates/kernel/src/space/manager.rs:14`,
`Mutex<HashMap<SpaceId, ContextSpace>>`) grows without bound during normal
operation. Every daemon turn:

1. mints a fresh ephemeral space — `execute.rs:311`
   `let turn_space = SpaceId::new();`
2. attaches a `Session` binding and an `Agora` binding
   (`execute.rs:312-335`) and writes a `turn_input` overlay
   (`execute.rs:336-342`);
3. both `attach_region` (`manager.rs:98-106`) and `set_overlay`
   (`manager.rs:37-50`) do `entry(space).or_insert_with(empty_space)`, so the
   turn creates a new `HashMap` entry;
4. the entry is **never removed** — the trait
   (`crates/fabric/src/include/space.rs:11-17`) exposes only `fork_space` and
   `attach_region`, and `InMemorySpaceManager` has no `release`.

Result: one leaked `ContextSpace` per turn → unbounded memory growth on a
long-running daemon. Documented in
`docs/arch/CURRENT_ARCHITECTURE_AND_COUPLING_ANALYSIS.md` §9.5 / §9.7 (severity
🔴), still unfixed.

### 1.1 What Phase 1 does *not* touch

`ProcessTable::spawn` (`crates/kernel/src/process/table.rs:139`) mints
`space: SpaceId::new()` but never registers it with the `SpaceManager`, so
`process.space` occupies **no** map entry today. The only leaking object is
`turn_space`. Wiring `process.space` (fork on spawn, release on exit) is a
correctness change, deferred to Phase 2 (§7).

---

## 2. Design decision that differs from doc §9.8

`docs/arch/CURRENT_ARCHITECTURE_AND_COUPLING_ANALYSIS.md` §9.8 lists
"add `release(space)` to the `SpaceManager` **trait**" as P0. That assumed the
consumer holds an `Arc<dyn SpaceManager>`.

Reality: the port is the **concrete** type —
`crates/kernel/src/service/mod.rs:75` `pub space_manager: InMemorySpaceManager`,
and `execute.rs:336` already calls the inherent (non-trait) `set_overlay`.

Therefore Phase 1 adds `release` as an **inherent, synchronous** method on
`InMemorySpaceManager`, mirroring the existing inherent `set_overlay` /
`get_space` / `get_bindings`. This fixes the leak with minimal churn and
without touching the trait. Promoting lifecycle methods onto the trait is part
of the Phase 2 trait-completion work (§7).

Synchronous (not `async`) is deliberate: the backing store is
`std::sync::Mutex`, and a sync method is what makes an RAII `Drop` guard
possible (§4).

---

## 3. Change 1 — `InMemorySpaceManager::release`

File: `crates/kernel/src/space/manager.rs` (add alongside `set_overlay`).

```rust
/// Remove a space and its bindings/overlay. Idempotent; returns whether an
/// entry existed.
pub fn release(&self, space: SpaceId) -> bool {
    self.spaces
        .lock()
        .map(|mut s| s.remove(&space).is_some())
        .unwrap_or(false)
}
```

- Idempotent: releasing an unknown / already-released space returns `false`,
  never errors.
- Infallible: a poisoned mutex yields `false` (the process is already in a
  broken state elsewhere); `release` does not add a new panic path.

---

## 4. Change 2 — RAII release guard in `execute_turn`

File: `crates/executive/src/service/daemon_turn/execute.rs`, installed
immediately after the `turn_space` attach/overlay block (after line 342).

```rust
struct SpaceReleaseGuard {
    subsystems: Arc<CoreSystems>,
    space: SpaceId,
}
impl Drop for SpaceReleaseGuard {
    fn drop(&mut self) {
        self.subsystems.ports.space_manager.release(self.space);
    }
}
let _space_guard = SpaceReleaseGuard {
    subsystems: self.subsystems.clone(),
    space: turn_space,
};
```

Rationale for RAII over a single explicit call:

- `execute_turn` returns `serde_json::Value` and today has **no early return
  after line 311** (verified: all `return json!(...)` sites are ≤ line 255), so
  a plain `release()` before the tail return would work now.
- The guard is panic-safe and survives future edits that introduce early
  returns after the space is created. Cost is one small struct + `Drop`.
- `Drop` is synchronous → it can call the synchronous `release` directly; no
  detached `tokio::spawn`, which the architecture forbids.

The guard holds an `Arc<CoreSystems>` clone (the orchestrator already stores
`subsystems: Arc<CoreSystems>`), reaching the concrete `space_manager` through
`ports`.

---

## 5. Data flow (unchanged during the turn)

```
create turn_space (SpaceId::new)
  → attach_region(Session binding)
  → attach_region(Agora binding)
  → set_overlay("turn_input", message)
  → install SpaceReleaseGuard
  → react loop reads bindings/overlay (unchanged)
  → execute_turn returns
  → guard Drop → release(turn_space) removes the HashMap entry
```

No behavior visible to the turn changes; only post-turn cleanup is added.

---

## 6. Testing

1. **Kernel unit test** (`crates/kernel/src/space/manager.rs` tests or a new
   test module):
   - `attach_region(fresh_space, binding)` → `get_space(fresh_space)` is `Some`.
   - `release(fresh_space)` returns `true` → `get_space(fresh_space)` is `None`.
   - second `release(fresh_space)` returns `false` (idempotent).
   - `set_overlay` then `release` also clears the entry.

2. **Regression test — no growth**:
   - Reuse the existing harness in
     `crates/executive/tests/context_space.rs` if it can drive the per-turn
     create→attach→set_overlay→release cycle.
   - Otherwise a manager-level loop test: run the cycle N (e.g. 1000) times and
     assert the internal map size stays constant/bounded (expose a test-only
     `len()` or reuse `get_space` probes).

Acceptance: both tests pass; `cargo test -p kernel` and the executive test
target are green; no clippy/fmt regressions.

---

## 7. Deferred to Phase 2 (separate spec)

The following are **correctness / model** changes, not leak fixes, and are out
of scope here. They map to doc §9.8 P1/P2:

| Item | Rationale |
|---|---|
| Turn reuses `process.space` instead of a per-turn `turn_space` | design `Final(2).md` §9.5; makes the space concept meaningful across turns |
| `ProcessTable::spawn` calls `space_manager.fork_space(parent.space, pid)` | process-space inheritance (`table.rs:139` currently mints-and-forgets) |
| Release `process.space` on process exit (`orchestrator.rs:146 exit_process`) | once `process.space` is actually registered |
| Promote `release` / `lookup` / `get_overlay` / `update_binding` onto the `SpaceManager` **trait** | trait becomes a real abstraction boundary |
| `ContextBinding::Agora` version updated on commit via `update_binding` | avoid stale/accumulating Agora bindings when the space is reused |

Phase 2 must resolve the binding-accumulation problem before switching to a
reused `process.space` (repeated `attach_region` would otherwise pile up
`Session`/`Agora` bindings across turns).

---

## 8. Risk

Low. Two additive changes: one new inherent method, one guard scoped to a
single function. No signature or trait changes, no change to turn behavior. The
only failure mode (double-release) is idempotent by construction.
