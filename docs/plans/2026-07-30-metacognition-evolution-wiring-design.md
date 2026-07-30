# Metacognition Evolution Wiring (Workstream C)

**Date:** 2026-07-30
**Status:** Revised design; governance decisions locked; apply remains blocked
until required A/B evidence, candidate-aware sandboxing, and restart-safe genome
persistence exist
**Scope:** Bring the dormant `metacog` self-evolution layer online as a
**governed candidate-evaluation-and-evolution** step in the post-turn path —
matching `architecture-overview.md:57` ("受治理候选评估与演化"). Evolution must
never let the model or runtime grant itself authority; every accepted candidate
must pass the existing authority-bound governance boundary
(`architecture-overview.md:141`, single-authority principle at `:117-121`).

> **Roadmap / premise correction (verified against repo state).** The task
> framing — "`evolution_coordinator.rs` is never called from the main turn loop
> and `PostTurnRuntimePort.post_evolution` is unused" — is **partially stale**.
> The verify-only path **is** wired end to end: `turn_coordinator.rs:412-418`
> spawns the post-turn projection after settlement, which reaches
> `ProductionPostTurnProjection::run_evolution`
> (`post_turn_projection.rs:172-174`) →
> `PostTurnDomainAdapter::post_evolution` (`request_ports.rs:144-160`) →
> `AletheonExecutive::post_evolution` (`orchestrator.rs:78-113`) →
> `EvolutionCoordinator::post_turn` (`evolution_coordinator.rs:137`). What is
> genuinely inert is **(1)** the loop is double-gated OFF by default, and
> **(2)** it stops at `meta.verify(...)` and **never reaches the governed
> `meta.apply(...)` boundary** — no component in the entire tree issues the
> permit + approval that `apply` demands. C is the design that closes that gap
> under governance. This doc anchors every claim to `path:line`.

## 1. Background & Problem

`metacog` is a ~9.1K-LOC src crate (`crates/metacog/src`, 48 files) with a
substantially-implemented governed mutation lifecycle. It is **dormant** for two
distinct reasons — a gating reason and a structural reason.

### 1.1 What is implemented and reachable

- **`EvolutionCoordinator`** — `crates/executive/src/core/evolution_coordinator.rs:99`.
  `post_turn()` (`:137`) reflects on turn metrics, accumulates a sliding
  reflection window (`:186-192`), and periodically calls `run_evolution()`
  (`:319`). It is attached to the runtime via `with_evolution`
  (`orchestrator.rs:66-73`) at bootstrap (`request.rs:521-529`).
- **Governed lifecycle** — `DefaultMetacogService`
  (`crates/metacog/src/governance/service.rs:277`) implements `verify` (`:372`),
  `apply` (`:462`), `rollback` (`:551`), `status` (`:635`). `apply`/`rollback`
  require `GovernedMutationEvidence { permit: ExecutionPermit, approval:
  ApprovalSnapshot }` (`service.rs:95-104`) and enforce it in `validate_evidence`
  (`:311-360`): capability must be `metacog.apply`/`metacog.rollback`
  (`:21-22, :328`), `approval.category` must be `ApprovalCategory::DaseinModification`
  (`:339`), status `Approved|Consumed`, and the approval subject must be bound to
  `mutation_id`/`operation`/hash (`:350-354`); permits are single-use
  (`ensure_unused_permit`, `:510, :650`). Constructed at `request.rs:539-544`,
  persisted to `metacog-mutations.json` (`with_state_path`, `service.rs:294`).
- **`DefaultMetaRuntime`** (`crates/metacog/src/governance/runtime.rs:39`,
  built at `request.rs:535`) composes real components: `CandidateGenerator`,
  `SandboxRunner`, `Evaluator`, `MigrationManager`, `RollbackManager`,
  `LineageTracker`.

### 1.2 Why it is inert — reason 1: double default-off gate

