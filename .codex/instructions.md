# Codex Project Instructions: Aletheon

## Tool and debugging efficiency

- Never invoke Cargo directly; use `bash scripts/cargo-agent.sh <cargo arguments>`.
- For repository overviews, batch-read known entry files before scoped search.
- Do not inventory one file extension at a time or alternate one model request per independent file.
- Treat `provider_unavailable`, `provider_rejected_request`, and any rendered inference error as test failures even if a monitor aggregate says PASS.
- Separate model rounds, provider retries, and tool-call counts in debug reports.
- System acceptance uses `sudo bash scripts/aletheon.sh deploy` and `/usr/bin/aletheon`; do not deploy or accept `~/.local/bin/aletheon`.

## Generalization and answer quality

- Never implement production branches for a fixed test prompt, phrase,
  language, repository name, checkout path, or expected answer.
- Test scenarios may be concrete; runtime decisions must use typed state,
  capability semantics, effective configuration, and token/tool/time budgets.
- Glob/path discovery is not content evidence. Do not reduce requests by
  blocking the reads needed to support the answer.
- Verify every claimed file or symbol and every claim that documentation or
  configuration is absent. Score analysis correctness, not just completion.
- Report cumulative provider tokens, active context occupancy, cache tokens,
  inference rounds, retries, and tools independently.
- Require both three fresh real-TUI repetitions for model-controlled behavior
  and a sustained multi-turn run in one unchanged session.
- Treat disagreement between monitor verdict, TUI frame, session, audit, or
  daemon logs as a failed test and fix the monitor.
- Treat effective model ID, display name, context capacity, runtime selection,
  session identity, and budgets as host-owned runtime facts. Never infer them
  from model prose or training priors.
- Do not report an asynchronous child result from a spawn handle. Observe its
  terminal snapshot or durable terminal receipt first.
- Do not reuse an accepted event schema for a payload with different semantics.
  Producer/schema/projector compatibility is a contract, not a naming detail.
- Keep per-session retry behavior distinct from machine-wide provider
  backpressure; both must be validated when request storms are investigated.

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
| `contracts` | `types/`, `include/`, `ipc/`, `events/`, `protocol/`, `primitives/`, `policy/`, `contract/`, `dasein/`, `adapters/` |
| `kernel` | `admission/`, `capability/`, `chronos/`, `operation/`, `process/`, `service/`, `space/`, `supervision/` |

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

Full `bash scripts/cargo-agent.sh test --workspace` is too slow for iterative development. Scale up with risk:

| Phase | Scope | Command |
|-------|-------|---------|
| **Feature work** (per-commit) | Affected crate only | `bash scripts/cargo-agent.sh test -p <crate> --lib --no-fail-fast` |
| **Cross-crate change** | Affected crates | `bash scripts/cargo-agent.sh test -p <crate1> -p <crate2> --lib --no-fail-fast` |
| **New integration test** | Specific test file | `bash scripts/cargo-agent.sh test -p <crate> --test <name> --no-fail-fast` |
| **Pre-PR to dev** | Affected crates all targets | `bash scripts/cargo-agent.sh test -p <crate> --all-targets --no-fail-fast` |
| **Merge to dev** | Full workspace | `bash scripts/cargo-agent.sh test --workspace --no-fail-fast` (only on PR to dev) |

**Do NOT run `bash scripts/cargo-agent.sh test --workspace` during feature-branch work.**
Use `bash scripts/cargo-agent.sh check --workspace --all-targets` to verify cross-crate compatibility.
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
Never include local AI tool names. Do not add automated co-author trailers.
