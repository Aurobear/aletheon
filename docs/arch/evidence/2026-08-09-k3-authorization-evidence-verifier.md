# Agent Kernel V2：K3 authorization evidence verifier

Date: 2026-08-09
Baseline: `0bf690b2`
State: opaque proof verifier + single-use store established; fail-closed on unavailable

## Context receipt (runbook §3.1)

```text
Slice: K3 authorization evidence verifier
Baseline commit: 0bf690b2
Direct prerequisites: K2 (sealed descriptor) — done
Current authoritative writer: unchanged legacy admission/production.rs
Target owner/writer: Kernel verifier port; not yet wired
IDs minted here: none — OpaqueProof is verifier-constructed, never a permit issuer
Production callers: none yet (verifier additive)
Test-only callers: 2 verifier tests (in-memory)
Installed/config callers: none
Tables/files/wire schemas: none changed
External side effects: none
Compatibility seam: none
Deletion owner: n/a
Unknowns/blockers: none
Expected files: crates/kernel/src/capability/verifier.rs, kernel Cargo.toml, K3 gate
Out-of-scope files: the K4 single admit/invoke path, executor cutover
```

## 1. What was created

`crates/kernel/src/capability/verifier.rs`:

- **`OpaqueProof`** — single-use opaque proof (nonce, expires_at_ms, scope, generation).
- **`AuthorizationEvidenceVerifier`** trait — `verify_and_consume(proof) -> Result<scope>`.
- **`EvidenceError`** — typed failures (VerifierUnavailable/AlreadyConsumed/Expired/WrongScope/WrongGeneration/Forged).
- **`InMemoryEvidenceVerifier`** — test single-use store.

## 2. Rules honoured (K3)

- opaque proof + nonce/expiry/scope/digest/generation validation + single-use store: `OpaqueProof` + `verify_and_consume` (single-use, expiry, scope checks).
- Application resolution adapter implements the verifier port, never exposes repository to Kernel: the Kernel only sees `AuthorizationEvidenceVerifier` (no `ApprovalStore`/repository).
- `DecisionRequestId` never directly constructed by a permit issuer: K3 gate rejects `DecisionRequestId(`/`ExecutionPermit(` in production verifier code.
- verifier unavailable → fail closed: `VerifierUnavailable` typed error.
- consumed evidence never reused: `AlreadyConsumed` on double verify (test-verified).

## 3. K3 gate (architecture-check.sh)

`ARCH_SKIP_K3_GATES` requires single-use/expiry/scope/unavailable checks; rejects any production `DecisionRequestId(`/`ExecutionPermit(` construction.

## 4. Validation

```text
bash scripts/cargo-agent.sh check -p kernel        PASS
bash scripts/cargo-agent.sh test -p kernel --lib capability::verifier  PASS (2 passed)
  - single_use_fails_closed_on_second_use
  - expired_fails_closed
bash scripts/cargo-agent.sh fmt --all -- --check   PASS
git diff --check                                    PASS
ARCH_SKIP_DEPENDENCIES=1 bash scripts/aletheon.sh acceptance architecture  PASS (23 findings, 0 deps)
```

## 5. Rollback

Delete `verifier.rs` + the capability/mod.rs module line + the K3 gate → exact baseline. No schema, no writer, no executor change.

## 6. Next

K4 (single admit/invoke/receipt/recovery path) strings budget/lease/deadline/sandbox/dispatch/usage/receipt into one state machine.