`EvolutionConfig` defaults `enabled=false` and `evolution_permitted=false`
(`evolution_coordinator.rs:48-49`); the gate at `:165` makes `post_turn` a no-op
unless **both** are true. At bootstrap, `enabled` is bound to the
`--enable-evolution` CLI flag (defaults `false`, `crates/aletheon/src/main.rs:104`;
threaded to `request.rs:522`), and **`evolution_permitted: false` is hardcoded**
(`request.rs:523`). So the loop cannot fire in a shipped runtime. The only
evolution RPC (`handle_evolution`, `rpc_reflection.rs:72-86`) is read-only log
recall.

### 1.3 Why it is inert — reason 2: the loop never reaches governed apply

Even fully enabled, `run_evolution` (`evolution_coordinator.rs:319-343`) calls
**only** `meta.verify(...)` (`:334`). It never calls `meta.apply(...)`. The doc
comment is explicit (`:316-318`): *"Applying or rolling back a verified mutation
remains a separate governed operation owned by Metacog."* The existing
integration tests lock this in — `evolution_integration.rs:210-214, 271-275`
assert `migrate_calls == 0` ("verification cannot bypass governed apply").

The result: **nothing in the production tree ever mints the evidence `apply`
requires.** `ApprovalCategory::DaseinModification` (`fabric/.../approval.rs:41`)
has zero production issuers — the sole `ApprovalRepository::create`
(`crates/executive/src/application/approval/repository.rs:448`) callers set
`ApplyCode` (`goal/attempt_coordinator.rs:1070`), `SendMail`
(`adapters/channel/gmail/report.rs:164`), and `ActivateGoal`
(`adapters/channel/gmail/goal_draft.rs:374`). The kernel
`ProductionAdmissionController::admit` (`crates/kernel/src/admission/production.rs:228`)
can mint a `metacog.apply` permit but no caller ever requests one. **The governed
apply boundary is complete but has no legitimate caller.** That missing,
authority-bound caller is the heart of Workstream C.

### 1.4 Thin candidate evidence, and known stubs

Candidate intents come only from `MutationIntentGenerator::from_reflections`
(`crates/metacog/src/improvement/promotion.rs:79`), a deterministic heuristic
over a reflection window built from turn metrics (success / tool_calls /
tool_errors / elapsed). No memory recall (Wave-3 dep A) or multi-agent
evaluation signal (dep B) feeds it. Structural stubs to respect in the design
(not to fix here):

- `MigrationManager::migrate(candidate)` (`evolution/migration.rs:163-202`) is
  bookkeeping only — cosmetic `to_version`, no genome persistence, always
  `success:true`.
- `SandboxRunner::run_tests` (`evolution/sandbox_runner.rs:32`) shells out to
  `cargo test --workspace` and **ignores the candidate genome** — "safety" then
  measures repo test health, not the mutation. This is a heavy, mis-scoped side
  effect that must be bounded/gated.
- `CandidateGenerator::generate` (`evolution/candidate.rs:33-90`) only applies
  `care.priorities`; `boundary.rules`/`tool.config`/`mutation.config` are
  recorded as text but not applied.
- `MorphogenesisPipeline` (`evolution/pipeline.rs:11`), `SpecEditor`
  (`governance/editor.rs`), `SelfReader` (`governance/self_reader.rs`),
  `experiment*`/`registry.rs` are real but **test-only / `#[allow(dead_code)]`** —
  the live path re-implements the sequence in `service.rs`.
- `RollbackManager` (`evolution/rollback.rs`) is **in-memory only** — rollback
  state is lost on restart (`runtime.rs:37-38`).

### 1.5 The hook point (how a turn ends)

The exact settlement/hook geography, verified for the recommended hook point:

```
TurnCoordinator::submit_with(...)                 turn_coordinator.rs:287
  run_started_turn -> runner(...) -> model+tool loop, session items appended
  -> CompletedExecution { result, projection }    turn_coordinator.rs:638
  settlement: kernel.succeed_operation/fail/cancel turn_coordinator.rs:390-409
  terminal?;   (settlement forced)                 turn_coordinator.rs:411
  if let Some(dispatch) = completed.projection {   turn_coordinator.rs:412
      tokio::spawn(dispatch.projector.project(...))  :413  (detached, post-settlement)
  }
```

