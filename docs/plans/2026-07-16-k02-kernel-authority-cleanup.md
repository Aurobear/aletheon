# K02 Kernel Authority Cleanup Implementation Plan

> **For agentic workers:** Execute this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make Kernel the only owner of Process, Operation, Space, supervision, mailbox lifecycle and hierarchical resource reservations, remove Agora from Kernel composition, and delete the duplicate Executive kernel.

**Architecture:** Extend the opaque `KernelRuntime` from K01 into the production composition boundary. Executive owns a separate `DomainPorts` object for Agora and other cognitive ports; all lifecycle callers use `Arc<KernelRuntime>` methods rather than tables. Kernel records resource ownership by root rollout → Process → Operation → capability and performs terminal cleanup through one lifecycle transaction. Unused Executive-kernel mechanisms are mapped to their authoritative replacements and deleted rather than retained as a second authority.

**Tech Stack:** Rust, Tokio, Fabric typed contracts, Kernel runtime, hierarchical reservations, architecture fitness scripts.

**Requirement anchors:** `docs/plans/2026-07-15-architecture-coupling-optimization-plan.md:249-283`, `:1008-1032`.

---

### Task 1: Delete the unused Executive-local kernel

**Files:**
- Delete: `crates/executive/src/impl/kernel/mod.rs`
- Delete: `crates/executive/src/impl/kernel/kernel.rs`
- Delete: `crates/executive/src/impl/kernel/supervisor.rs`
- Delete: `crates/executive/src/impl/kernel/global_pool.rs`
- Delete: `crates/executive/src/impl/kernel/ipc.rs`
- Modify: `crates/executive/src/impl/mod.rs`
- Create: `crates/executive/tests/no_duplicate_kernel.rs`

- [x] Add a static test proving no production symbol imports the duplicate module and recording replacement authorities: KernelRuntime/SupervisorTree, Kernel budget, Fabric mailbox, and scoped Agora Scratchpad.
- [x] Run the test during TDD and observe failure while the duplicate module exists.
- [x] Remove the module, all five implementation files, and the obsolete AgentKernel-only integration test. Update stale comments that still recommend `AgentKernel`.
- [x] Run the focused test and `cargo check -p executive --all-targets`; PASS.

### Task 2: Split KernelRuntime from Executive DomainPorts

**Files:**
- Create: `crates/executive/src/core/domain_ports.rs`
- Modify: `crates/executive/src/core/mod.rs`
- Modify: `crates/executive/src/core/core_systems.rs`
- Modify: `crates/executive/src/impl/daemon/handler/init.rs`
- Modify: `crates/kernel/src/runtime.rs`
- Delete: `crates/kernel/src/service/mod.rs`
- Modify: `crates/kernel/src/lib.rs`
- Test: `crates/executive/tests/kernel_domain_composition.rs`

- [ ] Test that Executive composes `Arc<KernelRuntime>` and `DomainPorts` separately, Kernel exposes no `AgoraOps`, and the old `ServicePorts` path does not compile/exist.
- [ ] Move AgoraRegistry into `DomainPorts`. Move clock, admission and logical mailbox access behind narrow `KernelRuntime` methods; keep concrete state fields private.
- [ ] Delete `ServicePorts`, its compatibility alias and Agora wiring.
- [ ] Run Kernel and Executive composition tests; expect PASS.

### Task 3: Migrate production lifecycle consumers

**Files:**
- Modify: `crates/executive/src/service/turn_coordinator.rs`
- Modify: `crates/executive/src/service/turn_service.rs`
- Modify: `crates/executive/src/service/turn_pipeline.rs`
- Modify: `crates/executive/src/service/daemon_turn/orchestrator.rs`
- Modify: `crates/executive/src/service/daemon_turn/lifecycle.rs`
- Modify: `crates/executive/src/service/daemon_turn/post_phases.rs`
- Modify: `crates/executive/src/service/exec_session.rs`
- Modify: `crates/executive/src/service/session_service.rs`
- Modify: `crates/executive/src/core/sub_agent.rs`
- Modify: `crates/executive/src/impl/approval/apply_coordinator.rs`
- Modify: `crates/executive/src/impl/goal/coordinator.rs`
- Test: `crates/executive/tests/kernel_runtime_production_paths.rs`

