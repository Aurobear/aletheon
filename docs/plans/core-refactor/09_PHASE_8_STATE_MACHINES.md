# Phase 8 High-Risk State Machine Decomposition Plan

> **For DeepSeek:** Execute this plan task-by-task. Do not reinterpret the architecture or combine stages. Check each box only after its evidence exists.

**Goal:** Decompose large stateful modules by unique state owner and event transitions only after their ports are stable.

**Architecture:** Phase 8 is five independent work packages. Each begins with a state/event/side-effect model, locks behavior with characterization tests, extracts pure transition logic, then isolates I/O ports.

**Tech Stack:** Rust 1.85+, Bash, Python 3, Cargo via `scripts/cargo-agent.sh`, repository architecture gates.

---

## Global execution constraints

- Treat `docs/arch/CORE_ARCHITECTURE_DECOUPLING_REFACTOR_PLAN.md` as the architecture source of truth.
- Re-read that document and every cited symbol before editing; record changed line anchors in the task report.
- Do not modify files outside the declared paths. Stop if a required change crosses the boundary and report it.
- Preserve unrelated working-tree changes. Never use `git reset --hard`, `git checkout --`, or broad cleanup commands.
- Never invoke Cargo directly. Use `bash scripts/cargo-agent.sh <cargo arguments>` and the narrowest package/test target.
- Do not run concurrent Executive or workspace builds. Only the final integration owner runs workspace-wide commands.
- Keep security-sensitive behavior fail-closed. Do not weaken credential, scope, sandbox, network, lease, or trust checks.
- Each non-trivial commit must use a conventional subject, blank line, problem/solution context, and concrete bullets.
- Before each commit run `git diff --cached --check` and inspect the complete staged diff.
- A task is incomplete if tests pass but its architecture gate, compatibility evidence, or inventory update is missing.

## Dependency map

| Work package | Hard prerequisite | Primary module |
|---|---|---|
| 8a Agent Control | Phase 2 + Phase 3 | `executive/src/service/agent_control/` or canonical application path |
| 8b Turn Pipeline | Phase 2 | `executive/src/service/turn_pipeline.rs` and turn application paths |
| 8c Mnemosyne Service | Phase 5 | `mnemosyne/src/service.rs` and canonical application path |
| 8d MCP Client/Auth | corresponding MCP adapter boundary stable | `corpus/src/tools/mcp/client.rs`, `auth.rs` |
| 8e Daemon Server | Phase 2 | Executive canonical host/daemon path |

Never combine two work packages in one commit series.

## Standard state-machine method

For each package:

- [ ] Write `STATE_MACHINE.md` beside the module or under its plan evidence directory.
- [ ] List states, accepted events, guards, transitions, side effects, terminal states, idempotency keys, persistence points, cancellation, timeout, and recovery.
- [ ] Add characterization tests for every existing transition and invalid transition.
- [ ] Extract a pure reducer returning `Transition { next_state, effects }` or an equivalent typed result.
- [ ] Keep I/O in an effect executor behind ports.
- [ ] Ensure there is exactly one mutation entry point.
- [ ] Add crash/restart and repeated-event tests where persistence exists.
- [ ] Update architecture inventory and largest-module metrics.

## Work package 8a: Agent Control

Owned responsibilities:

```text
spawn/admission -> running -> settlement -> terminal
messaging/follow-up/steer/cancel/wait
persistence and restart recovery
```

- [ ] Separate admission, lifecycle, messaging, settlement, runtime port, persistence port, and errors.
- [ ] Preserve identity depth, parent validation, budget/lease/storage reservation, cancellation, timeout, and settlement exactly-once behavior.
- [ ] No runtime-name policy branch.

Validation:

```bash
bash scripts/cargo-agent.sh test -p executive agent_control
bash scripts/cargo-agent.sh test -p executive --test agent_control_repository
bash scripts/cargo-agent.sh test -p executive --test agent_recovery
```

## Work package 8b: Turn Pipeline

- [ ] Model admission, pre-turn, cognitive execution, tool loop, post-turn, projection, completion/error/cancel states.
- [ ] Preserve event ordering, token ownership, streaming, compaction, cancellation, parity, and post-turn projections.
- [ ] Remove bypass paths that call concrete harness/tool/memory implementations.

Validation:

```bash
bash scripts/cargo-agent.sh test -p executive --test turn_pipeline_order
bash scripts/cargo-agent.sh test -p executive --test turn_engine_parity
bash scripts/cargo-agent.sh test -p executive --test conscious_action_outcome
```

## Work package 8c: Mnemosyne Service

- [ ] Model local write, projection, recall, supplemental recall/write, reconciliation, retention, and degraded states.
- [ ] Preserve local-memory availability when optional supplemental transport is absent.
- [ ] Isolate repository and supplemental transport effects.

Validation:

```bash
bash scripts/cargo-agent.sh test -p mnemosyne
bash scripts/cargo-agent.sh test -p mnemosyne --test unified_memory_contract
```

## Work package 8d: MCP Client/Auth

- [ ] Separate connection lifecycle, discovery generation, request routing, notifications, reconnect, elicitation, OAuth/token lifecycle, and shutdown.
- [ ] Keep all background tasks under `McpTaskSupervisor`.
- [ ] Preserve endpoint scoping, SSRF/network policy, credential release, size limits, and fail-closed elicitation.

Validation:

```bash
bash scripts/cargo-agent.sh test -p corpus mcp
bash scripts/cargo-agent.sh test -p corpus oauth
bash tests/architecture_check.sh
```

## Work package 8e: Daemon Server

- [ ] Separate listener/accept, authentication, client protocol negotiation, request dispatch, subscriptions, shutdown, and child/task supervision.
- [ ] Preserve protocol version rejection, frame limits, socket permissions, systemd activation, and graceful shutdown.
- [ ] Host calls application ports only.

Validation:

```bash
bash scripts/cargo-agent.sh test -p executive daemon
bash tests/systemd_runtime_boundary.sh
bash scripts/verify-systemd.sh --user-units
```

## Commit stages per work package

1. `test(<module>): characterize lifecycle transitions`
2. `refactor(<module>): extract pure state transitions`
3. `refactor(<module>): isolate transition side effects`
4. `chore(arch): enforce single state owner for <module>`
