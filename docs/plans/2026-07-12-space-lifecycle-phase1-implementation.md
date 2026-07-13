# Space Lifecycle Phase 1 (Leak Fix) Implementation Plan

> **For agentic workers:** Use `workflow-feature` or `plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Eliminate the per-turn `ContextSpace` memory leak by adding `InMemorySpaceManager::release` and releasing each turn's ephemeral space via an RAII guard.

**Architecture:** Two additive changes. (1) A synchronous inherent `release` (+ `space_count` observability helper) on `InMemorySpaceManager`, mirroring the existing inherent `set_overlay`/`get_space`. (2) A function-local `SpaceReleaseGuard` in `execute_turn` whose `Drop` calls `release(turn_space)` on every exit path. No trait, signature, or turn-behavior changes.

**Tech Stack:** Rust, `std::sync::Mutex`, `async_trait` (unchanged), `tokio` test harness.

**Source spec:** `docs/plans/2026-07-12-space-lifecycle-phase1-design.md`

---

## File Structure

- Modify: `crates/kernel/src/space/manager.rs` — add `release` + `space_count` + `#[cfg(test)] mod tests`.
- Modify: `crates/executive/src/service/daemon_turn/execute.rs` — add `use crate::core::core_systems::CoreSystems;` and install `SpaceReleaseGuard` after line ~342.

Out of scope (Phase 2, see design §7): `process.space` reuse, `fork_space` wiring on spawn, process-exit release, `SpaceManager` **trait** completion, Agora binding-version updates.

---

## Task 1: Add `release` + `space_count` to `InMemorySpaceManager` (TDD)

**Files:**
- Modify: `crates/kernel/src/space/manager.rs` (add methods in the `impl InMemorySpaceManager` block after `set_overlay`, ending line 51; add test module at end of file)

- [ ] **Step 1: Write the failing tests**

Append to `crates/kernel/src/space/manager.rs` (after line 107, end of file):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn release_is_idempotent_and_clears_entry() {
        let m = InMemorySpaceManager::new();
        let s = SpaceId::new();
        m.set_overlay(s, "turn_input", json!("hi")).unwrap();
        assert!(m.get_space(s).is_some());
        assert_eq!(m.space_count(), 1);

        assert!(m.release(s)); // entry existed
        assert!(m.get_space(s).is_none());
        assert_eq!(m.space_count(), 0);

        assert!(!m.release(s)); // idempotent: already gone
    }

    #[test]
    fn per_turn_cycle_does_not_grow() {
        let m = InMemorySpaceManager::new();
        // Simulate the daemon per-turn create -> overlay -> release cycle.
        for i in 0..1000 {
            let s = SpaceId::new();
            m.set_overlay(s, "turn_input", json!(i)).unwrap();
            assert!(m.release(s));
        }
        assert_eq!(m.space_count(), 0, "spaces must not accumulate across turns");
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p aletheon-kernel space::manager::tests 2>&1 | tail -20`
Expected: FAIL — compile error `no method named 'release'` / `no method named 'space_count'` found for struct `InMemorySpaceManager`.

- [ ] **Step 3: Write minimal implementation**

In `crates/kernel/src/space/manager.rs`, inside `impl InMemorySpaceManager` (immediately after the `set_overlay` method, i.e. after line 50 `}` and before line 51 closing `}` of the impl block), add:

```rust
    /// Remove a space and its bindings/overlay. Idempotent; returns whether an
    /// entry existed. Called when an ephemeral (per-turn) space is done, to
    /// prevent unbounded growth of the space table.
    pub fn release(&self, space: SpaceId) -> bool {
        self.spaces
            .lock()
            .map(|mut s| s.remove(&space).is_some())
            .unwrap_or(false)
    }

    /// Number of tracked spaces (observability / leak checks).
    pub fn space_count(&self) -> usize {
        self.spaces.lock().map(|s| s.len()).unwrap_or(0)
    }
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p aletheon-kernel space::manager::tests 2>&1 | tail -20`
Expected: PASS — `test result: ok. 2 passed`.

- [ ] **Step 5: Commit**

```bash
git add crates/kernel/src/space/manager.rs
git commit -m "feat(kernel): add InMemorySpaceManager::release + space_count

