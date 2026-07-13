# Space Lifecycle Phase 2b — Kernel-Owned Space Lifecycle (design + plan)

> Date: 2026-07-12
> Status: approved (autonomous goal execution), pending implementation
> Scope: move space lifecycle into the kernel `ProcessTable` — fork a child
> space from its parent on `spawn`, and release a process's space on terminal
> exit. Concrete shared `Arc<InMemorySpaceManager>` (NOT `Arc<dyn>` / trait
> promotion — YAGNI, only one impl + no dyn consumer).
> Predecessors: Phase 1 (leak stop-gap), Phase 2a (reuse process.space).

---

## 1. Rationale & deviation from earlier §7

The Phase 2a design's §7 listed `Arc<dyn SpaceManager>` + promoting
`release`/`upsert_binding`/… onto the `SpaceManager` **trait**. Verified reality:
`ProcessManager`/`SpaceManager` are traits but **never used as `dyn`** — every
consumer holds the concrete type (`ServicePorts.process_table: Arc<ProcessTable>`,
`space_manager: InMemorySpaceManager`). Promoting every inherent method onto the
trait is churn with no consumer. So Phase 2b uses a shared **`Arc<InMemorySpaceManager>`**:
`ProcessTable` gets a reference to the same instance `ServicePorts` exposes, and
executive call sites keep working unchanged through `Arc` `Deref`.

Trait promotion + `Arc<dyn>` stays deferred until a second backend or a `dyn`
consumer actually exists.

## 2. Scope boundary

**In scope (mechanism + main-table wiring):**
- `ProcessTable` holds a shared `Arc<InMemorySpaceManager>`.
- `spawn`: fork the child space from the parent's space when `spec.parent` is
  set and found; else mint a fresh root space (unchanged for parentless
  spawns, e.g. the main agent — preserves Phase 2a behavior).
- `mark_exit`: release the process's space on every terminal transition
  (Terminate/Kill/completion) — kernel-owned, can't be bypassed.
- `ServicePorts` shares one `Arc<InMemorySpaceManager>` between the field and
  the process table.
- Executive `exit_process` reverts to a plain `signal(Terminate)` — the kernel
  now owns release (removes the Phase-2a executive-side release).

**Deferred to Phase 2c (documented, not built here):** making the *production*
sub-agent path actually fork-inherit — i.e. `SubAgentSpawner` using the shared
`ServicePorts.process_table` (it currently builds its own via
`SubAgentSpawner::new()` at `core/orchestrator.rs:44`, inside the **dead**
`AletheonExecutive`) and passing `parent: Some(main_pid)` (currently `parent: None`
at `sub_agent.rs:232,325`). That is a separate subsystem tangled with dead-code
cleanup; the fork mechanism built here is unit-tested and ready for it.

---

## 3. Changes (exact)

### 3.1 `crates/kernel/src/process/table.rs`

**(a) import** (top of file, with the other `use crate::...`):
```rust
use crate::space::InMemorySpaceManager;
```

**(b) struct field** — add to `pub struct ProcessTable`:
```rust
    space_manager: Arc<InMemorySpaceManager>,
```

**(c) constructors** — replace `new` and add `with_space_manager`:
```rust
    /// Create a table with its own private space manager (tests, standalone).
    pub fn new(clock: Arc<dyn Clock>) -> Self {
        Self::with_space_manager(clock, Arc::new(InMemorySpaceManager::new()))
    }

    /// Create a table sharing a space manager with the rest of the kernel, so
    /// spawn/exit can fork and release context spaces.
    pub fn with_space_manager(
        clock: Arc<dyn Clock>,
        space_manager: Arc<InMemorySpaceManager>,
    ) -> Self {
        Self {
            clock,
            records: Mutex::new(HashMap::new()),
            space_manager,
        }
    }
```
(All 14 existing `ProcessTable::new(clock)` call sites keep compiling — each
gets a private space manager, which is correct for isolated tests.)

