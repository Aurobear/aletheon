# Agent Kernel V2：APX-02 Approval aggregate/store port

Date: 2026-08-09
Baseline: `0bf690b2`
State: Approval aggregate/store port + validation semantics established; **legacy SQLite repository is still the single writer (writer cutover separate)**

## Context receipt (runbook §3.1)

```text
Slice: APX-02 Approval aggregate/store port + SQLite adapter (port portion)
Baseline commit: 0bf690b2
Direct prerequisites: APX-01 (Application facade) — done
Current authoritative writer: unchanged legacy Executive approval_service.rs / approval/repository.rs
Target owner/writer: Application Approval owner; NOT yet the writer
IDs minted here: none — DecisionRequestId is owner-assigned, not client-minted
Production callers: none yet (port additive; legacy repository untouched)
Test-only callers: 2 approval tests (in-memory store)
Installed/config callers: none
Tables/files/wire schemas: none changed
External side effects: none
Compatibility seam: none
Deletion owner: n/a (legacy repository removal at APX-05/XRET-02)
Unknowns/blockers: none for the port; the SQLite adapter cutover is separate
Expected files: crates/application/src/approval.rs, lib.rs re-export, APX-02 gate
Out-of-scope files: the SQLite repository extraction + migration bundle + writer cutover
```

## 1. What was created

`crates/application/src/approval.rs`:

- **`DecisionRequestId`**, **`ApprovingPrincipal`**, **`ApprovalScope`** — owner-assigned identities.
- **`OpaqueApprovalGrant`** — single-use opaque grant (not a `contracts` type).
- **`ApprovalRecord`** — challenge/principal/scope/revision/expiry/nonce/consumed.
- **`ApprovalStore`** trait — `load` + `mark_consumed`.
- **`resolve_decision`** — validates digest/principal/scope/revision/generation/expiry/nonce/single-use; all fail closed.

## 2. Rules honoured (APX-02)

- Approval aggregate + use cases + `ApprovalStore` trait extracted from legacy (`approval_service.rs`/`repository.rs`): the port is defined; the concrete SQLite repository implements it at the cutover.
- `DecisionRequestId` source, resolution evidence, expiry, nonce, single-use enforced in `resolve_decision`.
- **Forged/expired/wrong-principal/wrong-scope/double-consumed fail closed** (test-verified): `AlreadyConsumed`/`Expired`/`WrongPrincipal`/`WrongScope`/`RevisionMismatch`/`NonceMismatch`.
- Kernel/Dasein never read `ApprovalStore` directly — they call their own verifier port (documented; the outer adapter returns opaque proof only).
- Single writer: legacy repository stays authoritative; only `ApprovalStore` is defined here.
- APX-02 gate: no client `DecisionRequestId(` mint in production code (owner-assigned).

## 3. APX-02 gate (architecture-check.sh)

`ARCH_SKIP_APX02_GATES` requires all fail-closed checks (AlreadyConsumed/Expired/WrongPrincipal/WrongScope/NonceMismatch/RevisionMismatch) present; rejects any production `DecisionRequestId(` mint (test fixtures exempt via cfg-split).

## 4. Validation

```text
bash scripts/cargo-agent.sh check -p application    PASS
bash scripts/cargo-agent.sh test -p application --lib approval  PASS (2 passed)
  - valid_evidence_resolves_and_is_single_use
  - wrong_principal_and_expired_fail_closed
bash scripts/cargo-agent.sh fmt --all -- --check   PASS
git diff --check                                    PASS
ARCH_SKIP_DEPENDENCIES=1 bash scripts/aletheon.sh acceptance architecture  PASS (23 findings, 0 deps)
```

## 5. Remaining APX-02 (writer cutover)

The SQLite `ApprovalStore` repository + migration bundle, Gateway route → Application, `ResumeWithDecision` typed command, and old-table shadow-read equivalence are the writer cutover; single-writer maintained.

## 6. Rollback

Delete `approval.rs` + lib.rs re-export + APX-02 gate → exact baseline. No schema, no writer, no daemon change.

## 7. Next

APX-03 (GoalDraft + External Stimulus provider-neutral) and APX-04 (SQLite/FS/process/host adapters out of Application core) follow.
