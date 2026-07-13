# Space Lifecycle Phase 2a (Reuse process.space) Implementation Plan

> **For agentic workers:** Use `workflow-feature` or `plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make daemon turns reuse the main agent's long-lived `process.space` instead of a per-turn ephemeral space, fix binding accumulation, and release the space on process exit — removing the Phase-1 per-turn guard.

**Architecture:** Executive-orchestrated. Expose `space` in `ProcessSnapshot` (read-only), add `InMemorySpaceManager::upsert_binding` (replace singleton bindings, append artifacts), point `execute_turn` at `inspect(main_pid).space`, and release the space in `exit_process`. No `SpaceManager` trait change, no `spawn` signature change (those are Phase 2b).

**Tech Stack:** Rust, `std::sync::Mutex`, `tokio` test harness, `async_trait` (unchanged).

**Source spec:** `docs/plans/2026-07-12-space-lifecycle-phase2a-design.md`
**Builds on:** Phase 1 (`InMemorySpaceManager::release` / `space_count` already exist).

---

## File Structure

- Modify: `crates/fabric/src/types/process.rs` — add `space: SpaceId` field to `ProcessSnapshot` (line 156).
- Modify: `crates/kernel/src/process/table.rs` — populate `space` in `snapshot()` (line 116); add `#[cfg(test)] mod tests`.
- Modify: `crates/kernel/src/space/manager.rs` — add `upsert_binding`; add tests to the existing `mod tests`.
- Modify: `crates/executive/src/service/daemon_turn/execute.rs` — reuse `process.space`, remove Phase-1 guard + now-unused `CoreSystems` import.
- Modify: `crates/executive/src/service/daemon_turn/orchestrator.rs` — release space in `exit_process` (line 146).
- Modify: `docs/arch/CURRENT_ARCHITECTURE_AND_COUPLING_ANALYSIS.md` — status update.

Out of scope (Phase 2b): `spawn → fork_space`, sub-agent space inheritance, `SpaceManager` trait promotion, kernel-owned release on `Terminate`.

---

## Task 1: Expose `space` in `ProcessSnapshot` (TDD)

**Files:**
- Modify: `crates/fabric/src/types/process.rs:156` (struct)
- Modify: `crates/kernel/src/process/table.rs:116-126` (constructor) + append test module

- [ ] **Step 1: Write the failing test** — append to `crates/kernel/src/process/table.rs` (end of file):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use fabric::types::process::SpawnSpec;

    #[tokio::test]
    async fn snapshot_exposes_process_space() {
        let table = ProcessTable::default();
        let h1 = table.spawn(SpawnSpec::default()).await.unwrap();
        let h2 = table.spawn(SpawnSpec::default()).await.unwrap();
        let s1 = table.inspect(h1.id).await.unwrap();
        let s1_again = table.inspect(h1.id).await.unwrap();
        let s2 = table.inspect(h2.id).await.unwrap();
        assert_eq!(s1.space, s1_again.space, "space stable per process");
        assert_ne!(s1.space, s2.space, "each spawn mints a unique space");
    }
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p aletheon-kernel process::table::tests 2>&1 | tail -20`
Expected: FAIL — compile error `no field 'space' on type ProcessSnapshot`.

- [ ] **Step 3a: Add the field** — in `crates/fabric/src/types/process.rs`, inside `pub struct ProcessSnapshot` (after `pub process_id: crate::types::operation::ProcessId,`, line 157):

```rust
    pub space: SpaceId,
```

(`SpaceId` is defined in this same module at line 27 — no import needed.)

- [ ] **Step 3b: Populate it** — in `crates/kernel/src/process/table.rs`, inside `fn snapshot` (after `process_id: runtime.record.process_id,`, line 118):

```rust
            space: runtime.record.space,
