# Private Composition Root and Split Bootstrap Implementation Plan

> **For agentic workers:** Use `workflow-feature` or `plans` to implement this plan task-by-task. Steps use checkbox (`- [x]`) syntax for tracking.

**Goal:** Delete the public `CoreSystems` god container, make daemon bootstrap the only code that knows all concrete implementations, and split the 2,077-line handler constructor into bounded subsystem and lifecycle builders.

**Architecture:** Production adapters receive explicit resource records containing only the handles they use. A private `DaemonComposition` exists only while bootstrap assembles the runtime and is consumed into `RequestHandler`; long-lived turn, capability, projection, and transport services retain narrow ports rather than a shared container. Bootstrap modules separate pure construction, fallible storage/external initialization, and task startup so partial initialization has an explicit owner and shutdown path.

**Tech Stack:** Rust, Tokio, async-trait, existing Fabric contracts, KernelRuntime, rusqlite-backed stores, static architecture tests

**Prerequisites:** X01 request ports complete; K01/K02 opaque Kernel runtime complete.

**Source requirements:** `docs/plans/2026-07-15-architecture-coupling-optimization-plan.md:1034-1063`

---

## Completion-code anchors (2026-07-16)

- The retired container file is absent and production references are rejected by `scripts/architecture-check.sh:97-128`.
- The private non-`Clone` composition root is confined to bootstrap at `crates/executive/src/impl/daemon/bootstrap/mod.rs:18-35`.
- Concrete stages are separated in `crates/executive/src/impl/daemon/bootstrap/{storage,google,runtime,channels,request}.rs`; the module boundary is declared at `crates/executive/src/impl/daemon/bootstrap/mod.rs:6-10`.
- The handler compatibility layer contains only request-facing accessors and notification wiring at `crates/executive/src/impl/daemon/handler/init.rs:1-41`.
- Static deletion, confinement, visibility, and size gates live at `crates/executive/tests/private_composition_root.rs:17-105`.
- The architecture baseline is shrink-only and concrete bootstrap imports are the sole composition exception at `scripts/architecture-check.sh:52-59`.

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

- [x] Add a failing static test that scans production Executive sources and permits `DaemonComposition` only below `impl/daemon/bootstrap/`, rejects `CoreSystems`, rejects `.subsystems`, and rejects a `bootstrap/mod.rs` longer than 700 lines.

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

- [x] Extend `scripts/architecture-check.sh` with the same production-only deletion gate so unit-test fixtures cannot mask a regression.
- [x] Run `cargo test -p executive --test private_composition_root`; expect FAIL because `CoreSystems` remains in the paths listed above.
- [x] Commit the red gate with subject `test(architecture): lock private composition boundary` and a body describing the forbidden escape paths.

### Task 2: Make capability execution depend on explicit resources

**Files:**
- Modify: `crates/executive/src/impl/daemon/handler/tool_executor.rs`
- Modify: `crates/executive/src/impl/daemon/handler/init.rs`
- Modify: `crates/executive/tests/capability_path.rs`

- [x] Add a test proving external/provider capability callers and turn callers share one `CapabilityService`, and that `tool_executor.rs` contains neither `CoreSystems` nor `Weak<...>`.
- [x] Introduce the exact resource record below and change `TurnToolExecutor::new` to receive it.

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

- [x] Rename `CoreCapabilityService` to `ProductionCapabilityService`, retain `CapabilityResources` directly, and use `resources.kernel` for transient lifecycle creation and settlement.
- [x] Construct one `Arc<dyn CapabilityService>` during bootstrap and pass clones to `HandlerPorts::transport` and provider workers; never reconstruct it inside a request or turn.
- [x] Run `cargo test -p executive --test capability_path --test governed_capability_path --test capability_invoker`; expect PASS.
- [x] Commit with subject `refactor(capability): inject explicit execution resources`.

### Task 3: Remove the container from post-turn projection

**Files:**
- Modify: `crates/executive/src/service/post_turn_projection.rs`
- Modify: `crates/executive/src/impl/daemon/handler/init.rs`
- Modify: `crates/executive/tests/turn_use_case_ports.rs`

- [x] Extend the projection outage test to assert projection construction has no `CoreSystems` input and terminal settlement remains earlier than projection spawn.
- [x] Add and consume this resource record:

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

