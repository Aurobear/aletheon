# Space Lifecycle Phase 2c — Production Sub-Agent Fork-Inheritance (design + plan)

> Date: 2026-07-12
> Status: approved (autonomous goal execution), pending implementation
> Scope: make the LIVE daemon sub-agent path (AgentTool → SubAgentSpawner)
> register sub-agents in the SHARED kernel `ProcessTable` and pass the main
> agent as their process parent, so Phase 2b's fork-on-spawn actually triggers
> in production (sub-agents inherit the main agent's context space, read-only).
> Predecessor: Phase 2b (kernel-owned fork/release mechanism).

---

## 1. Problem (verified)

Phase 2b made `ProcessTable::spawn` fork a child space from `spec.parent`'s
space. But in production that never fires because:

1. **Two ProcessTables.** The live AgentTool spawns via
   `subsystems.runtime` (`AletheonExecutive`, alive — `init.rs:362,538,619,637`)
   whose `SubAgentSpawner::new()` built its **own** `ProcessTable`
   (`sub_agent.rs:156`), separate from `ServicePorts.process_table` used by the
   main agent. Sub-agents are invisible to the shared table.
2. **`parent: None`.** `spawn_with_policy`/`spawn_tracked` hardcode
   `SpawnSpec { .. }` with the default `parent: None` (`sub_agent.rs:220-224`,
   ~325), so even in one table no fork parent is set.
3. **`main_pid` not reachable** in the AgentTool closure (captured at
   `init.rs:619`, before the orchestrator that owns `main_agent_process_id`
   exists at `init.rs:788`).

Note: the OperationRequest `parent: None` (`sub_agent.rs:232`) is the *operation*
tree, unrelated to *process* space forking — leave it alone.

---

## 2. Approach (lowest-risk)

- **Share tables** via the existing async injection block at `init.rs:580-591`
  (which already does `sub_agent_spawner_mut().with_runtime(...)`): add a
  `set_shared_tables(process_table, operation_table)` call using
  `subsystems.ports.*`. No init reordering.
- **Share `main_pid`**: move `main_agent_process_id` to a shared
  `Arc<Mutex<Option<ProcessId>>>` field on `CoreSystems` (constructed at
  `init.rs:536`, before the tool closure). Both the AgentTool closure and
  `DaemonTurnOrchestrator` read it. Avoids touching the 9-arg
  `DaemonTurnOrchestrator::new`.
- **parent param**: add `spawn_tracked_with_parent(task, parent_turn_id,
  restart_policy, parent: Option<ProcessId>)`; keep `spawn_tracked` delegating
  with `None` so existing callers/tests are unchanged. Live closure calls the
  new variant with the shared `main_pid`.

---

## 3. Changes (exact)

### 3.1 `crates/executive/src/core/sub_agent.rs`

**(a)** Add `set_shared_tables` on `SubAgentSpawner` (after `with_tables`):
```rust
    /// Repoint this spawner at externally-owned kernel tables so sub-agents
    /// register in the same ProcessTable/OperationTable as the main agent.
    /// Safe to call at bootstrap before any sub-agent is spawned.
    pub fn set_shared_tables(
        &mut self,
        process_table: Arc<ProcessTable>,
        operation_table: Arc<OperationTable>,
    ) {
        self.process_table = process_table;
        self.operation_table = operation_table;
    }
```

**(b)** Add a parent-aware variant. Rename the current `spawn_tracked` body to
`spawn_tracked_with_parent` (adding a `parent: Option<ProcessId>` param and
setting it in that method's `SpawnSpec`), and make `spawn_tracked` delegate:
```rust
    pub async fn spawn_tracked(
        &mut self,
        task: String,
        parent_turn_id: String,
        restart_policy: RestartPolicy,
    ) -> anyhow::Result<SubAgentHandle> {
        self.spawn_tracked_with_parent(task, parent_turn_id, restart_policy, None)
            .await
    }

    pub async fn spawn_tracked_with_parent(
        &mut self,
        task: String,
        parent_turn_id: String,
        restart_policy: RestartPolicy,
        parent: Option<ProcessId>,
    ) -> anyhow::Result<SubAgentHandle> {
        // ... existing spawn_tracked body, but the SpawnSpec uses `parent`:
        //     .spawn(SpawnSpec { parent, profile: ..., namespace: ..., .. })
    }
```
(Implementer: locate the existing `spawn_tracked` body, move it verbatim into
`spawn_tracked_with_parent`, add `parent,` to its `SpawnSpec { .. }`, ensure
`ProcessId` is imported.)

### 3.2 `crates/executive/src/core/core_systems.rs`

Add a shared main-agent slot field to `CoreSystems`:
```rust
    /// Shared main-agent process id, written by DaemonTurnOrchestrator's
    /// ensure_main_agent and read by tools (e.g. AgentTool) that need the
    /// process parent for space forking.
    pub main_agent_process_id: Arc<tokio::sync::Mutex<Option<fabric::ProcessId>>>,
```
(Match the existing import style; `Arc`/`Mutex` are already used in this file.)

### 3.3 `crates/executive/src/service/daemon_turn/orchestrator.rs`

- Remove the owned field `main_agent_process_id: Mutex<Option<ProcessId>>`
  (line 41) and its initializer `main_agent_process_id: Mutex::new(None),`
  (line 92).
- Everywhere it was used, reference `self.subsystems.main_agent_process_id`.

### 3.4 `crates/executive/src/service/daemon_turn/lifecycle.rs`

In `ensure_main_agent` (line 13), change
`let mut guard = self.main_agent_process_id.lock().await;` to
`let mut guard = self.subsystems.main_agent_process_id.lock().await;`
(Also update any other `self.main_agent_process_id` references in cancel/exit
methods — grep the daemon_turn dir.)

### 3.5 `crates/executive/src/impl/daemon/handler/init.rs`

- **CoreSystems construction (≈536):** add
  `main_agent_process_id: Arc::new(Mutex::new(None)),` to the struct literal.
- **Injection block (≈580-591):** after `.with_runtime(sub_agent_runtime);`,
  add a second statement in the same block scope:
```rust
        {
            let pt = subsystems.ports.process_table.clone();
            let ot = subsystems.ports.operation_table.clone();
            subsystems
                .runtime
                .lock()
                .await
                .sub_agent_spawner_mut()
                .set_shared_tables(pt, ot);
        }
```
- **AgentTool closure (≈619-650):** capture a clone of the shared slot before
  the closure (`let main_slot = subsystems.main_agent_process_id.clone();`),
  move it in, and at the spawn call read the parent and use the new variant:
```rust
                        let parent = *main_slot.lock().await;
                        let handle = runtime
                            .sub_agent_spawner_mut()
                            .spawn_tracked_with_parent(
                                up.clone(),
                                "agent-tool".into(),
                                RestartPolicy::Never,
                                parent,
                            )
                            .await?;
```
  (replacing the existing `.spawn_tracked(up.clone(), "agent-tool".into(), RestartPolicy::Never)` call).

---

## 4. Tasks (per-task commit)

### Task 1: spawner API — `set_shared_tables` + `spawn_tracked_with_parent` (TDD)
- [ ] Apply 3.1. Add a kernel-verifiable unit test in `sub_agent.rs` tests:
  build a shared `Arc<InMemorySpaceManager>` → `Arc<ProcessTable>` via
  `ProcessTable::with_space_manager`, construct `SubAgentSpawner` and
  `set_shared_tables`, spawn a "main" process in that table, seed a binding on
  its space, then `spawn_tracked_with_parent(..., Some(main_pid))` and assert
  the sub-agent's `inspect().space` differs from main's AND inherited the
  binding (proves production-shaped fork). (If SubAgentSpawner can't expose the
  child pid/space directly, assert via the shared space manager's
  `space_count`/`get_bindings`.)
- [ ] Verify `cargo test -p executive sub_agent 2>&1 | tail -20`; `cargo build --workspace`.
- [ ] Commit: `feat(executive): sub-agent spawner shared tables + parent-aware spawn`

### Task 2: bootstrap wiring — shared main_pid slot + shared tables + parent
- [ ] Apply 3.2, 3.3, 3.4, 3.5. Grep the whole `daemon_turn/` dir for any other
  `self.main_agent_process_id` and update to `self.subsystems.main_agent_process_id`.
- [ ] Verify `cargo build -p executive 2>&1 | tail -30` (bootstrap compiles);
  `cargo test -p executive 2>&1 | tail -20`.
- [ ] Commit: `feat(executive): share main-agent pid + kernel tables to sub-agent path`

### Task 3: validation + doc
- [ ] `cargo fmt --all --check`; `cargo clippy -p aletheon-kernel -p executive --all-targets` (warning-free); `cargo test -p aletheon-kernel -p executive`.
- [ ] Update `docs/arch/CURRENT_ARCHITECTURE_AND_COUPLING_ANALYSIS.md` §9.5:
  mark sub-agent production fork-inheritance DONE (Phase 2c). The Space
  subsystem now matches design `Final(2).md` §9.5 end-to-end. Note the only
  remaining §7 item is `SpaceManager` **trait** promotion (deferred, YAGNI —
  single impl, no `dyn` consumer).
- [ ] Commit: `docs(arch): mark sub-agent space fork-inheritance done (Phase 2c)`

---

## 5. Risk

Higher than 2a/2b — touches daemon bootstrap (`init.rs`) and orchestrator shared
state. Mitigations: (1) reuse the EXISTING async injection block rather than
reorder init; (2) shared slot on `CoreSystems` avoids changing the 9-arg
orchestrator constructor; (3) `spawn_tracked` keeps its signature (new
`_with_parent` variant) so existing tests are untouched; (4) full
`cargo build --workspace` + `cargo test -p executive` + reviewer as safety net.
If the bootstrap wiring cannot be made green within these files, STOP and report
(do not expand scope into the AletheonExecutive/orchestrator ownership refactor).
