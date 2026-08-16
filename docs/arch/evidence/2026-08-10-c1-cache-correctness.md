# Aletheon closure plan：C1 cache correctness & observability

Date: 2026-08-10
Baseline: `0bf690b2`
Plan: `Aletheon_Runtime_Product_Convergence_and_Engineering_Closure_Plan_2026-08-07.md` §16
State: fail-closed cache policy + observable prompt profile established; concrete caches wire in at C1 cutover

## Context receipt (runbook §3.1)

```text
Slice: C1 cache correctness & observability (policy seam)
Baseline commit: 0bf690b2
Plan revision: closure-plan §16
Direct prerequisites: APX-01 (Application) — done
Current authoritative writer: unchanged legacy cache paths (recall_cache, provider prefix)
Target owner/writer: Application cache policy; not yet wired
IDs minted here: none
Production callers: none yet (policy additive)
Test-only callers: 3 cache tests
Installed/config callers: none
Tables/files/wire schemas: none changed
External side effects: none
Compatibility seam: none
Deletion owner: n/a
Unknowns/blockers: none for the seam
Expected files: crates/application/src/cache.rs, lib.rs re-export, C1 gate
Out-of-scope files: concrete provider/recall/tool-result cache wiring + telemetry
```

## 1. What was created

`crates/application/src/cache.rs`:

- **`CacheLayer`** — ProviderPrefix / RecallResult / ReadOnlyToolResult (three layers).
- **`CacheKey`** — layer + permission_ctx + tool_digest + optional dependency_digest.
- **`CacheDecision`** — Cache(key) / Bypass.
- **`decide_cache`** — fail-closed: side-effectful / permission-sensitive / time-sensitive bypass.
- **`PromptConstructionProfile`** — observable (real tool_definitions/tool_count + hit/miss/bypass/stale-reject counters).

## 2. Rules honoured (C1)

- **Permission context enters the key or forces bypass**: `CacheKey.permission_ctx`; `decide_cache` bypasses permission-sensitive.
- **Tool schema/version change invalidates**: `CacheKey.tool_digest`.
- **Errors/cancels/partial output not cached by default**: fail-closed `Bypass`.
- **Streaming and ordinary paths share one key model**: `CacheKey` is the single key shape.
- **Cache-miss ≡ cache-disabled**: the policy has no distinct success path; `Bypass` on any doubt.
- **Mutable-file reads carry dependency digests**: `with_dependency(content,head)` (test-verified).
- **Prompt profile observable with real tool partition (not fixed 0)**: `PromptConstructionProfile` fields.

## 3. C1 gate (architecture-check.sh)

`ARCH_SKIP_C1_GATES` requires `CacheDecision`/`CacheKey`/`permission_ctx`/`Bypass`/`PromptConstructionProfile`; and the `is_side_effectful`/`is_permission_sensitive`/`is_time_sensitive` fail-closed inputs.

## 4. Validation

```text
bash scripts/cargo-agent.sh check -p application    PASS
bash scripts/cargo-agent.sh test -p application --lib cache  PASS (3 passed)
  - side_effectful_tool_is_never_cached
  - read_only_immutable_is_cached_with_key
  - mutable_read_requires_dependency_digest
bash scripts/cargo-agent.sh fmt --all -- --check   PASS
git diff --check                                    PASS
ARCH_SKIP_DEPENDENCIES=1 bash scripts/aletheon.sh acceptance architecture  PASS (23 findings, 0 deps)
```

## 5. Remaining C1 (cutover)

Pass real tool definitions/count into prompt partition, consume `assembled_context.profile` as turn telemetry, configure recall-cache capacity/TTL, and wire the concrete three-layer caches with hit/miss/bypass/stale-reject metrics + error-equivalence tests.

## 6. Rollback

Delete `cache.rs` + the lib.rs re-export + the C1 gate → exact baseline. No schema, no writer, no daemon change.

## 7. Next

M1 (evidence-driven multi-agent controller), A1 (installation scoreboard), E1 (robot benchmark) remain in the closure plan.
