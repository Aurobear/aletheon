# Aletheon Architecture Coupling Diagnosis

**Date:** 2026-08-16
**Branch at diagnosis:** `fix/tui-live-agent-inspector`
**Status:** accepted for execution — P0–P2 only; Finding F remains `NEEDS EVIDENCE`
**Source closeout:** P0–P2 implemented and source-validated on 2026-08-16; installed-runtime acceptance was not run and Finding F remains out of scope
**Execution packet:** [`docs/plans/2026-08-16-architecture-coupling-closeout.md`](../../plans/2026-08-16-architecture-coupling-closeout.md) (P0–P2 only)
**Review outcome:** Findings A–E `AGREE`; Finding F `NEEDS EVIDENCE`; P0/P1/P2 sequence `AGREE`
**Audience:** implementer and later reviewer. Re-read cited files before executing or revising a claim.
**Scope:** long-running operation, coupling, and module responsibility after the Executive / Fabric retirement.

This file is the accepted diagnosis, not the execution packet. Execute only the
linked P0–P2 closeout, one reviewed slice at a time. Finding F remains an
evidence requirement and must not become an implementation phase. This file
does not add requirements beyond the cited sources.

Sections 3–10 preserve the diagnosis-time evidence snapshot and therefore still
show the pre-closeout code facts. For the post-closeout source map, use
`docs/design/architecture-overview.md` and the living ledgers under
`config/architecture/`; do not reinterpret the historical locators below as
current-code claims.

---

## 0. How to review this document

For every claim in sections 3–8:

1. Re-open the cited `path:line`.
2. Re-run the listed verification command if one is given.
3. Mark the claim `AGREE` / `DISAGREE` / `NEEDS EVIDENCE`.
4. If code and this document disagree, treat the code as authority and record
   the delta. Do not silently pick a side.

Claims marked **INFERENCE** must not drive a cutover. They are hypotheses.

Out of scope for this review:

- Whether the TUI live-agent-inspector branch should merge
- New crate splits
- Installed-runtime acceptance
- Product questions (how autonomous the agent should be)

---

## 1. One-sentence judgment

The decoupling *direction* is correct. The current risk is not “missing layers”.
It is that Executive was physically removed, while production use cases, I/O,
and architecture ledgers still behave as if a god-crate composition root exists
— now named `aletheon`.

```text
Intended                                 Observed
--------                                 --------
host / composition                       aletheon (~81k LOC, 235 *.rs)
        |                                     |
        v                                     +-- wiring/application   real use cases
 application ---- ports                       +-- wiring/adapters      Google / Gmail / Pi / GBrain
        |                                     +-- wiring/daemon        process / RPC / bootstrap
        v                                     +-- wiring/composition   second assembly path
 domain / contracts                      crates/application (~6.5k); selected production use cases
                                          are live, but ApplicationFacade has no production caller
```

---

## 2. What is already standing — do not regress

These are confirmed and should remain constraints, not reopen as design debates.

| Fact | Evidence |
|---|---|
| Workspace members no longer include `executive` or `fabric` | `Cargo.toml:3-24` |
| `aletheon` is the composition root and depends on the domain crates | `crates/aletheon/Cargo.toml:18-32` |
| Daemon turn production caller is `TurnPipeline` | `architecture-status.toml:7-12`; constructed at `crates/aletheon/src/wiring/daemon/bootstrap/services.rs:990` |
| Runtime owns Session / Turn / Agent lifecycle authority | `architecture-status.toml:47-53`; `config/architecture/runtime-authority-census.tsv:9-24` |
| Provider-name branching and Fabric provider-specific types are ratcheted to zero | `config/architecture/metrics.env:4-10` |
| Compatibility debt is reduced to two persisted ExternalEvent v1 aliases | `config/architecture/compatibility-debt.tsv:3-4` |
| Unique mutation entries exist for five large state machines | `config/architecture/state-machine-inventory.tsv:2-6` |

Verify:

```text
rg -n 'executive|fabric' Cargo.toml
rg -n 'pub struct TurnPipeline' crates/aletheon/src/wiring/application/turn_pipeline.rs
rg -n 'ApplicationFacade|DefaultApplicationFacade' crates --glob '*.rs'
```

---

## 3. Finding A — `aletheon` is the new Executive

### Claim

