# Selective Agent Pattern Absorption Implementation Plan

> **For agentic workers:** Use `flow-feature` or `plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Absorb useful Codex/Pi patterns in bounded increments without duplicating Aletheon capabilities or replacing its canonical architecture.

**Architecture:** Establish a reproducible baseline first, then enable one
pattern at a time behind configuration or policy gates. Every increment must
beat the baseline on task success or operability without unacceptable latency,
token, cache, or recovery regression. SQLite, the existing context assembler,
the existing lifecycle/world-state ports, and the existing BM25 catalog remain
authoritative.

**Tech Stack:** Rust, existing Cognit/Executive/Corpus/Mnemosyne APIs, SQLite,
serde JSONL export, existing installed-runtime acceptance.

---

## Requirement and code anchors

- Selective reasoning/planning/critique originates at
  `docs/plans/gap-analysis-20260724.md:213-242`; current components are
  `crates/cognit/src/core/reasoner.rs:25`,
  `crates/cognit/src/core/planner.rs:17`, and
  `crates/cognit/src/core/critic.rs:12`.
- Fragment/world-state ideas originate at
  `docs/plans/gap-analysis-20260724.md:607-633`; Aletheon already has
  `ContextFragments` at
  `crates/executive/src/application/context_assembler.rs:23` and
  `LifecycleEffect::AddContextFragment` consumption at
  `crates/executive/src/application/context_fragment.rs:41`.
- Tool discovery originates at `docs/plans/gap-analysis-20260724.md:635-648`;
  the current implementation is
  `crates/corpus/src/tools/tools/search/tool_search.rs:7-29`.
- JSONL portability originates at
  `docs/plans/gap-analysis-20260724.md:514-531`; canonical resume/fork remains
  owned by `crates/executive/src/application/session_service.rs`.
- Prompt extraction originates at
  `docs/plans/gap-analysis-20260724.md:682-703`; the current default is
  `crates/cognit/src/config/mod.rs:575-580`.
- Window identity originates at
  `docs/plans/gap-analysis-20260724.md:705-720`.

## Global admission rule

For each track below:

```text
baseline -> gated increment -> focused tests -> installed A/B run
                                      |
                         no measurable benefit
                                      |
                               leave disabled
