# Agent Kernel V2：K4 single admit/invoke/receipt/recovery path

Date: 2026-08-09
Baseline: `0bf690b2`
State: single invocation state machine established; legacy `DefaultCapabilityInvoker` stays authoritative

## Context receipt (runbook §3.1)

```text
Slice: K4 single admit/invoke/receipt/recovery path (state machine seam)
Baseline commit: 0bf690b2
Direct prerequisites: K3 (verifier) + K2 (registry) + K1 (journal) — done
Current authoritative writer/executor: unchanged legacy DefaultCapabilityInvoker
Target owner/writer: Kernel single path; not yet wired
IDs minted here: none
Production callers: none yet (state machine additive)
Test-only callers: 3 invocation tests
Installed/config callers: none
Tables/files/wire schemas: none changed
External side effects: none
Compatibility seam: none
Deletion owner: n/a
Unknowns/blockers: none for the seam; the PR-C wiring (budget/lease/deadline/sandbox/dispatch/usage/receipt) is separate
Expected files: crates/kernel/src/capability/invocation.rs, capability/mod.rs, K4 gate
Out-of-scope files: the actual invoker wiring cutover, K5+
```

## 1. What was created

`crates/kernel/src/capability/invocation.rs`:

- **`InvocationState`** — Prepared/Observed/Settled/Recovering/Terminal.
- **`InvocationRecord`** — the single invocation record (capability, state, operation_epoch, sandbox_required).
- **`InvocationTransition`** — Admit/Observe/Settle/Recover/Terminal.
- **`TransitionResult`** — Ok / NeedsReconciliation / AlreadyTerminal.
- **`InvocationStateMachine::apply`** — the single state machine (pure).

## 2. Rules honoured (K4)

- budget/lease/deadline/sandbox/dispatch/usage/receipt strung into one state machine: `InvocationStateMachine` is the single path (PR-C wires the resources).
- prepared/observed crash window has explicit reconciliation: `Prepared → Settle/Recover` returns `NeedsReconciliation` (test-verified).
- no standalone permit issuer/settler or public component getter: K4 gate rejects `issue_permit`/`settle_permit`/`get_admission`/`get_lease` public fns.
- each operation generation switched once; shadow never performs a real effect: `AlreadyTerminal` rejects late transitions; the state machine is pure (no executor call).
- `TransitionResult::AlreadyTerminal` — terminal is never silently replayed.

## 3. K4 gate (architecture-check.sh)

`ARCH_SKIP_K4_GATES` rejects any standalone public `issue_permit`/`settle_permit`/`get_admission`/`get_lease`; requires `NeedsReconciliation` + `AlreadyTerminal`.

## 4. Validation

```text
bash scripts/cargo-agent.sh check -p kernel        PASS
bash scripts/cargo-agent.sh test -p kernel --lib capability::invocation  PASS (3 passed)
  - prepared_observe_settle_is_the_happy_path
  - prepared_settle_skips_observe_and_needs_reconciliation
  - terminal_rejects_late_transitions
bash scripts/cargo-agent.sh fmt --all -- --check   PASS
git diff --check                                    PASS
ARCH_SKIP_DEPENDENCIES=1 bash scripts/aletheon.sh acceptance architecture  PASS (23 findings, 0 deps)
```

## 5. Rollback

Delete `invocation.rs` + the capability/mod.rs module line + the K4 gate → exact baseline. No schema, no writer, no executor change.

## 6. Next

K5 (Linux/execd ProcessController adapter) and K6 (extension enforcement seams) close the kernel chain; then K7 cleanup.