`PostTurnOutcome` (`post_turn_projection.rs:18-31`) is populated at
`daemon_turn_engine.rs:131-151` from the parsed turn result
(`succeeded`, `completed_normally`, `tool_calls_made`, `tool_errors`,
`elapsed_ms`, `iterations`). The cognit completion gate that authorizes
`completed_normally` is `ProgressAuditor::audit` (`crates/cognit/src/core/progress_auditor.rs:116`),
returning `ProgressDecision` (`:11`) under `CompletionGateMode::{Shadow,Enforce}`
(`:109`); on decision it emits `GroundedCognitiveOutcome`
(`fabric/src/types/cognitive_workflow.rs:644`, built at
`crates/cognit/src/core/claim_auditor.rs:71`) that feeds dasein via
`GroundedDaseinOutcomeSink` (`dasein_workspace_adapter.rs:130-188`). Evolution
must sit **downstream** of this gate — it consumes settled outcomes and holds no
authority over them.

> Note: `docs/plans/2026-07-26-cognitive-closed-loop-design.md` referenced in the
> brief **does not exist**; the live metacog design material is
> `docs/design/metacog/{README.md, meta-runtime.md, morphogenesis.md}`
> (`meta-runtime.md:3` marks that layout "Historical design").

## 2. Goals / Non-goals

**Goals**

1. Bring the post-turn evolution step online as a **governed** step: verified
   candidates that recommend `Adopt` are routed to the existing
   `DefaultMetacogService::apply` boundary through a real,
   **authority-issued** permit + approval — never auto-applied.
2. Keep evolution strictly **off the turn's critical path**: turn success must be
   independent of any evolution outcome (reuse the detached spawn at
   `turn_coordinator.rs:412-418`).
3. Make evolution evidence **degrade safely** when Wave-3 deps A (memory) and B
   (multi-agent signal) are absent — candidates simply carry thin evidence and
   are more likely to be rejected / never approved.
4. Preserve reversibility and auditability: every accepted mutation has a durable
   `MutationReceipt` and a governed rollback path.
5. Persist and read back the accepted genome atomically before an apply receipt
   can report success; daemon restart must retain the effective version.
6. Evaluate the candidate genome itself through a bounded typed sandbox suite;
   repository compilation status is not candidate evidence.

**Non-goals**

- **Unbounded self-modification.** Any code, config, capability, prompt, or model
  change is out of scope. Evolution may touch **only the internal `Genome`**
  (`fabric/src/types/genome.rs:10`, "*Not code itself*", `:3-4`) — in practice
  the `care.priorities` / `boundary.rules` targets the `SpecEditor`
  (`editor.rs:31-55`) and `CandidateGenerator` (`candidate.rs:33`) support.
- **The runtime/model approving its own evolution.** Governance is a first-class
  constraint: the accept authority is the operator, routed through the existing
  approval subsystem. The self-approval guard already exists
  (`improvement/registry.rs:135-140`) and must be honored.
- Source-code mutation and production-time Cargo execution. Genome verification
  uses a typed in-process candidate sandbox; code builds remain operator/dev
  validation through `scripts/cargo-agent.sh`.
- Any "auto-apply on Adopt" behavior. `Adopt` is a recommendation, not authority.
- Turning evolution ON by default. Ships default-off, as today.

## 3. Design

### 3.1 Approach options

**(A) Direct auto-apply in the coordinator.** On `verify → Adopt`, have
`EvolutionCoordinator` mint a permit + approval and call `meta.apply`.
Rejected — this makes the runtime its own authority, violating
`architecture-overview.md:141` and the single-authority rule.

**(B) Verified-candidate parking + out-of-band governed apply (selected).**
The coordinator's job ends at producing a **durable verified candidate**
(already the case, `service.rs:449-458`). A new, thin
`GovernedEvolutionProposer` turns an `Adopt` verification into a
**pending `DaseinModification` approval** (no permit, no apply yet). The operator
resolves it through the existing `ApprovalService`
(`approval_service.rs:62, resolve() :134`); on approval, the resolve path mints
the `metacog.apply` permit via kernel admission and calls `meta.apply`. This
reuses the entire existing governance machinery and keeps the model/runtime out
of the authority loop.

