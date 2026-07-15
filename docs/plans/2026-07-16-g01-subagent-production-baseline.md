# G01 SubAgent Production Baseline Implementation Plan

> **For agentic workers:** Use `workflow-feature` or `plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Freeze the current production AgentTool-to-SubAgentSpawner vertical slice and make its known Harness gaps executable before control-service migration.

**Architecture:** Invoke the real Corpus `AgentTool` with an Executive closure backed by shared Kernel tables, then inspect the resulting Process, Operation, mailbox, allow-list and terminal mappings. Known cancellation/profile enforcement gaps remain explicit ignored targets for G04.

**Tech Stack:** Rust, Tokio, Corpus Tool API, Executive SubAgentSpawner, Kernel Process/Operation tables.

**Prerequisites:** S02 unified turn lifecycle.

**Source requirements:** `docs/plans/2026-07-15-subagent-unified-harness-plan.md:402-426`.

---

## Current-code anchors

- AgentTool owns a callback and mirrors Agent definitions at `crates/corpus/src/tools/tools/agent_tool.rs:15-52`.
- The callback receives only system prompt, user prompt and allowed tools at `agent_tool.rs:128-151`; model/max-iterations are dropped.
- Production builds an inline 20-step loop at `crates/executive/src/impl/daemon/handler/init.rs:1154-1331`.
- SubAgentSpawner creates Process, Operation and mailbox records at `crates/executive/src/core/sub_agent.rs:288-378`.

## Invariants and non-goals

- The actual AgentTool path creates one Process, one SubAgent Operation and one mailbox before work.
- Only profile-allowed tool names reach the execution closure.
- Success and failure produce structured ToolResult status.
- Default tests stay green; cancellation and profile max-iteration targets remain ignored until G04.
- G01 does not redesign contracts or repair the inline reasoning loop.

## File map

- Create: `crates/executive/tests/subagent_production_baseline.rs`
- Modify only for a proven regression: `crates/executive/src/core/sub_agent.rs`
- Modify only for a proven regression: `crates/corpus/src/tools/tools/agent_tool.rs`

### Task 1: Exercise the actual AgentTool vertical slice

- [ ] Construct shared TestClock ProcessTable/OperationTable and SubAgentSpawner.
- [ ] Construct one real AgentTool definition with a two-tool allow-list.
- [ ] Make its execution closure spawn tracked with a parent, transition Running then Completed, and capture ProcessSnapshot/mailbox evidence.
- [ ] Execute through the Tool trait and assert output plus exact allowed tools.

Run: `cargo test -p executive --test subagent_production_baseline successful_agent_tool_vertical_slice`

Expected: PASS with one Process, active SubAgent Operation and registered mailbox observed before cleanup.

### Task 2: Freeze failure and allow-list behavior

- [ ] Return an execution error and assert AgentTool produces `is_error=true` without leaking an untracked process.
- [ ] Assert an unknown Agent type never invokes the closure.
- [ ] Assert the closure receives exactly the configured tools, not registry defaults.

Run: `cargo test -p executive --test subagent_production_baseline error_`

Expected: PASS.

### Task 3: Add G04 target tests

- [ ] Add ignored cancellation-during-runtime target.
- [ ] Add ignored profile model/max-iterations target.
- [ ] Include real fixture bodies so G04 only removes `#[ignore]`, not rewrites acceptance.

Run: `cargo test -p executive --test subagent_production_baseline -- --list`

Expected: active baseline and both G04 target names are listed.

### Task 4: Verify and commit

```bash
cargo fmt --all -- --check
cargo clippy -p executive --test subagent_production_baseline -- -D warnings
cargo test -p executive --test subagent_production_baseline
cargo test --workspace
bash tests/architecture_check.sh
bash scripts/architecture-check.sh
```

Commit subject: `test(executive): lock subagent production baseline`

## Compatibility deletion gate

G04 must activate both ignored targets and delete the inline loop only after NativeCognitRuntime proves parity. G05 then deletes `ExecuteSubAgentFn` when AgentTool becomes a thin `AgentControlPort` client.

## Completion evidence

- [ ] real AgentTool produces Process/Operation/mailbox evidence;
- [ ] allow-list and success/failure mappings are frozen;
- [ ] unknown profiles cannot execute;
- [ ] G04 cancellation/profile targets exist and are ignored explicitly;
- [ ] workspace and architecture checks pass.
