# Private Composition Root and Split Bootstrap Implementation Plan

> **For agentic workers:** Use `workflow-feature` or `plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Delete the public `CoreSystems` god container, make daemon bootstrap the only code that knows all concrete implementations, and split the 2,077-line handler constructor into bounded subsystem and lifecycle builders.

**Architecture:** Production adapters receive explicit resource records containing only the handles they use. A private `DaemonComposition` exists only while bootstrap assembles the runtime and is consumed into `RequestHandler`; long-lived turn, capability, projection, and transport services retain narrow ports rather than a shared container. Bootstrap modules separate pure construction, fallible storage/external initialization, and task startup so partial initialization has an explicit owner and shutdown path.

**Tech Stack:** Rust, Tokio, async-trait, existing Fabric contracts, KernelRuntime, rusqlite-backed stores, static architecture tests

**Prerequisites:** X01 request ports complete; K01/K02 opaque Kernel runtime complete.

**Source requirements:** `docs/plans/2026-07-15-architecture-coupling-optimization-plan.md:1034-1063`

---

## Current-code anchors

- The remaining container is public and exposes every concrete group at `crates/executive/src/core/core_systems.rs:31-68`.
- Turn construction still accepts the container at `crates/executive/src/service/turn_pipeline.rs:60-101`.
- Production synchronous turn adapters still accept and retain it at `crates/executive/src/service/turn_runtime_ports.rs:106-149` and `:505-526`.
- The daemon orchestrator retains it at `crates/executive/src/service/daemon_turn/orchestrator.rs:27-49` and uses it for session/lifecycle state at `crates/executive/src/service/daemon_turn/execute.rs:62-69` and `crates/executive/src/service/daemon_turn/lifecycle.rs:13-45`.
- Projection construction still accepts it at `crates/executive/src/service/post_turn_projection.rs:56-78`.
- External capability execution retains a `Weak<CoreSystems>` at `crates/executive/src/impl/daemon/handler/tool_executor.rs:298-365`.
- Bootstrap builds the container at `crates/executive/src/impl/daemon/handler/init.rs:1060-1082`; the file is 2,077 lines.

## Invariants and non-goals

- Keep one `KernelRuntime`, one governed capability path, one canonical `SessionService`, and one `TurnCoordinator`.
- Do not move concrete stores or mutexes into `RequestHandler`, RPC adapters, Fabric, or Kernel.
- Do not change JSON-RPC method names, result shapes, Google authorization semantics, Goal recovery, Telegram ownership checks, or daemon/exec turn ordering.
- Do not create a replacement global service locator. `DaemonComposition` is private, non-cloneable, bootstrap-only, and cannot appear in a long-lived service field.
- Do not perform the optional physical crate split from source Phase 8.

### Task 1: Lock the composition-root deletion gate

**Files:**
- Create: `crates/executive/tests/private_composition_root.rs`
- Modify: `scripts/architecture-check.sh`

- [ ] Add a failing static test that scans production Executive sources and permits `DaemonComposition` only below `impl/daemon/bootstrap/`, rejects `CoreSystems`, rejects `.subsystems`, and rejects a `bootstrap/mod.rs` longer than 700 lines.

```rust
#[test]
fn god_container_is_deleted_and_composition_is_bootstrap_private() {
    let production = executive_production_sources();
    assert_no_symbol(&production, "CoreSystems");
    assert_no_field(&production, ".subsystems");
    assert_symbol_confined(&production, "DaemonComposition", "src/impl/daemon/bootstrap/");
    assert!(line_count("src/impl/daemon/bootstrap/mod.rs") <= 700);
}
```

- [ ] Extend `scripts/architecture-check.sh` with the same production-only deletion gate so unit-test fixtures cannot mask a regression.
- [ ] Run `cargo test -p executive --test private_composition_root`; expect FAIL because `CoreSystems` remains in the paths listed above.
- [ ] Commit the red gate with subject `test(architecture): lock private composition boundary` and a body describing the forbidden escape paths.

### Task 2: Make capability execution depend on explicit resources

**Files:**
- Modify: `crates/executive/src/impl/daemon/handler/tool_executor.rs`
- Modify: `crates/executive/src/impl/daemon/handler/init.rs`
- Modify: `crates/executive/tests/capability_path.rs`

- [ ] Add a test proving external/provider capability callers and turn callers share one `CapabilityService`, and that `tool_executor.rs` contains neither `CoreSystems` nor `Weak<...>`.
- [ ] Introduce the exact resource record below and change `TurnToolExecutor::new` to receive it.

```rust
#[derive(Clone)]
pub(crate) struct CapabilityResources {
    pub kernel: Arc<KernelRuntime>,
    pub tools: ToolRegistryHandle,
    pub runner: Arc<Mutex<ToolRunnerWithGuard>>,
    pub hooks: HookRegistryHandle,
    pub storm: Arc<Mutex<StormBreaker>>,
    pub memory_queue: Arc<Mutex<Vec<String>>>,
    pub approvals: Arc<Mutex<HashMap<String, bool>>>,
    pub perf: Arc<PerfCounter>,
    pub self_field: Arc<Mutex<SelfField>>,
}
```

- [ ] Rename `CoreCapabilityService` to `ProductionCapabilityService`, retain `CapabilityResources` directly, and use `resources.kernel` for transient lifecycle creation and settlement.
- [ ] Construct one `Arc<dyn CapabilityService>` during bootstrap and pass clones to `HandlerPorts::transport` and provider workers; never reconstruct it inside a request or turn.
- [ ] Run `cargo test -p executive --test capability_path --test governed_capability_path --test capability_invoker`; expect PASS.
- [ ] Commit with subject `refactor(capability): inject explicit execution resources`.

### Task 3: Remove the container from post-turn projection

**Files:**
- Modify: `crates/executive/src/service/post_turn_projection.rs`
- Modify: `crates/executive/src/impl/daemon/handler/init.rs`
- Modify: `crates/executive/tests/turn_use_case_ports.rs`

- [ ] Extend the projection outage test to assert projection construction has no `CoreSystems` input and terminal settlement remains earlier than projection spawn.
- [ ] Add and consume this resource record:

```rust
pub struct PostTurnProjectionResources {
    pub hooks: HookRegistryHandle,
    pub memory: Arc<dyn MemoryService>,
    pub auto_memory: Arc<Mutex<AutoMemory>>,
    pub reflector: Reflector,
    pub episodic: Arc<Mutex<EpisodicMemory>>,
    pub clock: Arc<dyn Clock>,
    pub executive: Arc<Mutex<AletheonExecutive>>,
    pub evolution: Arc<MorphogenesisPipeline<DefaultMetaRuntime>>,
    pub agora: Arc<dyn AgoraOps>,
    pub recall: Arc<Mutex<RecallMemory>>,
}
```

- [ ] Change `ProductionPostTurnProjection::new` to accept only `PostTurnProjectionResources`; assemble the record once in bootstrap.
- [ ] Run `cargo test -p executive --test turn_use_case_ports --test turn_coordinator_lifecycle`; expect PASS.
- [ ] Commit with subject `refactor(projection): inject post-turn resources`.

### Task 4: Remove the container from synchronous turn runtime ports

**Files:**
- Modify: `crates/executive/src/service/turn_runtime_ports.rs`
- Modify: `crates/executive/src/impl/daemon/handler/tool_executor.rs`
- Modify: `crates/executive/src/impl/daemon/handler/init.rs`
- Modify: `crates/executive/tests/turn_use_case_ports.rs`
- Modify: `crates/executive/tests/turn_pipeline_order.rs`

- [ ] Add a static assertion that `turn_runtime_ports.rs` has no `CoreSystems`, `subsystems`, or direct production capability construction.
- [ ] Replace `TurnRuntimePorts::production(CoreSystems, ...)` with the exact boundary below; adapters may retain individual handles but not the aggregate record after construction.

```rust
pub struct TurnRuntimeResources {
    pub hooks: Arc<dyn TurnHookUseCases>,
    pub storm: Arc<dyn StormUseCases>,
    pub models: Arc<dyn TurnModelUseCases>,
    pub self_policy: Arc<dyn TurnSelfUseCases>,
    pub sessions: Arc<dyn TurnSessionUseCases>,
    pub configuration: Arc<dyn TurnConfigurationUseCases>,
    pub observability: Arc<dyn TurnObservabilityUseCases>,
    pub approvals: Arc<dyn TurnApprovalUseCases>,
    pub capabilities: Arc<dyn TurnCapabilityUseCases>,
}
```

- [ ] Construct concrete adapters in bootstrap from the same handles used by Task 2; `ProductionTurnCapability` receives `CapabilityResources`, not a container.
- [ ] Preserve blocking order `context -> model -> approval/capability -> terminal settlement`.
- [ ] Run `cargo test -p executive --test turn_use_case_ports --test turn_pipeline_order --test sandbox_first_fail_closed`; expect PASS.
- [ ] Commit with subject `refactor(turn): inject synchronous runtime resources`.

### Task 5: Make TurnPipeline fully composed

**Files:**
- Modify: `crates/executive/src/service/turn_pipeline.rs`
- Modify: `crates/executive/src/service/daemon_turn/orchestrator.rs`
- Modify: `crates/executive/src/impl/daemon/handler/init.rs`
- Modify: `crates/executive/tests/turn_service_equivalence.rs`

- [ ] Add a constructor test showing a pipeline can be built from narrow ports without a god container.
- [ ] Replace the constructor with this explicit input:

```rust
pub struct TurnPipelineResources {
    pub session_gateway: Arc<SessionGateway>,
    pub notify: Arc<Mutex<Option<mpsc::Sender<String>>>>,
    pub clock: Arc<dyn Clock>,
    pub agora: Arc<dyn AgoraOps>,
    pub kernel: Arc<KernelRuntime>,
    pub daemon_cancel: Option<CancellationToken>,
    pub context: Arc<ContextAssembler>,
    pub canonical_sessions: Arc<SessionService>,
    pub projection: Arc<dyn PostTurnProjection>,
    pub runtime: Arc<TurnRuntimePorts>,
}
```

- [ ] Make `TurnPipeline::new(resources)` assign fields only; all adapter construction moves to bootstrap.
- [ ] Run `cargo test -p executive --test context_assembler --test turn_pipeline_order --test turn_service_equivalence`; expect PASS, including all eight equivalence scenarios.
- [ ] Commit with subject `refactor(turn): compose pipeline from narrow ports`.

### Task 6: Remove the container from DaemonTurnOrchestrator

**Files:**
- Modify: `crates/executive/src/service/daemon_turn/orchestrator.rs`
- Modify: `crates/executive/src/service/daemon_turn/lifecycle.rs`
- Modify: `crates/executive/src/service/daemon_turn/execute.rs`
- Modify: `crates/executive/src/impl/daemon/handler/init.rs`
- Modify: `crates/executive/tests/kernel_lifecycle_scenarios.rs`
- Modify: `crates/executive/tests/session_lifecycle_commands.rs`

- [ ] Add cancellation/restart tests proving the new lifecycle resource handles still create one main process, rotate the same default session, and cancel exactly one active turn.
- [ ] Introduce `DaemonTurnResources` with only `kernel`, `clock`, `agora`, `default_session_id`, `main_agent_process_id`, `turn_token`, `pipeline`, `coordinator`, `session_service`, and `notify`.
- [ ] Delete `DaemonTurnOrchestrator.subsystems`, unused mirrored handler fields, and constructor-side store/pipeline creation. Bootstrap constructs the canonical store, coordinator, session service, and pipeline before the orchestrator.
- [ ] Change `execute.rs` to read `default_session_id`; change `lifecycle.rs` to use `main_agent_process_id` and `turn_token`.
- [ ] Run `cargo test -p executive --test kernel_lifecycle_scenarios --test session_lifecycle_commands --test turn_coordinator_lifecycle`; expect PASS.
- [ ] Commit with subject `refactor(daemon): narrow turn orchestrator state`.

### Task 7: Split bootstrap and consume a private composition root

**Files:**
- Create: `crates/executive/src/impl/daemon/bootstrap/mod.rs`
- Create: `crates/executive/src/impl/daemon/bootstrap/storage.rs`
- Create: `crates/executive/src/impl/daemon/bootstrap/google.rs`
- Create: `crates/executive/src/impl/daemon/bootstrap/runtime.rs`
- Create: `crates/executive/src/impl/daemon/bootstrap/channels.rs`
- Create: `crates/executive/src/impl/daemon/bootstrap/request.rs`
- Modify: `crates/executive/src/impl/daemon/mod.rs`
- Modify: `crates/executive/src/impl/daemon/handler/init.rs`
- Modify: `crates/executive/tests/private_composition_root.rs`

- [ ] Move storage and memory creation to `storage.rs`, Google repository/OAuth/sync construction to `google.rs`, runtime/domain adapter construction to `runtime.rs`, Telegram/worker startup to `channels.rs`, and HandlerPorts assembly to `request.rs`.
- [ ] Define the private, non-`Clone` transient type below in `bootstrap/mod.rs`:

```rust
pub(super) struct DaemonComposition {
    pub kernel: Arc<KernelRuntime>,
    pub domains: DomainPorts,
    pub request: Arc<HandlerPorts>,
    pub turn: Arc<DaemonTurnOrchestrator>,
    pub lifecycle: DaemonLifecycle,
}