`crates/executive` retirement moved a god crate; it did not fully decompose it.
Selected pure use cases now live in `crates/application` and have production
callers, but `crates/aletheon/src/wiring/` still holds host, primary
orchestration, adapters, I/O, and a second composition path in one binary crate.

### Evidence

Line counts collected 2026-08-16 with `wc -l` / `find`:

| Surface | Lines | Files |
|---|---:|---:|
| `crates/aletheon/src` | 81872 | 235 |
| `crates/corpus/src` | 60305 | — |
| `crates/cognit/src` | 31662 | — |
| `crates/contracts/src` | 22016 | — |
| `crates/runtime/src` | 21824 | — |
| `crates/application/src` | 6511 | — |

Largest live files inside the composition root:

| File | Lines |
|---|---:|
| `crates/aletheon/src/wiring/application/turn_pipeline.rs` | 2524 |
| `crates/aletheon/src/wiring/application/agent_control/mod.rs` | 1780 |
| `crates/aletheon/src/wiring/daemon/server.rs` | 1748 |
| `crates/aletheon/src/wiring/daemon/bootstrap/request.rs` | 1558 |
| `crates/aletheon/src/wiring/daemon/bootstrap/services.rs` | 1061 |

Already extracted, with production callers (this is why “not decomposed” is
too strong):

- `DaemonLifecycleService` — `crates/aletheon/src/wiring.rs:141`
- `TransactionReviewService` / `SessionInputCoordinator` —
  `crates/aletheon/src/wiring/daemon/handler/ports.rs:44-45`

Directory responsibilities still inside `crates/aletheon/src/wiring/`:

```text
wiring/application/     turn, goal, agent_control, evaluation, conscious
wiring/adapters/        channel/gmail, google, gbrain, runtime/pi, session stores
wiring/daemon/          bootstrap, RPC handlers, server, protocol
wiring/composition/     second assembly, including TurnService
```

### Why it matters

A composition root may depend on many crates. It must not *implement* the
use-case layer, the adapter layer, and the daemon host in the same crate.
Same-crate “directory conventions” are not compiler-enforced boundaries.
A turn-semantics change currently spans daemon, exec, agent_control, and TUI
projection without a crate cut.

### Reviewer check

- Confirm `crates/executive` is absent from `Cargo.toml` members.
- Confirm the five files above still exist at comparable size.
- Reject the finding only if primary turn/goal/agent-control orchestration
  has also left `aletheon/src/wiring/application`. Selected
  `crates/application` callers alone are not a reject.

---

## 4. Finding B — two Application layers; production uses the unofficial one

### Claim

`crates/application` owns selected pure contracts and use cases that production
does call, but production does not call its top-level `ApplicationFacade`.
Primary turn, goal, and agent-control orchestration still lives under
`aletheon/src/wiring/application`, where concrete I/O also remains. The defect
is two inconsistent Application boundary stories, not an entirely unused crate.

### Evidence

Documented intent:

- `crates/application/src/lib.rs:1-7` — facade holds no repository and mints
  no core ID; depends only on `contracts` and `runtime`.
- `crates/application/src/use_case.rs:3-6,33-50` — `ApplicationFacade` is the
  Session/Turn/Delegate entry.

Production usage:

- `rg -n 'ApplicationFacade|DefaultApplicationFacade' crates --glob '*.rs'`
  returned only `crates/application/src/lib.rs` and
  `crates/application/src/use_case.rs` (the trait, the struct, and two
  in-crate tests). No production caller.
- Other `application::` surfaces do have production callers. Examples include
  `DaemonLifecycleService` (`crates/aletheon/src/wiring.rs:141`),
  `TransactionReviewService`, and `SessionInputCoordinator`
  (`crates/aletheon/src/wiring/daemon/handler/ports.rs:44-45`). These callers do
  not construct or enter `ApplicationFacade`.

Census already records the real owners and remaining I/O:

- Daemon turn owner is `TurnPipeline`
  (`config/architecture/application-use-case-census.tsv` APX-UC-01).
- Application-layer SQLite / `std::fs` / `Command` / `reqwest` are listed as
  APX-IO-01..04 in the same file, status `CLOSED`.

The census header does not define `CLOSED` as “concrete I/O has left the
application layer” (`application-use-case-census.tsv:4-5`). The current source
search above still returns live I/O, so the status cannot be used as evidence
that the boundary has converged.

### Responsibility split today

```text
crates/application                 selected pure contracts/use cases in production
                                   + unused top-level ApplicationFacade
aletheon/wiring/application        primary orchestration + policy + remaining I/O
```