- [x] Change `ProductionPostTurnProjection::new` to accept only `PostTurnProjectionResources`; assemble the record once in bootstrap.
- [x] Run `cargo test -p executive --test turn_use_case_ports --test turn_coordinator_lifecycle`; expect PASS.
- [x] Commit with subject `refactor(projection): inject post-turn resources`.

### Task 4: Remove the container from synchronous turn runtime ports

**Files:**
- Modify: `crates/executive/src/service/turn_runtime_ports.rs`
- Modify: `crates/executive/src/impl/daemon/handler/tool_executor.rs`
- Modify: `crates/executive/src/impl/daemon/handler/init.rs`
- Modify: `crates/executive/tests/turn_use_case_ports.rs`
- Modify: `crates/executive/tests/turn_pipeline_order.rs`

- [x] Add a static assertion that `turn_runtime_ports.rs` has no `CoreSystems`, `subsystems`, or direct production capability construction.
- [x] Replace `TurnRuntimePorts::production(CoreSystems, ...)` with the exact boundary below; adapters may retain individual handles but not the aggregate record after construction.

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

- [x] Construct concrete adapters in bootstrap from the same handles used by Task 2; `ProductionTurnCapability` receives `CapabilityResources`, not a container.
- [x] Preserve blocking order `context -> model -> approval/capability -> terminal settlement`.
- [x] Run `cargo test -p executive --test turn_use_case_ports --test turn_pipeline_order --test sandbox_first_fail_closed`; expect PASS.
- [x] Commit with subject `refactor(turn): inject synchronous runtime resources`.

### Task 5: Make TurnPipeline fully composed

**Files:**
- Modify: `crates/executive/src/service/turn_pipeline.rs`
- Modify: `crates/executive/src/service/daemon_turn/orchestrator.rs`
- Modify: `crates/executive/src/impl/daemon/handler/init.rs`
- Modify: `crates/executive/tests/turn_service_equivalence.rs`

- [x] Add a constructor test showing a pipeline can be built from narrow ports without a god container.
- [x] Replace the constructor with this explicit input:

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

- [x] Make `TurnPipeline::new(resources)` assign fields only; all adapter construction moves to bootstrap.
- [x] Run `cargo test -p executive --test context_assembler --test turn_pipeline_order --test turn_service_equivalence`; expect PASS, including all eight equivalence scenarios.
- [x] Commit with subject `refactor(turn): compose pipeline from narrow ports`.

### Task 6: Remove the container from DaemonTurnOrchestrator

**Files:**
- Modify: `crates/executive/src/service/daemon_turn/orchestrator.rs`
- Modify: `crates/executive/src/service/daemon_turn/lifecycle.rs`
- Modify: `crates/executive/src/service/daemon_turn/execute.rs`
- Modify: `crates/executive/src/impl/daemon/handler/init.rs`
- Modify: `crates/executive/tests/kernel_lifecycle_scenarios.rs`
- Modify: `crates/executive/tests/session_lifecycle_commands.rs`

- [x] Add cancellation/restart tests proving the new lifecycle resource handles still create one main process, rotate the same default session, and cancel exactly one active turn.
- [x] Introduce `DaemonTurnResources` with only `kernel`, `clock`, `agora`, `default_session_id`, `main_agent_process_id`, `turn_token`, `pipeline`, `coordinator`, `session_service`, and `notify`.
- [x] Delete `DaemonTurnOrchestrator.subsystems`, unused mirrored handler fields, and constructor-side store/pipeline creation. Bootstrap constructs the canonical store, coordinator, session service, and pipeline before the orchestrator.
- [x] Change `execute.rs` to read `default_session_id`; change `lifecycle.rs` to use `main_agent_process_id` and `turn_token`.
- [x] Run `cargo test -p executive --test kernel_lifecycle_scenarios --test session_lifecycle_commands --test turn_coordinator_lifecycle`; expect PASS.
- [x] Commit with subject `refactor(daemon): narrow turn orchestrator state`.

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

