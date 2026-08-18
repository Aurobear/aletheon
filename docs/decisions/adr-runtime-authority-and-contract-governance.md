# ADR: retain unified runtime authorities and governed contracts

- Status: accepted
- Date: 2026-08-07
- Decision owner: release architecture gate
- Implementation merge: PR #182, `98914cd28426b0da160b4e9352052cfc38274ae9`

## Context

The 2026-08 convergence work closed its implementation and installed MuJoCo
acceptance. Its temporary scheduling and status ledgers are no longer suitable
as permanent decision references: they mix dependency ordering, execution
history, local diagnostics, installed evidence, and post-MuJoCo physical safety
work.

The governed architecture registries still need durable references for every
migration and every post-baseline Fabric public type. This ADR retains the
architectural decisions after the completed planning ledgers are removed. Code,
contract registries, maintained testing guides, and Git history remain the
implementation and evidence sources of truth.

## Runtime authorities

```text
Client adapter -> ClientIntent -> CommandDispatcher -> TurnCoordinator
                                                |
                                                v
SessionAppendStore -> EventSourcedSessionStore -> EventSpine durable commit
                                                |
                             Session/Task/Activity projections
                                                |
                                   clients consume read models
```

1. `SessionAppendStore` remains the application mutation port
   (`crates/contracts/src/types/session.rs:319-358`). Its production implementation
   is `EventSourcedSessionStore` (`crates/adapters/sqlite/src/session/event_sourced_store.rs:70-94,229-377`),
   composed over EventSpine and a private projection store
   (`crates/aletheon/src/wiring/adapters/session/test_composition.rs:41-56`).
2. The durable, ordered EventSpine append is the authority commit. Session,
   Task, Activity, principal, and recovery materializers are deterministic read
   projections; projection state is not a second business commit
   (`crates/runtime/src/public_session_projection.rs:166-220`).
3. Turn lifecycle and settlement remain Host-owned
   (`crates/runtime/src/turn_pipeline_lifecycle.rs:99-129`). Model text,
   child self-reporting, presentation state, and monitor summaries cannot write
   authoritative terminal settlement.
4. Active plans remain `agora::TaskGraph`
   (`crates/agora/src/task_graph/mod.rs:54-82`); memory operations remain behind
   `mnemosyne::MemoryService` (`crates/mnemosyne/src/service.rs:378-430`); child
   lifecycle remains behind `AgentControlService`
   (`crates/executive/src/application/agent_control/mod.rs:120-219`). These are
   separate domain authorities rather than alternate Session writers.
5. Production persistence/bootstrap must fail closed. In-memory stores are test
   composition only; clients and the TUI consume snapshots/events and never
   mutate a projection database directly.

## Command and client entry authority

1. Fabric `CommandSpec` and `ClientIntent` are the shared command contract
   (`crates/contracts/src/contract/command.rs:112-188,201-311`). CLI, TUI, Gateway,
   and other input surfaces translate into this contract instead of defining
   independent command semantics.
2. Executive `CommandDispatcher` is the sole application handler for
   `ClientIntent` (`crates/application/src/command_dispatcher.rs:60-85`).
3. Help, completion, visibility, availability, and execution policy derive from
   the same command specifications. A compatibility parser or presentation
   handler must not become a second production command authority.

## Client protocol and projections

1. Run/exec uses the versioned client protocol and typed terminal envelopes;
   `ExecEvent`, `ExecTerminalKind`, and `ExecEventEnvelope` are distinct from
   Session/Turn schemas (`crates/gateway/src/protocol/exec.rs:11-72`).
2. Session protocol snapshots and tails are versioned and principal-scoped
   (`crates/contracts/src/protocol/client.rs:989-1180`;
   `crates/contracts/src/types/session.rs:262-358`).
3. Task, Activity, checkpoint, rewind, findings, review, and settlement are Host
   facts projected through the client protocol
   (`crates/contracts/src/protocol/client.rs:187-329,1182-1434`). The TUI displays
   these facts but does not own their state machine.
