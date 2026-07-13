# Space Lifecycle Fix — Phase 2a (Reuse `process.space`)

> Date: 2026-07-12
> Status: design approved, pending implementation plan
> Scope: **Phase 2a** — daemon turns reuse the main agent's long-lived
> `process.space` instead of a per-turn ephemeral space; fix binding
> accumulation; release the space on process exit. Executive-orchestrated, no
> kernel process↔space coupling, no `SpaceManager` **trait** change.
> Predecessor: `docs/plans/2026-07-12-space-lifecycle-phase1-design.md` (leak
> stop-gap). Follow-up: Phase 2b (§7).

---

## 1. Problem & context

Phase 1 stopped the leak by releasing a per-turn ephemeral `turn_space` at the
end of every `execute_turn`. That is a stop-gap: the space concept is still
per-turn and throwaway, and the real `process.space` is unused.

Two facts make the correct model cheap:

- The main agent is **one long-lived process**, spawned once and cached in
  `main_agent_process_id` (`crates/executive/src/service/daemon_turn/lifecycle.rs`),
  reused across every turn. Its space is naturally a per-session object.
- `ProcessRecord` already carries `space: SpaceId`
  (`crates/fabric/src/types/process.rs:115`), minted by `spawn`
  (`crates/kernel/src/process/table.rs:139`).

So if turns use the main agent's `process.space`, **exactly one** space is ever
registered — reused across all turns, released at shutdown. This removes
per-turn growth *and* makes `process.space` real, without `fork_space` (fork /
inheritance only matters for sub-agents → Phase 2b).

### 1.1 Three tensions this resolves (or defers)

| Tension | Phase 2a resolution |
|---|---|
| `process.space` not readable (`ProcessSnapshot` omits it, `table.rs:156-164`) | Expose it as a read-only snapshot field (§3, Change 1). |
| Binding accumulation — `attach_region` blindly pushes (`manager.rs:104`); reused space + per-turn `Agora(version)` re-attach grows O(turns) | Add `upsert_binding` keyed on variant discriminant (§3, Change 2). |
| Process & Space managers decoupled — `spawn` can't call `fork_space` without wiring | **Deferred to Phase 2b.** 2a needs no fork. |

---

## 2. Interaction with Phase 1 (important)

Phase 2a **removes** the Phase-1 `SpaceReleaseGuard` in `execute.rs`
(lines 348-360) and the per-turn `turn_space = SpaceId::new()` (line 312). The
two are **not additive**: Phase 1 released per turn; Phase 2a keeps one
long-lived space and releases it on process exit. `InMemorySpaceManager::release`
and `space_count` (added in Phase 1) are retained and reused.

---

## 3. Changes

### Change 1 — expose `space` in `ProcessSnapshot`

File: `crates/fabric/src/types/process.rs` (add field to `ProcessSnapshot`,
line 156) and `crates/kernel/src/process/table.rs` (`inspect`, line 194 — copy
`record.space` into the snapshot).

```rust
// fabric/src/types/process.rs — ProcessSnapshot
pub space: SpaceId,
```

Read-only exposure of data already stored in `ProcessRecord`. This is **not**
the `spawn→fork_space` coupling deferred to 2b — `spawn` still mints the space
exactly as today; we only let callers read it.

### Change 2 — `upsert_binding` (inherent) on `InMemorySpaceManager`

File: `crates/kernel/src/space/manager.rs` (add alongside `set_overlay` /
`release`).

