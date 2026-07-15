# Agora Typed Candidate Selection Implementation Plan

> **For agentic workers:** Use `workflow-feature` or `plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Admit bounded typed workspace candidates and deterministically select one winner or a dependency-required coalition with an auditable score explanation.

**Architecture:** Fabric defines immutable candidate/content/salience/provenance/visibility and broadcast vocabulary. Agora owns a per-space `CandidatePool` with explicit capacity/source-quota outcomes, stable content fingerprints, monotonic expiry and a configurable deterministic selector. A02 returns selection results only; A03 durably commits epochs and delivers them.

**Tech Stack:** Rust, serde/serde_json, SHA-256, deterministic monotonic time, existing Fabric cognitive contracts

**Prerequisites:** A01 commit `e54e5e1` is green.

**Source requirements:**
- `docs/plans/2026-07-15-dasein-agora-conscious-core-plan.md:417-483`
- `docs/plans/2026-07-15-dasein-agora-conscious-core-plan.md:485-515`
- `docs/plans/2026-07-15-dasein-agora-conscious-core-plan.md:827-845`
- `docs/plans/2026-07-15-dasein-agora-conscious-core-plan.md:910-915`

---

## Current-code anchors

- core Agora operations still carry JSON fields: `crates/fabric/src/include/agora.rs:23-48`;
- the workspace snapshot is transactional state, not a candidate queue: `crates/agora/src/workspace/mod.rs:329-355`;
- no candidate admission or selection implementation exists under `crates/agora/src`;
- deterministic time contracts already exist: `crates/fabric/src/types/time.rs:6-22`;
- canonical Evidence, Hypothesis, Plan, SelfSignal and AgentResult contracts already exist and must be reused rather than redefined.

## Invariants

1. Every candidate has validated provenance, visibility, lifecycle and bounded payload metadata.
2. Core selection never examines arbitrary JSON extension payloads.
3. Pool capacity and per-source quotas return typed rejection outcomes; overload is never silent.
4. Fingerprint-identical content deduplicates deterministically.
5. Score calculation uses all salience dimensions plus aging, unresolved dependency, repetition and refractory terms.
6. Tie-breaking is `(score desc, created_at asc, content_id asc)`.
7. A coalition contains extra candidates only when the primary winner names unresolved dependencies.
8. Only `SelectionResult.selected` can be projected into a future broadcast/global context.

## Explicit non-goals

- Dasein supplies self-relevance modulation through C01; A02 accepts the dimension but does not call Dasein.
- Durable broadcast epochs, subscriber delivery and acknowledgements belong to A03.
- Production processor wiring belongs to C01/C02.
- Legacy direct mutation cleanup remains gated on F01/X02.

## File map

- Create: `crates/fabric/src/types/workspace.rs`
- Modify: `crates/fabric/src/types/mod.rs`
- Modify: `crates/fabric/src/lib.rs`
- Create: `crates/fabric/tests/workspace_candidate_contract.rs`
- Create: `crates/agora/src/competition/mod.rs`
- Modify: `crates/agora/src/lib.rs`
- Create: `crates/agora/tests/candidate_selection.rs`
- Modify: `docs/plans/2026-07-15-executable-plan-decomposition-design.md`
- Modify: `docs/plans/2026-07-16-original-plan-coverage-matrix.md`

### Task 1: Define typed immutable workspace vocabulary

- [x] Add `ContentId`, `BroadcastEpoch`, typed content payloads and `WorkspaceContent` variants for observation, evidence, hypothesis, prediction/error, goal, concern, plan, action proposal, tool outcome, Agent result, reflection and extension.
- [x] Reuse existing Fabric types for Evidence, Hypothesis, Plan, SelfSignal and AgentResult.
- [x] Add `WorkspaceProvenance`, `VisibilityScope`, `SalienceVector`, `WorkspaceCandidate`, `CandidateScore`, `SelectionExplanation`, `SelectionResult` and `WorkspaceBroadcast`.
- [x] Use tagged snake-case serde and reject unknown/non-versioned extension schemas.

Run: `cargo test -p fabric --test workspace_candidate_contract contract_`

Expected: all variants round-trip and stable tags/IDs are asserted.

### Task 2: Validate bounds, provenance and lifecycle

- [x] Validate confidence and all eight salience dimensions as finite `[0,1]` values.
- [x] Require candidate source to equal provenance producer, non-empty source references, non-private empty visibility and expiry after creation.
- [x] Bound source references, dependencies and text/payload sizes.
- [x] Compute a stable SHA-256 content fingerprint excluding arrival time and salience.

Run: `cargo test -p fabric --test workspace_candidate_contract validation_ fingerprint_`

Expected: each invalid field fails closed and semantically identical content has one fingerprint.

### Task 3: Implement explicit bounded admission

- [x] Add `CandidatePoolConfig`, `CandidatePool` and typed `AdmissionOutcome`.
- [x] Partition pools by `AgoraSpaceId`; reject wrong-space candidates.
- [x] Expire candidates before admission, enforce total capacity and per-source quota, and deduplicate by fingerprint.
- [x] Keep rejection counters for capacity, source quota, invalid input and duplicates.

Run: `cargo test -p agora --test candidate_selection admission_`

Expected: overload, quotas, expiry and duplicate behavior are deterministic and observable.

### Task 4: Implement deterministic multidimensional selection

- [x] Add versioned `SelectionPolicy` with weights for all eight salience dimensions.
- [x] Add aging, unresolved-dependency boost, repetition penalty and source-refractory penalty.
- [x] Apply the stable tie-break order and ignition threshold.
- [x] Record every term in `CandidateScore` and return an engineering `SelectionExplanation`.

Run: `cargo test -p agora --test candidate_selection selection_`

Expected: a recorded fixture selects byte-identical IDs, scores and explanation on repeated runs.

### Task 5: Add quotas, anti-monopoly and coherent coalition

- [x] Track recent source winners and per-content repetition counts.
- [x] Penalize a source after configured consecutive wins when another eligible source exists.
- [x] Add only available unresolved dependency IDs behind the primary winner, ordered deterministically and bounded by coalition size.
- [x] Return starvation/refractory metrics and leave unselected content inside the private pool.

Run: `cargo test -p agora --test candidate_selection fairness_ coalition_ projection_`

Expected: an alternate source wins under repetition; coalitions contain only named dependencies; projection contains only selected IDs.

### Task 6: Verify and commit

```bash
cargo fmt --all -- --check
cargo clippy -p fabric -p agora --all-targets -- -D warnings
cargo test -p fabric
cargo test -p agora
cargo test --workspace
bash tests/architecture_check.sh
bash scripts/architecture-check.sh
```

Expected: all commands pass and architecture findings do not increase.

Commit subject: `feat(agora): select bounded typed candidates`

## Compatibility deletion gate

- JSON `PublishFact`, `ProposePlan`, `UpdateTask` and `EmitObservation` remain transactional compatibility operations until C01/F01 migrates their production producers.
- `WorkspaceContent::Extension` is admitted for storage/audit only and receives no core semantic inspection or implicit score boost.
- Remove typed-over-untyped trace adapters when F01 proves no production caller bypasses candidate admission.

## Completion evidence

- [x] contracts validate and round-trip;
- [x] pool overload/source quota/dedup/expiry are explicit;
- [x] fixture selection is deterministic;
- [x] all score terms are auditable;
- [x] alternate sources cannot be monopolized indefinitely;
- [x] coalitions contain only declared dependencies;
- [x] unselected content is absent from selection projection;
- [x] workspace and architecture checks pass.