### Why it matters

Two owners for one layer is the highest-cost responsibility defect. Later crate
splits will copy the confusion rather than remove it.

### Reviewer check

Re-run:

```text
rg -n 'ApplicationFacade|DefaultApplicationFacade' crates --glob '*.rs'
rg -n 'Connection::open|std::fs::|Command::new|reqwest' crates/aletheon/src/wiring/application --glob '*.rs'
```

Reject the finding only if living docs and production callers share one
Application owner story. Constructing `DefaultApplicationFacade` without
making it the turn/session owner is not enough.

---

## 5. Finding C — two Turn execution paths remain

### Claim

The repository documents a single `TurnEngine` production entry, but daemon and
CLI exec still execute through different orchestrators. `TurnService` is an
open compatibility seam that has not converged.

### Evidence

Authoritative status ledger (`architecture-status.toml:7-27`):

| Name | Role | Production caller | Status |
|---|---|---|---|
| `TurnPipeline` | daemon turn orchestrator | `DaemonTurnOrchestrator` | `production` |
| `TurnService` | CLI exec turn | `ExecSessionBuilder` | `internal_compatibility`, `converge_into = "SessionTurnEngine"` |
| `TurnCoordinator` | cancel / control shell | both | `production` |

Code:

- `TurnEngine` comment: “single production execution path”
  (`crates/aletheon/src/wiring/application/turn_engine.rs:1-2,14-21`).
- Daemon adapter: `DaemonTurnEngine` calls `TurnPipeline`
  (`daemon_turn_engine.rs:33-51`).
- Exec path: `ExecSessionBuilder` constructs `TurnService::new(...)`
  (`crates/aletheon/src/wiring/exec_session.rs:258-265`).
- `TurnService` is labeled “Compatibility facade over the canonical
  `TurnCoordinator`” (`wiring/composition/turn_service.rs:20-28`).
- `SessionTurnEngine` is named as the converge target in
  `architecture-status.toml:20` and mentioned as a string in
  `crates/aletheon/tests/daemon_turn_api_boundary.rs:117`. It is not a
  production type found by `rg -n 'struct SessionTurnEngine'`.

Documented reducer vs live orchestrator:

- `TURN_STATE_MACHINE.md:7-18` describes Admission → PreTurn → Cognitive →
  ToolLoop → PostTurn → Projection, with I/O behind ports.
- `turn_pipeline.rs:69-74` still bundles SelfField review, memory, hooks,
  tool setup, LLM selection, ReAct loop, event pumping, and post-turn
  settlement in one struct.

### Why it matters

Cancel, restart, and settlement can diverge between daemon and `aletheon exec`.
The `-m` flag already sends a single message through the daemon
(`crates/aletheon/src/main.rs:629-641`); the separate local path is the `exec`
subcommand (`crates/aletheon/src/main.rs:461-506`). For a long-running system,
two orchestrators are a correctness risk, not a style issue.

### Reviewer check

```text
rg -n 'TurnService::new|DaemonTurnEngine|impl TurnEngine' crates/aletheon/src --glob '*.rs'
rg -n 'struct SessionTurnEngine' crates --glob '*.rs'
```

Reject the finding only if exec now enters `TurnEngine` / `TurnPipeline` and
`TurnService` has no production constructor.

---

## 6. Finding D — domain crates still depend the wrong way

### Claim

Allowed workspace edges in `config/architecture-dependencies.txt` still encode
domain-to-domain and domain-to-runtime implementation coupling. Three high-cost
coupling groups are named below. One table row is an in-crate infrastructure
dependency, not a workspace crate edge; it belongs to the cognit group.

### Allowed edges that already contradict the intended layering

From `config/architecture-dependencies.txt:8-48`:

```text
cognit     -> contracts, dasein, runtime
dasein     -> contracts, corpus, kernel, runtime
mnemosyne  -> application, cognit, contracts, dasein, kernel, platform, runtime
corpus     -> agora, application, contracts, kernel, platform, runtime
```

Intended direction (from `CORE_REFACTOR_COMPLETION_REPORT.md:12-19` and
`docs/design/architecture-overview.md` layer table):

```text
contracts  <  kernel / runtime / cognit / dasein / corpus / mnemosyne
                   ^                                 ^
                   |        aletheon composes        |
                   +--------- application -----------+
```

### Three high-cost coupling groups