- [x] Move deployment storage/quota policy to `storage.rs`, Google repository/OAuth/sync construction to `google.rs`, provider and Agent runtime registration to `runtime.rs`, Telegram polling to `channels.rs`, and concrete memory/domain plus HandlerPorts assembly to the bounded `request.rs` composition stage.
- [x] Define the private, non-`Clone` transient type below in `bootstrap/mod.rs`:

```rust
pub(super) struct DaemonComposition {
    request: Arc<HandlerPorts>,
    active_connections: Arc<AtomicUsize>,
}

impl DaemonComposition {
    fn into_handler(self) -> RequestHandler {
        RequestHandler {
            ports: self.request,
            notify_tx: None,
            active_connections: self.active_connections,
        }
    }
}
```

- [x] Keep `handler/init.rs` as a compatibility entry containing only request-facing accessors and notification wiring; implement `RequestHandler::new` inside the private bootstrap `request.rs` stage. Cap handler init at 250 lines, focused stages at 700 lines, and the complete request composition stage at 1,500 lines.
- [x] Ensure every spawned task handle is transferred to an explicit health/lifecycle owner before returning and every fallible earlier stage owns no detached task.
- [x] Run `cargo test -p executive --test private_composition_root --test production_health --test google_sync_recovery --test telegram_restart_recovery`; expect PASS.
- [x] Commit with subject `refactor(bootstrap): split daemon composition stages`.

### Task 8: Delete CoreSystems and close X02

**Files:**
- Delete: `crates/executive/src/core/core_systems.rs`
- Modify: `crates/executive/src/core/mod.rs`
- Modify: `config/architecture-allowlist.txt`
- Modify: `scripts/architecture-check.sh`
- Modify: `docs/plans/2026-07-15-executable-plan-decomposition-design.md`
- Modify: `docs/plans/2026-07-16-original-plan-coverage-matrix.md`

- [x] Delete the module/export and prove both commands are empty:

```bash
rg -n '\bCoreSystems\b|\.subsystems\b' crates/executive/src crates/bin/src
rg -n 'DaemonComposition' crates/executive/src -g '*.rs' | grep -v 'impl/daemon/bootstrap/'
```

- [x] Remove every resolved exact entry from `config/architecture-allowlist.txt`; do not add a replacement exception for the composition root.
- [x] Run focused X02 tests:

```bash
cargo test -p executive --test private_composition_root \
  --test capability_path --test turn_use_case_ports \
  --test turn_service_equivalence --test kernel_lifecycle_scenarios \
  --test session_lifecycle_commands --test production_health
```

Expected: PASS with no ignored X02 test.

- [x] Run full validation:

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
scripts/architecture-check.sh
```

Expected: all commands PASS; architecture findings only decrease.

- [x] Mark X02 `done` only after the deletion searches, focused tests, workspace tests, and architecture gate pass.
- [x] Commit with subject `refactor(architecture): delete CoreSystems container` and a body listing the private bootstrap boundary and deleted escape paths.

## Completion evidence checklist

- [x] `RequestHandler`, `TurnPipeline`, `DaemonTurnOrchestrator`, projection, capability, and runtime adapters retain no container.
- [x] `CoreSystems` file, module, symbol, and field name are absent from production code.
- [x] `DaemonComposition` is private and confined to bootstrap.
- [x] `handler/init.rs` is at most 250 lines; focused bootstrap stages are at most 700 lines and the complete request composition stage is at most 1,500 lines.
- [x] No concrete store/registry/mutex crosses into RPC handlers.
- [x] Capability, approval, cancellation, session replay, daemon/exec equivalence, projection outage, health, Google recovery, and Telegram recovery tests pass.
- [x] Workspace fmt, Clippy, tests, and architecture fitness pass.

## Completion record (2026-07-16)

- Production deletion searches are empty for `CoreSystems` and `.subsystems`; `DaemonComposition` appears only under `impl/daemon/bootstrap/`.
- Focused X02 capability, projection, lifecycle, health, Google recovery, Telegram recovery, and daemon/exec equivalence tests pass.
- `cargo fmt --all --check` and `cargo clippy --workspace --all-targets -- -D warnings` pass.
- `cargo test --workspace --quiet -- --test-threads=1` passes; serial execution avoids two pre-existing parallel-test races that each pass in isolation.
- `scripts/architecture-check.sh` passes with 66 findings, 38 dependencies, 4 paths, and no additions.
