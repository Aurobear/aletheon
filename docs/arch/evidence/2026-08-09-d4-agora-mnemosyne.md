# Agent Kernel V2：D4 Agora/Mnemosyne convergence

Date: 2026-08-09
Baseline: `0bf690b2`
Plan: `docs/plans/2026-08-08-domain-authority-and-adapter-extraction.md` §D4
State: Agora non-authoritative workspace port established; Mnemosyne supplemental port already stable; no writer cutover

## Context receipt (runbook §3.1)

```text
Slice: D4 Agora/Mnemosyne convergence (port seam)
Baseline commit: 0bf690b2
Plan revision: domain-authority §D4
Direct prerequisites: D1 + D2/D3 — done
Current authoritative writer: unchanged legacy Executive memory/workspace + Agora workspace
Target owner/writer: Agora (non-authoritative workspace) + Mnemosyne (supplemental port); cutover later
IDs minted here: none
Production callers: none yet (ports additive)
Test-only callers: 1 workspace-port test
Installed/config callers: none
Tables/files/wire schemas: none changed
External side effects: none
Compatibility seam: none
Deletion owner: Executive memory/workspace merge → cutover
Unknowns/blockers: none for the seam
Expected files: crates/agora/src/workspace/port.rs, workspace/mod.rs, agora Cargo.toml, D4 gate
Out-of-scope files: GBrain file migration (E3), Executive memory/workspace merge
```

## 1. What was created

`crates/agora/src/workspace/port.rs`:

- **`WorkspaceProjection`** — rebuildable workspace projection (workspace_id, revision, items).
- **`ActiveWorkspacePort`** — non-authoritative port: `rebuild` + `degraded`.
- **`AgoraError`** — typed NotFound / Unavailable.
- **`InMemoryWorkspacePort`** — test port (rebuild + degraded).

Mnemosyne's `supplemental` port (lib.rs:154) is already the stable supplemental-memory surface E3 depends on — verified present, no change needed.

## 2. Rules honoured (D4)

- **Agora → non-authoritative active workspace**: `ActiveWorkspacePort` is read/rebuild only; no canonical session/turn write (D4 gate rejects Executive/session-write in the port).
- **Projection rebuildable + degraded mode**: `rebuild` + `degraded` (test-verified).
- Memory core separated from SQLite/provider at the cutover; GBrain file/worker/lease/writer switch is E3 (documented; not done here).
- Executive memory/workspace modules merge at the cutover (not this seam).
- Mnemosyne supplemental-memory port stable for E3 (already present).

## 3. D4 gate (architecture-check.sh)

`ARCH_SKIP_D4_GATES` rejects any `Executive`/session-write/`SessionAppendStore`/`AgentRunRepository` in the Agora workspace port; requires `ActiveWorkspacePort` with `rebuild` + `degraded`.

## 4. Validation

```text
bash scripts/cargo-agent.sh check -p agora         PASS
bash scripts/cargo-agent.sh test -p agora --lib workspace::port  PASS (1 passed)
  - rebuild_and_degraded_modes
bash scripts/cargo-agent.sh fmt --all -- --check   PASS
git diff --check                                    PASS
ARCH_SKIP_DEPENDENCIES=1 bash scripts/aletheon.sh acceptance architecture  PASS (23 findings, 0 deps)
```

## 5. Rollback

Delete `port.rs` + the workspace/mod.rs module line + agora Cargo.toml thiserror + D4 gate → exact baseline. No schema, no writer, no daemon change.

## 6. Next

D5 (Corpus catalog/executor split), then D6 (non-extension Fabric cleanup) closes the D-series.