| Group | Kind | What crosses | Evidence |
|---|---|---|---|
| `dasein` → `corpus` | workspace crate edge | Self uses tool-security implementation | `crates/dasein/src/bridge/policy.rs:2` `use corpus::security::policy::{PolicyEngine, PolicyVerdict}`; `crates/dasein/src/bridge/loop_detector.rs:3` `use corpus::security::loop_detector::...`; `crates/dasein/Cargo.toml:12` |
| `cognit` → `runtime` | workspace crate edge | Cognition uses lifecycle compaction / event bus | `crates/cognit/src/harness/linear/mod.rs:64,99`; `crates/cognit/src/adapters/inference/pulse.rs:16`; `crates/cognit/Cargo.toml:11` |
| `cognit` infra deps | in-crate production dependency, not a workspace edge | Domain crate owns HTTP/DB/gRPC clients | `crates/cognit/Cargo.toml:28-33` (`reqwest`, `rusqlite`, `tonic`) |
| `mnemosyne` hub | workspace crate edges | Memory crate depends on cognition + application + kernel + runtime + platform unconditionally | `crates/mnemosyne/Cargo.toml:10-16`. Feature `cognitive-memory` is off by default (`:44-45`), but the `cognit` crate dependency is not optional. |

`corpus` public surface also remains a dump:

- ~60k LOC
- `crates/corpus/src/lib.rs:31-33` re-exports `drivers::*`, `security::*`,
  `tools::*`

### Why it matters

A domain crate that imports another domain’s implementation cannot be replaced
or tested at a port. This is the coupling that crate renames do not fix.

### Reviewer check

```text
rg -n 'use corpus::' crates/dasein/src --glob '*.rs'
rg -n 'use runtime::' crates/cognit/src --glob '*.rs'
rg -n 'cognit|application|kernel|runtime|platform' crates/mnemosyne/Cargo.toml
```

Reject an individual edge only if the import is test-only or behind a
non-default feature that production daemon does not enable. `cognit` is a
required `mnemosyne` dependency today; that is not feature-gated.

---

## 7. Finding E — architecture ledgers describe a crate that no longer exists

### Claim

Several active control-plane files still name `executive` and `fabric` as
owners or paths. Two additional TSVs mix current-looking data with an
Executive-era snapshot and are not consumed by the gate. Together they cannot
reliably describe or protect the current tree.

### Evidence

| Ledger / doc | Still says | Code fact |
|---|---|---|
| `docs/design/architecture-overview.md:25-37,99-100` | Executive orchestrates; `TurnEngine` lives in `crates/executive/src/application/turn_engine.rs` | No `executive` member; `TurnEngine` is in `crates/aletheon/src/wiring/application/turn_engine.rs` |
| `config/architecture/executive-layers.tsv` entire file | `crates/executive/src/...` layer map | Paths do not exist |
| `config/architecture/persistence-surfaces.tsv:3,9,21` | owner = `executive` for google-event-store, goal-store, session-read-model | Stores live under `aletheon/wiring` and `adapters/sqlite` |
| `config/architecture/config-ownership.tsv` | owner = `executive.composition` for all listed keys | Config lives in `crates/aletheon/src/config/` |
| `config/architecture/fabric-public-types.tsv` | consumers include `executive`; owner column says `fabric` | Crate is `contracts` |
| `architecture-status.toml:63-66` | reviewed dependency still starts from `executive` | No `executive` workspace member exists |
| `config/architecture/module-boundaries.txt:3-20` | live dependency snapshots still name `fabric` | Shared-contract crate is `contracts` |
| `docs/arch/CORE_REFACTOR_VERIFICATION_STATUS.md:9` | “retain Fabric and Executive as physical crates” | Neither crate is a workspace member |
| `docs/arch/PUBLIC_API_CONTRACTION_INVENTORY.md:20-25,54` | Executive facade and `cargo check -p executive` | Package gone |
| `config/architecture/hotspot-budgets.tsv` | largest listed hotspot `runner.rs` 2042 | Live largest file is `turn_pipeline.rs` 2524, not in the table |
| `config/architecture/state-machine-inventory.tsv:3` | `largest_io_module` = `crates/executive/src/application/turn_pipeline.rs` | File is gone; live module is `crates/aletheon/src/wiring/application/turn_pipeline.rs` |
| `docs/design/roadmap/open-questions.md:33` | `crates/fabric/src/security/loop_detector.rs` | Implementation is under `corpus` (`dasein` imports `corpus::security::loop_detector`) |

