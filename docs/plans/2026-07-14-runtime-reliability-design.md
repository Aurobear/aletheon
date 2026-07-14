# Runtime Reliability Hardening Design

**Date:** 2026-07-14

**Status:** Approved design

**Scope:** Reliability work remaining after the agent/Google M0-M9 program lands

## 1. Context and objective

The agent/Google program defines a restart-safe vertical slice whose durable domain state lives in SQLite while live execution continues through `TurnPipeline`, `ProcessTable`, `OperationTable`, admission, approval, and memory interfaces (`docs/plans/2026-07-14-agent-google-plan.md:5-9`). It also requires transactional migrations and optimistic versions (`docs/plans/2026-07-14-agent-google-plan.md:58-69`) plus atomic result/outbox, inbox-completion, and cursor persistence (`docs/plans/2026-07-14-agent-google-plan.md:97-110`).

This design treats the completed M0-M9 implementation as its baseline. It does not duplicate that program. Its objective is to make the resulting runtime predictably recoverable across daemon crashes, ambiguous tool outcomes, delivery failures, cancellation, deadlines, and bounded shutdown.

The design borrows reliability principles from Codex—bounded inputs, explicit persistence and recovery contracts, small public APIs, integration-test-first validation, and focused modules—without copying Codex's product-specific crate topology.

## 2. Scope

### In scope

- Durable work claiming and recovery coordination.
- Explicit local transaction and external side-effect boundaries.
- Tool replay classification and ambiguous-outcome handling.
- Independent outbox delivery and bounded retry.
- Ordered shutdown and startup recovery.
- Structured error classification, health, and recovery observability.
- Deterministic fault-injection, restart, end-to-end, and soak tests.

### Out of scope

- Reimplementing M0-M9 functionality.
- Replacing the existing Turn or Goal runtimes.
- Changing generic `ProcessState` to encode Goal or delivery state.
- Full event sourcing of every repository.
- Telegram-specific concepts in generic Goal, Turn, or kernel contracts.
- Mirroring Codex's crate layout or adding crates without a demonstrated boundary need.

## 3. Architectural constraints

The existing plan makes daemon execution use `TurnPipeline` and keeps generic process state separate (`docs/plans/2026-07-14-agent-google-plan.md:40-55`). Goal execution is bounded: one `tick()` advances a bounded amount of work, and an unbounded model loop is forbidden (`docs/arch/agent-google/04_GOAL_RUNTIME_ARCHITECTURE.md:83-124`). Reliability hardening must preserve both properties.

```text
Telegram / CLI / future channels
              |
              v
      Durable Intake Boundary
   deduplicate, bind, persist input
              |
              v
       Recovery Coordinator
   claim, lease, replay, startup scan
              |
              v
 Existing Turn / Goal Runtime
 TurnPipeline + GoalCoordinator
 ProcessTable + OperationTable
              |
              v
      Durable Commit Boundary
 result, state, outbox in one commit
              |
              v
       Delivery Dispatcher
    deliver, retry, dead-letter
```

### 3.1 Durable Intake Boundary

The channel inbox, cursor, and binding repositories supplied by M1 remain the ingress source of truth. An external message is persisted before execution and receives a stable idempotency key. Duplicate provider updates must not create duplicate turns or Goals.

### 3.2 Recovery Coordinator

Recovery orchestration belongs in `executive`, because it coordinates persistent application work rather than kernel resource admission. It scans non-terminal records and classifies them as safe to replay, awaiting a decision, expired, or quarantined. It reconnects work to existing Turn and Goal entry points; it does not create another execution runtime.

### 3.3 Durable Commit Boundary

Where the final M0-M9 schema permits it, business result/error persistence, domain transition, inbox completion, outbox insertion, and a compact recovery audit record commit in one SQLite transaction. Repository APIs must expose the operation as a single semantic action rather than relying on callers to order several independent writes.

### 3.4 Delivery Dispatcher

External delivery happens only after the local commit. Delivery failure retries the outbox item and never reruns the model or tool merely to reconstruct a reply. Stable correlation and provider idempotency data are retained when the provider supports them.

## 4. Work state and ownership

Reliability state wraps durable work and does not replace `GoalState`, `ProcessState`, `OperationState`, or channel domain state.

