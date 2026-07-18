# Plan Blockage Analysis: What "blocked-by R2/R3" Really Means

> Generated: 2026-07-18 | Source: Code audit of all active plans + coverage matrix + execution index

---

## The "Blocked" Label is Misleading

The coverage matrix marks 3 plans as `blocked-by R2/R3 + V02`. This label refers to the **final aggregate acceptance gate**, not to implementation-level blocking. Each "blocked" plan has substantial code-level work that can proceed independently today.

---

## Per-Plan Analysis

### 1. Architecture Coupling Optimization — Most Phases Can Proceed NOW

| Phase | Status | Blocked by R2/R3? | Can proceed? |
|---|---|---|---|
| 0 — architecture drift gates | Partial | No | ✅ Yes — shrink-only gates need refinement |
| 1 — governed capability path | Partial | No | ✅ Yes — hook/plugin/host effects still outside boundary |
| 2 — Session/Turn/Item lifecycle | Partial | No | ✅ Yes — Interact still constructs JSON-RPC manually |
| 3 — Kernel authority cleanup | **Complete** | N/A | — |
| 4 — Executive use-case boundaries | Partial | No | ✅ Yes — 38+ concrete fields + 14 Mutex to untangle |
| 5 — authoritative domain facades | Partial | No | ✅ Yes — Dasein still uses concrete Kernel timer |
| 6 — event spine and projections | Partial | No | ✅ Yes — legacy Envelope transport remains |
| 7 — configuration/extensions/Interact | Partial | No | ✅ Yes — Corpus has parallel hook/skill module trees |
| 8 — optional crate split | Not started | No | ✅ Yes — evidence-driven, optional |

**Conclusion:** 8 of 9 phases are independently actionable. The R2/R3 dependency is only on the final gate.

### 2. Dasein-Agora Conscious Core — Phases 0-2 Can Proceed NOW

| Phase | Status | Blocked by R2/R3? | Can proceed? |
|---|---|---|---|
| 0 — source baseline | Partial | No | ✅ Yes — stale source descriptions vs emitted CareDecision |
| 1 — Dasein state engine | Partial | No | ✅ Yes — public layer mutation surfaces at `care.rs:101-111`, `identity.rs:55` |
| 2 — typed Agora/integrity | Partial | No | ✅ Yes — legacy direct `publish`/`update` at `ops/mod.rs:143-152` |
| 3 — competition/broadcast | **Code-complete** | N/A | — |
| 4 — recurrent loop | **Code-complete** | N/A | — |
| 5 — memory/metacog/agents | Partial | **Partially** | ⚠️ Dasein processor emits no response candidates; R2/R3 field metrics not wired |

**Conclusion:** 3 of 4 partial phases are independently actionable. Only Phase 5 has genuine R2/R3 coupling.

### 3. Executable Plan Decomposition — Genuinely Dependent

This is a meta-plan (decomposing 4 high-level plans into 42 executable units). Its terminal closure genuinely requires R2/R3 completion because all sub-plans must be resolved. However, 29/42 units already have ID + dependency mapping — the decomposition itself is architecturally complete.

### 4. V02 Production Migration — External, Not Code-Blocked

| Issue | Nature |
|---|---|
| Tasks 1-6 implementation artifacts | **Code exists** |
| Live Gmail/SubAgent/TUI scenarios | Requires real-host credentials |
| Injected-failure operator receipts | Requires physical host access |
| Aggregate operator receipt | Requires running the full migration script |

**Conclusion:** V02 is blocked by operational access, not by missing code or R2/R3.

---

## Root Cause: What Actually Blocks R2/R3 Itself?

R2/R3 is not blocked by anything external. **It has unfinished code.**

| Task | Status | What's Missing |
|---|---|---|
| Task 1 — Fabric contracts | ✅ Complete | — |
| Task 2 — R2 reader/fallback | ✅ Complete | — |
| Task 3 — Production reader/session | ⚠️ Partial | Production binding incomplete; still uses thread ID at `turn_pipeline.rs:114-136` |
| Task 4 — Metrics/trace | ⚠️ Partial | Types exist, coordinator doesn't record snapshots |
| Task 5 — Batch planning | ✅ Complete | — |
| **Task 6 — Action salience** | **❌ Missing** | Salience still constant at `conscious_action.rs:88-127`; urgency hardcoded `0.7` at `conscious_core_coordinator.rs:395-416` |
| **Task 7 — Proceed/Defer** | **❌ Missing** | `GovernedActionLoop` only returns `SelectedActionContext`; inner invoker always called at `governed_capability.rs:86-96,148-172` |
| **Task 8 — Arbitration mode config** | **❌ Missing** | No production arbitration-mode configuration or workspace-backed priority planner |
| **Task 9 — Cross-domain acceptance** | **❌ Missing** | No current-worktree acceptance tests, strict checks, or plan reconciliation |

---

## The Real Blockers: 4 Semantic Gaps R2/R3 Must Solve

These are the root-cause architecture problems that R2/R3 is designed to fix:

| # | Gap | Location | R2/R3 Task |
|---|---|---|---|
| 1 | **CareAction computed then discarded** | `dasein/src/dasein/reducer.rs:408-417` | Task 6 (real salience) |
| 2 | **Sub-agent context injection returns empty defaults** | `cognit/src/runtime/native_cognit.rs:428-436` | Task 3 (session identity) |
| 3 | **Agora Attention struct is dead code** | `agora/src/workspace/attention/mod.rs:7-31` | Task 4 (metrics drive attention) |
| 4 | **SelfField (8-layer) and DaseinModule disconnected** | Two separate subsystems | Task 2 (completed), Task 3 (wiring) |

---

## Priority Action Plan

### Can do NOW (independent of R2/R3):

1. **Architecture coupling:** Untangle TurnPipeline/DaemonTurnOrchestrator/TurnRuntimeResources concrete fields
2. **Dasein state engine:** Close public mutation surfaces
3. **Agora integrity:** Remove legacy direct publish/update
4. **Architecture drift gates:** Tighten Phase 0 checks
5. **Executive use-case boundaries:** Extract trait objects from TurnRuntimeResources

### Requires R2/R3 completion first:

1. **Complete R2/R3 Tasks 6-9** (the 4 Missing tasks)
2. **Wire CareAction → GovernedActionLoop** (semantic gap #1)
3. **Enable Observe-mode production default** (Task 8)
4. **Cross-domain acceptance tests** (Task 9)

### After R2/R3 is done:

1. Architecture coupling optimization final gate
2. Dasein-Agora conscious core final gate
3. Executable plan decomposition terminal closure
4. V02 aggregate operator receipt

---

## Summary

| Claim | Truth |
|---|---|
| "3 plans blocked by R2/R3" | True for **final gates only**. Code-level work in those plans is unblocked. |
| "R2/R3 is blocked" | False. R2/R3 has 4/9 tasks unfinished — pure implementation work, no external dependency. |
| "V02 blocks everything" | False. V02 is external (host access), not code-dependent. |
| **Root cause** | 4 semantic gaps + 4 missing R2/R3 tasks. Neither is a dependency-chain problem — both are execution-priority problems. |
