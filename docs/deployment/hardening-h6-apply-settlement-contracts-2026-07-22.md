# H6 apply and settlement error-contract evidence — 2026-07-22

## Requirement anchors

- H6 covers the approval apply coordinator, goal attempt/coordinator, lifecycle event publication,
  and transient approval delivery (`docs/plans/2026-07-21-production-readiness-hardening.md:214-222`).
- Every discarded result must be classified as best-effort, warn-and-continue, or propagate
  (`docs/plans/2026-07-21-production-readiness-hardening.md:224-226`).
- Apply/settle failure must preserve attempt history, diff hash, and a retryable state; closed
  oneshot receivers must report the correct terminal state
  (`docs/plans/2026-07-21-production-readiness-hardening.md:227-228`).
- Regression coverage must include duplicate apply, vanished consumer, and event publication
  failure (`docs/plans/2026-07-21-production-readiness-hardening.md:229`).

## Explicit result contracts

| Path | Contract | Evidence |
|---|---|---|
| requested lifecycle `EmitEvent` | **propagate**; an extension-requested event cannot silently disappear | `crates/executive/src/service/turn_pipeline.rs:119-159,1055-1064` |
| lifecycle effect audit event | **warn-and-continue**; observation failure cannot undo the effect | `crates/executive/src/service/turn_pipeline.rs:142-155` |
| abort lifecycle dispatch after a pipeline error | **warn-and-continue** while preserving the original error | `crates/executive/src/service/turn_pipeline.rs:1017-1036` |
| approval decision receiver gone | terminal `ConsumerGone`, warning, RPC false, no session grant | `crates/executive/src/service/admin_service.rs:97-228,544-576` |
| budget revoke after attempt persistence failure | propagate original and revoke failures together | `crates/executive/src/impl/goal/attempt_coordinator.rs:345-380,388-410` |
| terminal attempt budget settlement | persist attempt/evidence first; exact settlement replay is idempotent and recoverable without runtime reinvocation | `crates/executive/src/impl/goal/attempt_coordinator.rs:259-518`; `crates/executive/src/impl/goal/budget.rs:254-308` |
| apply failure | durable consumed receipt with diff hash, blocked goal, fresh verification/approval required | `crates/executive/src/impl/approval/apply_coordinator.rs:212-270,435-493` |
| memory projection / temporary artifact cleanup | explicit best-effort with degraded/cleanup warning | `crates/executive/src/impl/approval/apply_coordinator.rs:532-578,646-676` |
| goal summary/evidence read | return `ProjectionStatus::Degraded` and warn rather than silently returning no projection | `crates/executive/src/impl/goal/coordinator.rs:124-156` |

```text
runtime completes
    |
    +-- persist terminal attempt + evidence + reservation identity
    |
    +-- settle ledger ---- failure ----> return typed Budget error
    |                                  attempt/diff evidence remains durable
    |
retry same sequence
    +-- find terminal attempt
    +-- exact idempotent ledger settlement
    +-- reconstruct persisted runtime/Pi result
    `-- verifier/goal transition continues; runtime and apply are not repeated
```

Apply has a separate one-time receipt state machine: a failed apply is not retried with the consumed
approval. It remains blocked with the original diff hash and requires fresh verification and a new
approval. A callback after a durable receipt only reconciles the goal/cleanup state.

## Fault injection and regression evidence

Commands run through the repository build wrapper:

```bash
bash scripts/cargo-agent.sh test -p executive r#impl::goal::budget --lib
bash scripts/cargo-agent.sh test -p executive --test attempt_coordinator
bash scripts/cargo-agent.sh test -p executive --test coding_goal_flow
bash scripts/cargo-agent.sh test -p executive --test approved_apply_flow
bash scripts/cargo-agent.sh test -p executive --test admin_service
bash scripts/cargo-agent.sh test -p executive requested_lifecycle_event_publish_failure_is_propagated --lib
bash scripts/cargo-agent.sh fmt --all -- --check
bash scripts/cargo-agent.sh clippy -p executive --all-targets -- -D warnings
bash scripts/architecture-check.sh
git diff --check
```

Observed deterministic coverage:

- budget unit tests: 7 passed, including exact idempotent settlement and conflicting replay reject;
- attempt coordinator: 10 passed; an SQLite trigger aborts settlement, the terminal attempt and
  evidence remain, and retry completes with one runtime call total
  (`crates/executive/tests/attempt_coordinator.rs:500-542`);
- coding flow: 6 passed; injected settle failure retains the `CodingJobReport.diff_sha256`, retry
  persists the identical diff and invokes Pi/verifier only once
  (`crates/executive/tests/coding_goal_flow.rs:395-451`);
- approved apply: 5 passed, including one-time apply, receipt recovery, cancellation failure, and
  concurrent duplicate callbacks;
- admin service: 8 passed, including disconnected isolation and closed-consumer terminal delivery
  (`crates/executive/tests/admin_service.rs:300-366`);
- lifecycle publish regression: 1 passed; an invalid requested schema is returned as an error
  (`crates/executive/src/service/turn_pipeline.rs:1254-1271`).
- formatting, architecture check, and diff check passed. The plan's strict all-target clippy command
  remains red on pre-existing repository-wide lint debt (first failures are
  `crates/platform/src/structured_patch.rs:102` `uninlined_format_args`; with dependency linting
  suppressed, Executive still reports existing `core/config/backpressure.rs:34` and test-support
  dead-code/type-complexity findings). H6 does not broaden into mechanical repository-wide lint
  cleanup; all changed behavior is covered by the deterministic compile/test targets above.

## Compatibility and rollback

- Existing attempt rows without `budget_reservation_id` remain readable. Recovery only requires the
  field when a caller retries an already-terminal sequence that never completed settlement.
- No database schema changes were introduced. New attempts carry the reservation identity in their
  existing immutable JSON input.
- Approval RPC wire shape remains a boolean; `false` now accurately distinguishes a vanished
  decision consumer from successful delivery.
- Rollback is the independent H6 commit. Receipts, attempt evidence, and ledger rows must not be
  deleted during rollback.