- [ ] Add a static/runtime test listing every production Process/Operation/Space mutation and require it to pass through `KernelRuntime`.
- [ ] Replace direct table fields, constructors and method calls with the shared opaque runtime. Replace direct Space manager mutation with typed runtime methods.
- [ ] Keep operation scopes as cancellation-token helpers only; they must not become a second lifecycle registry.
- [ ] Require the architecture scan to reject new direct table imports outside Kernel.

### Task 4: Hierarchical budget ownership

**Files:**
- Modify: `crates/fabric/src/types/admission.rs`
- Modify: `crates/kernel/src/admission/budget.rs`
- Modify: `crates/kernel/src/runtime.rs`
- Test: `crates/kernel/tests/hierarchical_budget.rs`

- [ ] Define versioned root/process/operation/capability budget scope IDs and parent-bound reservation receipts.
- [ ] Test that children cannot exceed parents, sibling reservations contend atomically, settlement charges each ancestor exactly once, revocation restores the hierarchy, and terminal cleanup releases all descendant holds idempotently.
- [ ] Replace principal-only accounting with the hierarchy while retaining a bounded compatibility adapter only for admission callers during the same commit.
- [ ] Bind Agent, Turn and capability reservations to their Process/Operation/permit identities. Cognit iteration/tool counts remain local non-monetary guards.

### Task 5: Atomic terminal cleanup and supervision

**Files:**
- Modify: `crates/kernel/src/runtime.rs`
- Modify: `crates/kernel/src/process/table.rs`
- Modify: `crates/kernel/src/operation/table.rs`
- Modify: `crates/kernel/src/supervision/tree.rs`
- Test: `crates/kernel/tests/terminal_cleanup.rs`
- Test: `crates/executive/tests/supervision.rs`

- [ ] Introduce one terminal API that validates the state edge, cancels descendant Operations, releases Space, mailbox, leases and budget holds, publishes the terminal snapshot, then applies the supervision decision.
- [ ] Test every failure injection boundary and prove retry is idempotent without leaked resources or duplicate restart.
- [ ] Move restart execution into KernelRuntime using retained Process spawn metadata; callers receive a typed outcome rather than replaying `RestartDecision` branches.
- [ ] Remove public `mark_exit`, table mutation methods and caller-side supervisor decision handling after migration.

### Task 6: Deletion gates and validation

**Files:**
- Modify: `scripts/architecture-check.sh`
- Modify: `config/architecture-allowlist.txt`
- Modify: `config/architecture-path-inventory.txt`
- Modify: `docs/plans/2026-07-15-executable-plan-decomposition-design.md`
- Modify: `docs/plans/2026-07-16-original-plan-coverage-matrix.md`

- [ ] Prove `rg -n 'ServicePorts|ProcessTable|OperationTable|InMemorySpaceManager|executive::.*kernel' crates/executive/src crates/bin/src` has no production lifecycle authority hits.
- [ ] Prove Kernel has no Agora/domain dependency and only Executive composes `DomainPorts` with `KernelRuntime`.
- [ ] Run format, clippy, focused tests, `cargo test --workspace`, and all architecture checks.
- [ ] Mark K02 done only when every direct compatibility surface and Executive-local kernel file is deleted and terminal resource cleanup is deterministic.

**Deletion gate:** K02 is not complete while `ServicePorts`, a public lifecycle table mutation API, `executive/src/impl/kernel/`, caller-side restart decision replay, principal-only monetary accounting, or Kernel-owned Agora wiring remains.
