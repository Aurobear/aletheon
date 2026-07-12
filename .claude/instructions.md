# Aletheon Project Instructions

## Branch and PR workflow

- Treat `dev` as the default integration branch.
- Feature branches PR into `dev`. Only `dev` PRs into `main`.
- After a PR is merged, delete the merged feature branch locally and remotely.
- Do not mention local AI tool names in external-facing PR comments or commit messages.

## Crate module conventions

Each crate under `crates/` MUST organize source code as `src/<domain>/mod.rs` with
sub-files per concern. Single-file domain dumps are prohibited.

| Crate | Domains (each under `src/<domain>/`) |
|-------|--------------------------------------|
| `fabric` | `types/`, `include/`, `ipc/`, `events/`, `kernel/`, `policy/`, `primitives/`, `contract/`, `dasein/` |
| `kernel` | `admission/`, `capability/`, `chronos/`, `operation/`, `process/`, `space/`, `supervision/` |
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
                                        / Agora / Budget / Lease
```

Domain services (memory, corpus, dasein) are accessed through `CoreSystems`
sub-groups (`systems.memory`, `systems.security`, `systems.corpus`,
`systems.session`). New kernel-level primitives are prohibited during the
Phase 3-6 wiring window.

## Safety invariants

1. **Admission gate**: all tool execution MUST go through
   `AdmissionController::admit()` → `ExecutionPermit`. No direct
   `ToolRunner::run()` calls without a permit.
2. **SandboxFirst fail-closed**: if SelfField returns `SandboxFirst` and
   sandbox is unavailable, execution MUST stop. No prompt-only workaround.
3. **Agora transaction model**: shared state writes MUST use
   `agora.propose()` → `agora.commit()` with version CAS. Never use
   `agora.publish()` for data that needs consistency.
4. **Context space isolation**: each turn gets a private `ContextSpace` via
   `SpaceManager`. Turn input is private overlay data, not shared Agora fact.

## Test discipline

- Kernel timeout/deadline tests MUST use `VirtualClock`. No real `sleep`.
- `cargo test --workspace` MUST pass with no failures before any commit.
- New behavior MUST have a test. Refactors without new tests are OK only if
  `cargo check --workspace --all-targets` + existing tests pass.

## Phase constraints (Phase 3-6 wiring window)

This is a **wiring-only phase**. The following are prohibited:
- Adding new kernel primitives (ProcessTable, OperationTable, Clock, etc.
  already exist; do not add more)
- Renaming crates or moving files between crates
- Full legacy Event/EventBus cleanup (only targeted replacement)
- CRDT / distributed consistency / DDS implementations

Allowed: connecting existing kernel infrastructure to the production execution
path (`execute.rs`), grouping CoreSystems fields, adding schema enforcement.

## Commit conventions

- Prefix: `feat(domain):`, `refactor(domain):`, `fix(domain):`, `test(domain):`,
  `chore(domain):`, `security(domain):`
- Never include local AI tool names in commit messages.
- End commit messages with `Co-Authored-By: Claude <noreply@anthropic.com>`.