Many TSV files still carry `frozen_commit=` from the Executive-era tree.

`scripts/libexec/aletheon/architecture-check.sh` does **not** read
`executive-layers.tsv` or `state-machine-inventory.tsv`. Those files are
therefore not protected by `bash scripts/aletheon.sh acceptance architecture`.
P0 treats `executive-layers.tsv` as a historical retirement snapshot and
`state-machine-inventory.tsv` as a living but ungated inventory; each header
must say so explicitly.

### Why it matters

For a long-running architecture program, a false map is worse than no map.
Reviewers and future slices will optimize a ghost crate. Architecture checks
that still mention `executive::host` as a negative pattern cannot see
`aletheon::wiring`. Passing architecture acceptance does not prove these two
inventories are current.

### Reviewer check

```text
rg -n 'crates/executive|executive\.composition|crates/fabric' \
  docs/design/architecture-overview.md \
  config/architecture \
  docs/arch/CORE_REFACTOR_VERIFICATION_STATUS.md \
  docs/arch/PUBLIC_API_CONTRACTION_INVENTORY.md
test -d crates/executive; echo $?
```

Reject the finding only if those ledgers have already been rewritten to
`aletheon` / `contracts` owners and the hotspot table includes
`turn_pipeline.rs`.

---

## 8. Finding F — long-running closeout status is mixed

### Claim

Always-on operation requires one writer per durable fact, one turn settlement
path, and machine-scoped provider backpressure. The dual turn path remains a
confirmed risk and the persistence owner map is stale. Machine-scoped provider
backpressure and the cited monitor repairs exist in current source, but this
static diagnosis has no installed-runtime evidence that closes their acceptance
criteria or proves that every production caller uses them.

### Evidence

**Persistence owners are stale; duplicate writers are not established here.**

`config/architecture/persistence-surfaces.tsv` lists distinct stores for
agent-runs, agent-messages, recovery, runtime-processes, settlement receipts,
goals, google events, gbrain spool, consolidation, episodic, semantic,
memory-ops, meta-runtime, oauth vault, MCP tokens, artifacts, HIL evidence,
robot episodes, and session read-model. Several `owner` cells still say
`executive` (`:3,9,21`).

Census APX-UC-11 / APX-UC-13 additionally name `self_field.db` and
`evolution-proposals.db`. Those are different durable facts; their count alone
does not prove that one fact has multiple writers. The current metric records
`SESSION_APPEND_WRITERS=1` (`config/architecture/metrics.env:14`). Stale owner
cells are therefore confirmed ledger drift (Finding E), while any duplicate
writer claim needs table- and writer-specific evidence.

**Machine-scoped provider backpressure exists statically; production-wide
acceptance is not established here.**

The July backlog still describes RC-001 as open
(`docs/design/roadmap/runtime-correctness-backlog.md:17-33`), but current code
now contains:

- a dependency-neutral machine/provider admission contract
  (`crates/contracts/src/include/memory.rs:40-60`);
- process-global provider state in the machine-core process
  (`crates/cognit/src/adapters/inference/backpressure.rs:27-52,194-224`);
- socket-backed provider permit acquisition
  (`crates/aletheon/src/wiring/core_rpc/client.rs:74-102`); and
- daemon embedding integration through the same inference authority
  (`crates/aletheon/src/wiring/daemon/bootstrap/request.rs:657-669`).

That is implementation evidence, not installed closeout. This diagnosis did not
run the concurrent main-session, multi-session, and external-runtime acceptance
required to prove that no production caller bypasses the machine boundary in
`AGENTS.md:68-70`.

**Restart / settlement behavior may diverge, but the behavioral delta is not
shown.**

- Turn reducer doc: restart recovers through checkpoint, does not replay
  in-memory state (`TURN_STATE_MACHINE.md:23-24`).
- Exec path uses `TurnService`, which is not that reducer
  (`exec_session.rs:258-265`, Finding C).
- Agent recovery, session shadow, settlement receipts, and workspace
  checkpoints have separate owners in `runtime`, `aletheon/wiring`, and
  `adapters/sqlite` (see `runtime-authority-census.tsv` RA-S-14..18 and
  `persistence-surfaces.tsv` agent-recovery / agent-settlement-receipts).