**(d) `spawn` — fork from parent** (replace the body). Must NOT hold the
`records` lock across the `fork_space` `.await`:
```rust
    async fn spawn(&self, spec: SpawnSpec) -> anyhow::Result<ProcessHandle> {
        let process_id = ProcessId::new();
        // Look up the parent's space (scoped lock — released before await).
        let parent_space = {
            let records = self.records.lock().await;
            spec.parent
                .and_then(|pid| records.get(&pid).map(|r| r.record.space))
        };
        // Fork the child space from the parent (inherits bindings read-only),
        // or mint a fresh root space for parentless processes.
        let space = match parent_space {
            Some(parent_space) => self.space_manager.fork_space(parent_space, process_id).await?,
            None => SpaceId::new(),
        };
        let record = ProcessRecord {
            process_id,
            agent_id: spec.agent_id,
            parent: spec.parent,
            profile: spec.profile,
            state: ProcessState::Created,
            space,
            mailbox: MailboxId::new(),
            namespace: spec.namespace,
            created_at: self.clock.wall_now(),
            last_heartbeat: self.clock.mono_now(),
            exit: None,
        };
        let mut records = self.records.lock().await;
        records.insert(
            process_id,
            ProcessRuntime {
                record,
                notify: Arc::new(Notify::new()),
                active_operation: None,
            },
        );
        Ok(ProcessHandle { id: process_id })
    }
```

**(e) `mark_exit` — release the space** (modify): capture the space inside the
lock, release after dropping it:
```rust
    pub async fn mark_exit(&self, id: ProcessId, reason: ExitReason) -> anyhow::Result<()> {
        let (notify, space) = {
            let mut records = self.records.lock().await;
            let runtime = records
                .get_mut(&id)
                .ok_or_else(|| anyhow::anyhow!("unknown process: {:?}", id))?;
            runtime.record.exit = Some(ExitStatus {
                reason: reason.clone(),
                at: self.clock.mono_now(),
            });
            runtime.record.state = match reason {
                ExitReason::Failed(_) | ExitReason::Panic(_) => ProcessState::Failed,
                _ => ProcessState::Exited,
            };
            runtime.record.last_heartbeat = self.clock.mono_now();
            (runtime.notify.clone(), runtime.record.space)
        };
        // Kernel-owned lifecycle: free the process's context space on terminal exit.
        self.space_manager.release(space);
        notify.notify_waiters();
        Ok(())
    }
```

### 3.2 `crates/kernel/src/service/mod.rs`

- Field (line 75): `pub space_manager: Arc<InMemorySpaceManager>,`
- In `new()` (≈85/97) and `for_testing()` (≈124/130): create the Arc first and
  share it:
```rust
        let space_manager = Arc::new(InMemorySpaceManager::new());
        let process_table = Arc::new(ProcessTable::with_space_manager(
            clock.clone(),
            space_manager.clone(),
        ));
```
  and store `space_manager` (the Arc) in the struct literal. (Delete the old
  `let process_table = Arc::new(ProcessTable::new(clock.clone()));` and the old
  `let space_manager = InMemorySpaceManager::new();`.)

### 3.3 `crates/executive/src/service/daemon_turn/orchestrator.rs`

Revert `exit_process` to plain signal (kernel now releases on `mark_exit`):
```rust
    pub async fn exit_process(&self, process_id: ProcessId) -> anyhow::Result<()> {
        self.process_table
            .signal(process_id, ProcessSignal::Terminate)
            .await
    }
```
(Removes the Phase-2a inspect-then-release; no double-release, kernel owns it.)

Executive `execute_turn` is **unchanged** — the main agent is parentless, so its
space is still the lazily-created `record.space` (`SpaceId::new()`), reused per
Phase 2a; kernel release fires if/when `exit_process(main_pid)` runs at shutdown.

---