```text
Pending
   | claim
   v
Running ---- complete ---> Committed ---- delivered ---> Delivered
   |                          |
   | owner lost               +---- failure ----> DeliveryRetry
   v
Recoverable
   +---- safe replay -------> Pending
   +---- confirmation ------> AwaitingDecision
   +---- budget exhausted --> Failed
   +---- inconsistent ------> Quarantined
```

Every claimable record carries:

- `owner_instance_id`;
- monotonically changing `lease_generation`;
- `lease_expires_at` and `last_heartbeat_at`;
- `attempt_count` and `last_error`;
- optimistic `version`.

A daemon executes only work for which it holds the current, unexpired generation. A replacement daemon may claim work after expiry with a version-checked update. This prevents concurrent ownership but does not claim end-to-end exactly-once semantics.

The guarantees are:

- internal state transitions commit transactionally;
- model requests may be retried, but a stale attempt cannot commit over a newer generation;
- replayable tools use explicit replay rules;
- an unknown non-replayable side effect waits for a decision;
- outbound delivery is at-least-once unless the provider offers a stronger idempotency contract.

## 5. Tool execution recovery

Tool execution crosses the database boundary and therefore records four phases:

```text
Prepared -> Dispatched -> Observed -> Committed
```

- `Prepared`: no external call was issued; execution is safe.
- `Dispatched`: the call may have occurred and no outcome is known; consult replay policy.
- `Observed`: an outcome exists; resume the local commit without calling the tool again.
- `Committed`: no further execution occurs.

Tool registration declares one reliability policy:

- `ReadOnly`: automatic replay is allowed.
- `Idempotent { key_scope }`: replay is allowed with a stable idempotency key.
- `Compensatable`: recovery is allowed only through an explicit compensation contract.
- `NonReplayable`: an unknown outcome enters `AwaitingDecision`.

The default is `NonReplayable`. Reliability policy is metadata enforced at admission/execution boundaries, not inferred from tool names.

## 6. Recovery budgets and shutdown

Recovery is bounded by maximum attempts, maximum cumulative runtime, maximum consecutive identical failures, capped exponential backoff, and a startup batch limit. Exceeding a budget produces an explicit failed or decision-waiting state rather than an infinite retry loop.

Shutdown is ordered:

```text
stop intake
  -> prevent new claims
  -> cancel or await bounded execution
  -> commit already-observed outcomes
  -> release or shorten leases
  -> flush outbox/tracing within deadline
  -> exit
```

Work that exceeds the shutdown deadline remains recoverable. Shutdown must not falsely mark it completed or permanently failed. Explicit user cancellation remains cancelled and is never converted into recoverable work.

## 7. Error handling

Persisted runtime errors use stable categories:

| Category | Example | Default action |
|---|---|---|
| `Transient` | network interruption, rate limit | bounded backoff |
| `Resource` | token, time, or attempt budget exhausted | deterministic stop |
| `Cancelled` | user cancellation or shutdown cancellation | retain source; do not auto-resume user cancellation |
| `PolicyDenied` | admission or permission denial | no retry without changed conditions |
| `AmbiguousSideEffect` | dispatched tool with unknown result | await decision |
| `CorruptState` | illegal transition or broken relation | quarantine record |
| `Permanent` | invalid request or missing permanent resource | terminal failure |

Each record includes stable code, stage, retryability, sanitized summary and source chain, first/latest occurrence, repeat count, and available channel/conversation/turn/goal/process/operation identifiers.

A malformed record is quarantined without blocking unrelated recovery. Only global safety failures—an unreadable database, unsupported schema version, or failed required migration—prevent readiness.

## 8. Observability and health

Every durable work item has a stable `work_id`; every execution has a monotonic `attempt_id`; every daemon lifecycle has an `instance_id`. Tracing spans propagate these identifiers through intake, Turn/Goal execution, tool dispatch, commit, and delivery.

Startup recovery reports scanned, claimed, deferred, quarantined, and failed counts. Logs exclude provider tokens, OAuth secrets, and unbounded prompt/tool payloads.

Daemon health states are:

- `Ready`: persistence and recovery coordination are safe and intake may run.
- `Degraded`: work can continue, but backlog, provider failure, or recovery delay exists.
- `Draining`: shutdown is in progress and intake is closed.
- `Unhealthy`: persistence or ownership safety cannot be guaranteed.

Channel reachability alone does not establish readiness. Database write capability and a functioning recovery coordinator are required.