4. Safety denial is a typed blocked settlement, not a generic tool error. Large
   artifacts remain content-addressed references instead of inline Session
   payloads.

## Capability receipts and identifiers

1. Capability execution records a typed terminal receipt
   (`crates/contracts/src/include/turn.rs:111-217`). Async dispatch is not terminal
   success until the authoritative terminal snapshot or durable receipt is
   observed.
2. Identifiers remain domain-specific: governed operation identity is
   `fabric::OperationId` (`crates/contracts/src/types/operation.rs:9`), device-side
   correlation is `hardware::DeviceOperationId`
   (`crates/hardware/src/device.rs:4-8`), runtime process identity is
   `fabric::RuntimeProcessId` (`crates/contracts/src/types/process.rs:37-47`), and
   change/checkpoint identities retain their own types
   (`crates/contracts/src/types/change_transaction.rs:9-24`;
   `crates/application/src/workspace_checkpoint/mod.rs:30-42`).
3. Hardware reuses `fabric::PrincipalId`; it does not define a parallel
   principal wrapper (`crates/hardware/src/device.rs:4`). New generic ID or
   receipt wrappers require demonstrated shared semantics and registry review.
4. Mutation coverage, validation, compensation, retry classification, and
   settlement must remain typed. A model cannot override a Host policy denial
   or synthesize a successful receipt.

## Robot contracts and safety boundaries

1. Robot execution reuses the canonical Task/Activity/receipt mainline while
   retaining a domain-specific verifier and safety authority. Hardware is not a
   Bash/MCP tool.
2. Environment and safety facts are provider-attested typed contracts
   (`crates/contracts/src/types/embodiment.rs:35-158`). Robot failure, safe-stop,
   attempt, artifact, and immutable settlement facts remain explicit
   (`crates/contracts/src/types/robot_failure.rs:10-69`;
   `crates/contracts/src/types/episode_report.rs:37-151,335-720`).
3. Policy proposals must be goal-aligned and pass allowlist, schema, device,
   risk, expected-outcome, and authority validation; a `SafetyFallback` is not
   executable as a direct goal action
   (`crates/contracts/src/types/skill_proposal.rs:16-53`).
4. WBC/MPC, drivers, monotonic watchdog, command ownership, and emergency stop
   remain in the robot/Bridge real-time stack. Cloud inference and Aletheon do
   not assume hard-real-time control.
5. The installed MuJoCo R8/X13 chain is accepted and recorded in
   `docs/testing/robot-runtime.md`. Physical HIL/real execution remains a
   separate gate: before enablement it must prove the pinned device and safety
   manifests, local watchdog/ownership, an independently exercised hard stop,
   and emergency-stop behavior on the physical device. MuJoCo acceptance must
   never be described as physical real-ready evidence.

## Deployment and acceptance boundary

Development binaries, direct provider calls, direct Bridge tests, and isolated
daemons are diagnostic evidence only. Runtime-affecting changes are accepted
only after system deployment proves release, `/usr/bin/aletheon`, and every
running Aletheon executable have identical SHA-256 digests, restart counters
are stable, and a real request succeeds through the official user socket.

At the recorded R8/X13 acceptance, the installed chain reached a real Policy,
candidate Bridge, ROS, and Robot version 53 MuJoCo; positive stance/action,
negative safety denial, stable-window verification, durable EpisodeReport, and
post-restart evidence checks passed. The durable summary and reproduction entry
points are maintained in `docs/testing/robot-runtime.md`.

## Consequences

- Architecture registries cite this ADR instead of completed execution plans.
- Test guides retain reproducible evidence and explicit unaccepted boundaries;
  they do not become scheduling ledgers.
- New parallel writers, parsers, settlement authorities, generic identity
  wrappers, or robot real-time paths fail architecture review unless this ADR
  is explicitly superseded with migration and rollback evidence.
- Historical task ordering, transient failures, and per-node status remain in
  Git history rather than permanent live plans.