```

- [ ] **Step 4: Run to verify it passes**

Run: `cargo test -p aletheon-kernel process::table::tests 2>&1 | tail -20`
Expected: PASS — `test result: ok. 1 passed`.

- [ ] **Step 5: Confirm no other break** — `ProcessSnapshot` is constructed only in `snapshot()` (verified: single site). Build the workspace to be sure:

Run: `cargo build --workspace 2>&1 | tail -20`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/fabric/src/types/process.rs crates/kernel/src/process/table.rs
git commit -m "feat(kernel): expose space in ProcessSnapshot

Read-only exposure of ProcessRecord.space via inspect(), so callers can
read a process's context space. Enables per-session space reuse (Phase 2a)."
```

---

## Task 2: Add `InMemorySpaceManager::upsert_binding` (TDD)

**Files:**
- Modify: `crates/kernel/src/space/manager.rs` (add method after `release`; add tests to existing `mod tests`)

- [ ] **Step 1: Write the failing tests** — add inside the existing `#[cfg(test)] mod tests` block in `crates/kernel/src/space/manager.rs`:

```rust
    #[test]
    fn upsert_replaces_singletons_appends_artifacts() {
        use fabric::types::space::{AccessMode, AgoraSpaceId, AgoraVersion, ArtifactId, SessionId};
        let m = InMemorySpaceManager::new();
        let s = SpaceId::new();

        m.upsert_binding(s, ContextBinding::Agora(AgoraSpaceId("sess".into()), AgoraVersion(1)));
        m.upsert_binding(s, ContextBinding::Agora(AgoraSpaceId("sess".into()), AgoraVersion(2)));
        let b = m.get_bindings(s).unwrap();
        assert_eq!(b.iter().filter(|x| matches!(x, ContextBinding::Agora(_, _))).count(), 1);
        assert!(b.iter().any(|x| matches!(x, ContextBinding::Agora(_, AgoraVersion(2)))));

        m.upsert_binding(s, ContextBinding::Session(SessionId("x".into())));
        m.upsert_binding(s, ContextBinding::Session(SessionId("x".into())));
        let b = m.get_bindings(s).unwrap();
        assert_eq!(b.iter().filter(|x| matches!(x, ContextBinding::Session(_))).count(), 1);

        m.upsert_binding(s, ContextBinding::Artifact(ArtifactId("a".into()), AccessMode::ReadOnly));
        m.upsert_binding(s, ContextBinding::Artifact(ArtifactId("b".into()), AccessMode::ReadOnly));
        let b = m.get_bindings(s).unwrap();
        assert_eq!(b.iter().filter(|x| matches!(x, ContextBinding::Artifact(_, _))).count(), 2);
    }

    #[test]
    fn reused_space_does_not_grow_across_turns() {
        use fabric::types::space::{AgoraSpaceId, AgoraVersion, SessionId};
        let m = InMemorySpaceManager::new();
        let s = SpaceId::new(); // one long-lived space
        for v in 0..1000u64 {
            m.upsert_binding(s, ContextBinding::Session(SessionId("sess".into())));
            m.upsert_binding(s, ContextBinding::Agora(AgoraSpaceId("sess".into()), AgoraVersion(v)));
            m.set_overlay(s, "turn_input", serde_json::json!(v)).unwrap();
        }
        assert_eq!(m.space_count(), 1);
        assert_eq!(m.get_bindings(s).unwrap().len(), 2, "one Session + one Agora, no accumulation");
    }
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p aletheon-kernel space::manager::tests 2>&1 | tail -20`
Expected: FAIL — `no method named 'upsert_binding'`.

- [ ] **Step 3: Implement** — in `crates/kernel/src/space/manager.rs`, inside `impl InMemorySpaceManager`, after the `release` method:

```rust
    /// Insert or update a binding. Singleton-per-space variants (Session,
    /// Agora, MemoryView, WorldProjection) replace an existing binding of the
    /// same variant in place; Artifact bindings (multi-instance) are appended.
    /// Infallible: a poisoned mutex is a no-op.
    pub fn upsert_binding(&self, space: SpaceId, binding: ContextBinding) {
        if let Ok(mut spaces) = self.spaces.lock() {
            let entry = spaces.entry(space).or_insert_with(|| empty_space(space));
            let is_multi = matches!(binding, ContextBinding::Artifact(_, _));
            if !is_multi {
                entry
                    .bindings
                    .retain(|b| std::mem::discriminant(b) != std::mem::discriminant(&binding));
            }
            entry.bindings.push(binding);
        }
    }
```

- [ ] **Step 4: Run to verify it passes**

Run: `cargo test -p aletheon-kernel space::manager::tests 2>&1 | tail -20`
Expected: PASS — all `space::manager::tests` green (Phase-1 tests + 2 new).

- [ ] **Step 5: Commit**

```bash
git add crates/kernel/src/space/manager.rs
git commit -m "feat(kernel): add InMemorySpaceManager::upsert_binding

Replace singleton bindings (Session/Agora/MemoryView/WorldProjection) in
place, append Artifact bindings. Prevents binding accumulation when a
space is reused across turns (Phase 2a)."
```

---

## Task 3: `execute_turn` reuses `process.space`; remove Phase-1 guard

**Files:**
- Modify: `crates/executive/src/service/daemon_turn/execute.rs` (lines 10, 312-360)

- [ ] **Step 1: Remove the now-unused import** — delete line 10:

```rust
use crate::core::core_systems::CoreSystems;
```

(It was added in Phase 1 solely for the guard, which this task removes.)

- [ ] **Step 2: Replace the space block** — replace the entire region from line 312 (`let turn_space = SpaceId::new();`) through line 360 (the closing `};` of `let _space_guard = ...`) with:

```rust
        // Phase 2a: reuse the main agent's long-lived process space (one per
        // session, not per turn). Bindings are upserted so the Agora version is
        // refreshed in place rather than accumulating. Space is released on
        // process exit (see orchestrator::exit_process).
        let agent_space = match self.process_table.inspect(main_pid).await {
            Ok(snap) => snap.space,
            Err(e) => {
                tracing::warn!(target: "space", error = %e, "inspect(main_pid) failed; using ephemeral space for this turn");
                SpaceId::new()
            }
        };
        self.subsystems.ports.space_manager.upsert_binding(
            agent_space,
            ContextBinding::Session(SessionId(sess_id.clone())),
        );
        self.subsystems.ports.space_manager.upsert_binding(
            agent_space,
            ContextBinding::Agora(AgoraSpaceId(sess_id.clone()), AgoraVersion(agora_version)),
        );
        if let Err(e) = self.subsystems.ports.space_manager.set_overlay(
            agent_space,
            "turn_input",
            serde_json::json!(message),
        ) {
            tracing::warn!(target: "space", error = %e, "failed to store turn input overlay");
        }
```

Notes: `main_pid` is in scope (`execute.rs:42`); `self.process_table` is the direct `Arc<ProcessTable>` field (`orchestrator.rs:34`); `upsert_binding` is sync (no `.await`, no `Result`). `agora_version` is read just above (line 305-310) and unchanged. No other reference to `turn_space` exists after this block (verified).

- [ ] **Step 3: Verify build (guard + import gone, no unused warnings)**

Run: `cargo build -p executive 2>&1 | tail -20`
Expected: PASS — no `unused import: CoreSystems`, no reference-to-`turn_space` error.

- [ ] **Step 4: Confirm no stray `CoreSystems` / `turn_space` remain**

Run: `grep -n "CoreSystems\|turn_space\|SpaceReleaseGuard" crates/executive/src/service/daemon_turn/execute.rs`
Expected: no output.

- [ ] **Step 5: Run executive tests**

Run: `cargo test -p executive 2>&1 | tail -20`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/executive/src/service/daemon_turn/execute.rs
git commit -m "refactor(executive): reuse process.space in execute_turn

