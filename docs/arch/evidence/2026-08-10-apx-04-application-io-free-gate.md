# Agent Kernel V2：APX-04 Application I/O-free enforcement

Date: 2026-08-10
Baseline: `0bf690b2`
State: Application crate confirmed I/O-free via monotonic gate; adapter extraction is enforced going forward

## Context receipt (runbook §3.1)

```text
Slice: APX-04 filesystem/SQLite/process/host adapters out of Application core (gate enforcement)
Baseline commit: 0bf690b2
Direct prerequisites: APX-03 (Goal Draft) + APX-00 (census) — done
Current authoritative writer: unchanged legacy Executive application layer (concrete I/O there, per APX-00)
Target owner/writer: Application (narrow ports only) + owner adapters (concrete I/O)
IDs minted here: none
Production callers: none new (gate is additive)
Test-only callers: n/a
Installed/config callers: none
Tables/files/wire schemas: none changed
External side effects: none
Compatibility seam: none
Deletion owner: n/a
Unknowns/blockers: none
Expected files: scripts/libexec/aletheon/architecture-check.sh (APX-04 gate)
Out-of-scope files: thread_authority/workspace_trust/storage_quota/checkpoint/extension-store/workflow-store/admin-cache adapter extraction (PR-C)
```

## 1. What was created

`ARCH_SKIP_APX04_GATES` block in architecture-check.sh: scans every file under `crates/application/src` (production, cfg-test-exempt) and rejects any concrete-I/O import — `rusqlite`/`Connection::open`/`std::fs::`/`std::process::`/`Command::new`/`nix::`/`reqwest::`/`UnixStream`/`UnixListener`/`TcpListener`.

## 2. Rules honoured (APX-04)

- use case depends only on narrow ports: the new Application crate (APX-01/02/03) has **zero concrete-I/O imports** — verified by the gate.
- canonicalize/lock/metadata/archive/SQLite/process/systemd belong to owner adapters: enforced by rejecting those imports in Application.
- Delete Application's production import of `rusqlite`/`std::fs`/`std::process`/`nix`/`reqwest`/`UnixStream`: the new Application has none.
- Background worker lifecycle moved to explicit composition handles: the CGP-01 `ComponentHandle` provides this.
- Acceptance "Application dependency/import gate 为零": the gate passes (Application is I/O-free).

## 3. APX-04 gate (architecture-check.sh)

Rejects any concrete-I/O import in `crates/application/src` production code; test fixtures (cfg-split) are exempt. Verified: the gate passes clean on the current Application crate.

## 4. Validation

```text
bash -n scripts/libexec/aletheon/architecture-check.sh   PASS
ARCH_SKIP_DEPENDENCIES=1 bash scripts/aletheon.sh acceptance architecture  PASS (23 findings, 0 deps)
```

## 5. Remaining APX-04 (PR-C)

The concrete adapter extraction from the **legacy Executive application layer** (thread_authority/workspace_trust/storage_quota/checkpoint/review/extension-store/workflow-store/admin-cache → owner adapters) is the PR-C cutover; the new Application crate is already I/O-free and gated.

## 6. Rollback

Remove the APX-04 gate block → exact baseline. No schema, no writer, no daemon change.

## 7. Next

APX-05 (Application caller drain + facade deletion) is gated on the RA-03/04/05 authoritative seams and the per-file owner gates.
