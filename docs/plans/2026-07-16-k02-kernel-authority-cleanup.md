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

- [x] Test that Executive composes `Arc<KernelRuntime>` and `DomainPorts` separately, Kernel exposes no `AgoraOps`, and the old `ServicePorts` path does not compile/exist.
- [x] Move AgoraRegistry into `DomainPorts`. Move clock, admission and logical mailbox access behind narrow `KernelRuntime` methods; keep concrete state fields private.
- [x] Delete `ServicePorts`, its compatibility alias and Agora wiring.
- [x] Run Kernel and Executive composition tests and workspace all-target check; PASS.

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

- [x] Add a static/runtime test listing every production Process/Operation/Space mutation and require it to pass through `KernelRuntime`.
- [x] Replace direct table fields, constructors and method calls with the shared opaque runtime. Replace direct Space manager mutation with typed runtime methods.
- [x] Keep operation scopes as cancellation-token helpers only; they do not become a second lifecycle registry.
- [x] Add an executable production-source scan rejecting direct table imports outside Kernel; permanent script integration remains in Task 9.

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

### Task 6: Canonical lifecycle identities and operation kinds

**Files:**
- Modify: `crates/fabric/src/types/process.rs`
- Modify: `crates/fabric/src/types/operation.rs`
- Modify: `crates/kernel/src/runtime.rs`
- Modify: `crates/executive/src/core/sub_agent.rs`
- Test: `crates/kernel/tests/lifecycle_identity.rs`
- Test: `crates/executive/tests/kernel_runtime_production_paths.rs`

- [ ] Inventory every production conversion among `ProcessId`, `AgentId`, provider/session identifiers and OS PIDs. Add a failing architecture test for untyped string/UUID conversions outside the owning adapter.
- [ ] Define typed, versioned identity bindings owned by KernelRuntime: a logical Agent binds to one live Process generation, while an optional OS PID is metadata rather than lifecycle authority.
- [ ] Replace ad-hoc Operation kind strings with the canonical `OperationKind` enum and reject unknown persisted discriminants explicitly instead of silently mapping them.
- [ ] Test stable serialization, process-generation replacement, stale Agent/Process bindings, optional PID reuse, and all production Operation kinds.

### Task 7: Canonical capability lifecycle and injected clocks

**Files:**
- Modify: `crates/executive/src/impl/capability_invoker.rs`
- Modify: `crates/executive/src/core/core_systems.rs`
- Modify: `crates/cognit/src/**/*.rs` (only files found by the concrete-clock production scan)
- Modify: `crates/dasein/src/**/*.rs` (only files found by the concrete-clock production scan)
- Modify: `crates/agora/src/**/*.rs` (only files found by the concrete-clock production scan)
- Modify: `scripts/architecture-check.sh`
- Test: `crates/executive/tests/capability_path.rs`
- Test: `crates/executive/tests/kernel_domain_composition.rs`

- [ ] Add a failing static test proving every production capability invocation follows `Executive governance -> DefaultCapabilityInvoker -> Kernel admission/reservation -> Corpus execution -> Kernel settlement`; reject direct admit/execute/settle paths.
- [ ] Make `DefaultCapabilityInvoker` the single inner production invoker and retain policy review/approval only in its governed Executive wrapper.
- [ ] Scan domain production sources for concrete `SystemClock` construction/imports. Change affected constructors to accept `Arc<dyn Clock>`; concrete clock construction is allowed only in the composition root and tests.
- [ ] Add architecture gates for both invariants and run the focused Executive/domain tests.

### Task 8: Real lifecycle failure scenarios

**Files:**
- Create: `crates/executive/tests/kernel_lifecycle_scenarios.rs`
- Modify: `crates/kernel/tests/terminal_cleanup.rs`
- Modify: `crates/executive/tests/supervision.rs`

- [ ] Build a deterministic harness with `TestClock`, injectable cleanup failures and observable reservation/mailbox/Space counts.
- [ ] Verify a successful Turn leaves no Operation, capability hold, lease or mailbox residue while its owning Process remains healthy.
- [ ] Verify user cancellation and tool failure cancel descendants and settle/release every resource exactly once.
- [ ] Verify a sub-agent crash performs cleanup before one policy-authorized restart, and retrying the same terminal event neither leaks resources nor starts a duplicate generation.
- [ ] Verify daemon reconstruction from persisted lifecycle metadata cannot revive terminal Operations or stale identity bindings.
- [ ] Verify concurrent sibling reservations contend atomically under a shared parent and never exceed the root rollout reservation.

### Task 9: Deletion gates and validation

**Files:**
- Modify: `scripts/architecture-check.sh`
- Modify: `config/architecture-allowlist.txt`
- Modify: `config/architecture-path-inventory.txt`
- Modify: `docs/plans/2026-07-15-executable-plan-decomposition-design.md`
- Modify: `docs/plans/2026-07-16-original-plan-coverage-matrix.md`

- [ ] Prove `rg -n 'ServicePorts|ProcessTable|OperationTable|InMemorySpaceManager|executive::.*kernel' crates/executive/src crates/bin/src` has no production lifecycle authority hits.
- [ ] Prove Kernel has no Agora/domain dependency and only Executive composes `DomainPorts` with `KernelRuntime`.
- [ ] Run format, clippy, focused tests, `cargo test --workspace`, and all architecture checks.
- [ ] Run the real lifecycle scenario suite in addition to focused unit/integration tests.
- [ ] Mark K02 done only when every direct compatibility surface and Executive-local kernel file is deleted, identity/kind mappings are canonical, the capability path and clock boundaries are singular, and terminal resource cleanup is deterministic.

**Deletion gate:** K02 is not complete while `ServicePorts`, a public lifecycle table mutation API, `executive/src/impl/kernel/`, caller-side restart decision replay, principal-only monetary accounting, Kernel-owned Agora wiring, ad-hoc lifecycle identity/kind conversion, a second production capability path, or domain-owned concrete clock construction remains.