## 9. Multi-round validation

### Round 1: post-M0-M9 architecture audit

Re-read final code rather than trusting plan completion markers. Trace channel, Turn, Goal, process, operation, and outbox paths. Produce a matrix of applicable Codex ideas, intentionally rejected Codex designs, and remaining Aletheon gaps. Check dependency direction, duplicate abstractions, public API growth, and oversized high-touch modules.

### Round 2: deterministic component tests

Use temporary SQLite databases, fake clocks, fake providers, and fake channels to test:

- idempotency and duplicate input;
- optimistic conflicts and lease races;
- legal and illegal recovery states;
- budgets and backoff;
- shutdown deadlines;
- independent outbox delivery;
- every tool replay policy;
- schema upgrade and compatibility behavior.

Tests must avoid real sleeps and uncontrolled networks.

### Round 3: fault-injection integration tests

For each boundary, run `start -> inject failure -> stop -> restart -> recover -> assert final state`:

1. after inbox commit and before execution;
2. after model response and before result commit;
3. after tool dispatch and before outcome recording;
4. after result/outbox commit and before delivery;
5. after provider success and before local acknowledgement;
6. during draining;
7. while two daemon instances contend for recovery;
8. during SQLite busy/write failure or provider throttling.

Assertions cover final state, attempt count, tool call count, and observable side-effect count.

### Round 4: vertical end-to-end tests

Exercise the full path:

```text
provider update -> inbox -> Turn/Goal -> model -> admitted tool
  -> result/outbox -> reply -> daemon restart -> status/recovery
```

Cases include ordinary chat, one persistent Goal, replayable and non-replayable tools, cancellation, timeout, budget exhaustion, duplicate updates, delivery recovery, and isolation of one corrupt record.

### Round 5: regression and operational gate

Run, in order:

1. new reliability tests;
2. affected `executive`, `kernel`, and `fabric` tests;
3. `cargo fmt --all -- --check`;
4. workspace Clippy under the repository's agreed lint command;
5. `cargo test --workspace`;
6. `cargo build --workspace`;
7. a bounded daemon soak test with repeated restarts and database-invariant checks.

Failures are classified as implementation, timing/test-harness, or contract failures. Increasing retry counts is not an acceptable substitute for diagnosis.

## 10. Delivery stages

1. **R0 — Baseline audit and invariant tests.** Verify the completed M0-M9 code and lock down current behavior.
2. **R1 — Claiming, leases, and recovery coordination.** Add durable ownership with version-checked takeover.
3. **R2 — Tool recovery semantics.** Add execution phases and explicit replay policies.
4. **R3 — Outbox and shutdown hardening.** Separate delivery retry and enforce ordered draining.
5. **R4 — Fault injection, end-to-end, and soak validation.** Exercise restart boundaries and concurrency.
6. **R5 — Operations documentation and final architecture review.** Record invariants, recovery procedures, and remaining risks.

Each stage must be independently reviewable and validated. It must not be mixed into the stage commits used to land M0-M9.

## 11. Acceptance criteria

- A crash at every identified persistence or side-effect boundary has a deterministic recovery outcome.
- Duplicate inbound messages do not duplicate Turn/Goal creation or committed results.
- Delivery failure never causes model or tool re-execution merely to reconstruct output.
- Stale lease holders cannot commit after ownership changes.
- Unknown non-replayable side effects fail closed into an explicit decision state.
- User cancellation, timeout, and budget exhaustion survive restart with their original semantics.
- One corrupt record does not block unrelated work or daemon startup.
- Shutdown stops intake, drains within a bound, and leaves unfinished work recoverable.
- All five validation rounds pass, including repeated-restart soak invariants.
- The final implementation preserves existing crate direction and uses the established Turn and Goal execution paths.

## 12. Assumptions to verify before implementation

These are deliberate assumptions because tonight's M0-M9 implementation is still changing:

- The final channel repository exposes an atomic result/inbox/outbox commit operation.
- The final Goal repository retains optimistic versions and a query for non-terminal work.
- Tool invocations have stable operation identifiers that can carry recovery metadata.
- The daemon has a single lifecycle boundary where intake, workers, delivery, and shutdown can be ordered.

R0 must verify each assumption against final code. A failed assumption changes the implementation plan, not the reliability guarantees in this design.
