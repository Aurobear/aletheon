# Agent Kernel V2：K5 Linux/execd ProcessController adapter

Date: 2026-08-09
Baseline: `0bf690b2`
State: ProcessController port established; legacy execd client stays authoritative; installed smoke is K5 PR-C

## Context receipt (runbook §3.1)

```text
Slice: K5 Linux/execd ProcessController adapter (port seam)
Baseline commit: 0bf690b2
Direct prerequisites: K4 + K0 (execd census) — done
Current authoritative writer: unchanged legacy execd client (executive/src/adapters/channel/execd_client.rs) + execd ProcessManager
Target owner/writer: platform ProcessController; not yet wired
IDs minted here: none — PidGeneration is adapter-local
Production callers: none yet (port additive)
Test-only callers: 1 process-controller test
Installed/config callers: none
Tables/files/wire schemas: none changed
External side effects: none
Compatibility seam: none
Deletion owner: n/a
Unknowns/blockers: none for the seam; installed spawn/read/cancel/restart smoke is the PR-C deployment slice
Expected files: crates/platform/src/process_controller.rs, platform lib.rs, K5 gate
Out-of-scope files: execd adapter cutover, installed verification
```

## 1. What was created

`crates/platform/src/process_controller.rs`:

- **`PidGeneration`** — pid + generation (stale-handle fencing).
- **`SpawnRequest`** — command/args/own_process_group.
- **`ProcessStatus`** — Running / Exited / FailedToStart.
- **`ProcessController`** trait — spawn/read/cancel/restart/status.
- **`ProcessError`** — typed (FailedToStart/UnknownPid/AlreadyTerminal/ReadTimeout).
- **`InMemoryProcessController`** — test controller.

## 2. Rules honoured (K5)

- `ProcessController` introduced, owning process group, signal, stdio, reconnect, PID generation.
- **execd has no silent in-process fallback**: `ProcessError::FailedToStart` is a typed terminal — a spawn that cannot start fails closed (no fallback).
- Legacy execd client stays authoritative until the new adapter smoke passes, then one-way switch (PR-C).
- Rollback only switches binary/config, keeps new journal/receipt and reconciles (PR-C).

## 3. K5 gate (architecture-check.sh)

`ARCH_SKIP_K5_GATES` requires `FailedToStart` (no-silent-fallback) and PID `generation` (stale-handle fencing).

## 4. Validation

```text
bash scripts/cargo-agent.sh check -p platform      PASS
bash scripts/cargo-agent.sh test -p platform --lib process_controller  PASS (1 passed)
  - controller_spawns_with_pid_generation
bash scripts/cargo-agent.sh fmt --all -- --check   PASS
git diff --check                                    PASS
ARCH_SKIP_DEPENDENCIES=1 bash scripts/aletheon.sh acceptance architecture  PASS (23 findings, 0 deps)
```

## 5. Remaining K5 (PR-C)

Installed-binary spawn/read/cancel/restart smoke, then the legacy execd client one-way switch; rollback switches binary/config only. Deployment-gated per §14.2.

## 6. Rollback

Delete `process_controller.rs` + the platform lib.rs module line + the K5 gate → exact baseline. No schema, no writer, no daemon change.

## 7. Next

K6 (extension enforcement seams/gates) delivers the stable Kernel seam + per-family registration gate; then the E-series joint cutovers consume K6.
