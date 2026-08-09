# Agent Kernel V2：RA-05 PR-C Runtime AgentSupervisor writer (deployable)

Date: 2026-08-10
Baseline: `0bf690b2`
Plan: `docs/plans/2026-08-08-runtime-authority-consolidation.md` §7 RA-05, runbook PR-C
State: deployable Runtime AgentSupervisor writer built; legacy AgentControlService stays authoritative until the PR-C deployment

## Context receipt (runbook §3.1)

```text
Slice: RA-05 PR-C AgentSupervisor writer (code)
Baseline commit: 0bf690b2
Plan revision: runtime-authority-consolidation §7 RA-05, runbook §7.3 PR-C
Direct prerequisites: RA-05 PR-A (AgentSupervisor seam + DelegateBackendRegistry) — done
Current authoritative writer: unchanged legacy AgentControlService/AgentRuntimeRegistry
Target owner/writer: RuntimeAgentSupervisor; not yet wired
IDs minted here: none — the writer assigns run/generation IDs at runtime
Production callers: none yet (writer additive)
Test-only callers: 2 agent_writer tests
Installed/config callers: none (not yet deployed)
Tables/files/wire schemas: none changed
External side effects: none (no deploy yet)
Compatibility seam: none (PR-B shadow is the deploy step)
Deletion owner: legacy AgentControl → RA-06/XRET-02
Unknowns/blockers: the installed acceptance (deploy + Native/Pi drill + rollback binary) is the deployment slice
Expected files: crates/runtime/src/agent_writer.rs, lib.rs re-export, RA-05C gate
Out-of-scope files: Pi concrete migration (E6-K6d), the daemon wiring + deployment
```

## 1. What was created

`crates/runtime/src/agent_writer.rs`:

- **`RuntimeAgentSupervisor`** — one spawn/wait/cancel entry over the generic `DelegateBackendRegistry`; the Runtime resolves and **pins** the running binding per AgentRunId (reload never swaps a running binding).
- **`StubBackend`** — test backend.

## 2. Rules honoured (RA-05 PR-C)

- **Runtime assigns child/generation/run ID**: the writer resolves through the registry and returns the `DelegateReceipt` (agent_run + generation).
- **One spawn/send/wait/cancel/recovery entry**: `spawn`/`wait`/`cancel` are the only entry points (RA-05C gate).
- **Backend reload does not swap a running binding**: `bindings: HashMap<AgentRunId, Arc<dyn DelegateBackend>>` pins the exact backend.
- **No Pi file migration or double execution**: RA-05C gate rejects `PiRuntime`/`PiRpcRuntime`/`register_pi_runtime`; E6-K6d owns Pi.
- **wait before terminal never returns success**: `wait` returns only the backend's authoritative typed terminal (fail-closed).
- Legacy writer stays authoritative until PR-C deployment.

## 3. RA-05C gate (architecture-check.sh)

`ARCH_SKIP_RA05C_GATES` requires `RuntimeAgentSupervisor`/`DelegateBackendRegistry`/`bindings`; rejects Pi symbols.

## 4. Validation

```text
bash scripts/cargo-agent.sh check -p runtime       PASS (0 errors/warnings)
bash scripts/cargo-agent.sh test -p runtime --lib agent_writer  PASS (2 passed)
  - supervisor_spawns_and_wait_returns_authoritative_terminal
  - cancel_uses_the_pinned_backend
bash scripts/cargo-agent.sh fmt --all -- --check   PASS
git diff --check                                    PASS
ARCH_SKIP_DEPENDENCIES=1 bash scripts/aletheon.sh acceptance architecture  PASS (23 findings, 0 deps)
```

## 5. Remaining RA-05 PR-C (deployment)

Wire into the daemon (gated, like RA-03), then maintenance/drain + freeze + switch, `sudo deploy` + SHA-256 compare + systemd restart counter + Native/Pi installed drill + cancel/restart/reconcile drills + rollback binary. Per plan §12.4, RA-03/04/05 must not be merged.

## 6. Rollback

Delete `agent_writer.rs` + the lib.rs re-export + the RA-05C gate → exact baseline. No schema, no writer, no daemon change.

## 7. Next

The RA-03/04/05 PR-C **deployment** (gated wiring exists for RA-03; RA-04/05 writers ready), then E6-K6d (Pi DelegateBackend) and RA-06.