```

No track may be enabled by default until its listed acceptance gate passes.

---

### Task 1: Establish the absorption benchmark and receipts

**Files:**
- Create: `tests/fixtures/agent_pattern_absorption/cases.json`
- Create: `scripts/libexec/aletheon/agent-pattern-benchmark.sh`
- Create: `docs/testing/agent-pattern-absorption.md`
- Test: `tests/production/agent_pattern_absorption.sh`

- [ ] **Step 1: Define the fixed case set**

Create JSON cases for: simple read-only answer, multi-file implementation plan,
failed-deployment diagnosis, risky mutation review, deferred-tool discovery,
50-message fact retention, and session export/import. Each case must declare
expected evidence, forbidden behavior, maximum attempts, and whether mutation
is allowed.

- [ ] **Step 2: Write the failing receipt test**

Require a receipt containing:

```json
{
  "schema_version": 1,
  "variant": "baseline",
  "case_id": "string",
  "success": true,
  "input_tokens": 0,
  "output_tokens": 0,
  "cache_hit_tokens": 0,
  "latency_ms": 0,
  "tool_calls": 0
}
```

Reject missing fields, unknown cases, non-installed binaries, and alternate
sockets.

- [ ] **Step 3: Run the receipt test**

```bash
bash tests/production/agent_pattern_absorption.sh
```

Expected: FAIL until the runner writes valid installed-runtime receipts.

- [ ] **Step 4: Implement the runner**

The runner must call `/usr/bin/aletheon` through the official user socket, save
raw output and structured receipts under
`target/agent-pattern-absorption/`, and never call providers directly.

- [ ] **Step 5: Record the unmodified baseline**

Run every case three times. Store median success, token, cache-hit, latency, and
tool-call values. Later tracks compare against this immutable receipt set.

---

### Task 2: Gate Planner and Critic; do not add a universal Reasoner call

**Files:**
- Modify: `crates/cognit/src/harness/linear/mod.rs`
- Modify: `crates/cognit/src/harness/linear/step.rs`
- Modify: `crates/cognit/src/config/mod.rs`
- Test: `crates/cognit/src/harness/linear/mod.rs`
- Test: `crates/cognit/tests/cognitive_session.rs`

- [ ] **Step 1: Add an explicit policy type**

Add:

```rust
pub enum DeliberationPolicy {
    Off,
    ComplexTasks,
    HighRiskOnly,
}
```

Default it to `Off`. Do not expose chain-of-thought text to users or persist it
as canonical conversation history.

- [ ] **Step 2: Add failing routing tests**

Prove simple chat never invokes Planner/Critic; a declared complex task may
invoke Planner once; a high-risk final answer may invoke Critic once; rejection
cannot exceed the existing verifier retry limit.

- [ ] **Step 3: Reuse existing seams**

Use Planner only to produce a bounded private plan before the first model/tool
iteration. Adapt Critic through the existing verifier seam at
`crates/cognit/src/harness/linear/step.rs:82-100`. Keep Reasoner disabled as a
separate model call; provider-native reasoning remains the default.

- [ ] **Step 4: Run focused validation**

```bash
bash scripts/cargo-agent.sh test -p cognit deliberation
bash scripts/cargo-agent.sh test -p cognit cognitive_session
```

Expected: PASS with policy `Off` behavior identical to baseline.

- [ ] **Step 5: Admission gate**

Enable a policy only if complex/high-risk case success improves and median
latency and total tokens each regress by no more than 25%. Otherwise retain
`Off`.

---

### Task 3: Type existing context fragments and add section-level change detection

**Files:**
- Modify: `crates/executive/src/application/context_assembler.rs`
- Modify: `crates/executive/src/application/context_fragment.rs`
- Modify: `crates/executive/src/application/lifecycle_contributors.rs`
- Test: `crates/executive/tests/context_assembler.rs`
- Test: `crates/executive/tests/world_state.rs`

- [ ] **Step 1: Add fragment identity without a parallel framework**

Extend existing fragments with a stable `kind`, `role`, `content`, and
content-hash. Preserve the existing 16KB/bounded-injection rules and lifecycle
effect path.

- [ ] **Step 2: Add failing ordering and diff tests**

Prove stable ordering, identical fragments are not re-emitted as changed, one
changed world-state section does not invalidate unrelated sections, and removed
sections emit an explicit tombstone.

- [ ] **Step 3: Persist only the comparison receipt**

Persist section hashes/version metadata in canonical session items; do not
duplicate full subsystem state. Render changed dynamic fragments into the user
context while leaving the stable system prefix cacheable.

- [ ] **Step 4: Admission gate**

Three-turn installed tests must preserve exact model-visible semantics while
improving cache-hit tokens or reducing repeated injected tokens. Otherwise keep
full rendering.

---

### Task 4: Improve the existing BM25 catalog

**Files:**
- Modify: `crates/corpus/src/tools/tools/search/mod.rs`
- Modify: `crates/corpus/src/tools/tools/search/tool_search.rs`
- Modify: `crates/corpus/src/tools/tools/executor.rs` (`Tool` search metadata)
- Test: existing tests in `crates/corpus/src/tools/tools/search/`

- [ ] **Step 1: Add optional search text to the existing Tool contract**

Add a default method that derives searchable text from name and description;
allow tools to override it with synonyms. Do not add a second catalog or search
tool.

- [ ] **Step 2: Add ranking fixtures**

Cover agent delegation, file inspection, memory lookup, shell execution, and
session management. Assert the intended tool appears in the first three results
and hidden tools never appear.

- [ ] **Step 3: Run focused tests**

```bash
bash scripts/cargo-agent.sh test -p corpus tools::tools::search
```

Expected: PASS.

- [ ] **Step 4: Admission gate**

Installed deferred-tool cases must reduce directly exposed schemas without
lowering tool-selection success. Keep frequently used tools direct.

---

### Task 5: Add versioned JSONL session interchange over canonical SQLite

**Files:**
- Create: `crates/executive/src/application/session_interchange.rs`
- Modify: `crates/executive/src/application/session_service.rs`
- Modify: `crates/executive/src/host/daemon/handler/rpc/rpc_session.rs`
- Modify: `crates/interact/src/tui/registry.rs`
- Test: `crates/executive/tests/session_interchange.rs`

- [ ] **Step 1: Define the versioned envelope**

Each line must contain `schema_version`, `session_id`, `sequence`, `item_id`,
`parent_item_id`, `kind`, `created_at`, and `payload`. Export reads canonical
SQLite in sequence order; it never becomes a live persistence backend.

- [ ] **Step 2: Add round-trip and rejection tests**

Assert export/import/export semantic equality, fork parent preservation,
idempotent re-import, checksum/tamper rejection, unknown-version rejection, and
principal/workspace isolation.

- [ ] **Step 3: Add explicit RPC/TUI commands**

Add `/session-export <path>` and `/session-import <path>` only after path
authorization passes existing workspace/security policy. Import must append
through `SessionService`, not write SQLite tables directly.

- [ ] **Step 4: Admission gate**

Round-trip a real installed session and resume it after daemon restart. SQLite
must remain the sole canonical source after import.

---

### Task 6: Extract and tighten the default prompt by responsibility

**Files:**
- Create: `crates/cognit/prompts/default_system.md`
- Modify: `crates/cognit/src/config/mod.rs:575-580`
- Test: `crates/cognit/tests/facade_contract.rs`

- [ ] **Step 1: Move the current default verbatim**

Use `include_str!(\"../../prompts/default_system.md\")`. First preserve exact
text so extraction and behavior change are separate commits.

- [ ] **Step 2: Add bounded behavioral sections**

Add only: evidence-before-claims, inspect-before-edit, deterministic validation,
permission safety, concise failure reporting, and tool-discovery guidance.
Do not duplicate AGENTS.md, dynamic skills, model identity, or tool schemas.

- [ ] **Step 3: Add prompt contract tests**

Assert required sections exist, forbidden dynamic content is absent, and the
prompt remains below a fixed token/character budget recorded in the test.

- [ ] **Step 4: Admission gate**

The installed benchmark must improve search/validation behavior without
increasing turn-one prompt tokens by more than 5%.

---

### Task 7: Add compaction window identity only as diagnostic metadata

**Files:**
- Modify: canonical compaction item type in `crates/executive/src/application/`
- Modify: `crates/executive/src/host/daemon/session_manager.rs`
- Modify: session persistence migration beside the canonical session store
- Test: `crates/executive/tests/session_use_case_port.rs`

- [ ] **Step 1: Add optional IDs to persisted compaction records**

Store `window_id` and `previous_window_id` as UUIDs when compaction succeeds.
The first compacted window has no previous ID. Failed compaction creates no new
window.

- [ ] **Step 2: Add restart/fork tests**

Assert monotonic linkage across multiple compactions, persistence across daemon
restart, and correct parent linkage after session fork.

- [ ] **Step 3: Keep prompt injection disabled**

Expose IDs through status/debug/session export only. Add model-visible injection
later only if a benchmark proves it improves multi-compaction continuity.

- [ ] **Step 4: Admission gate**

Accept when diagnostics can identify the exact compaction chain without adding
tokens to ordinary prompts or changing canonical message semantics.

---

### Task 8: Validate and promote increments independently

**Files:** none unless a failing gate identifies a scoped defect

- [ ] Run formatting and narrow checks through `scripts/cargo-agent.sh`.
- [ ] Run the focused tests listed by the enabled track.
- [ ] Run `bash scripts/aletheon.sh deploy`.
- [ ] Verify release, installed, machine-daemon, and user-daemon SHA-256 equality.
- [ ] Verify stable systemd restart counters.
- [ ] Run the full installed benchmark three times per baseline and candidate.
- [ ] Enable only tracks whose individual admission gates pass; keep all others disabled.

## Completion definition

This plan is intentionally incremental. Each track is independently complete
only after its admission gate and installed-runtime acceptance pass. Finishing
one track does not authorize or imply completion of another.
