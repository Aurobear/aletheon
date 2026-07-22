# P3 RobotHarness Implementation Plan

> **For agentic workers:** Use Goal mode and execute tasks in order. Update checkboxes only after the stated test passes. Do not use subagents unless explicitly enabled.

**Goal:** Add deterministic outcome verification and a bounded RobotHarness without changing the Linear harness.

**Architecture:** Fabric owns stable contracts, Metacog evaluates predicates, Executive owns production world-state and execution composition, Cognit owns the state machine, and Mnemosyne persists immutable episodes.

**Tech Stack:** Rust 1.85+, Tokio, serde/serde_json, existing Fabric clocks and Aletheon stores.

---

## Preconditions

- [ ] P2 live MuJoCo acceptance exists and both repositories are clean.
- [ ] Re-read `2026-07-22-p3-robot-harness-design.md` and current cited symbols.
- [ ] Create a feature branch from updated `origin/dev`.
- [ ] Use `bash scripts/cargo-agent.sh`; never invoke Cargo directly.

## Task 1: Fabric expected-outcome contract

**Files:**
- Create: `crates/fabric/src/types/expected_outcome.rs`
- Modify: `crates/fabric/src/types/mod.rs`
- Test: `crates/fabric/tests/expected_outcome_schema.rs`

- [ ] Add failing serde/schema tests for Equals, NotEquals, inclusive Range, signed Change, nested All/Any, depth 9 rejection, 65-node rejection, empty path, empty All/Any, NaN and Infinity.
- [ ] Implement `ExpectedOutcome`, `OutcomePredicate`, `NumericRange`, `NumericChange`, and `OutcomeContractError`; constructors validate depth ≤8 and nodes ≤64.
- [ ] Run `bash scripts/cargo-agent.sh test -p fabric --test expected_outcome_schema`; expect PASS.
- [ ] Commit `feat(fabric): define deterministic expected outcomes` with full body.

Required public shape:

```rust
pub struct ExpectedOutcome {
    pub predicate: OutcomePredicate,
    pub freshness_ms: u64,
    pub stable_window_ms: u64,
    pub timeout_ms: u64,
}

pub enum OutcomePredicate {
    Equals { path: String, value: serde_json::Value },
    NotEquals { path: String, value: serde_json::Value },
    Range { path: String, min: Option<f64>, max: Option<f64> },
    Change { path: String, min_delta: Option<f64>, max_delta: Option<f64> },
    All { predicates: Vec<OutcomePredicate> },
    Any { predicates: Vec<OutcomePredicate> },
}
```

## Task 2: Fabric verification and world-state ports

**Files:**
- Create: `crates/fabric/src/types/outcome_verification.rs`
- Create: `crates/fabric/src/types/world_state.rs`
- Modify: `crates/fabric/src/types/mod.rs`
- Test: `crates/fabric/tests/outcome_verification_schema.rs`

- [ ] Test stable serde tags for `Matched`, `RetryableMismatch`, `ReplannableMismatch`, `Unsafe`, and `Unknown`.
- [ ] Add `VerificationReport` with decision, evaluated sequence, observed paths, reasons and EvidenceRef; no free-form success boolean.
- [ ] Add async `WorldStatePort::latest(device)` and `WorldStatePort::observe_until(device, after_sequence, deadline)` returning normalized `WorldSnapshot`.
- [ ] Run Fabric tests and commit `feat(fabric): add world-state verification ports`.

## Task 3: Deterministic OutcomeVerifier in Metacog

**Files:**
- Create: `crates/metacog/src/outcome_verifier.rs`
- Modify: `crates/metacog/src/lib.rs`
- Test: `crates/metacog/tests/outcome_verifier.rs`

- [ ] Write table-driven tests for every predicate, missing/type mismatch, boundary inclusivity, stale snapshot, stable-window reset, provider failure, Provider success with no delta, and unsafe fault observation.
- [ ] Implement dot-path traversal over JSON objects only; arrays and escaped expressions are unsupported and return Unknown.
- [ ] Evaluate Change using before/after numeric values; stable window requires every new sequence in the window to match.
- [ ] Map explicit safety faults to Unsafe, inconclusive evidence to Unknown, deterministic unmet predicates to RetryableMismatch on first attempt and ReplannableMismatch after retry.
- [ ] Run `bash scripts/cargo-agent.sh test -p metacog --test outcome_verifier`; expect PASS.
- [ ] Commit `feat(metacog): verify embodied outcomes from state deltas`.

## Task 4: Production WorldModel adapter

**Files:**
- Create: `crates/executive/src/service/world_state.rs`
- Modify: `crates/executive/src/service/mod.rs`
- Test: `crates/executive/tests/world_state.rs`

