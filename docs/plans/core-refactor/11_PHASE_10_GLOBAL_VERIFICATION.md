# Phase 10 Global Verification and Crate-Split Review Plan

> **For DeepSeek:** Execute this plan task-by-task. Do not reinterpret the architecture or combine stages. Check each box only after its evidence exists.

**Goal:** Prove every architecture requirement with repository-wide evidence, close compatibility debt, and decide from measured data whether physical crate splits are justified.

**Architecture:** The single integration owner runs all workspace-wide checks serially, compares Phase 0 metrics to final metrics, audits each acceptance criterion, and records an evidence-backed keep/split decision.

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

## Prerequisites and ownership

Prerequisite: Phase 9 and all selected Phase 8 work packages complete. This is the only plan whose owner may run workspace-wide Cargo commands.

Owned outputs:

- Update all `config/architecture/` inventories and metrics
- Create `docs/arch/CORE_REFACTOR_COMPLETION_REPORT.md`
- Update `docs/arch/CORE_ARCHITECTURE_DECOUPLING_REFACTOR_PLAN.md` status only after proof
- Remove zero-count compatibility entries
- No feature implementation unless fixing a discovered verification failure in a separate commit

## Task 1: Requirement-by-requirement audit

Build a table covering all §16 acceptance criteria with:

```text
criterion | authoritative evidence | command/path | result | remaining risk
```

- [x] Fabric contains no provider-specific shared contracts.
- [x] Application/domain dependency directions pass.
- [x] Goal/Agent Control are coding-runtime-name neutral.
- [x] Cognit is channel neutral.
- [x] Memory core is product neutral.
- [x] Hardware core is ROS/vendor neutral.
- [x] New provider addition does not require core enum modification.
- [x] Integration failures normalize without provider-text parsing.
- [x] Config/secret/adapter construction ownership is centralized.
- [x] Public implementation trees/imports are closed.
- [x] Legacy data/config/API behavior is compatible or explicitly rejected.
- [x] State machines have unique owners and transition evidence.

## Task 2: Metrics delta

Compare frozen Phase 0 and final values:

- core external-product hits;
- public impl/adapter exports;
- cross-crate impl references;
- forbidden infrastructure dependencies;
- provider-name/URL/error-text business branches;
- Fabric provider-specific types;
- compatibility entries and call counts;
- largest stateful modules and mutation entry points;
- package compile/test timings where reproducible.

Every target marked `-> 0` must equal zero. A non-zero value requires an explicit architecture-design amendment, not an undocumented waiver.

## Task 3: Serial validation lanes

Run in this order, never concurrently:

```bash
bash scripts/architecture-check.sh
bash tests/architecture_check.sh
bash tests/architecture_path_inventory.sh
bash tests/operations_cli_static_test.sh
bash tests/systemd_runtime_boundary.sh
bash scripts/cargo-agent.sh fmt --all -- --check
bash scripts/cargo-agent.sh check --workspace --all-targets
bash scripts/cargo-agent.sh test --workspace
bash scripts/cargo-agent.sh clippy --workspace --all-targets -- -D warnings
bash scripts/cargo-agent.sh doc --workspace --no-deps
```

Record duration, result, and log/artifact location for every command. A broad green workspace test does not replace the architecture audit table.

## Task 4: Compatibility and security evidence

- [x] Run every legacy config fixture through canonical normalization.
- [x] Run persistence migration fixtures including failure/retry/idempotency.
- [x] Run credential redaction, OAuth scope, sandbox/network, workspace trust, lease, emergency stop, and provider-unavailable fail-closed tests.
- [x] Verify optional absent integrations degrade explicitly and invalid configured integrations fail.
- [x] Verify old protocol versions are accepted or rejected exactly as inventories specify.

## Task 5: Crate-split review

For Fabric and Executive separately record:

```text
current dependency fan-in/fan-out
incremental build cost
public API size
ownership clarity
release/versioning need
cycle risk
candidate split and migration cost
```

Decision options:

- `keep`: internal layering provides sufficient isolation;
- `split-later`: evidence supports a split but it is a separate approved project;
- `split-now`: allowed only through a new design and implementation plan, never appended to this phase.

## Task 6: Completion report and final commit

- [x] Completion report links every evidence artifact.
- [x] Architecture design status changes from design baseline to implemented only if all criteria are proven.
- [x] Remove zero-count allowlist/compatibility entries.
- [x] Confirm `git status --short` contains no unowned changes.

Commit:

```text
docs(arch): record core refactor completion evidence
```

The commit body must summarize metrics deltas, validation lanes, compatibility/security evidence, and crate-split decisions.
