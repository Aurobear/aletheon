# Codex Project Instructions: Aletheon

## Branch and PR workflow

- Treat `dev` as the default integration branch unless the user says otherwise.
- Feature branches PR into `dev`. Only `dev` PRs into `main`.
- After a PR/MR is merged, clean up the merged feature branch locally and remotely when safe.
- Before deleting a branch, verify it is merged and not protected, still open, or shared by another active PR/MR.
- If deletion requires credentials or elevated permissions, ask the user or maintainer.
- Do not mention local AI tool names in external-facing PR/MR comments or commit messages.

## Crate module conventions

Each crate under `crates/` MUST organize source code as `src/<domain>/mod.rs`
with sub-files per concern. No single-file domain dumps. No module file may
exceed 2000 lines without a split plan.

| Crate | Domain layout |
|-------|--------------|
| `fabric` | `types/`, `include/`, `ipc/`, `events/`, `kernel/`, `policy/`, `primitives/`, `contract/`, `dasein/` |
| `kernel` | `admission/`, `capability/`, `chronos/`, `operation/`, `process/`, `service/`, `space/`, `supervision/` |
| `executive` | `core/`, `service/`, `impl/`, `bridge/`, `tools/`, `host/` |

## Service access

- Kernel primitives: route through `ServicePorts` (ProcessTable, OperationTable, Clock, SupervisorTree, Mailbox, Admission, Agora, Budget, Lease).
- Domain services: route through `CoreSystems` grouped fields (`systems.memory`, `systems.security`, `systems.corpus`, `systems.session`).

## Safety invariants

1. All tool execution MUST pass through `AdmissionController::admit()` → `ExecutionPermit`.
2. `SandboxFirst` MUST fail-closed — no prompt-only workaround.
3. Agora shared writes MUST use `propose()` → `commit()` with version CAS.
4. Each turn gets a private `ContextSpace`; turn input is private overlay.

## Test discipline

- Kernel timeout/deadline tests use `VirtualClock` — no real `sleep`.
- New behavior requires tests.

### Test scope by phase

Full `cargo test --workspace` is too slow for iterative development. Scale up with risk:

| Phase | Scope | Command |
|-------|-------|---------|
| **Feature work** (per-commit) | Affected crate only | `cargo test -p <crate> --lib --no-fail-fast` |
| **Cross-crate change** | Affected crates | `cargo test -p <crate1> -p <crate2> --lib --no-fail-fast` |
| **New integration test** | Specific test file | `cargo test -p <crate> --test <name> --no-fail-fast` |
| **Pre-PR to dev** | Affected crates all targets | `cargo test -p <crate> --all-targets --no-fail-fast` |
| **Merge to dev** | Full workspace | `cargo test --workspace --no-fail-fast` (only on PR to dev) |

**Do NOT run `cargo test --workspace` during feature-branch work.**
Use `cargo check --workspace --all-targets` to verify cross-crate compatibility.
Full workspace tests only at dev merge time.

## Phase constraints (current wiring window)

Prohibited:
- New kernel primitives
- Crate renames or cross-crate file moves
- Full legacy Event/EventBus cleanup
- CRDT / distributed consistency

Allowed:
- Connecting existing kernel infrastructure to `execute.rs`
- Grouping CoreSystems fields
- Schema enforcement and targeted event transport replacement

## Commit format

`type(domain): message` — types: feat, refactor, fix, test, chore, security.
Never include local AI tool names. End with `Co-Authored-By: Claude <noreply@anthropic.com>`.