**(C) Full A/B-experiment loop.** Wire `experiment.rs` / `experiment_store.rs`
/ `registry.rs` into the live runtime for baseline-vs-candidate outcome
comparison. Deferred to a later wave — depends on real A/B signal (dep B) and is
orthogonal to the governance wiring.

### 3.2 (a) Hook point — reuse the existing async/background spawn

**Locked decision: no new hook.** The correct hook already exists and is correctly
shaped: the detached `tokio::spawn` at `turn_coordinator.rs:413`, strictly after
kernel settlement (`terminal?;`, `:411`), fire-and-forget. Evolution stays
**async/background**, never synchronous. The design changes *what the step does*
and *how often*, not *where it fires*. Rationale: a synchronous evolution step
would couple candidate verification latency into the turn — unacceptable. The
current direct `cargo test --workspace` runner (`sandbox_runner.rs:32`) is
removed rather than retained in production. `project()` already isolates the evolution error
(`post_turn_projection.rs:142-153`) and the spawn already swallows failures with a
`warn!` (`turn_coordinator.rs:414-416`).

### 3.3 (b) What a candidate is, and what evidence it is evaluated against

A **candidate** is a `fabric::RuntimeCandidate { id, genome, changes, generated_at }`
(`fabric/src/include/meta.rs:17`) produced from a `fabric::MutationIntent
{ target, change, reason, reversible }` (`fabric/src/include/self_field.rs:129`),
verified into a `VerificationReceipt` (`service.rs:83-92`) that carries a
`score`, a `VerificationDecision` (`Adopt|PartialAdopt|Reject|NeedsMoreTesting`,
`service.rs:74-81`) and a `verification_hash`.

Evidence sources, layered by availability (this is the A/B degradation surface):

| Evidence | Source | path:line | Wave dep |
|---|---|---|---|
| Reflection window (turn metrics) | `MutationIntentGenerator::from_reflections` | `promotion.rs:79` | none (always present) |
| Sandbox test result | `SandboxRunner::run_tests` | `sandbox_runner.rs:32` | none |
| Candidate score / recommendation | `Evaluator::evaluate` | `candidate_evaluator.rs:26` | none |
| Memory-recalled prior outcomes | mnemosyne recall (A) | future | **A** |
| Multi-agent evaluation receipts | Planner/Executor/Reviewer signal (B) | future | **B** |

The current evaluator does not consume A/B evidence and can recommend `Adopt`
from a passing test result plus one bounded change
(`candidate_evaluator.rs:34-99`). Safe degradation therefore cannot be emergent.
Add an explicit `EvolutionEvidencePolicy`: an approval proposal requires a
candidate-aware sandbox receipt plus the configured minimum number of durable
coding-evaluation/multi-agent receipts for the same rubric and genome lineage.
When A/B are unavailable, verification may park a candidate for inspection, but
the proposer records `insufficient_evidence` and creates no approval.

### 3.3.1 Candidate-aware sandbox

Replace the production `SandboxRunner` behavior that invokes
`cargo test --workspace` while ignoring `_candidate`
(`crates/metacog/src/evolution/sandbox_runner.rs:28-40`). A genome candidate is
data, not source code, so repository compilation is neither isolation nor proof
that the candidate is safe.

`GenomeCandidateSandbox` runs in-process over an isolated temporary genome/state
root and must:

1. load the exact candidate digest;
2. validate immutable identity/boundary rules and numerical ranges;
3. replay a bounded, versioned evaluation corpus against baseline and candidate;
4. reject regressions in mandatory safety gates even when aggregate score rises;
5. emit a durable `CandidateSandboxReceipt` containing candidate digest, corpus
   version, passed/failed cases, elapsed budget, and terminal status.

The daemon never launches Cargo. Repository checks remain implementation
validation commands run through `bash scripts/cargo-agent.sh` outside the
production mutation path.

### 3.4 (c) Governance / approval boundary — the core of C

Single-authority is preserved by **splitting proposal from authorization**:

- The `GovernedEvolutionProposer` (new, thin) observes a verified `Adopt`
  candidate and creates a **pending** approval via the sole approval factory
  `ApprovalRepository::create(ApprovalCreate { .. })`
  (`repository.rs:448`), with `category = ApprovalCategory::DaseinModification`
  (`fabric/.../approval.rs:41`) and `subject.attributes` bound to
  `mutation_id` / `operation="apply"` / `verification_hash` exactly as
  `validate_evidence` requires (`service.rs:350-354`). It mints **no permit** and
  performs **no apply**. `PartialAdopt` remains parked until a future typed
  subset-candidate flow re-verifies the exact reduced change set; it never
  inherits the original verification hash. This is the runtime's only reach.
- The **operator** resolves the pending approval through the existing
  `ApprovalService::resolve` (`approval_service.rs:134`) / approval RPC
  (`rpc_approval.rs`). Only on human `Approved` does the resolve path request
  kernel admission for `CapabilityId("metacog.apply")`
  (`production.rs:228` mints it) and call `DefaultMetacogService::apply`
  (`service.rs:462`) with the assembled `GovernedMutationEvidence`.
- The self-approval guard (`registry.rs:135-140`) and the `DaseinModification`
  category ensure the model can never stand in for the authority.

**Scope of what apply may touch:** only the `Genome` — routed through
`SpecEditor` targets `care.priorities` / `boundary.rules` /
`identity.{name,description}` (`editor.rs:31-55`). It may **never** touch source
code, on-disk config (beyond the genome artifact), prompts, or model weights
(§2). This boundary is enforced structurally: `apply` calls `runtime.migrate`
(`service.rs:521`). The live composition must configure an explicit genome path,
and migration must write temp + fsync + atomic rename, read the genome back,
verify its digest/version, then update the in-memory effective genome. A receipt
cannot be `success=true` before durable read-back succeeds.

```
                 settled turn (post-gate)
                          |
      turn_coordinator.rs:413  tokio::spawn (detached, non-authoritative)
                          v
     EvolutionCoordinator::post_turn        evolution_coordinator.rs:137
        reflect + window + schedule check
                          v
     run_evolution -> meta.verify(intent)   evolution_coordinator.rs:334
                          v
     DefaultMetacogService::verify          service.rs:372
        generate -> sandbox -> evaluate -> Decision + durable candidate
                          |
             Adopt + evidence policy? ---- no --> park with typed reason, STOP
                          | yes
                          v
     [NEW] GovernedEvolutionProposer
        ApprovalRepository::create(category=DaseinModification, pending)  repository.rs:448
                          v
     ======================= AUTHORITY BOUNDARY =======================
        operator ApprovalService::resolve(Approved)   approval_service.rs:134
                          v
        kernel admit CapabilityId("metacog.apply") -> ExecutionPermit  production.rs:228
                          v
     DefaultMetacogService::apply(ApplyMutation{verification, evidence})  service.rs:462
        validate_evidence -> migrate genome -> MutationReceipt (durable)  service.rs:503-548
                          v
        rollback available via governed RollbackMutation                  service.rs:551
```

### 3.5 (d) Persistence & reversibility

Already durable: verified candidates + apply/rollback receipts persist to
`metacog-mutations.json` via `update_and_persist` (`service.rs:719-731`,
atomic temp-rename `:248-274`); each `MutationReceipt` (`service.rs:120-131`)
carries `permit_id`, `approval_id`, `receipt_hash`, and version transition.
Lineage is appended to JSONL (`LineageTracker::with_path`, `lineage.rs:94`).
Reversibility is a governed `RollbackMutation` (`service.rs:107-111, 551`)
requiring its own `metacog.rollback` permit + `DaseinModification` approval — the
same authority boundary, in reverse. **Design fix required:** persist the
`RollbackManager` snapshot stack (`rollback.rs`, currently in-memory only,
`runtime.rs:37-38`) so rollback survives restart; otherwise a post-restart
rollback restores only the lineage/version, not the genome snapshot. The fix is
one atomic genome store containing the current version, content digest, previous
version chain, and full rollback snapshots. Apply and rollback both persist and
read back this store before emitting a success receipt.

### 3.6 (e) Scheduling — periodic, not every turn