- [ ] Test per-device latest sequence, stale rejection, waiter wake-up, timeout, duplicate sequence, lower-sequence rejection, bounded device count and evidence preservation.
- [ ] Implement an `Arc<RwLock<...>>` adapter with injected Fabric Clock; never read wall clock or environment inside methods.
- [ ] Wire P2 observations into this adapter at the existing embodiment service boundary, not through EventBus state authority.
- [ ] Run focused Executive test and commit `feat(executive): host production embodied world state`.

## Task 5: EmbodiedEpisode contract and repository

**Files:**
- Create: `crates/fabric/src/types/embodied_episode.rs`
- Create: `crates/mnemosyne/src/embodied_episode.rs`
- Modify module exports
- Test: `crates/mnemosyne/tests/embodied_episode.rs`

- [ ] Define immutable episode/attempt DTOs with goal ID, attempt operation ID, expected outcome, before/after snapshots, result, verification, recovery and evidence.
- [ ] Test schema version, append idempotency, conflicting replay rejection, ordering, reopen durability and that no raw image/joint stream is stored inline.
- [ ] Implement repository using current Mnemosyne persistence conventions and transactional append.
- [ ] Run focused tests; commit `feat(mnemosyne): persist embodied execution episodes`.

## Task 6: RobotHarness state machine

**Files:**
- Create: `crates/cognit/src/harness/robot/{mod.rs,state.rs,session.rs}`
- Modify: `crates/cognit/src/harness/mod.rs`
- Test: `crates/cognit/tests/robot_harness.rs`

- [ ] Define `RobotState` exactly as Observe/Plan/Authorize/Execute/Verify/Retry/Replan/Recover/Settle/SafeStop/Completed/Failed.
- [ ] Inject narrow ports for planning, governed execution, verification, episode append and safe stop; Cognit must not depend on Executive/Hardware.
- [ ] Test matched path, one retry with new operation, one replan with new authorization, retry exhaustion, unknown/unsafe SafeStop, cancellation, and episode emission.
- [ ] Enforce `MAX_RETRIES=1`, `MAX_REPLANS=1`; no config may raise them in P3.
- [ ] Run Cognit test and commit `feat(cognit): add bounded robot harness state machine`.

## Task 7: Add `HarnessKind::Robot` atomically with factory

**Files:**
- Modify: `crates/cognit/src/harness/mod.rs`
- Modify: `crates/executive/src/service/harness_factory.rs`
- Modify: `crates/executive/src/impl/daemon/bootstrap/*` only at verified factory composition points
- Test: `crates/executive/tests/robot_harness_factory.rs`

- [ ] First add a failing test that `robot` selects RobotHarness and missing required ports fails preflight; retain Linear selection parity.
- [ ] Add the enum variant only in the same commit that provides the real factory and all required dependencies.
- [ ] Ensure no `robot` alias falls back to Linear and no new daemon entry exists.
- [ ] Run factory, TurnEngine parity and daemon-turn tests; commit `feat(executive): construct the real robot harness`.

## Task 8: Recovery and settlement integration

**Files:**
- Create: `crates/executive/src/service/embodied_recovery.rs`
- Modify: verified RobotHarness composition only
- Test: `crates/executive/tests/embodied_recovery.rs`

- [ ] Test new operation identity for retry, new admission for replan, exactly-once settlement, failed episode append not changing execution truth, and SafeStop priority.
- [ ] Implement deterministic mapping: RetryableMismatch→one retry; ReplannableMismatch→one replan; Unsafe/Unknown/exhausted→SafeStop and failed settlement.
- [ ] Run focused tests; commit `feat(executive): govern embodied recovery and settlement`.

## Task 9: P3 MuJoCo E2E

**Files:**
- Create: `crates/executive/tests/robot_harness_mujoco.rs` or an external ignored live harness following current integration-test conventions
- Update: `docs/plans/2026-07-22-p3-robot-harness-design.md` status/evidence only after success

- [ ] Run P2 Bridge and standard Kuavo MuJoCo.
- [ ] Verify finite movement produces Matched from actual before/after state and completes settlement.
- [ ] Inject a Provider success without state change; verify mismatch, one retry/replan boundary, then SafeStop and failed settlement.
- [ ] Verify stale state immediately causes Unknown and SafeStop.
- [ ] Record commits, operations, sequences, timings and artifact hashes; no secrets/raw high-frequency data.

## Task 10: Final P3 validation

- [ ] Run formatter and focused Fabric/Metacog/Cognit/Mnemosyne/Executive suites through cargo-agent.
- [ ] Run `bash scripts/cargo-agent.sh build --workspace` and `test --workspace` only as verification owner.
- [ ] Confirm `rg -n 'hardware|kuavo|rospy|geometry_msgs' crates/cognit/src/harness/robot` returns no dependency leak.
- [ ] Confirm Linear parity remains green and P2 Provider tests remain green.
- [ ] Commit test evidence separately; report rollback and remaining P4 scope.