The first two bullets establish two execution paths. Separate components in the
third bullet do not by themselves prove conflicting recovery or settlement
semantics. Until a differing transition, durable receipt, or restart outcome is
shown, the behavioral-divergence part remains **NEEDS EVIDENCE**.

**The cited operator-signal implementations have changed.**

The July backlog says monitor health queried the wrong systemd scope and TUI
settle could time out after `turn_done` (RC-002 / RC-003). Current health code
selects user or system scope from the socket and reads `NRestarts`
(`tools/aletheon-monitor/src/tools/health.py:62-97`). Current TUI settlement
joins durable `turn_done`, a stable frame, a visible prompt, and absence of a
busy state (`tools/aletheon-monitor/src/tools/tui.py:233-264`). These changes
remove the cited static defects. This diagnosis still lacks installed regression
evidence for their closeout criteria.

### Why it matters

Long-running correctness is not “the process stays up”. It is: no duplicate
side effects after restart, no cross-session bleed, no provider stampede, no
false operator restart. Current static code narrows several risks, but it does
not replace installed acceptance. Conversely, an old open backlog item must not
be reported as a current implementation absence when the source has changed.

### Reviewer check

Mark this finding **NEEDS EVIDENCE**. Keep the checks in this section. Do not
promote them into a P3 execution packet. A later installed closeout may close
the finding; this diagnosis must not invent that work order.

Do not infer duplicate persistence writers from the number of distinct stores.
Do not infer restart divergence from separate component ownership without a
differing durable outcome.

Search before disagreeing:

```text
rg -n 'RC-001|machine-wide|MachineProviderBackpressure|acquire_provider_permit' \
  docs crates --glob '*.{md,rs,toml}'
rg -n 'systemctl_scope|NRestarts|tui_wait_turn_done|completion_source' \
  tools/aletheon-monitor/src/tools/{health,tui}.py
```

---

## 9. Vocabulary collisions that hide owners

These are naming defects that increase false dependencies. Confirmed by crate
and module names; no extra runtime evidence required.

| Token | Distinct meanings now in tree |
|---|---|
| `runtime` | `crates/runtime` (Session/Agent/Turn authority); `kernel::runtime::KernelRuntime`; `aletheon/wiring/adapters/runtime` (Pi / native-cognit / provider worker); leftover `user_runtime` composition |
| Turn entry | `TurnEngine`, `TurnPipeline`, `TurnService`, `TurnCoordinator`, planned `SessionTurnEngine` |

A reviewer should not accept a new type whose name is only “Runtime” or
“Turn*” without an owner prefix.

---

## 10. Recommended sequence — review this as priority, not as a work order

This section is a suggested order. It is not a commitment to implement, and it
does not invent features.

### P0 — make the map equal the territory

1. Rewrite `docs/design/architecture-overview.md` and owner columns in
   living `config/architecture/*.tsv` so `executive` / `fabric` are historical
   only.
2. Mark `executive-layers.tsv` as a historical retirement snapshot. Mark
   `state-machine-inventory.tsv` living/ungated and remap its stale TurnPipeline
   row; `:3` still points at deleted
   `crates/executive/src/application/turn_pipeline.rs`.
3. Put `turn_pipeline.rs` on `hotspot-budgets.tsv`. Freeze new responsibilities
   from entering it while it is the largest live file.
4. Keep `TurnService → TurnEngine` as the single open converge item in
   `architecture-status.toml`. Do not open a parallel crate-split.

### P1 — one Turn, one Application

5. Production has one turn entry: `TurnEngine`. `TurnService` becomes a thin
   adapter or is deleted. Daemon and `aletheon exec` must share the same reducer;
   `aletheon -m` already enters the daemon path.
6. Choose exactly one:
   - Move pure use cases (goal state, approval decision, turn policy) from
     `wiring/application` into `crates/application`, leave I/O in adapters; or
   - Define `crates/application` as the narrow pure-contract/use-case owner,
     remove the unused `ApplicationFacade`, and explicitly leave host
     orchestration in `aletheon/wiring/application`.
   Do not keep two undocumented ownership stories.

### P2 — cut the three high-cost coupling groups

7. `dasein` → `corpus` becomes a `contracts` policy port.
8. `cognit` → `runtime` compaction / event-bus becomes a port;
   `rusqlite` / `reqwest` leave cognit production dependencies.
9. `mnemosyne` → `cognit` / `application` becomes optional/feature or a port.
   Default daemon graph must not require them if the feature is off.