Keep the existing `trigger_every_n_turns` (bootstrap sets `10`,
`request.rs:524`) + `trigger_on_failure` (`evolution_coordinator.rs:198-200`)
for the **verify** step, run in background. The candidate-aware sandbox has an
explicit elapsed/case budget and performs no Cargo build. The **apply** step is never
scheduled — it is strictly operator-driven and out-of-band. There is no
dream-cycle in the codebase today (`grep` for `dream*` is empty); introducing one
is out of scope. Verify frequency must be bounded because each trigger can invoke
candidate replay work; a per-turn cadence is explicitly rejected.

## 4. Error handling

- **Isolation (must never break a turn).** Evolution runs in the detached spawn
  (`turn_coordinator.rs:413`); `project()` accumulates and returns evolution
  errors without touching settlement (`post_turn_projection.rs:142-153`), and the
  spawn logs and drops them (`:414-416`). Turn settlement already completed at
  `terminal?;` (`:411`) before the spawn — structurally independent.
- **Governance rejection.** `verify` returning `Reject`/`NeedsMoreTesting`
  parks the candidate `Rejected` (`service.rs:439-447`) and rolls back the
  runtime candidate (`:443`); no approval is requested. A denied operator
  approval leaves the mutation un-applied (`Verified`, not `Applied`).
- **Apply failure / uncertainty.** `apply` maps runtime uncertainty to
  `ReconciliationRequired` (`service.rs:491-495, 524-533`) rather than silent
  success, and marks `Applying` durably before migrating (`:511-516`) so a
  crash is recoverable.
- **Replay / forgery.** Permits are single-use (`ensure_unused_permit`,
  `service.rs:510, 650`); evidence is bound to `mutation_id`+hash
  (`validate_evidence :350-354`); idempotent re-apply requires matching
  permit/approval (`:481-490`).
- **Resource/budget bounds.** Gate `GenomeCandidateSandbox` behind the enabled
  flag and bounded cadence (§3.6); cap replay cases, elapsed time, and verify
  concurrency (the service already serializes via `operations` async-mutex,
  `service.rs:280,374`). Exhaustion yields `NeedsMoreTesting`, never `Adopt`.

## 5. Verification

Unit (metacog, extend `crates/metacog/tests/service_contract.rs`):

- **Ungoverned candidate is rejected:** `apply` with a missing / wrong-category
  (`!= DaseinModification`) / expired / unbound-subject approval returns
  `MetacogError::Unauthorized` — asserts `validate_evidence`
  (`service.rs:328-358`). Include a "self-issued" evidence case.
- **Verify never migrates:** `verify` on `Adopt` leaves lifecycle `Verified`
  with no `apply_receipt` (extends existing `evolution_integration.rs:210-214`
  `migrate_calls == 0`).
- **Rollback requires its own governed evidence** (`metacog.rollback` +
  `DaseinModification`).
- **Candidate-aware sandbox:** changing the candidate digest changes the sandbox
  receipt; an immutable-boundary regression is rejected even when repository
  tests pass.
- **Evidence policy:** `Adopt` without the required A/B receipt set parks with
  `insufficient_evidence` and creates no approval.
- **Restart durability:** apply persists and reads back the genome; reopening the
  runtime sees the applied digest; governed rollback after reopen restores the
  previous digest.

Unit (executive):

- **Proposer parks, never applies:** `GovernedEvolutionProposer` on an `Adopt`
  verification creates exactly one pending `DaseinModification` approval and
  invokes neither admission nor `apply`.
- **Turn success independent of evolution:** a turn whose evolution step errors
  (or panics in the spawn) still returns `Ok(TurnResult)` — asserts the
  isolation at `turn_coordinator.rs:412-418` and `post_turn_projection.rs:142-153`.

Integration (`crates/executive/tests/evolution_integration.rs`):

- Full governed happy path with a mock approval authority: verify(Adopt) →
  pending approval → operator resolve(Approved) → permit minted → `apply` →
  durable `MutationReceipt`; then governed rollback restores prior version.
- Default-off posture unchanged: with `enabled=false`/`permitted=false` the loop
  is a no-op (existing `disabled_coordinator_is_a_noop`,
  `evolution_integration.rs:343-388`) and no approval is ever created.

Commands (narrowest first, via the wrapper):