## 4. Tasks (TDD, per-task commit)

### Task 1: kernel space_manager wiring + fork-on-spawn + release-on-exit
- [ ] Apply 3.1(a)-(e). Add two tests to the existing `#[cfg(test)] mod tests`
  in `table.rs`:
```rust
    #[tokio::test]
    async fn spawn_forks_child_space_from_parent() {
        use crate::chronos::SystemClock;
        use fabric::types::space::{ContextBinding, SessionId};
        let sm = std::sync::Arc::new(InMemorySpaceManager::new());
        let table = ProcessTable::with_space_manager(std::sync::Arc::new(SystemClock::new()), sm.clone());
        let parent = table.spawn(SpawnSpec::default()).await.unwrap();
        let parent_space = table.inspect(parent.id).await.unwrap().space;
        sm.upsert_binding(parent_space, ContextBinding::Session(SessionId("s".into())));
        let child = table
            .spawn(SpawnSpec { parent: Some(parent.id), ..SpawnSpec::default() })
            .await
            .unwrap();
        let child_space = table.inspect(child.id).await.unwrap().space;
        assert_ne!(child_space, parent_space, "child gets its own space");
        let cb = sm.get_bindings(child_space).unwrap();
        assert!(cb.iter().any(|b| matches!(b, ContextBinding::Session(_))), "inherited parent binding");
    }

    #[tokio::test]
    async fn terminate_releases_process_space() {
        use crate::chronos::SystemClock;
        use fabric::types::space::{ContextBinding, SessionId};
        let sm = std::sync::Arc::new(InMemorySpaceManager::new());
        let table = ProcessTable::with_space_manager(std::sync::Arc::new(SystemClock::new()), sm.clone());
        let h = table.spawn(SpawnSpec::default()).await.unwrap();
        let space = table.inspect(h.id).await.unwrap().space;
        sm.upsert_binding(space, ContextBinding::Session(SessionId("s".into())));
        assert_eq!(sm.space_count(), 1);
        table.signal(h.id, fabric::types::process::ProcessSignal::Terminate).await.unwrap();
        assert!(sm.get_space(space).is_none(), "space released on terminate");
    }
```
- [ ] Apply 3.2 (`service/mod.rs`).
- [ ] Verify: `cargo test -p aletheon-kernel process::table::tests` (all pass);
  `cargo build --workspace`.
- [ ] Commit: `feat(kernel): kernel-owned space lifecycle — fork on spawn, release on exit`

### Task 2: executive exit_process revert
- [ ] Apply 3.3. Verify `cargo build -p executive` && `cargo test -p executive`.
- [ ] Commit: `refactor(executive): delegate space release to kernel on exit`

### Task 3: validation + doc
- [ ] `cargo fmt --all --check`; `cargo clippy -p aletheon-kernel -p executive --all-targets` (warning-free); `cargo test -p aletheon-kernel -p executive`.
- [ ] Update `docs/arch/CURRENT_ARCHITECTURE_AND_COUPLING_ANALYSIS.md` §9.5/§9.8:
  mark `ProcessTable::spawn`→`fork_space` and kernel-owned release DONE
  (Phase 2b, concrete `Arc<InMemorySpaceManager>`); note sub-agent *production*
  fork-inheritance + `SpaceManager` trait promotion remain Phase 2c.
- [ ] Commit: `docs(arch): mark kernel-owned space lifecycle done (Phase 2b)`

---

## 5. Risk

Moderate. The `spawn` rewrite must not hold `records` across the `fork_space`
await (handled via scoped lookup). `mark_exit` now releases on every exit — this
is the intended semantic (each process owns its space). Field-type change to
`Arc<InMemorySpaceManager>` is transparent to executive via `Deref` (verified: all
consumers call `&self` methods, no moves/`&mut`). No trait or `spawn` signature
change; the 14 `ProcessTable::new` call sites are untouched.
