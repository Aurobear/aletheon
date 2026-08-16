# RA05-F1 AgentControl rich-facade audit

Date: 2026-08-12
Scope: audit only; no Agent production behavior changed
Requirement: `docs/plans/2026-08-12-migration-closeout-execution-plan.md §9.1`

## Context receipt

Runtime is already the production identity, generation, lifecycle journal and
terminal-fence owner. Production constructs `RuntimeAgentSupervisor`, replays
its durable stream, and injects it into `AgentControlService::new_runtime_only`
(`crates/aletheon/src/wiring/daemon/bootstrap/services.rs:474-518`). The rich
Executive service remains in the production path because Runtime delegates
concrete process, mailbox and projection effects back through
`RuntimeObservedAgentBackend` (`crates/executive/src/application/agent_control/runtime_bridge.rs:20-29`).

`RA-A-10` therefore remains `INVESTIGATE:RA-05`; this audit does not claim
caller-zero or close the ledger row.

## Field ownership classification

| Field(s) | Classification | Evidence / production caller |
|---|---|---|
| `runtime_agent_supervisor` | `RUNTIME_AUTHORITY` | Runtime selects, mints, journals, generation-fences, sends, waits and cancels; Executive forwards at `agent_control/mod.rs:1309-1428,1431-1557`. |
| `runtimes` | `COMPAT_TEST_ONLY` | Documented compatibility-only catalog at `agent_control/mod.rs:96-100,122-149`; production calls `new_runtime_only` at `services.rs:506`. |
| `kernel`, `runtime_process_supervisor`, `live`, `tasks` | `HOST_ADAPTER` | Concrete process admission/binding, live cancellation and task draining remain host effects (`agent_control/mod.rs:90,108-109,115`; `runtime_bridge.rs:321-364`). |
| `repository`, `event_projections` | `HOST_ADAPTER` | SQLite/read-model projection and compatibility snapshots; production wraps it with `RuntimeAgentRunProjection` at `services.rs:497-504`. It must not become a second lifecycle authority. |
| `event_spine`, `events` | `HOST_ADAPTER` | Durable sink/presentation publication supplied by composition; Runtime owns event decisions and calls the injected sink (`services.rs:477-480`). |
| `admission`, `budget_controller` | `APPLICATION_POLICY` | Host policy and Kernel resource admission, not Runtime lifecycle mutation (`agent_control/mod.rs:93,112`). |
| `agent_profiles`, `agent_profile_catalog`, `runtime_profile_requirements`, `capability_history` | `APPLICATION_POLICY` | Resolve profile/capability constraints before Runtime selection (`agent_control/mod.rs:116-120,1271-1348`). |
| `cognitive_task_admission`, `topology_routes` | `APPLICATION_POLICY` | Agora/task and communication policy; not lifecycle authority (`agent_control/mod.rs:110,121`). |
| `agent_memory_vault`, `durable_memory` | `HOST_ADAPTER` | Concrete memory effects owned outside the Runtime writer (`agent_control/mod.rs:111-112`). |
| `timer` | `HOST_ADAPTER` | Bounded host wait implementation; Runtime exposes the authoritative wait/generation contract (`agent_control/mod.rs:106`). |
| `settlement_generation`, `settlement_receipts`, `settlement_metrics`, `lifecycle_hooks` | `HOST_ADAPTER` | Compatibility receipt/resource cleanup and external hooks. They may observe/project Runtime settlement but must not mint or reverse it (`agent_control/mod.rs:113-116`). |

## Method-path classification

| Method/path | Classification | Result |
|---|---|---|
| `spawn_intent` profile resolution and selection request construction | `APPLICATION_POLICY` | Keep outside Runtime writer; pass one typed request to Runtime (`agent_control/mod.rs:1267-1407`). |
| `spawn` / `spawn_via_runtime` | `RUNTIME_AUTHORITY` plus `HOST_ADAPTER` | Runtime mints identity and calls the pinned backend; Executive only admits/launches the supplied identity (`spawning.rs:3-56`, `runtime_bridge.rs:51-92`). |
| `wait`, `send`, `cancel` public port methods | `RUNTIME_AUTHORITY` facade | Production already calls generation-fenced Runtime APIs, then reads the host projection (`agent_control/mod.rs:1431-1557`). |
| `wait_local`, `send_local`, `cancel_local` | `HOST_ADAPTER` | Backend implementations invoked by `RuntimeObservedAgentBackend`; they operate on an already-authorized Runtime run (`runtime_bridge.rs:170-289`). |
| `inspect`, `list` | `HOST_ADAPTER` read facade | Projection queries only; no lifecycle mutation (`agent_control/mod.rs:1560-1578`). |
| `reconcile_startup` / `StartupRecoveryHost` | `RUNTIME_AUTHORITY` decision plus `HOST_ADAPTER` observation/effect | Runtime recovery coordinator decides; Executive observes Kernel/checkpoints and applies concrete cleanup (`agent_control/mod.rs:165-327,571-756`). |
| sibling routing, broadcast authorization, profile/admission helpers | `APPLICATION_POLICY` | Remain host policy behind narrow ports (`agent_control/mod.rs:758-1055`). |
| `new_legacy` and fallback branches without a supervisor | `COMPAT_TEST_ONLY` | No production caller; retained rollback/test surface (`agent_control/mod.rs:332-405`, `spawning.rs:12-14`). |

## Caller census

Production callers:

- composition constructs and injects the rich facade:
  `crates/aletheon/src/wiring/daemon/bootstrap/services.rs:450-680`;