Point turns at the main agent's long-lived process.space via
inspect(main_pid), upsert Session/Agora bindings (no accumulation), and
remove the Phase-1 per-turn SpaceReleaseGuard. Space is now released on
process exit instead of per turn (Phase 2a)."
```

---

## Task 4: Release the space in `exit_process`

**Files:**
- Modify: `crates/executive/src/service/daemon_turn/orchestrator.rs:146-150`

- [ ] **Step 1: Replace `exit_process`** — replace the body:

```rust
    pub async fn exit_process(&self, process_id: ProcessId) -> anyhow::Result<()> {
        self.process_table
            .signal(process_id, ProcessSignal::Terminate)
            .await
    }
```

with:

```rust
    pub async fn exit_process(&self, process_id: ProcessId) -> anyhow::Result<()> {
        // Capture the process's space before termination so we can release it.
        let space = self
            .process_table
            .inspect(process_id)
            .await
            .ok()
            .map(|snap| snap.space);
        self.process_table
            .signal(process_id, ProcessSignal::Terminate)
            .await?;
        if let Some(space) = space {
            self.subsystems.ports.space_manager.release(space);
        }
        Ok(())
    }
```

- [ ] **Step 2: Verify build**

Run: `cargo build -p executive 2>&1 | tail -20`
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add crates/executive/src/service/daemon_turn/orchestrator.rs
git commit -m "fix(executive): release process.space on exit_process

Fetch the process's space via inspect() before Terminate and release it
from the space manager after, so the long-lived per-session space is
cleaned up on process exit (Phase 2a)."
```

---

## Task 5: Full-workspace validation + doc status

**Files:** `docs/arch/CURRENT_ARCHITECTURE_AND_COUPLING_ANALYSIS.md`

- [ ] **Step 1: Format check**

Run: `cargo fmt --all --check`
Expected: PASS. (If it fails, `cargo fmt --all` and re-stage into the relevant task commit or a small follow-up commit.)

- [ ] **Step 2: Clippy**

Run: `cargo clippy -p aletheon-kernel -p executive --all-targets 2>&1 | tail -30`
Expected: PASS — no warnings (no unused imports, no dead code).

- [ ] **Step 3: Targeted tests**

Run: `cargo test -p aletheon-kernel -p executive 2>&1 | tail -20`
Expected: PASS — including `snapshot_exposes_process_space`, `upsert_replaces_singletons_appends_artifacts`, `reused_space_does_not_grow_across_turns`, and Phase-1 tests.

- [ ] **Step 4: Update the coupling doc** — in `docs/arch/CURRENT_ARCHITECTURE_AND_COUPLING_ANALYSIS.md` §9.5 / §9.7 / §9.8: mark "execute_turn 复用 process.space" and "turn_space 泄露" as ✅ Phase 2a done (executive-orchestrated, `upsert_binding` + release on `exit_process`); keep `ProcessTable::spawn` → `fork_space`, sub-agent inheritance, and `SpaceManager` **trait** promotion as Phase 2b pending.

```bash
git add docs/arch/CURRENT_ARCHITECTURE_AND_COUPLING_ANALYSIS.md
git commit -m "docs(arch): mark process.space reuse done (Phase 2a)"
```

---

## Self-Review

- **Spec coverage:** design Change 1 → Task 1; Change 2 → Task 2; Change 3 (incl. guard + import removal) → Task 3; Change 4 → Task 4; §6 tests → Task 1 Step 1, Task 2 Step 1, Task 5 Step 3; §2 Phase-1-guard removal → Task 3 Steps 1-2; §7 deferrals → excluded.
- **Placeholder scan:** none — all code, commands, and expected outputs concrete.
- **Type consistency:** `ProcessSnapshot.space: SpaceId` matches `runtime.record.space` and `snap.space` reads in Tasks 3/4; `upsert_binding(&self, SpaceId, ContextBinding)` used identically in impl, tests, and `execute_turn`; component types imported from `fabric::types::space`; `self.process_table` (field, `orchestrator.rs:34`) and `self.subsystems.ports.space_manager` used consistently.
```
