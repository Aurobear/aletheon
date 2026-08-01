# Aletheon Memory Agent and Scoring Implementation Plan

> **For agentic workers:** Execute this plan in order. The daemon remains the only authority and database writer; the independent Memory Agent process is a bounded scheduler/client, not a second memory kernel.

**Goal:** Add a restart-safe Aletheon-managed Memory Agent service that scrubs and evaluates durable observations, applies a versioned host-computed score, and advances authoritative lifecycle receipts through bounded leases.

**Architecture:** Mnemosyne extends its existing intake ledger with bounded queue accounting, lease ownership, and atomic claim/complete operations. Executive owns a versioned deterministic policy and a maintenance controller; optional AgentRuntime output is only a structured proposal and never gains database or supplemental credentials. A separate `aletheon memory-agent serve` process connects only to the official user socket, negotiates a maintenance capability, and repeatedly requests bounded work. Systemd and deployment verification install and validate the same release binary for daemon and Memory Agent.

**Tech Stack:** Rust 1.85, rusqlite WAL/FULL, Fabric versioned JSON-RPC, Tokio, clap, systemd user units, existing `AgentControlPort`, existing Fabric data-governance scrubber.

---

## Requirement and code anchors

- Independent service ownership and no direct DB access: `docs/plans/2026-08-01-unified-memory-gateway-design.md:section 6.1` (`lines 353-369`).
- Aletheon-owned runtime selection and host verification: `docs/plans/2026-08-01-unified-memory-gateway-design.md:section 6.2` (`lines 371-388`).
- Durable leases and bounded run budgets: `docs/plans/2026-08-01-unified-memory-gateway-design.md:section 6.3` (`lines 390-401`).
- Hard gates, axes, thresholds, and policy versioning: `docs/plans/2026-08-01-unified-memory-gateway-design.md:section 6.4` (`lines 403-437`).
- The current lifecycle ledger and legal revision model are at `crates/mnemosyne/src/intake.rs:121-143`, `crates/mnemosyne/src/intake.rs:167-264`, and `crates/mnemosyne/src/intake.rs:410-493`.
- The existing runtime authority is `AgentControlService` at `crates/executive/src/application/agent_control/mod.rs:136-160`; composition currently erases its port after tool registration at `crates/executive/src/host/daemon/bootstrap/services.rs:171-242`.
- The unified CLI currently has no Memory Agent command at `crates/aletheon/src/main.rs:113-196`.
- Reviewed user-unit installation is owned by `scripts/libexec/aletheon/install-systemd.sh:50-90` and `scripts/lib/aletheon/install.sh:93-116`.

### Task 1: Scrub before durable intake and persist scrub evidence

**Files:**
- Modify: `crates/mnemosyne/src/intake.rs`
- Modify: `crates/executive/src/application/memory_gateway.rs`
- Modify: `crates/executive/tests/memory_gateway.rs`
- Modify: `crates/mnemosyne/tests/memory_intake_ledger.rs`

- [ ] Add failing tests proving raw provider keys, authorization headers, email/PII, and private-key blocks never appear in the intake database or observation JSON.
- [ ] Apply `fabric::data_governance::scrub_for_projection` at the Executive boundary before `MemoryIntakeLedger::observe`; preserve `scrub_policy_version` and `scrub_redactions` as typed host fields.
- [ ] Raise, never lower, the effective sensitivity when scrub results classify content as restricted.
- [ ] Bound intake rows and aggregate payload bytes before insert; exact idempotent duplicates remain readable at capacity, while new inserts fail with a typed capacity reason.
- [ ] Run `bash scripts/cargo-agent.sh test -p executive --test memory_gateway` and `bash scripts/cargo-agent.sh test -p mnemosyne --test memory_intake_ledger`.

### Task 2: Add durable maintenance leases and atomic lifecycle settlement

**Files:**
- Modify: `crates/mnemosyne/src/intake.rs`
- Modify: `crates/mnemosyne/src/lib.rs`
- Create: `crates/mnemosyne/tests/memory_maintenance_lease.rs`

