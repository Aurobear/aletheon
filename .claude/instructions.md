# Aletheon Project Instructions

## Branch and PR workflow

- Treat `dev` as the default integration branch.
- Feature branches PR into `dev`. Only `dev` PRs into `main`.
- After a PR is merged, delete the merged feature branch locally and remotely.
- Do not mention local AI tool names in external-facing PR comments or commit messages.

## Multi-agent coordination

For non-trivial tasks (3+ files touched, or research-then-implement), use the
Agent tool to dispatch subagents. Do NOT do everything sequentially in the main
context.

| Agent | Purpose |
|-------|---------|
| `repo-researcher` | Read-only codebase exploration, mapping structure, identifying change points |
| `developer` | Implementation within allowed paths, validation, reporting results |
| `tester` | Focused validation of changes, failure classification |
| `fixer` | Targeted fix for named failures identified by tester |
| `reviewer` | Final review of changed files against requirements |
| `planner` | Task decomposition and implementation planning |

### When to use subagents

- Task touches 3+ files → spawn coordinator + parallel `developer` agents
- Research AND implementation needed → spawn `repo-researcher` + `developer` in parallel
- Debugging then fixing → spawn `tester` → `fixer`
- Plan Mode was used and plan is approved → invoke `/workflow` to execute
- Context is full (long history, many files read) → offload to `/workflow` or focused subagent

### Context management

- Main agent keeps <50% of context window
- Compress each subagent result immediately: retain only STATUS/SUMMARY/CHANGED_FILES/FAILURES
- Max 4 concurrent subagents
- Exploration and diagnosis are read-only by default

### Plan Mode → Workflow routing

- After Plan Mode approval, do NOT execute sequentially — invoke `/workflow`
- Exception: plan touches only 1 file → execute directly
- Pass the plan summary and file list so workflow does not re-plan from scratch

## Crate module conventions

Each crate under `crates/` MUST organize source code as `src/<domain>/mod.rs` with
sub-files per concern. Single-file domain dumps are prohibited.

| Crate | Domains (each under `src/<domain>/`) |
|-------|--------------------------------------|
| `fabric` | `types/`, `include/`, `ipc/`, `events/`, `kernel/`, `policy/`, `primitives/`, `contract/`, `dasein/` |
| `kernel` | `admission/`, `capability/`, `chronos/`, `operation/`, `process/`, `service/`, `space/`, `supervision/` |
| `executive` | `core/`, `service/`, `impl/`, `bridge/`, `tools/`, `host/` |
| `agora` | single crate concern (workspace, ops, persistence, attention, etc.) |
| `cognit` | `core/`, `harness/`, `impl/`, `bridge/`, `testing/` |
| `corpus` | `tools/`, `security/`, `drivers/`, `hook/`, `skill/` |

No new crate may introduce a `src/` directory without sub-domain grouping.
No file may exceed 2000 lines without a plan to split it into sub-files.

## Service access rules

Production path MUST access kernel primitives through `ServicePorts`:

```
DaemonTurnOrchestrator → ServicePorts → ProcessTable / OperationTable / Clock
                                        / SupervisorTree / Mailbox / Admission
                                        / Agora / Budget / Lease / SpaceManager
```

Domain services (memory, corpus, dasein) are accessed through `CoreSystems`
sub-groups (`systems.memory`, `systems.security`, `systems.corpus`,
`systems.session`).

## Safety invariants

1. **Admission gate**: all tool execution MUST go through
   `AdmissionController::admit()` → `ExecutionPermit`. No direct
   `ToolRunner::run()` calls without a permit.
2. **SandboxFirst fail-closed**: if SelfField returns `SandboxFirst` and
   sandbox is unavailable, execution MUST stop. No prompt-only workaround.
3. **Agora transaction model**: shared state writes MUST use
   `agora.propose(author)` → `agora.commit()` with version CAS. Never use
   `agora.publish()` for data that needs consistency. Always pass real
   ProcessId — `ProcessId(Uuid::nil())` is forbidden in production paths.
4. **Space lifecycle**: `ProcessTable::spawn()` forks a space from the parent
   (or root). `ProcessTable::reap()` releases it. `execute_turn` reuses
   `process.space` — do NOT create per-turn temporary SpaceIds. Space
   lifecycle = process lifecycle.

## Dependency injection

- All time access MUST go through `dyn Clock` (not `chrono::Utc::now()` or
  `std::time::Instant::now()` directly).
- Async sleep/timeout MUST use `kernel::chronos::Timer::sleep/timeout(clock, ...)`.
- `wall_to_datetime(WallTime)` is the only bridge from Clock time to chrono types.
- New `Arc<dyn Clock>` fields on structs that serve only to pass the clock to
  child constructors and are never read directly should carry `#[allow(dead_code)]`.

## Test discipline

- Kernel timeout/deadline tests MUST use `TestClock`. No real `sleep`.
- `cargo test --workspace` MUST pass with no failures before any commit.
- New behavior MUST have a test. Refactors without new tests are OK only if
  `cargo check --workspace --all-targets` + existing tests pass.

## Commit conventions

- Prefix: `feat(domain):`, `refactor(domain):`, `fix(domain):`, `test(domain):`,
  `chore(domain):`, `security(domain):`
- Never include model names or AI attribution lines in commit messages.
- Stage `Cargo.lock` alongside `Cargo.toml` when adding/removing crates or deps.

## Phase constraints (Phase 3-6 wiring window)

This is a **wiring-only phase**. The following are prohibited:
- Adding new kernel primitives (ProcessTable, OperationTable, Clock, etc.
  already exist; do not add more)
- Renaming crates or moving files between crates
- Full legacy Event/EventBus cleanup (only targeted replacement)
- CRDT / distributed consistency / DDS implementations

Allowed: connecting existing kernel infrastructure to the production execution
path (`execute.rs`), grouping CoreSystems fields, adding schema enforcement.