Finding F stays **NEEDS EVIDENCE**. Its checks (machine-permit caller census,
one differing restart outcome, no duplicate-writer inference from store count)
remain in §8. They are not P0–P2 work items.

### Explicitly not recommended now

- Splitting `aletheon` into a new `executive`-shaped crate.
- Splitting `corpus` / `runtime` by line count into many new crates.
- Adding census rows marked `CLOSED` instead of moving code.
- Reopening provider-name / Fabric-type work that `metrics.env` already holds
  at zero.

---

## 11. Claims the author is *not* making

- That Phase 10 / XRET-05 / CGP-08 work should be reverted.
- That `aletheon` depending on domain crates is itself a layering violation.
  Composition-root fan-out is expected (`architecture-status.toml:47-53`).
- That file length alone is a defect. Length is a hotspot signal only when
  the file also owns state, policy, I/O, and lifecycle together
  (`CORE_ARCHITECTURE_DECOUPLING_REFACTOR_PLAN.md:107` principle; this
  diagnosis applies that test to `turn_pipeline.rs`).
- That installed-runtime acceptance was run for this diagnosis. It was not.
- That the current branch’s TUI inspector change is good or bad. Not reviewed.

---

## 12. Reviewer report template

Recorded review outcome is shown in the header. Reuse this template only if
later code changes invalidate a finding:

```text
Verdict: AGREE | AGREE WITH FIXES | REJECT

Finding A  aletheon-is-new-executive          AGREE/DISAGREE/NEEDS EVIDENCE
Finding B  dual-application                   AGREE/DISAGREE/NEEDS EVIDENCE
Finding C  dual-turn-paths                    AGREE/DISAGREE/NEEDS EVIDENCE
Finding D  reverse-domain-edges               AGREE/DISAGREE/NEEDS EVIDENCE
Finding E  ledger-drift                       AGREE/DISAGREE/NEEDS EVIDENCE
Finding F  long-running-closeout-mixed        AGREE/DISAGREE/NEEDS EVIDENCE

P0/P1/P2 sequence                             AGREE/REORDER/REJECT
If REORDER, write the new order and why. P3 is not an execution phase.

Disagreements (claim, cited path, code reality, proposed correction):
- ...

Evidence this diagnosis missed:
- ...
```

Do not add work items that are not implied by an AGREE finding.
Do not propose a crate split unless Findings A–C are rejected *and* a
release/ownership/cycle reason is cited.

---

## 13. Source snapshot

| Item | Value |
|---|---|
| Diagnosis date | 2026-08-16 |
| Workspace path | `/home/aurobear/Workspace/aletheon` |
| Branch | `fix/tui-live-agent-inspector` |
| Method | static read of workspace Cargo.toml, crate lib/Cargo files, architecture ledgers, and the cited production symbols |
| Not run | `scripts/aletheon.sh deploy`, workspace test, real-TUI acceptance |

Primary sources:

- `architecture-status.toml`
- `config/architecture/module-boundaries.txt`
- `config/architecture-dependencies.txt`
- `config/architecture/application-use-case-census.tsv`
- `config/architecture/persistence-surfaces.tsv`
- `config/architecture/hotspot-budgets.tsv`
- `config/architecture/state-machine-inventory.tsv`
- `config/architecture/executive-layers.tsv`
- `config/architecture/metrics.env`
- `docs/design/architecture-overview.md`
- `docs/arch/CORE_REFACTOR_COMPLETION_REPORT.md`
- `docs/design/roadmap/runtime-correctness-backlog.md`
- `tools/aletheon-monitor/src/tools/health.py`
- `tools/aletheon-monitor/src/tools/tui.py`
- `crates/contracts/src/include/memory.rs`
- `crates/cognit/src/adapters/inference/backpressure.rs`
- `crates/cognit/src/ports/inference.rs`
- `crates/aletheon/src/wiring/core_rpc/client.rs`
- `crates/aletheon/src/wiring/core_rpc/server.rs`
- `crates/aletheon/src/main.rs`
- `crates/aletheon/src/wiring/application/turn_engine.rs`
- `crates/aletheon/src/wiring/application/turn_pipeline.rs`
- `crates/aletheon/src/wiring/composition/turn_service.rs`
- `crates/aletheon/src/wiring/exec_session.rs`
- `crates/application/src/use_case.rs`
- `crates/dasein/Cargo.toml`, `crates/cognit/Cargo.toml`, `crates/mnemosyne/Cargo.toml`