```rust
/// Insert or update a binding. For singleton-per-space variants (Session,
/// Agora, MemoryView, WorldProjection) an existing binding of the same variant
/// is replaced in place; Artifact bindings (multi-instance) are appended.
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

`ContextBinding` has 5 variants and no discriminant helper
(`crates/fabric/src/types/space.rs:82-92`), so `std::mem::discriminant` is the
matching mechanism. `attach_region` is left untouched for compatibility.

### Change 3 — `execute_turn` uses the main agent's space

File: `crates/executive/src/service/daemon_turn/execute.rs` (lines 312-360).

- Delete the `SpaceReleaseGuard` struct + `_space_guard` binding (lines 348-360).
- Replace `let turn_space = SpaceId::new();` (line 312) with the main agent's
  space, read from the process snapshot:

```rust
let agent_space = match self.subsystems.ports.process_table.inspect(main_pid).await {
    Ok(snap) => snap.space,
    Err(e) => {
        // Fail soft: fall back to an ephemeral space for this turn only.
        tracing::warn!(target: "space", error = %e, "inspect(main_pid) failed; using ephemeral space");
        SpaceId::new()
    }
};
```

- Change the two `attach_region(turn_space, ...)` calls (lines 316-335) to
  `upsert_binding(agent_space, Session(...))` and
  `upsert_binding(agent_space, Agora(sess_id, version))`, and the `set_overlay`
  (line 336) to target `agent_space`. `Session` upsert is idempotent across
  turns; `Agora` upsert refreshes the version in place; the `turn_input`
  overlay key overwrites (bounded).

> `main_pid` is already available at this point (`ensure_main_agent` at
> `execute.rs:42`; used again at line 359/554). `process_table` is reachable via
> `self.subsystems.ports.process_table` (`Arc<ProcessTable>`,
> `kernel/src/service/mod.rs:52`).

### Change 4 — release `process.space` on process exit

File: `crates/executive/src/service/daemon_turn/orchestrator.rs` (`exit_process`,
line 146).

```rust
pub async fn exit_process(&self, process_id: ProcessId) -> anyhow::Result<()> {
    let space = self
        .subsystems
        .ports
        .process_table
        .inspect(process_id)
        .await
        .ok()
        .map(|s| s.space);
    self.subsystems
        .ports
        .process_table
        .signal(process_id, ProcessSignal::Terminate)
        .await?;
    if let Some(space) = space {
        self.subsystems.ports.space_manager.release(space);
    }
    Ok(())
}
```

The main agent only exits at daemon shutdown, so its single space lives for the
session and is released whenever `exit_process(main_pid)` runs (or freed on
teardown if no shutdown hook calls it). Either way it is bounded — not a leak.

---

## 4. Data flow (across turns)

```
ensure_main_agent → main_pid (cached, long-lived)
  space = inspect(main_pid).space              ← same space every turn
turn N:   upsert Session (noop after first) · upsert Agora(vN) · overlay turn_input=msg
turn N+1: upsert Agora(vN+1) REPLACES vN       · overlay overwrites   ← no growth
daemon shutdown: exit_process(main_pid) → release(space)
```

`space_count()` stays at 1 for the session; the space holds exactly one
`Session` binding, one `Agora` binding (current version), and one `turn_input`
overlay key.

---

## 5. Error handling

- `inspect(main_pid)` failure → warn + fall back to an ephemeral `SpaceId::new()`
  for that turn (degrades to Phase-1-like behavior, never panics). Because the
  Phase-1 guard is gone, an ephemeral fallback space is *not* released; this is
  an error path only, acceptable and logged.
- `upsert_binding` / `release` are infallible on a poisoned mutex (no-op).

---

## 6. Testing

1. **Kernel unit — `upsert_binding`** (`manager.rs` tests):
   - `upsert_binding(s, Agora(a, v1))` then `upsert_binding(s, Agora(a, v2))` →
     `get_bindings(s)` has exactly one `Agora` binding with version `v2`.
   - `upsert_binding(s, Session(x))` twice → exactly one `Session` binding.
   - two distinct `Artifact` bindings → both retained (append semantics).
2. **Kernel unit — snapshot exposes space**: spawn a process, `inspect(pid)`,
   assert `snapshot.space == ` the record's space (non-nil, stable).
3. **Regression — no growth across turns** (kernel-level simulation of the turn
   cycle on a single reused space): loop N times doing `upsert Session`,
   `upsert Agora(vN)`, `set_overlay("turn_input", …)`; assert `space_count() == 1`
   and the space has exactly 2 bindings throughout.
4. **Executive**: `cargo test -p executive` stays green (turn behavior
   unchanged from the caller's perspective; only the space identity/lifecycle
   changed). Confirm the removed guard causes no per-turn release regression by
   asserting, in an integration test if the harness allows, that `space_count()`
   does not grow over multiple `execute_turn` calls.

Acceptance: all tests pass; `cargo fmt --all --check`, `cargo clippy -p
aletheon-kernel -p executive --all-targets` clean.

---

## 7. Deferred to Phase 2b (separate spec)

| Item | Rationale |
|---|---|
| `ProcessTable::spawn` calls `space_manager.fork_space(parent.space, pid)` | Kernel-owned space lifecycle (design `Final(2).md` §9.5); requires `space_manager` as `Arc<dyn SpaceManager>` injected into `ProcessTable` |
| Parent→child space inheritance for sub-agents | Sub-agents get real, forked spaces with downgraded write authority (`ContextBinding::fork_inherited`) |
| Promote `release`/`lookup`/`get_overlay`/`upsert_binding` onto the `SpaceManager` **trait** | Trait becomes the abstraction boundary once a second backend or `dyn` consumer exists |
| Kernel-owned release on `Terminate` signal | Move release from executive `exit_process` into `ProcessTable` so the invariant can't be bypassed |

---

## 8. Risk

Low-to-moderate. Change 1 adds a struct field (touches every `ProcessSnapshot`
construction — grep to update all call sites). Changes 2-4 are additive or
localized. The main behavioral shift is space identity moving from per-turn to
per-session; mitigated by the fail-soft fallback and the no-growth regression
test. No `SpaceManager` trait or `spawn` signature changes.
