# Core Refactor DeepSeek Execution Guide

> **For DeepSeek:** Execute this plan task-by-task. Do not reinterpret the architecture or combine stages. Check each box only after its evidence exists.

**Goal:** Provide the authoritative order, handoff contract, stop rules, and evidence format for executing every core-refactor phase.

**Architecture:** One master guide points to independent phase plans. Phase 0 freezes the baseline; later plans may execute only when their prerequisites and handoff evidence are committed.

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

## Document index and hard dependencies

| Order | Document | Hard prerequisite |
|---|---|---|
| 0 | `01_PHASE_0_ARCHITECTURE_BASELINE_AND_GATES.md` | current architecture design |
| 1 | `02_PHASE_1_FABRIC_CONTRACT_PURIFICATION.md` | Phase 0 |
| 2 | `03_PHASE_2_EXECUTIVE_LAYERING.md` | Phase 0 |
| 3 | `04_PHASE_3_CODING_RUNTIME_DECOUPLING.md` | Phase 2 |
| 4 | `05_PHASE_4_CONFIG_OWNERSHIP.md` | Phase 2 |
| 5 | `06_PHASE_5_SUPPLEMENTAL_MEMORY.md` | Phase 2 |
| 6 | `07_PHASE_6_CHANNEL_IDENTITY_SOURCES.md` | Phase 1 + Phase 2 |
| 7 | `08_PHASE_7_INFERENCE_ADAPTERS.md` | Phase 2 |
| 8 | `09_PHASE_8_STATE_MACHINES.md` | per-subtask prerequisites inside that plan |
| 9 | `10_PHASE_9_PUBLIC_API_CONTRACTION.md` | Phases 1–7 |
| 10 | `11_PHASE_10_GLOBAL_VERIFICATION.md` | Phase 9 and completed Phase 8 tasks |

Phases 3, 4, 5, and 7 may be assigned independently only after Phase 2 is merged. Phase 6 additionally waits for Phase 1. Never execute two plans in the same worktree at the same time.

## Required DeepSeek task report

For every task, return exactly:

```text
STATUS: complete | blocked | failed
PLAN: <plan filename>
TASKS_COMPLETED: <checkbox numbers>
REQUIREMENT_ANCHORS: <doc section/line anchors re-read>
CODE_ANCHORS: <path:line anchors re-read>
CHANGED_FILES: <one path per line>
VALIDATION: <command => result>
ARCHITECTURE_EVIDENCE: <gate/inventory delta>
COMPATIBILITY_EVIDENCE: <old input => canonical behavior>
SECURITY_EVIDENCE: <applicable fail-closed checks>
COMMITS: <hash and subject>
FAILURES_OR_FOLLOWUPS: <none or exact issue>
```

## Stop rules

Stop without editing further when:

1. Requirement text and current code disagree about existing behavior.
2. A required persistence/wire owner is absent from Phase 0 inventories.
3. A task needs paths outside its declared ownership.
4. A compatibility migration cannot preserve existing data or explicitly reject it.
5. A security test would require weakening fail-closed behavior.
6. Another uncommitted change overlaps an owned file.
7. A required narrow validation fails for a reason outside the task.

## Per-task work loop

- [ ] Re-read architecture sections and exact current symbols.
- [ ] Confirm prerequisites and a clean/non-overlapping owned path set.
- [ ] Write the narrow failing test or gate fixture.
- [ ] Run it and capture the expected failure.
- [ ] Implement the smallest architecture-aligned change.
- [ ] Run narrow tests and architecture checks.
- [ ] Update inventories, compatibility tables, and metrics.
- [ ] Inspect/stage only owned files and commit.
- [ ] Produce the required report.

## Branch and commit discipline

Use one branch per phase or Phase 8 subtask. Do not squash distinct migration stages. Recommended commit sequence:

```text
test(...): lock <boundary> contract
refactor(...): introduce <canonical port/model>
refactor(...): migrate <callers/adapters>
chore(arch): enforce <boundary> gate
docs(arch): record <inventory/compatibility> evidence
```