- [ ] Add failing tests for one-owner claim, competing claim rejection, lease expiry recovery, restart recovery, bounded batch size, and idempotent terminal settlement.
- [ ] Persist leases in a dedicated table keyed by `(phase, scope, watermark)` and durable intake ID; never rely on process memory for ownership.
- [ ] Claim `observed` work and create the `evaluating` revision in one immediate transaction. Reclaim an expired `evaluating` item without writing a duplicate lifecycle revision.
- [ ] Require owner token, current revision, and unexpired lease when settling; delete the lease only in the same transaction that commits the terminal/candidate receipt.
- [ ] Expose a bounded queue/oldest-age/expired-lease status snapshot without returning observation content.
- [ ] Run `bash scripts/cargo-agent.sh test -p mnemosyne --test memory_maintenance_lease`.

### Task 3: Implement a versioned host scoring policy

**Files:**
- Create: `crates/executive/src/composition/config/memory_policy.rs`
- Modify: `crates/executive/src/composition/config/mod.rs`
- Modify: `config/default.toml`
- Modify: `config/schema/aletheon-config.schema.json`
- Create: `crates/executive/src/application/memory_policy.rs`
- Create: `crates/executive/tests/memory_policy.rs`

- [ ] Add failing table tests for every hard gate, every axis boundary, totals clamped to `0..=100`, and decisions at 54/55/74/75/79/80.
- [ ] Define a validated `MemoryPolicyConfig` containing policy version, axis maxima/penalties, promote/candidate/remote thresholds, lease/run limits, and allowed projection kinds. Reject duplicate/empty policy versions and invalid sums/ranges.
- [ ] Compute all axis totals in host code from typed observation, scrub evidence, source references, lifecycle evidence, novelty/conflict facts, and verified binding state. Model text cannot set scope, authority, score, threshold, or final decision.
- [ ] Return `MemoryScorecardV1`, stable reason codes, and a typed decision. `ApprovedCore` is never an automatic output.
- [ ] Run `bash scripts/cargo-agent.sh test -p executive --test memory_policy` and `bash scripts/cargo-agent.sh test -p executive composition::config --lib`.

### Task 4: Add the daemon maintenance controller and optional semantic proposal

**Files:**
- Create: `crates/fabric/src/protocol/memory_maintenance.rs`
- Modify: `crates/fabric/src/protocol/mod.rs`
- Modify: `crates/fabric/src/protocol/client.rs`
- Modify: `crates/fabric/src/types/agent_control.rs`
- Modify: `crates/runtime/src/manifest.rs`
- Create: `crates/executive/src/application/memory_maintenance.rs`
- Modify: `crates/executive/src/application/mod.rs`
- Modify: `crates/executive/src/host/daemon/bootstrap/services.rs`
- Modify: `crates/executive/src/host/daemon/bootstrap/request.rs`
- Modify: `crates/executive/src/host/daemon/handler/ports.rs`
- Modify: `crates/executive/src/host/daemon/handler/mod.rs`
- Modify: `crates/executive/src/host/daemon/server.rs`
- Modify: `crates/executive/src/host/daemon/protocol.rs`
- Create: `crates/executive/tests/memory_maintenance.rs`

- [ ] Add additive `memory_maintenance_v1` capability plus typed status/run/task/proposal/receipt contracts with strict byte/item/deadline/provider-round/retry/tool-call/projected-write bounds.
- [ ] Add `MemoryProposal` to runtime capability manifests and the Fabric mapping; declare it only on adapters that can return bounded structured output.
- [ ] Preserve `Arc<dyn AgentControlPort>` from daemon composition and inject it into `MemoryMaintenanceController` without exposing a DB handle or supplemental credential to a runtime.
- [ ] Run deterministic gates/scoring first. For semantic duplicate/conflict work only, select by manifest, health history, effective policy, and budget; spawn with no tools and no writable workspace, wait for the authoritative terminal snapshot, parse a deny-unknown-fields proposal, and recompute the decision in host code.
- [ ] If no runtime matches or provider execution fails, leave semantic work pending with a durable reason and continue deterministic maintenance.
- [ ] Return only committed lifecycle/maintenance receipts. An async spawn result is never terminal success.
- [ ] Run `bash scripts/cargo-agent.sh test -p fabric --test protocol_schema` and `bash scripts/cargo-agent.sh test -p executive --test memory_maintenance`.