Idempotent removal of a space entry, with a count helper for leak
checks. Enables per-turn space cleanup (Phase 1 leak fix)."
```

---

## Task 2: Install `SpaceReleaseGuard` in `execute_turn`

**Files:**
- Modify: `crates/executive/src/service/daemon_turn/execute.rs` (add import near line 9; insert guard after line 342)

- [ ] **Step 1: Add the `CoreSystems` import**

In `crates/executive/src/service/daemon_turn/execute.rs`, after line 9
(`use super::orchestrator::DaemonTurnOrchestrator;`), add:

```rust
use crate::core::core_systems::CoreSystems;
```

- [ ] **Step 2: Insert the release guard**

In the same file, immediately after the `set_overlay` block that ends at
line 342 (the `}` closing the `if let Err(e) = self.subsystems.ports.space_manager.set_overlay(...)` block) and before line 344
(`let self_field_arc_for_react = ...`), insert:

```rust
        // Release the ephemeral turn space on every exit path (including panics
        // and any future early returns) so the space table does not grow
        // unbounded across turns. See docs/plans/2026-07-12-space-lifecycle-phase1-design.md.
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

`Arc<CoreSystems>`, `SpaceId`, and `release` are all in scope: `self.subsystems`
is `Arc<CoreSystems>` (`orchestrator.rs:44`), `SpaceId` is imported
(`execute.rs:26`, and is `Copy`), and `ports.space_manager` is the concrete
`InMemorySpaceManager` (`aletheon-kernel` `service/mod.rs:75`).

- [ ] **Step 3: Build to verify it compiles**

Run: `cargo build -p executive 2>&1 | tail -20`
Expected: PASS — builds with no errors. (`_space_guard` is intentionally
unused-by-name; the leading underscore suppresses the warning while keeping it
live until end of scope.)

- [ ] **Step 4: Confirm no regression in executive tests**

Run: `cargo test -p executive 2>&1 | tail -20`
Expected: PASS — existing executive tests remain green (turn behavior
unchanged; only post-turn cleanup added).

- [ ] **Step 5: Commit**

```bash
git add crates/executive/src/service/daemon_turn/execute.rs
git commit -m "fix(executive): release per-turn ContextSpace to stop leak

Install an RAII SpaceReleaseGuard after the turn_space attach/overlay
block so execute_turn releases the ephemeral space on every exit path,
eliminating unbounded space-table growth (Phase 1)."
```

---

## Task 3: Full-workspace validation

**Files:** none (validation only)

- [ ] **Step 1: Format check**

Run: `cargo fmt --all --check`
Expected: PASS — no diff. (If it fails, run `cargo fmt --all` and amend the
relevant commit.)

- [ ] **Step 2: Clippy on touched crates**

Run: `cargo clippy -p aletheon-kernel -p executive --all-targets 2>&1 | tail -30`
Expected: PASS — no new warnings. In particular no `dead_code` warning for the
guard struct (it is constructed and dropped).

- [ ] **Step 3: Targeted test run**

Run: `cargo test -p aletheon-kernel -p executive 2>&1 | tail -20`
Expected: PASS — all tests green, including the two new kernel tests
(`release_is_idempotent_and_clears_entry`, `per_turn_cycle_does_not_grow`).

- [ ] **Step 4: Update the coupling-analysis doc status**

In `docs/arch/CURRENT_ARCHITECTURE_AND_COUPLING_ANALYSIS.md`, update §9.7 /
§9.8: mark "`turn_space` 泄露" and "`SpaceManager` 加 `release()`" as ✅ Phase 1
done (inherent method on `InMemorySpaceManager`), and note that `process.space`
reuse / `fork_space` wiring / trait promotion remain Phase 2.

```bash
git add docs/arch/CURRENT_ARCHITECTURE_AND_COUPLING_ANALYSIS.md
git commit -m "docs(arch): mark per-turn space leak fixed (Phase 1)"
```

---

## Self-Review

- **Spec coverage:** design §3 → Task 1 `release`; §4 guard → Task 2; §6 tests → Task 1 Steps 1/4 (unit + no-growth) and Task 3 Step 3; §2 inherent-not-trait decision → honored (no trait edit); §7 deferrals → excluded from all tasks.
- **Placeholder scan:** none — all code, commands, and expected outputs are concrete.
- **Type consistency:** `release(&self, space: SpaceId) -> bool` and `space_count(&self) -> usize` are used identically in tests, guard, and impl; `SpaceId` is `Copy` (reused by value); `subsystems: Arc<CoreSystems>` matches `orchestrator.rs:44`.
```
