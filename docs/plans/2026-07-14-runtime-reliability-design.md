# Runtime Reliability Hardening Design

**Date:** 2026-07-14

**Status:** Draft — Conditional on R0 audit

**Scope:** Candidate reliability work to reassess after the agent/Google M0-M9 program stabilizes

## 1. Context and objective

The agent/Google program defines a restart-safe vertical slice whose durable domain state lives in SQLite while live execution continues through `TurnPipeline`, `ProcessTable`, `OperationTable`, admission, approval, and memory interfaces (`docs/plans/2026-07-14-agent-google-plan.md:5-9`). It also requires transactional migrations and optimistic versions (`docs/plans/2026-07-14-agent-google-plan.md:58-69`) plus atomic result/outbox, inbox-completion, and cursor persistence (`docs/plans/2026-07-14-agent-google-plan.md:97-110`).

This draft targets the future, stabilized M0-M9 implementation as its baseline. The current branch is still changing and does not yet satisfy that premise. R0 must audit the stable code, resolve the architecture blockers in this document, and produce a revised design for approval before any reliability implementation plan is written.

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

### 3.0 Current spec-to-code differences

The following table records the current branch as observed on 2026-07-15. These are audit inputs, not implementation requirements, because the branch remains in active development.

| Draft description | Current code reality | Agree? |
|---|---|---|
| M0-M9 is the completed baseline (`docs/plans/2026-07-14-runtime-reliability-design.md:7-13` before this revision) | Goal chat still reports that creation is unavailable (`crates/executive/src/impl/channel/router.rs:394-411`) | No |
| Result, domain transition, inbox, outbox, and audit may share one SQLite transaction (`docs/plans/2026-07-14-runtime-reliability-design.md:75-77` before this revision) | Channel state uses a dedicated `channels.db` (`crates/executive/src/impl/channel/store.rs:12-18`) while Goal state uses `objectives.db` (`crates/executive/src/impl/daemon/handler/init.rs:254-257`) | No |
| Every claimable record has owner, generation, lease, heartbeat, and version (`docs/plans/2026-07-14-runtime-reliability-design.md:102-110` before this revision) | `channel_inbox` has status and attempt count but none of those ownership fields (`crates/executive/src/impl/channel/store.rs:50-64`) | No |
| Tool registration declares replay policy (`docs/plans/2026-07-14-runtime-reliability-design.md:133-140` before this revision) | Model-visible `ToolDefinition` contains only name, description, and input schema (`crates/fabric/src/types/llm_types.rs:12-18`) | No |
| Tool execution persists Prepared, Dispatched, Observed, and Committed (`docs/plans/2026-07-14-runtime-reliability-design.md:120-131` before this revision) | `OperationTable` is an in-memory `HashMap` (`crates/kernel/src/operation/table.rs:16-35`) and `OperationState` has no recovery phase (`crates/fabric/src/types/operation.rs:50-94`) | No |
| A stale lease generation cannot commit (`docs/plans/2026-07-14-runtime-reliability-design.md:110-115` before this revision) | Startup recovery changes Running Goals to Ready without an owner-generation predicate (`crates/executive/src/impl/goal/store.rs:244-285`) | No |
| Outbox delivery has bounded retry and dead-letter behavior (`docs/plans/2026-07-14-runtime-reliability-design.md:79-81,142-145` before this revision) | Outbox selection continuously includes pending and failed rows, without a next-attempt or dead-letter condition (`crates/executive/src/impl/channel/store.rs:280-300`) | Partial |
| Health represents Ready, Degraded, Draining, and Unhealthy (`docs/plans/2026-07-14-runtime-reliability-design.md:178-191` before this revision) | Health currently reports a fixed `status: ok` plus counters (`crates/executive/src/impl/daemon/handler/rpc/rpc_health.rs:93-115`) | No |

R0 must regenerate this table from the stable branch. Line numbers and facts in this snapshot must not be reused without re-reading the code.

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

The desired guarantee is one atomic local commit where all participating state shares a database. The current split between channel and Goal databases makes a cross-domain transaction impossible without changing the persistence topology. R0 must choose the atomicity model in section 9.1 before repository APIs are designed.

### 3.4 Delivery Dispatcher

External delivery happens only after the local commit. Delivery failure retries the outbox item and never reruns the model or tool merely to reconstruct a reply. Stable correlation and provider idempotency data are retained when the provider supports them.

## 4. Work state and ownership

The following work state is a candidate model, not yet an approved schema. R0 must first identify the authoritative durable aggregate and prove that this model will not become a fifth competing business state machine.

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

If R0 approves lease-based durable claiming, every claimable record will carry:

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

The candidate tool recovery protocol records four phases across the database boundary:

```text
Prepared -> Dispatched -> Observed -> Committed
```

- `Prepared`: no external call was issued; execution is safe.
- `Dispatched`: the call may have occurred and no outcome is known; consult replay policy.
- `Observed`: an outcome exists; resume the local commit without calling the tool again.
- `Committed`: no further execution occurs.

The approved implementation must associate each executable tool with one reliability policy, but R0 must decide the metadata API and registration owner before this becomes an implementation requirement:

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

## 9. R0 architecture decisions

R0 is a read-only audit and architecture decision gate. It starts only after the current implementation work is stable. It must resolve all four sections below, revise this draft, and obtain approval before any R1-R5 plan or production code is created.

### 9.1 Persistence topology and atomicity

R0 must trace every channel, Turn, Goal, attempt, result, and outbox write and select exactly one model:

1. one database and connection for participating aggregates;
2. SQLite `ATTACH` with documented deployment, locking, backup, and recovery constraints;
3. separate databases coordinated through a durable saga/intent log and reconciliation.