### Task 5: Authenticate and implement the independent Memory Agent client

**Files:**
- Modify: `crates/executive/src/host/daemon/server.rs`
- Modify: `crates/executive/src/host/daemon/protocol.rs`
- Create: `crates/interact/src/memory_client.rs`
- Modify: `crates/interact/src/lib.rs`
- Create: `crates/aletheon/src/memory_agent.rs`
- Modify: `crates/aletheon/src/main.rs`
- Create: `crates/aletheon/tests/memory_agent_client.rs`

- [ ] Capture peer PID from `SO_PEERCRED` and derive an `OfficialMemoryAgent` connection role only when `/proc/<pid>/exe` matches the running release executable and the process command is the typed `memory-agent serve` subcommand. JSON fields cannot mint this role.
- [ ] Intersect `memory_maintenance_v1` to false for ordinary Claude/Codex/TUI connections; reject maintenance requests again at dispatch as defense in depth.
- [ ] Implement a reusable versioned socket client that performs initialize/initialized, verifies the negotiated capability, sends bounded run/status requests, validates typed responses, and reconnects with capped exponential backoff.
- [ ] Add `aletheon memory-agent serve --official-user-socket` and a one-shot `run --max-items N --dry-run` diagnostic. Neither command opens Aletheon or GBrain persistence directly.
- [ ] On shutdown/incompatible protocol, stop claiming new work; already claimed work is reported only after a terminal daemon receipt.
- [ ] Run `bash scripts/cargo-agent.sh test -p aletheon --test memory_agent_client` and focused protocol/server unit tests.

### Task 6: Install, supervise, and verify the Memory Agent service

**Files:**
- Create: `config/aletheon-memory-agent.user.service`
- Modify: `scripts/libexec/aletheon/install-systemd.sh`
- Modify: `scripts/lib/aletheon/install.sh`
- Modify: `scripts/lib/aletheon/service.sh`
- Modify: `scripts/lib/aletheon/runtime_gate.sh`
- Modify: `scripts/lib/aletheon/verify.sh`
- Modify: `scripts/libexec/aletheon/verify/systemd.sh`
- Modify: `docs/deployment/systemd.md`
- Modify: `tests/suites/deployment/systemd_runtime_boundary.sh`

- [ ] Install the reviewed unit beside `aletheon.service`; order it after `aletheon.socket`, restart on failure with bounded rate limits, and harden it with no writable workspace or credential environment.
- [ ] For system deployment use `/usr/bin/aletheon`; for explicit rootless user deployment rewrite both daemon and Memory Agent units to the same concrete user binary.
- [ ] Start/restart Memory Agent only after daemon protocol smoke passes. Verify active state, stable restart counter, and running executable digest in the same gate as core/user daemons.
- [ ] Extend deployment tests to fail on stale binary paths, missing unit hardening, omitted capability smoke, or digest mismatch.
- [ ] Run `bash scripts/aletheon.sh test deployment`.

### Task 7: Verify and commit the stage

**Files:**
- Verify all files above.

- [ ] Run every focused command from Tasks 1-6.
- [ ] Run `bash scripts/cargo-agent.sh check -p executive -p interact -p aletheon`.
- [ ] Run `bash scripts/cargo-agent.sh fmt --all -- --check`.
- [ ] Run `git diff --check`, inspect the complete staged diff, and commit policy/ledger, controller/protocol, and service/deployment as separate reviewable stages.
- [ ] Do not claim final installed acceptance until supplemental binding and client migration are complete; the final wave still requires `sudo bash scripts/aletheon.sh deploy`, digest equality for all running processes, stable counters, and a real official-socket LLM request.

## Acceptance

- No raw secret-bearing observation reaches durable intake; scrub evidence and effective sensitivity are host-generated.
- Claims and terminal receipts survive daemon and Memory Agent restarts without double processing.
- Scores are reproducible from a versioned config and typed evidence; model output cannot raise authority or bypass a gate.
- Runtime selection is capability/health/budget driven, and unavailable semantic runtimes degrade to pending rather than blocking local memory.
- Ordinary clients cannot negotiate or dispatch maintenance operations.
- The independent process uses only the official socket and the installed Aletheon binary; it never opens Mnemosyne/GBrain storage or receives write credentials.
