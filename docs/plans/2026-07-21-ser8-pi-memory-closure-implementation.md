# SER8 Pi + Memory Closure Implementation Plan

> **For agentic workers:** Use `workflow-feature` or `plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Run a bounded Pi coding attempt in an isolated fixture worktree, settle it into local and GBrain memory, and schedule the same audited path with a systemd user timer.

**Architecture:** Keep `pi-rpc` resident/shared and `pi-coder` worktree-isolated. Add explicit opt-in network propagation to both Pi sandboxes, use the existing Agent/Goal settlement spine for durable outcomes, and deploy only user-scoped configuration and timer assets.

**Tech Stack:** Rust 1.88, Tokio, Fabric sandbox policies, Pi 0.80.10 JSON/RPC protocols, SQLite/Mnemosyne, GBrain MCP, systemd user units.

---

### Task 1: Propagate explicit Pi network policy

**Files:**
- Modify: `crates/executive/src/impl/runtime/pi.rs`
- Modify: `crates/executive/src/impl/runtime/pi_rpc.rs`
- Test: `crates/executive/tests/pi_runtime.rs`
- Test: `crates/executive/tests/pi_rpc_runtime.rs`

- [ ] Add tests whose sandbox doubles capture `SandboxConfig.policy` and assert
  `restrict_network == false` only when `PiRuntimeConfig.network_enabled` is
  true; assert the default remains restricted.
- [ ] Remove the M4 blanket rejection. Resolve the strict sandbox profile for
  the trusted `WorkspacePolicy`, then set only `restrict_network` from the
  explicit Pi configuration. Preserve protected paths and namespace checks.
- [ ] Carry `network_enabled` in `ResolvedPiConfig` and use the same policy
  helper for the `pi-coder` and `pi-rpc` `wrap_argv` calls.
- [ ] Run:
  `bash scripts/cargo-agent.sh test -p executive --test pi_runtime --test pi_rpc_runtime`
  and expect all selected tests to pass.
- [ ] Commit the runtime and test changes with a conventional subject and a
  body describing opt-in behavior and fail-closed defaults.

### Task 2: Enable bounded Goal/Agent routing

**Files:**
- Create: `agents/orchestrator-agent.md`
- Modify: `config/production.toml.example`
- Test: `crates/executive/tests/agent_profile_bootstrap.rs`

- [ ] Add a profile fixture proving the main profile exposes only the existing
  explicit Agent control tools required to select `pi-rpc` or `pi-coder` and
  does not acquire unrestricted host privileges.
- [ ] Add the reviewed profile and production example routing with finite
  elapsed time, steps, output bytes, and tool allowlists.
- [ ] Run the narrow profile/bootstrap test through `scripts/cargo-agent.sh`.
- [ ] Install the profile into `~/.local/state/aletheon/agents/`, update the
  private user config, restart `aletheon.service`, and verify both Pi runtime
  registration messages without printing environment values.
- [ ] Commit repository profile/config/test changes separately.

### Task 3: Prove isolated Pi settlement on a disposable fixture

**Files:**
- Create at runtime: `/tmp/aletheon-pi-fixture-<epoch>/`
- Create evidence: `/tmp/aletheon-goal-evidence/pi-fixture-<epoch>/`

- [ ] Generate and commit a tiny Rust repository whose test fails until one
  literal is changed; record its base commit and clean status.
- [ ] Submit a bounded `pi-coder` attempt through the real user daemon. Do not
  point the request at the Aletheon checkout.
- [ ] Assert the source fixture stays unchanged before review, the managed
  worktree contains exactly the expected file change, the diff hash matches
  the evidence, and the attempt has a terminal settlement.
- [ ] Exercise review/apply against the fixture, run its narrow test, and prove
  no Aletheon source file changed.
- [ ] Query Mnemosyne for the operation/goal outcome and save only redacted
  receipts, hashes, paths, and statuses.

### Task 4: Prove GBrain projection and recovery

**Files:**
- Modify if production routing is missing: the narrow Agent settlement-to-memory adapter identified by the Task 3 trace
- Test: corresponding executive or Mnemosyne contract test
- Create evidence: `/tmp/aletheon-goal-evidence/gbrain-recovery-<epoch>/`

- [ ] First trace Task 3's terminal event to local Mnemosyne and GBrain; do not
  infer a missing producer from type declarations alone.
- [ ] If no production projection exists, write a failing test for one terminal
  Agent settlement becoming an `ExperienceEvent::GoalOutcome`, then add the
  minimal adapter using the existing composite memory service.
- [ ] Verify a canary outcome can be searched through authenticated GBrain MCP.
- [ ] Stop only the GBrain service/process, settle a second fixture outcome,
  assert one bounded spool record, restart GBrain, and poll until it drains and
  the canary appears exactly once.
- [ ] Run the narrow memory/GBrain tests through `scripts/cargo-agent.sh` and
  commit any required repository change separately.

### Task 5: Deploy bounded user timer

**Files:**
- Create: `scripts/aletheon-pi-scheduled-task.sh`
- Create: `deploy/systemd/user/aletheon-pi-closure.service`
- Create: `deploy/systemd/user/aletheon-pi-closure.timer`
- Test: `scripts/tests/aletheon-pi-scheduled-task.bats` if the repository's
  existing shell-test convention supports it; otherwise validate with
  `systemd-analyze verify` and an injected fake client.

- [ ] Implement a wrapper using `flock --nonblock`, `timeout`, a fixed canonical
  fixture/workspace input, private evidence files, and the existing user socket.
  It must reject symlinked/unsafe evidence paths and never source or echo secrets.
- [ ] Add a oneshot unit with `NoNewPrivileges`, a private umask, bounded runtime,
  and an environment file reference; add a persistent timer with randomized delay.
- [ ] Validate failure, timeout, and overlap paths, then install under
  `~/.config/systemd/user/`, daemon-reload, and manually start the service once.
- [ ] Enable the timer only after the manual run projects an outcome to GBrain.
- [ ] Commit wrapper, units, and deterministic tests.

### Task 6: Final acceptance record

**Files:**
- Create: `docs/deployment/ser8-acceptance-2026-07-21.md`
- Modify: `docs/plans/2026-07-21-aletheon-ser8-deployment.md` only to correct
  observed runtime IDs/statuses while preserving its deployment history

- [ ] Record source commit, deployed binary hash, unit properties, Pi identity,
  three real-TUI receipts, fixture diff/settlement hashes, Mnemosyne/GBrain
  evidence, recovery result, and timer properties with `path:line` anchors.
- [ ] Run formatting and the narrow tests from prior tasks; only the integration
  owner may run any workspace-wide verification.
- [ ] Inspect staged diffs, ensure no secrets or disposable paths entered tracked
  content, commit the acceptance record, and mark the goal complete only after
  every design verification gate has authoritative evidence.
