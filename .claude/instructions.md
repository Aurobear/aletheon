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

## Architectural invariants (anti-shadow-system)

One authoritative implementation per runtime concept. Every new feature MUST
extend the existing implementation, never build a parallel one alongside it.

This is not a style preference — it is the macro-kernel's core discipline
(`docs/arch/Aletheon_MacroKernel_Architecture_Final(2).md:1073`):
authoritative runtime objects, single state ownership, structured lifecycles,
non-bypassable capability governance. Two implementations of the same concept
produce two different safety policies, two memory paths, two Agora views, two
scheduling policies — and the system quietly diverges until no one knows which
path a given input takes.

### The authoritative implementation per concept

| Concept | One true implementation | Location |
|---------|------------------------|----------|
| Task/process lifecycle | `AgentProcess` + `SubAgentSpawner` + `ProcessTable` | `crates/executive/src/core/sub_agent.rs` |
| Work scheduling + cancellation | `OperationTable` (cancellation tree) | `crates/kernel/src/operation/` |
| Failure recovery + restart | `SupervisorTree` (OTP-style restart policies) | `crates/kernel/src/supervision/` |
| Turn execution path | `TurnPipeline::run()` (PreTurn → ReActLoop → PostTurn) | `crates/executive/src/service/` |
| Tool safety gate (per-call) | `SelfField::review()` inside ReActLoop | `crates/dasein/` — via TurnPipeline |
| Shared state (working memory) | `Agora` (CAS propose → commit) | `crates/agora/` |
| Memory persistence + recall | `Mnemosyne` backends (episodic / semantic / procedural) | `crates/mnemosyne/src/impl/backends/` |
| Approval / human-in-the-loop | `SessionGateway::approval_flow` | `crates/executive/src/core/session_gateway/approval_flow.rs` |
| OAuth token storage + refresh | `McpOAuthProvider` + `TokenStore` | `crates/corpus/src/tools/mcp/auth.rs` |
| Budget + quota enforcement | `AdmissionController::admit()` | `crates/kernel/src/admission/` |

A plan or design that introduces a new state machine, a new store, a new
worker trait, a new token vault, a new approval manager, or a new execution
loop MUST answer: **why does the existing implementation not fit, and what is
the cost of extending it instead?** If the answer is "I didn't check," the
plan is rejected.

### Historical instances (this has happened multiple times)

1. **Pre-convergence "multiple truth sources"** (2026-07-13 resolved):
   three parallel ReActLoops (daemon chat, Executive, bin exec), each with
   different safety/memory/Agora semantics. Resolved by unifying all paths
   into `TurnPipeline::run()`. Documented in
   `docs/arch/CURRENT_ARCHITECTURE_AND_COUPLING_ANALYSIS.md:235-236`.
   This was the **most dangerous issue** in the project's history.

2. **Agent-Google plan review** (2026-07-14): proposed `GoalSupervisor` +
   `GoalWorker` + `GoalStore` duplicating `SubAgentSpawner` + `AgentProcess` +
   `ProcessTable`; proposed `CredentialVault` duplicating `McpOAuthProvider` +
   `TokenStore`; proposed `ApprovalManager` duplicating `approval_flow.rs`;
   proposed `DeepSeekWorker::execute()` bypassing `TurnPipeline::run()`.
   Review: `docs/plans/2026-07-14-agent-google-review.md`.

### Design / Plan review checklist

Before approving any design or plan that adds a new subsystem, verify:

- [ ] Does an authoritative implementation for this concept already exist?
- [ ] If yes, does the plan extend it or duplicate it?
- [ ] Does the new feature route through `TurnPipeline::run()`?
- [ ] Does tool execution go through `SelfField::review()`?
- [ ] Do new state machines reuse `AgentProcess` / `OperationTable`?
- [ ] Does new persistence extend an existing mnemosyne backend?
- [ ] Does new auth/token storage extend `McpOAuthProvider` / `TokenStore`?
- [ ] Does new approval logic integrate with `SessionGateway::approval_flow`?
- [ ] Does new scheduling use `OperationTable`'s cancellation tree?

## Dependency injection

- All time access MUST go through `dyn Clock` (not `chrono::Utc::now()` or
  `std::time::Instant::now()` directly).
- Async sleep/timeout MUST use `kernel::chronos::Timer::sleep/timeout(clock, ...)`.
- `wall_to_datetime(WallTime)` is the only bridge from Clock time to chrono types.
- New `Arc<dyn Clock>` fields on structs that serve only to pass the clock to
  child constructors and are never read directly should carry `#[allow(dead_code)]`.

## Test discipline

- Kernel timeout/deadline tests MUST use `TestClock`. No real `sleep`.
- New behavior MUST have a test. Refactors without new tests are OK only if
  `cargo check --workspace --all-targets` + existing tests pass.

### Test scope by phase

Full `cargo test --workspace` is too slow for iterative development (10+ minutes).
Scale test scope to the risk of the change:

| Phase | Scope | Command |
|-------|-------|---------|
| **Feature work** (per-commit) | Affected crate only | `cargo test -p <crate> --lib --no-fail-fast` |
| **Cross-crate change** | Affected crates | `cargo test -p <crate1> -p <crate2> --lib --no-fail-fast` |
| **New integration test** | Specific test file | `cargo test -p <crate> --test <name> --no-fail-fast` |
| **Pre-PR to dev** | Affected crates all targets | `cargo test -p <crate> --all-targets --no-fail-fast` |
| **Merge to dev** | Full workspace | `cargo test --workspace --no-fail-fast` (only on PR to dev) |

**Rule**: Do NOT run `cargo test --workspace` during feature-branch development.
Only run it when the PR targets `dev`. Before that, test only the crates you touched.
A `cargo check --workspace --all-targets` is sufficient to catch cross-crate breakage.

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