```
bash scripts/cargo-agent.sh test -p metacog
bash scripts/cargo-agent.sh test -p executive --test evolution_integration
bash scripts/cargo-agent.sh fmt --all -- --check
bash scripts/aletheon.sh test architecture
```

After implementation, the integration owner must run
`sudo bash scripts/aletheon.sh deploy`, prove equal SHA-256 digests for the
release, installed, machine-daemon, and user-daemon executables, observe stable
restart counters, and complete a real LLM-backed `/usr/bin/aletheon` turn through
the official user socket. Acceptance must observe candidate parking, human
approval, durable read-back, daemon restart, and governed rollback in the
installed runtime; rendered output, approval rows, genome store, mutation
receipts, and daemon logs must agree. Temporary homes, direct service calls, and
isolated daemons are diagnostic only.

## 6. Files touched

- `crates/executive/src/core/evolution_coordinator.rs` — surface the verified
  `VerificationReceipt`s to the caller so the proposer can act on `Adopt`
  (they are already returned in `EvolutionSummary.verification_receipts`,
  `:60-69`); no change to the verify-only contract at `:316-343`.
- `crates/executive/src/host/daemon/bootstrap/request_ports.rs` — in
  `PostTurnDomainAdapter::post_evolution` (`:144-160`), after evidence-priority
  handling (`:165-167`), invoke the new proposer on `Adopt` verifications.
- **New** `crates/executive/src/application/governed_evolution_proposer.rs`
  — `GovernedEvolutionProposer`: verified-`Adopt` → pending `DaseinModification`
  approval via `ApprovalRepository::create` (`repository.rs:448`). No permit, no
  apply.
- `crates/executive/src/application/mod.rs` — register the proposer module.
- `crates/executive/src/application/approval_service.rs` — extend the resolve
  path (currently auto-coordinates only `ApplyCode`, `:154`) to, on
  `DaseinModification` `Approved`, request `metacog.apply` admission and call
  `DefaultMetacogService::apply`.
- `crates/executive/src/host/daemon/bootstrap/request.rs` — keep default-off,
  configure the durable genome path/store, and expose `evolution_permitted` only
  after prerequisites are healthy (currently hardcoded `false`, `:523`).
- `crates/metacog/src/evolution/sandbox_runner.rs` — replace direct Cargo with
  `GenomeCandidateSandbox` and versioned replay corpus.
- `crates/metacog/src/evolution/migration.rs`, `rollback.rs`, and
  `governance/runtime.rs` — atomic genome store, read-back verification, durable
  snapshots, and restart-safe rollback.
- `crates/metacog/src/governance/service.rs` — apply success only after durable
  genome read-back; evidence policy and typed insufficient-evidence state.
- Tests: `crates/metacog/tests/service_contract.rs`,
  `crates/executive/tests/evolution_integration.rs`.

## 7. Scope boundary

This is **Workstream C, Wave 3.** Apply has hard dependencies on A/B evidence,
candidate-aware sandboxing, and the durable genome store. Before those exist, C
may land verify/parking/governance plumbing only; it must keep
`evolution_permitted=false`, create no apply approval, and make no claim of
production evolution capability.

C explicitly **excludes**: any source/config/prompt/model self-modification
(genome-only, §2); auto-apply on `Adopt`; turning evolution on by default;
wiring the full A/B experiment scheduler (`experiment*.rs`, option C in §3.1).
Candidate-aware sandboxing and persistent genome/rollback state are now required
parts of bringing governed apply online, not follow-ups.

## Locked review decisions

1. **Authority:** every genome mutation and rollback requires a human-resolved
   `DaseinModification` approval; there is no automated self-approval tier.
2. **Mutation scope:** evolution is genome-only and never uses `ApplyCode` to
   alter source, prompts, model routing, capabilities, or general configuration.
3. **Default posture:** `evolution_permitted` remains false until A/B evidence,
   candidate sandbox, and durable genome-store health are all available. It then
   becomes operator-configurable but still defaults false.
4. **Sandbox:** production uses a candidate-aware typed replay sandbox and never
   invokes Cargo. Direct workspace Cargo is not accepted as candidate evidence.
5. **Durability:** current genome and rollback snapshots are persisted and read
   back; lineage alone is insufficient for post-restart reversal.