- Corpus publishes `agent_spawn`, `agent_wait`, `agent_send`, `agent_cancel`,
  and `agent_list` through `AgentControlPort`:
  `crates/aletheon/src/wiring/daemon/bootstrap/runtime.rs:361-377`;
- Memory Agent and extension publication retain the same injected facade:
  `crates/aletheon/src/wiring/daemon/bootstrap/request.rs:1149` and
  `crates/aletheon/src/wiring/daemon/bootstrap/extensions.rs:168`;
- Pi is a pinned host backend and keeps only a weak service reference:
  `crates/aletheon/src/wiring/adapters/runtime/pi_rpc.rs:57,137`.

Test/rollback callers use `new_legacy` and supervisor-less fallbacks. They are
not evidence that the production path has a second Runtime registry.

## Recommended single implementation slice

**RA05-F2: replace the recursive rich-service backend with a narrow
`AgentHostEffects` adapter.**

Today `RuntimeObservedAgentBackend` retains `Weak<AgentControlService>` and
re-enters private `wait_local`, `send_local`, `cancel_local`, repository and
launcher helpers. That cycle is the smallest remaining structural reason the
rich facade cannot become caller-zero:

```text
AgentControlService -> RuntimeAgentSupervisor
        ^                       |
        | RuntimeObservedAgentBackend
        +-----------------------+
```

The next slice should introduce one host-effects object containing only the
already-minted-identity operations needed by `DelegateBackend` and
`ObservedAgentBackend`. Runtime continues to own selection, identity,
generation, journal and terminal state. Do not move profile policy, Kernel,
SQLite or launchers into Runtime.

Allowed files:

- `crates/executive/src/application/agent_control/runtime_bridge.rs`
- `crates/executive/src/application/agent_control/mod.rs`
- `crates/executive/src/application/agent_control/spawning.rs`
- `crates/aletheon/src/wiring/daemon/bootstrap/services.rs`
- focused AgentControl/runtime bridge tests
- `config/architecture/runtime-authority-census.tsv` only after production
  caller evidence changes; keep `RA-A-10` open for this slice

Forbidden:

- changes to Session or Turn writers;
- copying `AgentControlService` into Runtime;
- changing Runtime identity/generation/terminal schemas;
- deleting legacy compatibility code or closing `RA-A-10` in this slice.

Focused validation:

```bash
bash scripts/cargo-agent.sh test -p executive --test agent_control_spawn
bash scripts/cargo-agent.sh test -p executive --test agent_control_operations
bash scripts/cargo-agent.sh test -p executive --test runtime_process_reconciliation
bash scripts/cargo-agent.sh test -p runtime --lib agent_writer
ARCH_SKIP_DEPENDENCIES=1 bash scripts/aletheon.sh acceptance architecture
git diff --check
bash scripts/cargo-agent.sh fmt --all -- --check
```

Installed acceptance is required because the slice changes daemon bootstrap,
Agent tools, Pi delegation and restart recovery.

## RA05-F2 implementation result

Every Runtime backend binding now accepts the narrow host-effects port rather
than `AgentControlService`: the generic observed backend constructors, pinned
native/package launchers, extension reload publication, and Pi binding all
carry `Arc/Weak<dyn AgentHostEffects>`. Composition performs the single rich
service-to-host-port coercion and preserves that erased handle in
`AgentServices`; extension reload no longer stores or upgrades a weak rich
facade. This removes concrete rich-facade knowledge from the Runtime backend
graph without changing Runtime identity, generation, journal, or terminal
ownership.

Focused validation passed:

- `agent_control_spawn`: 8/8;
- `agent_control_operations`: 6/6;
- `runtime_process_reconciliation`: 1/1;
- Runtime `agent_writer`: 24/24;
- architecture acceptance, formatting and diff checks.
- repository-owned changed validation: 91/91.

At the end of F2, `RA-A-10` remained open because public Agent tools still
consumed a concrete type named as a lifecycle service. F3 below records the
subsequent authority-name retirement and final state-machine ledger closure.

## RA05-F3 authority-name retirement and ledger closure

The remaining concrete type is now explicitly `AgentHostAdapter`. Production
and fixture composition construct that adapter; the obsolete
`AgentControlService` symbol and its `AgentControlPort` implementation are
caller-zero across the workspace. This is not a cosmetic alias: no alias is
retained, so a caller cannot continue treating the Executive object as the
Agent lifecycle service under the old authority name.

Production tools receive a separate `RuntimeAgentControlFacade`; the host
adapter no longer implements `AgentControlPort`. All lifecycle mutation reaches
`RuntimeAgentSupervisor`; its other work is
profile/admission policy, Kernel/process/mailbox effects, projection reads,
and rollback-fixture compatibility. `RuntimeObservedAgentBackend` and package
reloads retain only `AgentHostEffects`, not the concrete adapter.

Accordingly `RA-A-10` is closed as a duplicate **state-machine authority**.
The remaining host adapter is still owned by the RA-06 physical deletion
inventory and must not be copied into Runtime or mistaken for an empty
Executive crate.

Final validation selected 92 repository-owned changed steps and passed all
92, including the full workspace all-target check and architecture/script
gates. System deployment passed at SHA
`7cafd24442d0ff0efa2540b9cf495b2280c962aa5efb6cacaca05311d7d15986`;
release, installed binary, and both running daemon executables matched, with
both services active and `NRestarts=0`. The official installed Agent path
selected `pi-coder`, produced durable Agent
`6b6bd424-27bb-45f7-8139-9eee1386c920` with status `succeeded`, and returned
`RA_A_10_CLOSED` only after terminal observation. The parent used three
inference rounds, zero provider retries, and two Agent tool calls; the
post-deploy forbidden-error scan was empty.
