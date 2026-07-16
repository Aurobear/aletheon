# V01 Cross-Domain Acceptance Suite Implementation Plan

**Goal:** Prove the integrated architecture with deterministic lifecycle, isolation, recurrence, replay and functional-indicator tests rather than isolated unit success.

**Architecture:** A deterministic in-process harness composes real coordinators and file-backed repositories with fake external providers, then compares causal receipts and projection checksums across replay and ablations.

**Tech Stack:** Rust integration tests, SQLite fixtures, deterministic Clock/IDs, machine-readable reports

**Source requirements:** `docs/plans/2026-07-15-dasein-agora-conscious-core-plan.md:703-747` and the acceptance clauses at `docs/plans/2026-07-15-architecture-coupling-optimization-plan.md:1106-1160`.

**Prerequisites:** C02, M07, M08, R02 and Q02.

## Current-code anchors

- Existing domain fixtures start at `crates/agora/tests/transaction_integrity.rs:1-4` and `crates/mnemosyne/tests/unified_memory_contract.rs:1-5`, but they do not compose all domains.
- Deterministic Kernel time is already injected through Fabric `Clock` and used by Agora construction at `crates/executive/src/impl/daemon/bootstrap/request.rs:631`.
- Architecture gates run through `scripts/architecture-check.sh` and `justfile:50`.
- Existing TUI scenario inputs begin at `tests/tui_scenarios/basic_response.txt:1` and are presentation-oriented rather than a complete cross-domain causal fixture.

## Invariants and non-goals

- The suite does not depend on live network services.
- Friendly model prose is not acceptance evidence.
- Functional indicators are not claims of phenomenal consciousness.

## Key contracts

```rust
pub struct AcceptanceEvidence { pub fixture_version: u32, pub event_checksum: String, pub projection_checksums: BTreeMap<String, String>, pub indicator_results: Vec<IndicatorResult>, pub limitations: Vec<String> }
```

## Task 1: Build one deterministic cross-domain harness

**Create:** `crates/executive/tests/support/conscious_core_harness.rs`
**Create:** `crates/executive/tests/fixtures/conscious_core/baseline_v1.json`
**Create:** `crates/executive/tests/cross_domain_acceptance.rs`

- [ ] Inject deterministic clock, IDs, provider responses, policy decisions and bounded queues.
- [ ] Run the real Executive coordinators with SQLite repositories in a temporary data root.
- [ ] Capture R01 envelopes, projection checksums, Dasein versions, Agora epochs, Agent tree and memory receipts.
- [ ] Fail on unexpected external network/process use.

Run: `cargo test -p executive --test cross_domain_acceptance harness_replays_identically`

## Task 2: Prove lifecycle, authority and replay

**Modify:** `crates/executive/tests/cross_domain_acceptance.rs`

- [ ] Cover observation -> selection -> action -> outcome recurrence.
- [ ] Cover daemon restart during memory lease, Agent run, mailbox delivery and event projection.
- [ ] Cover duplicate delivery/idempotency, cancellation and bounded overload.
- [ ] Rebuild Session, debug, memory, Agent and metrics projections and compare checksums.
- [ ] Attempt self mutation, capability execution and broad memory writes through every public path; only governed operations succeed.

Run: `cargo test -p executive --test cross_domain_acceptance lifecycle authority replay`

## Task 3: Prove memory and Agent isolation

**Modify:** `crates/executive/tests/cross_domain_acceptance.rs`

- [ ] Inject irrelevant/adversarial local and GBrain recall; verify it remains candidate data and cannot mutate self/policy.
- [ ] Spawn sibling Agents; verify context, mailbox, memory and worktree isolation.
- [ ] Promote one child result and verify full G09/M08 receipt lineage.
- [ ] Verify ordinary spawn cannot create an independently persistent subject.

Run: `cargo test -p executive --test cross_domain_acceptance memory_isolation agent_isolation promotion`

## Task 4: Implement functional indicator and ablation measurements

**Create:** `crates/executive/tests/functional_indicators.rs`
**Create:** `crates/fabric/src/types/conscious_core_trace.rs`

- [ ] Measure recurrence, global availability, capacity bottleneck, attention modulation and temporal continuity.
- [ ] Measure prediction error, self-attribution, metacognitive calibration, agency and narrative cause references.
- [ ] Run workspace, recurrence and Dasein ablations against the same fixture and record metric deltas.
- [ ] Add surprise, competition fairness, mutation integrity and narrative-faithfulness cases.
- [ ] Exclude hidden reasoning and never interpret model self-report as evidence.

Run: `cargo test -p executive --test functional_indicators`

## Task 5: Publish machine-readable evidence and gates

**Create:** `tools/acceptance_report.py`
**Create:** `docs/testing/cross-domain-acceptance.md`
**Modify:** `justfile`

- [ ] Emit fixture version, commit, config schema, indicator definitions, results and limitations as JSON plus a concise Markdown report.
- [ ] Add `just acceptance` running architecture, deterministic replay, isolation and ablation suites serially where required.
- [ ] Fail when ignored acceptance tests, unbounded timeouts or fixture drift are present.

Run: `just acceptance`

## Final verification and commit

Run: `scripts/architecture-check.sh && cargo test --workspace --all-targets --no-fail-fast`

Inspect the staged diff, then commit with subject `test(acceptance): prove cross-domain invariants` and a body that records the source requirement, authority/bypass problem, implemented boundaries, focused tests and deletion evidence.

## Completion evidence

- [ ] All causal, replay, authority, isolation and restart cases pass twice from clean roots.
- [ ] Ablations produce reproducible measurements with explicit limitations.
- [ ] No acceptance result claims phenomenal consciousness.
