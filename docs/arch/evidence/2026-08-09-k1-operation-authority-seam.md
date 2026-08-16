# Agent Kernel V2：K1 durable Operation authority seam

Date: 2026-08-09
Baseline: `0bf690b2`
Plan: `docs/plans/2026-08-08-kernel-enforcement-consolidation.md` §10 K1
State: durable Operation authority seam established; **no writer change**, legacy `OperationTable` stays authoritative

## Context receipt (runbook §3.1)

```text
Slice: K1 Operation authority + durable seam
Baseline commit: 0bf690b2
Plan revision: kernel-enforcement §10 K1
Direct prerequisites: K0 (effect census) + D1 (contracts seed) — done
Current authoritative writer: unchanged legacy in-memory OperationTable (kernel/src/operation/table.rs)
Target owner/writer: Kernel (durable ExecutionJournal); not yet wired
IDs minted here: none — OperationId is fabric-owned; this module defines state/epoch shapes
Production callers: none yet (contract/port only)
Test-only callers: none
Installed/config callers: none
Tables/files/wire schemas: none changed
External side effects: none
Compatibility seam: none
Deletion owner: n/a
Unknowns/blockers: none
Expected files: crates/kernel/src/operation/journal.rs, operation/mod.rs, K1 gate
Out-of-scope files: all writer cutovers, K2+
```

## 1. What was created (kernel plan §10 K1)

`crates/kernel/src/operation/journal.rs` — the durable Operation authority seam:

- **`OperationState`**: Submitted/Running/Succeeded/Failed/Panicked with `is_terminal()`.
- **`OperationEpoch(u64)`**: writer-generation gate (prevents a stale writer mutating a newer epoch).
- **`OperationRecord`**: Kernel-owned durable shape (`id`, `state`, `epoch`, `parent`).
- **`OperationReceipt`**: ref Runtime/owners hold; the Kernel owns the terminal (plan: "Runtime 只保存 Operation binding/receipt ref，不复制 Kernel terminal").
- **`OperationCommand`**: Submit/Start/Settle.
- **`ExecutionJournal` trait**: `read` + `replay_after` — **read/shadow only, no write method**.

## 2. Rules honoured (kernel plan §10 K1)

- Contract/port only: nothing writes here; the legacy `OperationTable` remains the single authoritative writer (no new/double writer).
- V2 only shadows/replays/reads — `ExecutionJournal` has no write/append signature (K1 gate enforces).
- Runtime stores only binding/receipt refs, never copies Kernel terminal.
- Rollback: legacy-authoritative; the journal module is additive and un-wired.

## 3. K1 gate (architecture-check.sh)

`ARCH_SKIP_K1_GATES` rejects (against Rust code, ignoring doc comments/strings):
- any mention of the legacy `OperationTable` in journal code;
- any write/append/insert/remove line;
- any `async fn ...(&mut self)` write signature;
- any `ExecutionJournal` trait method that is not `read`/`replay_after`.

Mutation-tested: adding `pub async fn append_record(&mut self)` rejected.

## 4. Validation

```text
bash scripts/cargo-agent.sh check -p kernel       PASS (0.31s)
bash scripts/cargo-agent.sh fmt --all -- --check   PASS
git diff --check                                    PASS
ARCH_SKIP_DEPENDENCIES=1 bash scripts/aletheon.sh acceptance architecture  PASS (23 findings, 0 deps)
```

## 5. Rollback

Delete `journal.rs` + the `pub mod journal;` re-exports + the K1 gate → exact baseline. No schema, no table, no writer, no daemon change.

## 6. Next

K1 done → **K2** (sealed descriptor + `CapabilityExecutor`) can follow. The K0 finding (in-memory non-durable OperationTable) is the exact gap this seam's `ExecutionJournal`/`OperationEpoch` addresses at K4.
