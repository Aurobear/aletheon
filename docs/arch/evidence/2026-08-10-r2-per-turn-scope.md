# Aletheon closure plan：R2 per-turn OperationScope

Date: 2026-08-10
Baseline: `0bf690b2`
Plan: `Aletheon_Runtime_Product_Convergence_and_Engineering_Closure_Plan_2026-08-07.md` §10
State: per-turn scoped OperationScope with RAII drain established; legacy `TurnPipeline.current_scope` global slot not yet removed (that's the R2 cutover)

## Context receipt (runbook §3.1)

```text
Slice: R2 per-turn OperationScope + full-path cleanup
Baseline commit: 0bf690b2
Plan revision: closure-plan §10
Direct prerequisites: R1 (typed Turn outcome) — done
Current authoritative writer: unchanged legacy TurnPipeline.current_scope (Arc<Mutex<Option<OperationScope>>>)
Target owner/writer: Runtime per-turn scope; not yet wired
IDs minted here: none
Production callers: none yet (scope additive)
Test-only callers: 3 per-turn-scope tests
Installed/config callers: none
Tables/files/wire schemas: none changed
External side effects: none
Compatibility seam: none
Deletion owner: TurnPipeline current_scope → RA-04/XRET-02
Unknowns/blockers: none for the seam
Expected files: crates/runtime/src/per_turn_scope.rs, lib.rs re-export, R2 gate
Out-of-scope files: TurnPipeline current_scope removal + resource census
```

## 1. What was created

`crates/runtime/src/per_turn_scope.rs`:

- **`ScopedResource`** — resource bound to a turn generation, idempotent drain.
- **`ScopeExit`** — Settle / Abort.
- **`PerTurnScope`** — local per-turn scope (generation, resources, drained flag).
- **`ScopeGuard`** — RAII: `settle()`/`abort()` explicit; `Drop` triggers fallback abort drain.

## 2. Rules honoured (R2)

- Scope created and owned locally per `run_turn`: `PerTurnScope::new(generation)` is local, not shared.
- **RAII guard / Drop fallback cleanup**: `ScopeGuard::drop` drains if not explicitly drained (early-return/panic/disconnect safe).
- Normal end `settle_and_drain()`, abnormal end `abort_and_drain()`: both present.
- **Cleanup idempotent**: `drained` flag; a second drain is a no-op (test-verified).
- External PID/tool/temp/lease bound to turn generation: `ScopedResource` carries `turn_generation` (fencing).
- **No daemon-global mutable slot**: R2 gate rejects `current_scope`/`set_current_scope`/`take_current_scope` in the seam.

## 3. R2 gate (architecture-check.sh)

`ARCH_SKIP_R2_GATES` rejects any `current_scope`/`set_current_scope`/`take_current_scope` global slot; requires `settle_and_drain`/`abort_and_drain`/`Drop for ScopeGuard`/idempotent `drained`.

## 4. Validation

```text
bash scripts/cargo-agent.sh check -p runtime       PASS
bash scripts/cargo-agent.sh test -p runtime --lib per_turn_scope  PASS (3 passed)
  - settle_drains_all_and_is_idempotent
  - guard_drop_triggers_fallback_abort
  - concurrent_turns_do_not_share_scope
bash scripts/cargo-agent.sh fmt --all -- --check   PASS
git diff --check                                    PASS
ARCH_SKIP_DEPENDENCIES=1 bash scripts/aletheon.sh acceptance architecture  PASS (23 findings, 0 deps)
```

## 5. Remaining R2 (cutover)

Enumerate all `OperationScope` resource types + release methods, remove `TurnPipeline.current_scope` global slot, add resource-count/undrained gauge, and drive cancel/timeout through the same abort protocol. Gated on RA-04.

## 6. Rollback

Delete `per_turn_scope.rs` + the lib.rs re-export + the R2 gate → exact baseline. No schema, no writer, no daemon change.

## 7. Next

R3 (typed Command Output + protocol versioning) completes the R-series.