The audit must define the atomic local boundary, the consistency guarantee across databases, the authoritative recovery record, crash windows, compensation behavior, and migration path. Saga/intent log is the current preference because it preserves repository ownership, but it is not approved until the stable write paths are audited.

### 9.2 Durable work, attempt, and side-effect schema

R0 must map the identity and authority relationships among Turn, Goal, Process, Operation, channel state, and any reliability records. The candidate relationship is:

```text
Turn or Goal (business authority)
  `-- Work (durable scheduling/recovery authority)
       `-- Attempt (lease generation and execution outcome)
            `-- SideEffect (tool or delivery uncertainty)
```

The audit must decide:

- what `work_id` identifies and whether every Turn has a stable durable ID;
- whether a Goal owns one Work or multiple Works over its lifecycle;
- how attempts relate to kernel `ProcessTable` and the in-memory `OperationTable`;
- which record is authoritative for each restart decision;
- how illegal cross-state combinations are detected and reconciled;
- which records are historical journals versus mutable state.

No reliability wrapper may duplicate a business completion state already owned by Turn or Goal.

### 9.3 Tool reliability metadata contract

R0 must enumerate native, MCP, plugin, deferred, and legacy tool registration paths before selecting the public API. Candidate placements are a Tool trait contract, a `ToolRegistry` sidecar, or an admission-policy registry. A sidecar is currently preferred so model-visible `ToolDefinition` remains separate from execution policy, but it is not approved.

The revised design must specify:

- the metadata type and registry owner;
- registration and validation for native, MCP, and plugin tools;
- stable idempotency-key propagation;
- legacy and dynamically discovered tool behavior;
- fail-closed handling for missing metadata;
- authorization and idempotency for approve, reject, and compensate actions.

Until approved, an implementation must not add reliability fields to `ToolDefinition` or choose another public API opportunistically.

### 9.4 Lifecycle supervisor and health contract

Current lifecycle ownership is distributed: the host spawns MCP, runs the Unix server, cancels turns, and separately stops the pulse (`crates/executive/src/host/mod.rs:159-207`), while each Unix connection is drained with its own timeout (`crates/executive/src/impl/daemon/server.rs:106-135`). R0 must decide whether one `RuntimeSupervisor` owns:

```text
RuntimeSupervisor
  |-- intake cancellation
  |-- claim loop
  |-- active attempts
  |-- delivery dispatcher
  |-- MCP server
  |-- health state
  `-- one global shutdown deadline
```

The revised design must define task registration, cancellation ordering, one global deadline, forced-abort semantics, state persistence before abort, and health/readiness ownership. The supervisor may coordinate lifecycle but must not absorb subsystem business logic.

### 9.5 Quantitative contract register

R0 must derive or propose explicit, testable values for each item below. No placeholder value may pass the next approval gate:

- lease TTL and heartbeat interval;
- startup recovery batch size;
- maximum attempts, initial backoff, multiplier, jitter rule, and backoff cap;
- one global shutdown deadline and its allocation policy;
- soak duration, restart count, concurrency, and invariant checks;
- dead-letter threshold, retention, inspection, and idempotent requeue behavior;
- stale-generation updates returning `affected_rows == 0`;
- persisted distinction between user and shutdown cancellation;
- provider message ID and idempotency-key storage requirements;
- readiness response schema, transport status mapping, and degraded behavior;
- SQLite busy timeout, WAL checkpoint policy, and disk-full test method;
- authorization and duplicate-action behavior for approve, reject, and compensate.

## 10. Multi-round validation

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

## 11. Delivery stages

1. **R0 — Architecture decision gate.** After the branch stabilizes, re-audit actual M0-M9 code, resolve sections 9.1-9.5, revise this design, and obtain approval. R0 is not a production implementation stage.
2. **R1 — Claiming, leases, and recovery coordination.** Add durable ownership with version-checked takeover.
3. **R2 — Tool recovery semantics.** Add execution phases and explicit replay policies.
4. **R3 — Outbox and shutdown hardening.** Separate delivery retry and enforce ordered draining.
5. **R4 — Fault injection, end-to-end, and soak validation.** Exercise restart boundaries and concurrency.
6. **R5 — Operations documentation and final architecture review.** Record invariants, recovery procedures, and remaining risks.

No R1-R5 implementation plan may be written before the revised design passes the post-R0 approval gate. After approval, each implementation stage must be independently reviewable and validated and must not be mixed into the stage commits used to land M0-M9.

## 12. Provisional acceptance criteria

These criteria express desired reliability outcomes. They are not implementation-ready until R0 supplies the topology, schema, APIs, and quantitative values required to test them.

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

## 13. R0 entry and exit criteria

R0 may start only when the owner declares the current M0-M9 implementation stable enough to audit and the working tree/target commits forming the baseline are identified.

R0 must verify these current assumptions against that baseline:

- The final channel repository exposes an atomic result/inbox/outbox commit operation.
- The final Goal repository retains optimistic versions and a query for non-terminal work.
- Tool invocations have stable operation identifiers that can carry recovery metadata.
- The daemon has a single lifecycle boundary where intake, workers, delivery, and shutdown can be ordered.

R0 exits only when it has:

- regenerated the spec-to-code difference table with fresh anchors;
- resolved sections 9.1-9.5 with code evidence;
- revised architecture, schemas, contracts, and provisional acceptance criteria;
- removed all undecided implementation-facing choices;
- received explicit design approval.

A failed assumption may change this design and its guarantees. Until R0 exits, this document must not be used to produce an R1-R5 implementation plan.
