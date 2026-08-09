# Agent Kernel V2：RA-05 AgentSupervisor + DelegateBackendRegistry owner seam

Date: 2026-08-09
Baseline: `0bf690b2`
Plan: `docs/plans/2026-08-08-runtime-authority-consolidation.md` §7 RA-05, runbook PR-A/PR-C
State: AgentSupervisor seam established (PR-A); **generic registry only, no Pi migration, writer cutover is PR-C**

## Context receipt (runbook §3.1)

```text
Slice: RA-05 AgentSupervisor + DelegateBackendRegistry (PR-A portion)
Baseline commit: 0bf690b2
Plan revision: runtime-authority-consolidation §7 RA-05
Direct prerequisites: RA-04 (Turn reducer) — done
Current authoritative writer: unchanged legacy AgentControlService/AgentRuntimeRegistry
Target owner/writer: Runtime AgentSupervisor; NOT yet the writer (PR-A seam)
IDs minted here: none — AgentRunId/Generation are Runtime-assigned (ids.rs)
Production callers: none yet (seam additive; legacy agent path untouched)
Test-only callers: 2 supervisor tests (stub backend)
Installed/config callers: none
Tables/files/wire schemas: none changed
External side effects: none
Compatibility seam: none
Deletion owner: n/a (AgentControl deletion is RA-06/XRET-02)
Unknowns/blockers: none for the seam; the PR-C writer cutover is a separate deployment slice
Expected files: crates/runtime/src/agent_supervisor.rs, lib.rs re-export, RA-05 gate
Out-of-scope files: Pi concrete files (E6-K6d), the actual AgentControl→Runtime cutover (PR-C)
```

## 1. What was created

`crates/runtime/src/agent_supervisor.rs`:

- **`DelegateBackendId`** — stable backend id (native / pi-coder).
- **`DelegateSpawnRequest`** — typed spawn (parent session/turn, backend, profile); Runtime assigns AgentRunId/Generation.
- **`DelegateReceipt`** — Runtime-assigned agent_run + generation.
- **`DelegateBackend`** trait — `spawn`/`cancel`/`wait`.
- **`DelegateBackendRegistry`** — generic id→launcher map; rejects duplicate/empty ids.
- **`AgentSupervisorSeam`** — single spawn entry over the registry.

## 2. Rules honoured (plan §7 RA-05)

- **Generic `DelegateBackendRegistry` only, no Pi concrete files migrated** (RA-05 gate rejects `PiRuntime`/`PiRpcRuntime`/`register_pi_runtime`; E6-K6d owns Pi).
- **Single spawn/send/wait/cancel/recovery entry**: `AgentSupervisorSeam::spawn` is the one entry (PR-C adds the full lifecycle/mailbox/recovery).
- **Running AgentRun pinned to backend generation; reload does not swap binding**: documented in the trait/registry contract; enforced at PR-C.
- **No double writer**: legacy AgentControlService stays authoritative until PR-C.
- Production Markdown loader → profile port + precise naming: at PR-C.
- Pi stays on the one-way legacy backend seam until E6-K6d: this seam is generic only.

## 3. RA-05 gate (architecture-check.sh)

`ARCH_SKIP_RA05_GATES` rejects any Pi symbol (`PiRuntime`/`PiRpcRuntime`/`register_pi_runtime`) in the generic seam; requires the `DelegateBackendRegistry` and duplicate-backend rejection.

## 4. Validation

```text
bash scripts/cargo-agent.sh check -p runtime       PASS
bash scripts/cargo-agent.sh test -p runtime --lib agent_supervisor  PASS (2 passed)
  - registry_rejects_duplicate_backend
  - supervisor_spawns_via_registry_with_runtime_assigned_ids
bash scripts/cargo-agent.sh fmt --all -- --check   PASS
git diff --check                                    PASS
ARCH_SKIP_DEPENDENCIES=1 bash scripts/aletheon.sh acceptance architecture  PASS (23 findings, 0 deps)
```

## 5. Remaining RA-05 PR-C (separate deployment slice, §14.2)

The actual AgentControl→Runtime cutover — lifecycle/mailbox/recovery/settlement + AgentStream adapter, Runtime-assigned child/generation/run IDs, caller → `SpawnDelegate`, running-binding generation pinning, profile-port loader rename, installed Native/Pi acceptance + rollback binary — is a **deployment-gated slice**.

## 6. Rollback

Delete `agent_supervisor.rs` + lib.rs re-export + RA-05 gate → exact baseline. No schema, no writer, no daemon change.

## 7. Next

With RA-02..RA-05 PR-A seams complete, the strict main-writer chain's PR-C cutovers (each independently deployed per §14.2) are the remaining deployment work; then E6-K6d (Pi DelegateBackend) and RA-06.