impl DaemonComposition {
    pub(super) fn into_handler(self, active: Arc<AtomicUsize>) -> RequestHandler {
        RequestHandler::from_ports(self.request, active)
    }
}
```

- [ ] Keep `handler/init.rs` as a compatibility entry containing only `RequestHandler::new`, notification accessors, and a call to `bootstrap::build(config).await`; cap it at 250 lines.
- [ ] Ensure every spawned task handle is transferred into `DaemonLifecycle` before returning and every fallible earlier stage owns no detached task.
- [ ] Run `cargo test -p executive --test private_composition_root --test production_health --test google_sync_recovery --test telegram_restart_recovery`; expect PASS.
- [ ] Commit with subject `refactor(bootstrap): split daemon composition stages`.

### Task 8: Delete CoreSystems and close X02

**Files:**
- Delete: `crates/executive/src/core/core_systems.rs`
- Modify: `crates/executive/src/core/mod.rs`
- Modify: `config/architecture-allowlist.txt`
- Modify: `scripts/architecture-check.sh`
- Modify: `docs/plans/2026-07-15-executable-plan-decomposition-design.md`
- Modify: `docs/plans/2026-07-16-original-plan-coverage-matrix.md`

- [ ] Delete the module/export and prove both commands are empty:

```bash
rg -n '\bCoreSystems\b|\.subsystems\b' crates/executive/src crates/bin/src
rg -n 'DaemonComposition' crates/executive/src -g '*.rs' | grep -v 'impl/daemon/bootstrap/'
```

- [ ] Remove every resolved exact entry from `config/architecture-allowlist.txt`; do not add a replacement exception for the composition root.
- [ ] Run focused X02 tests:

```bash
cargo test -p executive --test private_composition_root \
  --test capability_path --test turn_use_case_ports \
  --test turn_service_equivalence --test kernel_lifecycle_scenarios \
  --test session_lifecycle_commands --test production_health
```

Expected: PASS with no ignored X02 test.

- [ ] Run full validation:

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
scripts/architecture-check.sh
```

Expected: all commands PASS; architecture findings only decrease.

- [ ] Mark X02 `done` only after the deletion searches, focused tests, workspace tests, and architecture gate pass.
- [ ] Commit with subject `refactor(architecture): delete CoreSystems container` and a body listing the private bootstrap boundary and deleted escape paths.

## Completion evidence checklist

- [ ] `RequestHandler`, `TurnPipeline`, `DaemonTurnOrchestrator`, projection, capability, and runtime adapters retain no container.
- [ ] `CoreSystems` file, module, symbol, and field name are absent from production code.
- [ ] `DaemonComposition` is private and confined to bootstrap.
- [ ] `handler/init.rs` is at most 250 lines and no bootstrap module exceeds 700 lines.
- [ ] No concrete store/registry/mutex crosses into RPC handlers.
- [ ] Capability, approval, cancellation, session replay, daemon/exec equivalence, projection outage, health, Google recovery, and Telegram recovery tests pass.
- [ ] Workspace fmt, Clippy, tests, and architecture fitness pass.
