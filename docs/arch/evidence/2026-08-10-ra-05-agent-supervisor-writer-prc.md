# Agent Kernel V2：RA-05 PR-C Runtime AgentSupervisor writer (deployable)

Date: 2026-08-10
Baseline: `0bf690b2`
Plan: `docs/plans/2026-08-09-agent-kernel-v2-deepseek-implementation-plan.md` §12.4 RA-05, runbook PR-C
State: Runtime AgentSupervisor identity/lifecycle writer and typed host delegate-spawn adapter are deployed; AgentRun lifecycle/recovery/mailbox writes now pass Runtime-first through an explicit host projection bridge, while rich AgentControl remains the concrete execution adapter. Official daemon/client deployment is green; the full Agent spawn/wait acceptance drill remains open.

## Context receipt (runbook §3.1)

```text
Slice: RA-05 PR-C AgentSupervisor writer (code)
Baseline commit: 0bf690b2
Plan revision: agent-kernel-v2-deepseek-implementation-plan §12.4 RA-05, runbook §7.3 PR-C
Direct prerequisites: RA-05 PR-A (AgentSupervisor seam + DelegateBackendRegistry) — done
Current authoritative writer: RuntimeAgentSupervisor owns the durable Runtime Agent lifecycle receipt, generation fence, and typed delegate admission
Target owner/writer: RuntimeAgentSupervisor + `RuntimeObservedAgentBackend`; AgentControl owns only Kernel/process/mailbox host facts until RA-06
IDs minted here: Runtime child UUID + generation, converted mechanically to legacy Fabric AgentId at the adapter boundary
Production callers: daemon bootstrap replays Runtime Agent events, injects the supervisor, registers one observed backend per execution route, and routes AgentControl spawn through `spawn_host_request`
Test-only callers: 2 agent_writer tests
Installed/config callers: installed daemon uses the injected supervisor and dynamic profile enum schema
Tables/files/wire schemas: `aletheon.event.runtime_agent/v1` versioned AgentStream lifecycle/mailbox receipts
External side effects: durable EventSpine append; child execution remains AgentControl-owned
Compatibility seam: one-way Runtime AgentStream adapter plus Runtime-first `RuntimeAgentRunProjection` bridge and observed wait/send/cancel bridge; Fabric AgentControl remains the host admission/execution and SQL projection adapter
Deletion owner: legacy AgentControl → RA-06/XRET-02
Unknowns/blockers: the installed acceptance (deploy + Native/Pi drill + rollback binary) is still required for RA-05 closure; current deployment verification covers daemon/client health but not the full Agent spawn/wait drill.
Expected files: crates/runtime/src/agent_writer.rs, lib.rs re-export, RA-05C gate
Out-of-scope files: Pi concrete migration (E6-K6d); the daemon wiring is part of
this slice, while rollback-drill artifacts remain deployment follow-up.
```

## 1. What was created

`crates/runtime/src/agent_writer.rs`:

- **`RuntimeAgentSupervisor`** — one spawn/wait/cancel entry over the generic `DelegateBackendRegistry`; the Runtime resolves and **pins** the running binding per AgentRunId (reload never swaps a running binding).
- **`StubBackend`** — test backend.
- **Runtime-owned manifest catalog** — `DelegateBackendRegistry` stores the
  selectable `RuntimeManifest` beside each backend, performs deterministic
  Runtime selection, and rejects duplicate/invalid registrations.
- **Typed host spawn** — `DelegateSpawnRequest` carries host intent but no
  child identity/generation; Runtime mints both before invoking the observed
  backend, which must return the exact receipt.
- **Versioned AgentStream envelope** — `AgentStreamEvent` carries schema
  version, monotonic logical sequence, and a SHA-256 payload digest. Durable
  replay validates the envelope before lifecycle recovery and resumes sequence
  allocation after the replay watermark; old unwrapped events are accepted
  only through an explicit legacy replay adapter.
- **Host observation bridge** — lifecycle observations from the compatibility
  AgentControl stream are recorded as typed Runtime AgentStream messages before
  the richer SQL/memory projection receives them; terminal settlement remains
  Runtime-first and fail-closed.
- **Package reload binding** — extension executable publication now synchronizes
  the package runtime IDs/manifests into the same Runtime supervisor catalog;
  removed IDs are withdrawn for new admission while active bindings remain
  pinned.
- **Runtime-first AgentRun projection bridge** — `AgentRunProjection` replaces
  the authority-shaped repository name; `RuntimeAgentRunProjection` appends
  admission, start/terminal, recovery, and mailbox delivery receipts to
  AgentStream before forwarding rich request/workspace/payload/lease data to
  the compatibility SQLite projection. A failed projection is fenced as a
  Runtime terminal rather than silently leaving an unowned admission.
- Composed AgentControl sends now enter through a generation-fenced Runtime
  receipt (`send_with_generation`); the observed host bridge returns only a
  bounded delivery/sequence receipt and no longer re-enters the public
  AgentControl send method (`crates/executive/src/application/agent_control/mod.rs:1357-1425`, `runtime_bridge.rs:96-169`).
- Backend spawn receipts are checked against both Runtime-assigned run identity
  and generation; a backend cannot substitute a second child identity
  (`crates/runtime/src/agent_writer.rs:520-548`). Terminal append/publication is
  serialized to prevent concurrent wait/cancel/recovery double settlement.

## 2. Rules honoured (RA-05 PR-C)

- **Runtime assigns child/generation/run ID**: the writer resolves through the registry and returns the `DelegateReceipt` (agent_run + generation).
- **One spawn/send/wait/cancel/recovery entry**: Runtime exposes the generic entry, owns the durable lifecycle fence, and routes observed host wait/send/cancel through one typed adapter; rich admission/execution remains a host compatibility adapter.
- **Backend reload does not swap a running binding**: `bindings: HashMap<AgentRunId, Arc<dyn DelegateBackend>>` pins the exact backend.
- **No Pi file migration or double execution**: RA-05C gate rejects `PiRuntime`/`PiRpcRuntime`/`register_pi_runtime`; E6-K6d owns Pi.
- **wait before terminal never returns success**: `wait` returns only the backend's authoritative typed terminal (fail-closed).
- Legacy AgentControl remains a host execution/projection seam; Runtime is the durable identity, selection, lifecycle, recovery, mailbox-delivery, and terminal observer in production. SQL is explicitly a rebuildable rich projection, not the lifecycle authority.

## 3. RA-05C gate (architecture-check.sh)

`ARCH_SKIP_RA05C_GATES` requires `RuntimeAgentSupervisor`/`DelegateBackendRegistry`/`bindings`; rejects Pi symbols.

## 4. Validation

```text
bash scripts/cargo-agent.sh check -p runtime       PASS (0 errors/warnings)
bash scripts/cargo-agent.sh test -p runtime --lib  PASS (55 passed; AgentSupervisor/AgentWriter focused invariants included)
  - accepted_recovery_and_mailbox_receipts_are_runtime_owned
bash scripts/cargo-agent.sh test -p executive --test agent_recovery  PASS (6 passed)
bash scripts/cargo-agent.sh test -p executive --test agent_control_repository --test agent_mailbox --test agent_cleanup  PASS (13 passed)
  - supervisor_spawns_and_wait_returns_authoritative_terminal
  - cancel_uses_the_pinned_backend
bash scripts/cargo-agent.sh fmt --all -- --check   PASS
git diff --check                                    PASS
ARCH_SKIP_DEPENDENCIES=1 bash scripts/aletheon.sh acceptance architecture  PASS (23 findings, 0 deps)
```

## 5. Remaining RA-05 migration debt

The typed spawn adapter, Runtime-owned manifest selection, replay/recovery fence,
profile-port rename, versioned AgentStream sink, dynamic profile schema, observed host
wait/send/cancel adapter, host observation bridge, package catalog sync, and the
Runtime-first AgentRun projection bridge are complete. Remaining work is the
full AgentControl execution/process/workspace split, Pi E6-K6d backend migration,
the full Agent spawn/wait acceptance route, and the maintenance/drain + rollback-binary drill.
Until those are complete this slice is **CODE_CANDIDATE / DEPLOYMENT_PENDING**, not
RA-05/RA-06 closure.

Installed evidence (2026-08-10, Runtime-first projection bridge):
the latest `sudo bash scripts/aletheon.sh deploy` completed successfully;
the post-restart daemon health snapshot reports
`agent_recovery.class=ready,count=0`, and official Memory Agent/client request
smoke passed. `target/release/aletheon` and `/usr/bin/aletheon` currently report
`sha256=fd51336cf9d42f2bd00d28afdd9ea7c33eb897d23f67245ecee9920444be14ee`;
the user daemon is `active`, `NRestarts=0`, and `MainPID=958167`.

The required post-deploy Agent spawn/wait route is still **not accepted**:
three fresh official `/usr/bin/aletheon` requests with
`--require-agent-runtime pi-coder` did reach `agent_wait` and each returned an
authoritative terminal receipt (`runtime_id=pi-coder`, `status=succeeded`).
This proves the installed client route, but it is not substituted for the
required real-TUI/multi-turn and rollback drill; those remain open.

Latest installed observation after the send/receipt and generation-fence changes
(2026-08-10 15:07 UTC): the same SHA-256
`3538dadb745daba6c4ce8956359770f9cc7aeba23c55c0999319ad7e771b791d` matched
release/installed/running executables, both core and user services reported
`NRestarts=0`, and an official `/usr/bin/aletheon --require-agent-runtime
pi-coder exec --output json` returned `status=completed`, output `AGENT_OK`, and
zero provider retries. Full Pi cancel/restart/rollback and real-TUI evidence are
still required before RA-05 closure.

### 2026-08-11 continuation: installed Agent route evidence

The official scheduled Pi fixture was run through `/usr/bin/aletheon` and the official
user socket. One run reached `agent_wait` with `status=succeeded`, and the receipt proves
Runtime-assigned `profile=code-agent`, `runtime=pi-coder`, bounded budget and no out-of-scope
file writes. A subsequent consecutive run failed closed because no durable terminal projection
arrived before the client timeout; that failure is retained as acceptance evidence rather than
treated as success. After the redeploy, three consecutive official
`/usr/bin/aletheon --require-agent-runtime pi-coder exec --output json` read-only runs reached
`status=completed` with zero provider retries. An explicit one-second timeout also returned a
durable `status=failed` receipt with `error_code=cancellation_unconfirmed` rather than false
success. This closes the basic installed route/fail-closed cancellation evidence, but RA-05
remains open pending restart reconciliation, backend-generation rollback, and Pi/E6 caller
cutover.

### 2026-08-11 continuation: Pi route receives a dedicated E6 binding

`crates/executive/src/adapters/runtime/pi.rs` now exposes the reviewed
`PiDelegateBackend` binding. It accepts only the `pi-coder` backend ID and a
host-admitted request, then invokes host admission/preparation with the
composition-injected Pi launcher without re-selecting it from the legacy
registry or minting a second AgentRun, process, generation, or terminal.
`build_agent_services` replaces only the generic compatibility binding for
`pi-coder` (`crates/executive/src/host/daemon/bootstrap/services.rs:518-553`);
all other runtime IDs remain on the generic bridge. This is a production caller
cutover seam, not the final Pi implementation deletion: PiRuntime/PiRpcRuntime
and their direct process/protocol code remain until the E6 installed
equivalence and active-child drain/reconcile gates pass.

After deployment, three consecutive official `/usr/bin/aletheon
--require-agent-runtime pi-coder exec --output json` requests completed with
one tool call, two iterations, zero provider retries, and no provider error.
Release/installed/running daemon SHA-256 matched
`e6da9553d65b6b41c86cd851c5a4b750e1bcad8583d3585b6108576b7ae71d3a`.
The basic Pi route is wired and live; restart reconciliation, generation
rollback, process-group orphan checks, and final Pi legacy caller-zero evidence
remain open.

### 2026-08-11 continuation: E6 process-controller ownership seam

`crates/kernel/src/process/controller.rs:1-117` now owns the injected
`ProcessController` boundary. Pi RPC receives a `ManagedProcess` from that
Kernel controller and delegates process-group termination back to it;
`crates/executive/src/adapters/runtime/pi_rpc.rs:46-160,330-347` no longer
constructs `tokio::process::Command` or signals a process group directly.
The daemon composition root injects `LinuxProcessController` when preparing the
resident Pi runtime (`host/daemon/bootstrap/request.rs:1046`). A focused test
proves managed group spawn/reap, and the six-test Pi RPC integration target
remains green. This advances the E6 process ownership seam, but does not claim
Kernel-level lease parity, installed orphan/restart equivalence, or final Pi
legacy deletion.

The process-controller change was deployed and rechecked on 2026-08-10:
`sudo bash scripts/aletheon.sh deploy` passed; release, `/usr/bin/aletheon`,
and running machine/user daemon executables matched
`32b805f47038a9bffc5878dd933da66adf2149a8faa2494c9b78260a16f581c4`.
Both core and user services reported `ActiveState=active` and `NRestarts=0`.
After the Kernel process-controller move, three consecutive official Pi requests completed with
`status=completed`, zero provider retries, and the expected exact outputs.
This is stronger installed route evidence, but still not the disposable-host
orphan/restart/rollback matrix required to close E6.

### 2026-08-11 continuation: Kernel-owned restart reconciliation

The durable external-child reclaim path now also belongs to the Kernel
controller: `crates/kernel/src/process/controller.rs:19-24,54-156` validates
PID/start-time identity and performs bounded process-group termination, while
`crates/executive/src/adapters/runtime/process_supervisor.rs:1-38` only maps the
typed Kernel outcome into the AgentControl recovery port. The Executive adapter
no longer contains a second `/proc` parser or signal implementation. The
existing PID-reuse/reclaim integration test remains green. This closes the
code-side E6 process-controller/reconcile seam; installed orphan/restart and
rollback drills are still required before E6 closure.

After the direct Pi launcher cutover and Kernel reclaim move, deployment
verification passed with release/installed SHA-256
`021045c6141fd8e03bfb2552d900935c8efaca718dd5cd78094a40e22dcf1e93`;
`aletheon-core.service` and the official user `aletheon.service` were active
with `NRestarts=0`. Three consecutive official Pi requests returned exact
expected output with `status=completed`, one iteration, zero provider retries,
and no provider errors. This is installed route evidence, not the disposable
host failure matrix or rollback evidence.

## 6. Rollback

Delete `agent_writer.rs` + the lib.rs re-export + the RA-05C gate → exact baseline. No schema, no writer, no daemon change.

## 7. Next

The remaining AgentControl execution/process/workspace split, E6 reconnect and
lease-parity work, official Gateway/host cutover, installed failure/rollback
drill, and RA-06 deletion audit.

### 2026-08-11 continuation: static runtime launchers are pinned at composition

The generic static Runtime backends now receive composition-pinned launchers
(`crates/executive/src/host/daemon/bootstrap/services.rs:493-526`) rather than
resolving an executable from the legacy `AgentExecutionRegistry` inside the
spawn task. Dynamic extension reloads intentionally remain on the compatibility
bridge until their own caller-zero cutover. The deployed SHA was
`021045c6141fd8e03bfb2552d900935c8efaca718dd5cd78094a40e22dcf1e93`; three
official Pi requests returned exact `PI_PINNED_LAUNCHER_1..3` outputs, one
iteration each, zero provider retries, and both daemons remained active with
`NRestarts=0`. This advances RA-05/E6 production wiring but does not close the
failure/rollback or RA-06 deletion gates.

The extension catalog path now uses the same pinned-launcher adapter for newly
registered package runtimes (`crates/executive/src/host/daemon/bootstrap/extensions.rs:55-145`).
Existing managed bindings are not replaced during reload; only future package
admissions see the new binding. Focused extension tests passed (18 tests), and
installed verification at SHA
`021045c6141fd8e03bfb2552d900935c8efaca718dd5cd78094a40e22dcf1e93` remained
stable with three exact Pi outputs (`PI_EXTENSION_PIN_1..3`). This removes the
remaining dynamic extension spawn-time registry lookup; full RA-06 deletion and
rollback gates remain open.

### 2026-08-11 continuation: Goal attempts enter Runtime through a typed command

`crates/executive/src/application/goal/runtime_executor.rs:1-244` adds the
`GoalAttemptBackend` and `RuntimeGoalAttemptExecutor`. Production Goal worker
composition in `crates/executive/src/host/daemon/bootstrap/request.rs:1016-1176`
registers distinct `goal-attempt:<runtime>` DelegateBackend bindings and
constructs `GoalWorker::new_with_executor`; the worker no longer receives a
`ProviderWorkerRegistry` on the daemon path. The compatibility
`SubAgentRuntime` is kept behind the backend only, translating the legacy task
to `runtime::DelegateCommand::GoalAttempt`. Runtime allocates the run and
generation, owns terminal settlement, and exposes a typed result only after
`wait` (`crates/runtime/src/agent_supervisor.rs:19-105`,
`crates/runtime/src/agent_writer.rs:636-688`).

Focused validation passed: `bash scripts/cargo-agent.sh test -p executive
runtime_goal_executor_uses_typed_command_and_runtime_terminal -- --nocapture`
(1 passed), package check and format check passed. The legacy
`RegistryAttemptExecutor` and Pi compatibility runtime remain as explicit
rollback/compatibility surfaces; Pi caller-zero, installed Goal execution,
failure/rollback drills, and RA-06 deletion are still open.

After the Goal Runtime bridge change, the installed acceptance rerun passed:
`bash scripts/cargo-agent.sh build --release -p aletheon && sudo bash scripts/aletheon.sh deploy`
completed with release/installed/running executable SHA-256
`fb4fa90bc28447814bd3eeeb1222e3ca4521fa633ddc1ee765d06a9d7df48b1f`; both
systemd daemons were active with `NRestarts=0`. Three consecutive official
`/usr/bin/aletheon --require-agent-runtime pi-coder exec --output json` requests
reached `status=completed`, with zero provider retries and no rendered provider
error. Goal worker is disabled in the installed configuration, so this is Pi
and general installed route evidence, not installed Goal route acceptance.

A final post-fix deploy (typed failure projection cleanup) also passed at SHA
`33eb6fd6af9fb5f0ab6acd147b44c0c01d0844b42002835dd24760b900c5ea2c`.
Three fresh official `pi-coder` requests returned exit code 0, terminal
`error_code=null`, and `provider_retries=0`; the deploy verification observed
both daemons active and stable. Goal worker remains disabled by effective
installed configuration, so this still does not close installed Goal acceptance.

2026-08-11 caller-cut continuation: the Executive runtime catalog and
AgentControl compatibility launcher now consume Runtime's typed
`DelegateTaskBackend`/`DelegateExecutionContext` contract. `ProviderWorkerRuntime`
and `PiRuntime` implement the typed task backend; `SubAgentRuntime` is no longer
referenced by production AgentControl, Goal, provider-worker, or Pi composition
paths. The daemon no longer registers the legacy `PiRuntime` into
`ProviderWorkerRegistry`; production Pi admission uses the injected
`PiRpcRuntime`/`PiDelegateBackend` path. Legacy `SubAgentRuntime` declarations
remain only as a guarded compatibility module until the E6/RA-06 deletion gate.
Focused `agent_control_spawn` (8), Goal worker (1), Pi runtime (7), and registry
(1) tests passed; architecture acceptance has no new findings.

2026-08-11 post-caller-cut deployment: the legacy Pi registration was removed
from daemon bootstrap; only the resident `PiRpcRuntime` launcher and
`PiDelegateBackend` are composed in production. Release, installed, and both
running daemon executable hashes matched
`f9b6ceb860814a76b539d00c934a4268161a5ee9f5ed56124bdde5564e18c0d6`; both
services were active with `NRestarts=0`. Three official `pi-coder` runs exited
successfully with `provider_retries=0` and `error_code=null`. This is stronger
Pi caller-cut evidence, but legacy Pi implementation deletion and active-child
reconcile/rollback gates remain open.

2026-08-11 Goal catalog decoupling continuation: `register_goal_runtimes` now
returns typed `(RuntimeId, Arc<dyn DelegateTaskBackend>)` bindings directly;
request composition registers those bindings into AgentExecutionRegistry and
Runtime's GoalAttemptBackend map without an Executive-owned
`ProviderWorkerRegistry`. `AletheonExecutive` no longer stores or exposes that
registry. This removes the duplicate composition root and keeps Goal's
production path on one typed Runtime catalog. Package check, focused Goal/
AgentControl/Pi/registry tests, format, architecture acceptance, and installed
acceptance passed. Installed/runtime SHA parity is
`5ec5d52d622616e6e0597a0782b21c66d52de15c8f89c0662b64f7c87641a1ad`; both
services were active with `NRestarts=0`; three official Pi calls completed with
zero provider retries and null error codes. Goal worker remains disabled in the
effective installed configuration, so Goal-specific installed acceptance is
still open. Legacy compatibility constructors and the declaration-only
`SubAgentRuntime` module remain until E6/RA-06 deletion, restart/reconnect,
active-child drain/reconcile, and rollback gates are complete.

2026-08-11 lifecycle/adapter decoupling continuation: Runtime AgentSupervisor
now has a real admission gate and `drain_active`/`replace_backend_after_drain`
contract. Drain freezes new delegates, enumerates accepted/observed/bound
non-terminal runs, attempts cancellation for every child, and leaves the writer
retired on any failure; launcher replacement reopens admission only after the
new binding registers (`crates/runtime/src/agent_writer.rs`). AgentControl
shutdown invokes the same Runtime drain fence. Runtime tests now cover active
child drain, retired admission, and launcher replacement; 13 Agent writer tests
passed.

Pi configuration resolution was split from the compatibility `PiRuntime`: the
resident RPC adapter now consumes `resolve_pi_config` directly and no longer
constructs the legacy runtime for validation (`crates/executive/src/adapters/runtime/pi.rs`,
`pi_rpc.rs`). Focused AgentControl/Goal/Pi/registry tests (17), runtime checks,
format, architecture acceptance, release build, and installed acceptance passed.
Installed/runtime SHA parity is
`f8cec72f96c09d542725e7d2cf6f2ab2e3b93e71ff5e053ab0e511bd8e49fb99`; both
systemd daemons were active with `NRestarts=0`; three official Pi requests
completed with zero provider retries and null error codes. This advances E6
process-lifecycle and concrete-adapter separation, but Pi protocol reconnect,
rollback drill, legacy implementation deletion, and installed Goal acceptance
remain open.

The RA-00 authority census was corrected so the former Executive-owned
`ProviderWorkerRegistry` composition row now points to the typed Goal runtime
registration site (`config/architecture/runtime-authority-census.tsv:RA-A-09`);
architecture-check remains green. The registry type itself remains an explicit
compatibility/test surface under RA-A-08 until the final RA-06 deletion gate.

Post-recovery-evidence deployment: Runtime orphan recovery now appends an
explicit `AgentRunRecovery(decision=settle-orphan:interrupted)` receipt before
the authoritative `AgentRunSettled(Interrupted)` event. This makes the
non-reconnect disposition durable and distinguishable from normal completion;
stream replay tests cover the ordered recovery+terminal pair. Release,
installed, and both running daemon executable hashes matched
`9d694538f29d37033df112e753d0e6cb99ec5bac744eb872f6ebbc926a4c0eac`; both
services were active with `NRestarts=0`; three official Pi requests completed
with zero provider retries and null error codes.

2026-08-11 duplicate-catalog removal: the Executive `ProviderWorkerRegistry`,
`RegistryAttemptExecutor`, and legacy `register_pi_runtime` helper were removed.
Goal isolated fixtures now inject `AttemptExecutor` directly, runtime catalog
tests exercise Runtime's `DelegateBackendRegistry`, and Pi tests validate the
standalone configuration resolver. The remaining generic compatibility seam is
only `SubAgentRuntime/SubAgentExecutionContext`; the RA-00 census and
architecture gate were updated to remove the deleted Executive registry path.
This is caller-zero evidence for the duplicate provider catalog, not yet the
final Pi implementation or generic facade deletion.

Post-catalog-cut deployment: the duplicate Executive `ProviderWorkerRegistry`
and `RegistryAttemptExecutor` are gone; Goal production and isolated tests now
inject typed `AttemptExecutor`, and Runtime's own `DelegateBackendRegistry` is
the only delegate registry. `register_pi_runtime` was also removed; Pi tests
exercise `resolve_pi_config`/resident RPC composition. Release, installed, and
both running daemon hashes matched
`555cd36540e0c584217cbabc96c276f9aa5772e38844b674897e46ab42fd7fd6`; both
services were active with `NRestarts=0`; three official Pi requests completed
with zero provider retries and null error codes. The remaining deletion seam is
Pi `PiRuntime/PiRpcRuntime` and generic `SubAgentRuntime`, subject to E6/RA-06
protocol, drain/reconcile, rollback, and installed Goal gates.

2026-08-11 E6/RA-06 implementation correction: the concrete Pi compatibility
implementations were physically consolidated. `pi_rpc.rs` now defines the
single `PiDelegateBackend`, which owns the reviewed Pi executable policy,
process controller, RPC protocol, terminal/cancel path, and the one-time host
service binding (`crates/executive/src/adapters/runtime/pi_rpc.rs:48-147`,
`:242-318`). Request composition prepares that backend and the daemon service
composition binds/registers it directly in Runtime's `DelegateBackendRegistry`
(`crates/executive/src/host/daemon/bootstrap/request.rs:1041-1064`,
`crates/executive/src/host/daemon/bootstrap/services.rs:526-542`). The former
`PiRuntime` and `PiRpcRuntime` symbols are absent from production and focused
test code; the old one-shot test surface is reduced to the canonical manifest
contract. The declaration-only `SubAgentRuntime`/`SubAgentExecutionContext`
facade is also caller-zero and its module/re-export were deleted; XRET-00 now
expects the single remaining `legacy_session_service.rs` compatibility row.

Focused Runtime (58), Goal (14), AgentControl spawn (8), Pi RPC (6), Pi
manifest (1), registry (1), format, package check, and architecture checks
passed. A release deployment at SHA
`cb843b29dc81c86aa7e21f36fdd1d38b3d61937b8c54f015e1b228184b569855` previously
matched `target/release/aletheon`, `/usr/bin/aletheon`, and both running daemon
executables; `aletheon-core.service` and the user daemon were active with
`NRestarts=0`, and three official `pi-coder` calls returned authoritative
completed results with no provider retries or errors. Because the subsequent
shutdown-order and facade-deletion edits still require a fresh deployment,
that SHA is not final evidence for the current tree.

The active-child restart exercise remains **open/failed evidence**: the client
did not produce an authoritative terminal snapshot, the old daemon aborted its
connection after the bounded drain timeout, and the new daemon recovered with
zero open rows. No reconnect/settlement receipt can be linked to that run, so
the E6 reconnect/active-child gate and rollback drill are not claimed complete.
Installed Goal acceptance is likewise open because the effective installed
configuration has no enabled `[goal_runtime]` worker (`crates/cognit/src/config/mod.rs:360-370`).

2026-08-11 post-E6-deletion deployment: release build and `sudo bash
scripts/aletheon.sh deploy` passed for the current tree. SHA-256 matched across
`target/release/aletheon`, `/usr/bin/aletheon`, the running `aletheon-core.service`
process, and the running user daemon: `43da0e042e0d3b67ed21b4f3e62475b8cfdd355111a8f70b72e4448952c00fdd`.
Both services were `active/running` with `NRestarts=0`. Three official socket
requests through `/usr/bin/aletheon --require-agent-runtime pi-coder exec`
returned authoritative `completed` snapshots and exact requested output; no
error code or provider retry was reported. This closes the current Pi
installed-equivalence smoke, but not the failed active-child reconnect drill,
rollback binary drill, or installed Goal gate.

Post-deletion test correction: `gate1_process_operation` no longer includes the
retired `core/sub_agent.rs` at compile time; it asserts the module is absent and
keeps AgentControl as the sole process owner. All 8 Gate-1 tests pass, with
format, architecture, and diff checks still green.

2026-08-11 Runtime catalog closure: production daemon composition no longer
constructs or passes `AgentExecutionRegistry`. Native Cognit and opt-in Goal
launchers are supplied as typed `RuntimeLauncherBinding` values and registered
once into Runtime's `DelegateBackendRegistry`
(`crates/executive/src/host/daemon/bootstrap/request.rs:938-1008`,
`crates/executive/src/host/daemon/bootstrap/services.rs:390-530`). Package
executables use a scoped `PackageRuntimeStaging` only until supervisor binding;
reloads then update the Runtime catalog while existing runs retain pinned
backends (`crates/executive/src/host/daemon/bootstrap/extensions.rs:15-280`).
The Executive registry remains available only to explicit integration fixtures
and the legacy constructor, not installed composition; the RA-00 census was
corrected accordingly.

Checkpoint recovery now also follows the pinned Runtime backend generation:
`DelegateRecoveryRequest` and `RuntimeAgentSupervisor::resume_from_checkpoint`
forward recovery to the bound backend, while the host adapter validates its
resumability and performs only the rich checkpoint operation
(`crates/runtime/src/agent_supervisor.rs:130-180`,
`crates/runtime/src/agent_writer.rs:1030-1062`,
`crates/executive/src/application/agent_control/runtime_bridge.rs:120-158`).
The Runtime suite passes 59 tests, including wrong-generation recovery fencing;
AgentControl spawn/recovery (8/6), Pi RPC (6), extension recovery (2), format,
check, and architecture acceptance also pass.

2026-08-11 current-tree RA-05 deployment: after removing the production
`AgentExecutionRegistry` construction and package-registry methods, release build
and `sudo bash scripts/aletheon.sh deploy` passed. SHA-256 was identical across
`target/release/aletheon`, `/usr/bin/aletheon`, system/user running executables:
`bc40081d60297a42ad1b54e0bab2a1f6e80e4aa999645f256708a0ad4b0a15f0`. Both
services were active/running with `NRestarts=0`. Three official Pi requests
returned authoritative `completed` snapshots with exact outputs and
`provider_retries=0`; this verifies the current Runtime catalog cutover in the
installed Pi path.

The attempted installed active-child exercise was intentionally classified as
failed evidence: `--require-agent-runtime pi-coder` selects the top-level Pi
runtime, and the request's long-running Pi shell/tool retries did not create a
nested AgentRun receipt. A separate provider prompt was blocked by host policy
before AgentControl child admission. No reconnect/settlement claim is made.

2026-08-11 observed-run recovery continuation: `ObservedAgentBackend` now has a
typed checkpoint-resume operation. `RuntimeAgentSupervisor::resume_from_checkpoint`
falls back to the composition-pinned observed host adapter when a daemon restart
has removed the process-local `AgentBinding`; it does not reselect a backend or
mint a new identity (`crates/runtime/src/agent_writer.rs:81-105`,
`crates/runtime/src/agent_writer.rs:1040-1074`). A deterministic test covers
resume through that binding-free path. Runtime now passes 60 tests, the migration
matrix passes all 12 declared components, and release deploy passed at SHA
`33b63603d953ad5fc2adc2142d397b7a42a163f81d31a04297e46b89532cc4fc` with
matching target/installed/system/user executables, both daemons `NRestarts=0`,
and an official Pi request completing with zero provider retries. This improves
recovery correctness but does not close the real active-child reconnect,
rollback-binary, or installed Goal gates.

2026-08-11 observed recovery routing correction: the composition-wide observed
bridge now resolves the durable `runtime_id` through Runtime's already-bound
`DelegateBackendRegistry` when no process-local launcher exists, then forwards
checkpoint recovery to that pinned backend (`crates/executive/src/application/agent_control/runtime_bridge.rs:120-176`).
It never falls back to `AgentExecutionRegistry` or reselects a launcher. Focused
Runtime (60), Executive recovery (6), migration matrix (12 components), format,
check, and architecture acceptance pass. The fresh release/deploy at SHA
`c0971b34b7e507301aae7c7837ed09bee9289941a7ef6cb22e14f498a700a235` matched
all four executable locations, both daemons were active with `NRestarts=0`, and
the official Pi request `OBSERVED_RESUME_DEPLOY_1` completed with
`provider_retries=0`. Real active-child reconnect/settlement, rollback binary,
and installed Goal acceptance remain open.

2026-08-11 authority census cleanup: RA-A-08 is now CLOSED because
`DelegateBackendRegistry` is the Runtime-owned catalog used by the production
supervisor, and RA-D-03 is CLOSED because Pi/Native `RuntimeId` helpers are
registration inputs rather than a second selection authority. RA-A-10/A-11/A-12
remain `INVESTIGATE:RA-05` because the Executive AgentControl service and its
SQLite projection still own rich host admission/projection responsibilities
until the RA-06/APX-05 caller-zero gate.

2026-08-11 compatibility boundary relocation: the remaining
`AgentExecutionRegistry` fixture catalog moved from the AgentControl execution
module into `crates/executive/src/compatibility/legacy_runtime_registry.rs`.
`application::agent_control` only re-exports it for explicit legacy tests; the
installed composition still constructs no such catalog. Focused AgentControl
spawn/recovery and Pi RPC tests remain green, and the architecture inventory
records the file as a fixture-only compatibility sub-seam rather than a second
production authority.

2026-08-11 compatibility relocation deployment: the registry-boundary change
passed Executive check, AgentControl spawn/recovery (8/6), Pi RPC (6), format,
and architecture acceptance. Release deploy passed at SHA
`2c9edd2c6374f36fc5cfe8ce2c0b5429f0d722a59373294364aaca9657e8ab28`; target,
installed, system, and user executables matched, both daemons were active with
`NRestarts=0`, and deploy's official client real-request smoke passed. The
legacy registry remains explicit fixture compatibility; no installed production
construction was added.

2026-08-11 compatibility export tightening: `AgentExecutionRegistry` is no
longer re-exported from the public `application::agent_control` facade. Black-box
fixtures import it only through the hidden `executive::testing::agent_control`
surface, while the implementation remains under `compatibility/`. This removes
the compatibility catalog from the application public API without changing the
installed Runtime authority. The focused suites and architecture acceptance
remain green; the latest deploy retained SHA
`2c9edd2c6374f36fc5cfe8ce2c0b5429f0d722a59373294364aaca9657e8ab28`.

2026-08-11 rollback preparation: a distinct baseline release binary was built
from committed HEAD `8a813861` in an isolated temporary worktree and target,
with SHA `68868aba58ab9a5342d239a74ebac1638cd7f5c9dfb456020e65166b169b4631`.
This supplies the required distinct candidate/baseline inputs for the release
lane, but the destructive installed-host rollback recipe was not run because
this machine is not a disposable systemd VM/container
(`tests/production/lib/installed_host.sh:7-28`). No rollback pass is claimed.

2026-08-11 legacy constructor tightening: the old `AgentControlService::new`
entry was renamed to hidden `new_legacy`; production composition continues to
use `new_runtime_only`, and all black-box fixture call sites now make the
compatibility choice explicit. Focused AgentControl spawn/recovery tests and
architecture acceptance pass. Release deploy at SHA
`b1e347c4c3c0b2a6a76f41f519f7c6ca04a07c2f5a74c241b7a9037cae527b43` passed with
stable system/user daemons and official client smoke. The compatibility
constructor remains available only for fixture/rollback compatibility until the
RA-06/APX-05 caller-zero gate.

2026-08-11 checkpoint replay correction: daemon bootstrap now uses
`RuntimeAgentSupervisor::replay_from_stream` before host startup reconciliation
instead of `recover_from_stream`. The latter intentionally fences every
unresolved Runtime run as `Interrupted`; using it before the host sees
`process_live`/checkpoint state made `RuntimeResumability::Checkpointed` runs
impossible to resume after a daemon restart. The new replay-only path restores
Accepted/Started/terminal state and advances the stream cursor, while
`AgentControlService::reconcile_startup` remains the sole place that chooses
Resume/Finalize/Interrupt and records the Runtime-first recovery receipt.

Focused evidence:
- `bash scripts/cargo-agent.sh test -p runtime --lib agent_writer` PASS (16 passed), including
  `replay_stream_defers_orphan_fence_for_host_checkpoint_reconciliation` and the
  existing orphan-settlement/wrong-generation tests.
- `bash scripts/cargo-agent.sh test -p executive --lib application::agent_control::runtime_projection::tests` PASS (3 passed).
- `bash scripts/cargo-agent.sh test -p executive --test agent_recovery --test turn_coordinator_lifecycle` PASS (20 passed).
- `bash scripts/cargo-agent.sh check -p runtime -p executive` PASS.
- `ARCH_SKIP_DEPENDENCIES=1 bash scripts/aletheon.sh acceptance architecture` PASS.

This closes a code-side recovery ordering defect and provides deterministic
checkpoint-preservation evidence. Installed active-child reconnect/settlement,
rollback binary, and enabled Goal acceptance are still open; no installed
reconnect pass is claimed from this change alone.

The Executive recovery suite now also exercises the production-shaped
`RuntimeAgentRunProjection` path: a replayed Runtime AgentStart is reconciled
as an orphan through the Runtime terminal fence before the SQLite row is
transitioned (`runtime_projection_reconciles_replayed_orphan_before_sql_terminal_projection`).
The focused recovery test passes alongside the 16 Runtime Agent writer tests;
this is deterministic code-side evidence, not an installed reconnect receipt.

2026-08-11 Runtime/host projection crash-window correction: startup recovery now
also enumerates open Runtime lifecycle identities that have no process-local
binding (`crates/runtime/src/agent_writer.rs:918-944`). For UUID-shaped host
runs absent from the SQLite projection, `AgentControlService::reconcile_startup`
records `host:missing-projection` and fences `Interrupted` before publishing
any host terminal (`crates/executive/src/application/agent_control/mod.rs:572-640`).
This closes the admission/SQL-projection crash window without treating generic
`run-*` delegate identities as AgentControl rows.

Focused evidence:
- `bash scripts/cargo-agent.sh test -p runtime --lib agent_writer`: 17 passed.
- `bash scripts/cargo-agent.sh test -p executive --test agent_recovery`: 8 passed,
  including `runtime_reconcile_fences_started_run_when_host_projection_was_never_committed`.
- `bash scripts/cargo-agent.sh check -p runtime -p executive`: PASS.

This is deterministic Runtime-first orphan fencing only. Installed active-child
reconnect/settlement, rollback binary, and enabled Goal acceptance remain open.

2026-08-11 compatibility boundary narrowing: `AgentControlService` no longer
imports the concrete `AgentExecutionRegistry`; its hidden legacy constructor
accepts only the narrow `CompatibilityRuntimeCatalog` port
(`crates/executive/src/application/agent_control/mod.rs:127-180`). The concrete
catalog remains in `compatibility/legacy_runtime_registry.rs` and implements
that port, while production composition still uses Runtime's
`DelegateBackendRegistry`. AgentControl spawn/recovery focused suites pass.
This is a dependency-boundary reduction, not the final RA-06 physical deletion.

2026-08-11 recovery receipt idempotency: `RuntimeAgentSupervisor` now replays
and fences duplicate recovery decisions per AgentRun, so a crash after a
recovery append but before terminal projection cannot duplicate the same
`AgentRunRecovery` event (`crates/runtime/src/agent_writer.rs:39-53,385-429,730-778`).
The Runtime writer test repeats the same recovery call and verifies the stream
remains append-once for that decision.

2026-08-11 mailbox observation cutover: successful Runtime supervisor sends
now append a bounded `AgentRunMessage` observation and, when the host returns a
delivery id, an `AgentRunMailbox` delivery receipt under the same terminal
write fence (`crates/runtime/src/agent_writer.rs:1081-1179`). This prevents a
successful host mailbox operation from disappearing from the Runtime Agent
stream and rejects an observation that races an already fenced terminal.
Manual mailbox lifecycle writes now use the same fence (`:442-479`). The
observed-host test covers both message and delivered-receipt events; the
focused Runtime Agent writer suite remains 17 passing tests.

The open-run enumeration now unions accepted, started, and already-bound
Runtime identities rather than relying only on the admission map
(`crates/runtime/src/agent_writer.rs:949-978`). This preserves the recovery
signal for an older/replayed Started-only lifecycle and is covered by the
extended `open_lifecycle_runs_include_unbound_admission_and_drop_terminal_runs`
test.

Mailbox delivery state is now replayed and idempotent by `(AgentRunId,
delivery_id)`: identical retries acknowledge, `Pending` may advance once, and
an already terminal delivery cannot be rewritten
(`crates/runtime/src/agent_writer.rs:446-529,795-810`). The focused writer
tests verify both repeated `Delivered` receipts and replay-after-restart do not
append a duplicate event, and a replayed mailbox after terminal is ignored;
the suite is now 19 passing tests.

2026-08-11 maintenance safety follow-up: `RuntimeAgentSupervisor::resume_admission`
now returns a typed error unless the supervisor is actually draining and its
active Runtime run set is empty (`crates/runtime/src/agent_writer.rs:1075-1089`).
This prevents an uncoordinated caller from reopening delegate admission while
children remain live or without a preceding freeze. The Runtime Agent writer
focused suite remains 19 passing tests; installed active-child drain/reconnect
and rollback evidence remain open.

The maintenance negative case also covers a draining flag set before an active
child is cancelled: reopening admission remains rejected until the active run
set is empty. This is covered by the same 19-test Runtime Agent writer suite.

AgentStream replay integrity follow-up: `replay_from_stream` now rejects
non-monotonic or duplicate logical sequence numbers before lifecycle state is
replayed. The focused Runtime Agent writer suite is now 20 passing tests.

Agent lifecycle replay now also verifies that each terminal carries the same
SessionId as its Runtime start/admission event; cross-session terminal evidence
is rejected as `UnknownSchema`. The focused Runtime Agent writer suite is now 21
passing tests.

Agent admission conflict typing follow-up: repeated Runtime admission now
returns `WrongSession` for a cross-session identity conflict and
`WrongGeneration` only for a generation conflict. The focused Agent writer suite
is now 22 passing tests.
## 2026-08-11 follow-up: admission writer fence

`RuntimeAgentSupervisor::record_accepted` and `record_started` now hold the
Runtime admission read fence and the terminal writer mutex across their
check/append/publication sequence. A maintenance drain therefore blocks both
typed spawn and direct host lifecycle admission with `RuntimeError::Retired`,
and concurrent identical callbacks append one `Accepted` and one `Started`
event only. The Executive compatibility adapter carries the Runtime-issued
generation on `ValidatedAgentIdentity` instead of maintaining a second
generation map.

Evidence:

```text
bash scripts/cargo-agent.sh test -p runtime --lib agent_writer  # 23 passed
bash scripts/cargo-agent.sh test -p executive --test agent_control_spawn # 8 passed
bash scripts/cargo-agent.sh check -p runtime -p executive       # passed
ARCH_SKIP_DEPENDENCIES=1 bash scripts/aletheon.sh acceptance architecture
# 22 findings, 0 dependencies, 4 paths; no additions
```

Installed reconnect, rollback, caller-zero and privileged deployment gates
remain open; this is a local writer invariant, not installed acceptance.

2026-08-11 lifecycle reducer ownership follow-up: the Agent lifecycle state
machine and daemon generation fence are now physically implemented in Runtime
(`crates/runtime/src/agent_lifecycle.rs`, `crates/runtime/src/generation_fence.rs`).
Executive's former modules are compatibility re-exports only
(`crates/executive/src/application/agent_control/lifecycle.rs` and
`generation_fence.rs`), so SQLite projection/settlement adapters consume the
Runtime reducer types without retaining a second state-machine implementation.
Runtime library tests: 77 passed. This is a code-side ownership move; rich
AgentControl admission/recovery/settlement and installed reconnect/rollback
acceptance remain open.

The lifecycle ownership move was rebuilt and installed through the canonical
boundary. `target/release/aletheon` and `/usr/bin/aletheon` both have SHA-256
`c09a49b52f42fe0bf2991835c25c369a2406698c2a9543dcadfed791d3de5be4`; machine
and user daemon units are active/running with `NRestarts=0`, and deployment's
official client and Memory-Agent protocol smoke checks pass. This verifies the
new Runtime reducer in the installed binary, not the still-open full RA-05
reconnect/rollback gate.

2026-08-11 Agent projection-port ownership follow-up: the Agent journal
records and `AgentRunProjection` port moved to
`crates/runtime/src/agent_repository.rs`; Executive's
`application/agent_control/repository.rs` is now a compatibility re-export.
The SQLite implementation remains an adapter and no duplicate record schema
was introduced. Runtime/Executive/Aletheon checks and architecture acceptance
pass. The canonical installed deployment was rerun; release and installed
SHA-256 is `22bdb1ff0523a8a99909ff29d02d238da652d8b2b4e79afa58023b1f7212275d`,
with both daemon units active and zero restarts and official client/Memory
Agent smoke passing.

2026-08-11 Agent execution-event ownership follow-up: the typed
`AgentRuntimeEvent` contract and host observation sink moved to
`crates/runtime/src/agent_events.rs`; Executive's execution module now uses
compatibility aliases while retaining only host projection/adapters. This
separates Runtime lifecycle event semantics from Executive memory/SQL effects.
Runtime (77), Executive (762), Aletheon check, formatting, and architecture
acceptance pass. Canonical deployment verification passed at SHA
`0dc00889724f147e56dc152a41bc5b36f4394eaff11ba9c214f86f5aaba68a95`; both
units remain active with zero restarts and official client/Memory-Agent smoke
passed.

2026-08-11 Runtime-supervisor constructor cutover: production
`AgentControlService::new_runtime_only` now requires an already-constructed
`Arc<runtime::RuntimeAgentSupervisor>` and installs it during construction;
the delayed optional builder setter is no longer used by the daemon bootstrap.
Both Aletheon and the retained Executive host bootstrap pass the same Runtime
supervisor instance, making production construction fail-closed if the
Runtime owner is absent. Focused Runtime/Executive/Aletheon checks and
architecture acceptance pass. Canonical deployment verification passed at
SHA `3fcbe2d8d2e8f70b6dbe0ad83bb8208509a8e2de1371d7cc23bac8050cc2c06d` for
both `target/release/aletheon` and `/usr/bin/aletheon`; machine and user
daemons are active/running with `NRestarts=0`, and official client/Memory-Agent
smoke checks passed. Rich AgentControl admission/recovery/settlement,
SQLite-adapter extraction, installed reconnect/rollback, and XRET-04 caller
zero remain open.

2026-08-11 Runtime SQLite adapter extraction: the concrete
`SqliteAgentRunProjection` and its four versioned Agent migrations moved out
of Executive into `crates/adapters/sqlite/src/runtime_agent/mod.rs` and its
`migrations/` bundle. Runtime owns the `AgentRunProjection` port and canonical
request-hash helper; Executive's `adapters::agent_control` and testing surface
are compatibility re-exports only. Aletheon production now imports the
adapter package directly, while the retained Executive host remains a
rollback/compatibility caller. The new adapter unit test, Executive Agent
repository/mailbox/recovery/cleanup tests, Runtime/Executive/Aletheon checks,
formatting, architecture acceptance, and `git diff --check` pass. This closes
the physical SQLite projection move, but not the rich AgentControl authority,
reconnect/rollback, or XRET-04 deletion gates.

The adapter extraction was rebuilt and installed through the canonical sudo
boundary. Release and installed binaries both carry SHA-256
`4dfb55a68000d0ccc4f38bd32b062121b4d579845932203ca1283ee1cf3d4790`; machine
and user daemon services are active/running with zero restarts, and official
client/Memory-Agent smoke checks passed. This proves the new physical SQLite
adapter is in the installed production binary.

2026-08-11 Runtime recovery ownership follow-up: `AgentRecoveryCoordinator`,
`AgentRecoveryObservation`, bounded recovery reporting, and the
`RuntimeProcessSupervisor` port moved to `crates/runtime/src/agent_recovery.rs`.
Executive's `application/agent_control/recovery.rs` is now a compatibility
re-export, so restart decision semantics and process-reclaim boundary no
longer live in the Application tree. Executive recovery integration tests
continue to pass against the Runtime implementation; rich settlement/resource
reclaim remains host-adapter work still open under RA-05.

Canonical sudo deployment after the recovery move passed at SHA
`4d54721eede4d9790973e4b2f2439812875d6441c464ce15e465d34a177eb483` for both
release and installed binaries. Machine/user daemons are active with zero
restarts, and official client/Memory-Agent smoke checks passed.

2026-08-11 Runtime settlement-policy follow-up: the pure recovery-to-resource
disposition mapping moved to `crates/runtime/src/agent_settlement.rs` and is
re-exported through Executive only for compatibility. The Runtime test covers
Resume/Finalize/Interrupt/Reclaim semantics; the Executive settlement engine
still remains a host/resource adapter and is intentionally not mislabeled as
fully migrated.

Canonical sudo deployment after the settlement-policy move passed at SHA
`c2ec37bbb4072447f192a20ae6c8a001bc51479b495d7b3267f6504de0c5f1c8` for
release and installed binaries; both daemon units are active with zero
restarts and official client/Memory-Agent smoke checks passed.

2026-08-11 extension-boundary follow-up: `aletheon-extension` no longer
depends on Executive. Its provider router is now a pure extension contract;
the `ExtensionProviderLauncher` lifecycle adapter and skill-catalog adapter
live in Aletheon composition (`crates/aletheon/src/wiring/daemon/bootstrap/`).
This removes the reverse Executive dependency from the extension package while
preserving the existing host compatibility seam at the composition boundary.
Extension (7) and Aletheon (162) tests plus architecture acceptance pass.

Canonical sudo deployment after the extension-boundary move passed at SHA
`c504d89ff9436ef74385f3757144574ec4b5849bb9e9f88339d93c7f93036ef7`; both
daemon units are active with zero restarts and official client/Memory-Agent
smoke checks passed.

2026-08-11 Runtime mailbox ownership follow-up: `AgentRuntimeInbox` and
`AgentMailboxBridge` moved to `crates/runtime/src/agent_mailbox.rs`, including
schema validation, bounded delivery, signal cancellation, and cancellation
ordering. Executive's mailbox module is now a compatibility re-export and
the execution input consumes the Runtime inbox directly. This removes another
Agent lifecycle/mailbox implementation from the Application owner; rich
Executive send/admission facade remains open.

Canonical sudo deployment after the mailbox move passed at SHA
`4ffdfe5a13531cf8797f79b2b3c5b577d21c0ec3b7f8c1f13eeeac7630c5e61d` for both
release and installed binaries. Machine/user daemon services are active with
zero restarts, and the official client/Memory-Agent smoke checks passed.

2026-08-11 AgentStream adapter ownership follow-up: the one-way
`RuntimeAgentStreamAdapter` moved from Executive's AgentControl execution
module into `crates/runtime/src/agent_stream_adapter.rs`. It now accepts the
Runtime event contract, records non-terminal observations and the terminal
fence through `RuntimeAgentSupervisor`, and only then forwards the host event
sink. Executive retains the Spine/memory/projection sinks, but no longer owns
the Runtime-to-host terminal ordering adapter. Runtime/Executive/Aletheon
checks and the architecture gate pass; the rich AgentControl facade and
backend bridge remain open under RA-05.

2026-08-11 profile adapter ownership follow-up: the duplicated Markdown
profile loader implementation was removed from both Executive and Aletheon
composition modules. The shared filesystem/frontmatter adapter now lives in
`crates/adapters/agent-profile/src/lib.rs`; both composition roots retain only
compatibility re-exports and continue to implement Runtime's `AgentProfilePort`
contract through the same loader. Eleven adapter tests, Executive/Aletheon
checks, and architecture acceptance pass. This closes the duplicate
profile-loader implementation, not the remaining Executive host composition
tree.

Canonical sudo deployment after the AgentStream/profile-adapter ownership moves
passed at SHA `242454c6fd378e87c08a66bb07157e9e4683dc26f37b49711ca266b832941a3a`
for release and installed binaries. Both daemon services are active with zero
restarts, and official client/Memory-Agent smoke checks passed.

### 2026-08-12 Runtime settlement receipt ownership follow-up

The settlement idempotency contract now lives in `runtime::settlement_receipts`
(`SettlementReceiptStore` plus the in-memory fixture). The concrete SQLite
receipt adapter moved to `adapters-sqlite::settlement::SqliteSettlementReceiptStore`;
Executive's settlement engine consumes the Runtime port and keeps only the
resource/evidence/application orchestration. The Executive path is a
compatibility re-export, not a second store implementation.

Verification:

- `bash scripts/cargo-agent.sh test -p adapters-sqlite --lib settlement`: 1 passed;
- `bash scripts/cargo-agent.sh test -p executive --lib application::agent_control::settlement`: 14 passed;
- `bash scripts/cargo-agent.sh check -p runtime -p adapters-sqlite -p executive --lib`: passed;
- `git diff --check`: passed.

The SQLite adapter still performs its compatibility `CREATE TABLE IF NOT EXISTS`
open-time initialization. The canonical `aletheon migrate` bundle and
open-without-DDL gate remain intentionally open; this slice does not claim that
migration-order requirement is complete.

Canonical installed-runtime verification after this slice passed via
`sudo bash scripts/aletheon.sh deploy` at SHA
`335ac7d11efece11c7ab370e233f1e8b6060dc70f2454ad1553902df71d517ec`.
`target/release/aletheon` and `/usr/bin/aletheon` match; machine-core and user
units are `active/running` with `NRestarts=0`; official Memory Agent and client
real-request smoke checks passed.

A follow-up migration-order deployment then passed at SHA
`b4bd2bb6671c8973e585a8a91fba5428b40da4dba08ba6efbc330009cc0c7e5e` after
moving settlement schema creation into the explicit adapter `migrate` phase;
`open` is now verified DDL-free by adapter tests. Release/installed parity,
stable daemons, and official client/Memory-Agent smoke all passed.

The settlement evidence contract and EventSpine adapter then moved into
`runtime::settlement_evidence`; Executive now supplies only settlement
resource orchestration and compatibility exports. Settlement focused tests
(14) and Aletheon/Executive checks pass. Installed acceptance passed at SHA
`a1b803a3cf4f7117abc3a30755a54362acf5acd3a43bd89b493c966346a56003`, with
release/installed parity, both daemons active and zero restarts, and official
client/Memory-Agent smoke passing.

The Runtime settlement resource/lease ports (`SettlementResourcePort` and
`SettlementLeasePort`) now own the contract; Executive retains concrete host
resource and repository adapters only. Focused settlement tests and checks
pass. Installed verification passed at SHA
`21df331853994ce6d116447a2ec2fda42ecc3a4e9b7e99813829156f57794dfb`, with
release/installed parity, active zero-restart daemons, and official smoke
checks.

The Runtime process-registration contract (`RuntimeProcessRegistrationPort`
and its fail-closed no-op) now lives in `runtime::process_registration`; the
Executive concrete kernel/repository binding remains only as a host adapter.
Focused Executive and Aletheon checks, architecture acceptance, and installed
runtime verification passed at SHA
`4e4533b7a0f099352384356ef33b74422fff4cab9cb005208b155a6aebba99d2`.
Release/installed parity and zero-restart daemon stability were observed.

The `RuntimeProcessRegistrationPort` compatibility caller-zero was completed:
Executive internals and tests now import the Runtime contract directly, and
Executive no longer re-exports that process-registration port/no-op. The
concrete `DurableRuntimeProcessRegistration` remains a host adapter by design.
Focused tests, architecture acceptance, and installed verification passed at
SHA `d8cb500b37667538b6ee002e3409ca981aec042a2b05240ca09a6089548cedb1`, with
release/installed parity and both daemons active at zero restarts.

The Runtime process-registration caller-zero follow-up was installed and
verified at SHA `d8cb500b37667538b6ee002e3409ca981aec042a2b05240ca09a6089548cedb1`.
The subsequent no-op import cleanup and same source state were redeployed at
SHA `d8cb500b37667538b6ee002e3409ca981aec042a2b05240ca09a6089548cedb1` with
release/installed parity, stable zero-restart daemons, and official smoke
checks passing.

### 2026-08-12 Runtime settlement-engine ownership cutover

The RA-05 settlement ordering/idempotency engine is now physically owned by
`runtime::settlement_engine::SettlementEngine`
(`crates/runtime/src/settlement_engine.rs:25-68`). Its quiesce, resource,
lease, evidence and receipt dependencies are narrow Runtime ports
(`crates/runtime/src/settlement_ports.rs:6-53`); the generation fence and
idempotency/reparent/terminal ordering remain inside Runtime. The Executive
module now keeps only host resource/repository adapters and a compatibility
re-export (`crates/executive/src/application/agent_control/settlement.rs:18-31`).
`LiveAgentRun` implements the Runtime quiesce port as a one-way host adapter
(`crates/executive/src/application/agent_control/settlement.rs:26-30`).

This advances the approved RA-05 requirement to migrate AgentControl
settlement ownership to Runtime (`docs/plans/2026-08-09-agent-kernel-v2-deepseek-implementation-plan.md:507-516`),
but does not claim RA-05 complete: rich AgentControl admission, mailbox,
recovery projection and the remaining Executive compatibility facade still
require caller-zero and installed recovery/rollback evidence.

Verification:

```text
bash scripts/cargo-agent.sh check -p runtime -p executive                 PASS
bash scripts/cargo-agent.sh test -p executive --lib settlement            PASS (27 tests)
bash scripts/cargo-agent.sh test -p executive --test agent_control_spawn \
  --test agent_recovery --test agent_mailbox                             PASS (21 tests)
bash scripts/cargo-agent.sh fmt --all -- --check                          PASS
```

Installed verification after this cutover passed via
`sudo bash scripts/aletheon.sh deploy`: release and `/usr/bin/aletheon` SHA
`8325ab6c17b205a5ad95a16d6055e7385754fbe228b1f3b41695bf66b8b1d8a0` matched;
machine/user daemons were active with `NRestarts=0`, and the official client
plus Memory-Agent smoke checks passed.

### 2026-08-12 Runtime admission-contract ownership cutover

The Agent admission contract is now physically owned by Runtime:
`AgentAdmissionRequest`, `AgentAdmissionPort`, `AgentAdmissionLease`, metrics,
storage intent, and cognitive-workspace narrowing live in
`crates/runtime/src/agent_admission.rs:13-136`. The convenience constructor
also mints through Runtime's `mint_agent_run_uuid` (`:31-43`); production
adapters receive the Runtime identity with `new_for_agent`.

Executive keeps only the concrete policy/budget adapter and a thin forwarding
wrapper (`crates/executive/src/application/agent_control/admission.rs:17-26`).
This removes another AgentControl authority contract from Executive while
preserving the existing budget implementation and test fixtures.

Verification:

```text
bash scripts/cargo-agent.sh test -p executive --test agent_admission \
  --test agent_control_spawn                                      PASS (18 tests)
bash scripts/cargo-agent.sh fmt --all -- --check                  PASS
```

RA-05 remains open because Executive still owns the concrete admission policy
adapter, rich lifecycle facade, recovery orchestration and host mailbox
operations; those are adapters to migrate next, not Runtime contract debt.

Installed verification after this admission-contract cutover passed via
`sudo bash scripts/aletheon.sh deploy`: release and `/usr/bin/aletheon` SHA
`b688bb9452b53ff7d9f451630796242ed6d89ae1fd25648d8cff0c9bcdeaee48` matched;
machine/user daemons were active with `NRestarts=0`, and the official client
plus Memory-Agent smoke checks passed.

### 2026-08-12 Executive Agent compatibility re-export cleanup

The obsolete Executive-only compatibility modules for Agent lifecycle,
mailbox, recovery and repository were deleted. Runtime contracts are now
re-exported directly from `crates/executive/src/application/agent_control/mod.rs`
(`mod.rs:54-75`), while the concrete budget policy and host adapters remain in
Executive. The deleted modules were:

```text
crates/executive/src/application/agent_control/lifecycle.rs
crates/executive/src/application/agent_control/mailbox.rs
crates/executive/src/application/agent_control/recovery.rs
crates/executive/src/application/agent_control/repository.rs
```

This removes four Executive compatibility surfaces without moving Pi or host
implementation code. Architecture inventories and the XRET ledger were
updated; `architecture-check` reports no additions.

Installed verification after this cleanup passed via
`sudo bash scripts/aletheon.sh deploy`: release and `/usr/bin/aletheon` SHA
`4a0f9f754b7627e9617d482645679305db6911b93e133355e377da4136ccd4d1` matched;
machine/user daemons were active with `NRestarts=0`, and official client plus
Memory-Agent smoke checks passed.

### 2026-08-12 Runtime recovery orchestration ownership cutover

The Runtime recovery coordinator now owns the paginated startup reconciliation loop,
recovery decision/transition ordering, host action boundary, and unreconciled-row
fence (`crates/runtime/src/agent_recovery.rs`). `AgentRecoveryHost` is the only
host callback seam: Executive supplies Kernel/process observations and concrete
settlement/resume adapters through `StartupRecoveryHost`
(`crates/executive/src/application/agent_control/mod.rs:189-339`).
`AgentControlService::reconcile_startup` now delegates to
`AgentRecoveryCoordinator::reconcile_with` and only retains the Runtime orphan
projection fence (`.../agent_control/mod.rs:579-641`).

Focused validation:

- `bash scripts/cargo-agent.sh test -p runtime --lib` — 80 passed.
- `bash scripts/cargo-agent.sh test -p executive --test agent_recovery` — 8 passed.
- `ARCH_SKIP_DEPENDENCIES=1 bash scripts/aletheon.sh acceptance architecture` —
  22 migrations, 67 acceptance IDs, 1110 Fabric public types; no additions.
- `git diff --check` — passed.

This advances RA-05 authority ownership but does not close the remaining
`AgentControlService` rich admission/settlement facade, caller-zero, installed
restart/reconcile, or rollback gates.

### 2026-08-12 Runtime orphan-projection fence cutover

`RuntimeAgentSupervisor::reconcile_missing_projections` now owns fencing
Runtime-admitted UUID runs whose host projection was lost in the admission
crash window (`crates/runtime/src/agent_writer.rs:130,1119-1187`). Executive
provides only the read-only `AgentProjectionPresence` adapter and consumes the
interrupted count from Runtime (`crates/executive/src/application/agent_control/mod.rs:342-360,610-626`).
This removes the UUID parsing and Runtime recovery/terminal writes from the
Executive startup facade while preserving the existing recovery test semantics.

Installed verification after this cutover passed at SHA
`481732a4bfc57308ea05e12b7f3bd65b28360c453c24f4093e833da0560f945d`; both
system/user daemons were active with zero restarts and official client/Memory
Agent smoke passed.

### 2026-08-12 Runtime background-resource registration cutover

The pure background producer registration lifecycle (`BackgroundResourceRegistration`)
now lives in `crates/runtime/src/agent_resources.rs` and is exported from Runtime.
Executive `application/agent_control/execution.rs` keeps only a compatibility
re-export; `AgentRuntimeInput` and `LiveAgentRun` therefore consume the Runtime
owner without duplicating the cancellation/stopped-state fence.

Focused validation passed: Executive `live_runs` (8 tests), Agent spawn (8),
Agent recovery (8), architecture acceptance, and `git diff --check`. Installed
verification passed at SHA
`56c4143ebd31a0cae243a898ad32328c10a671d344f8730f9cacf33453b25ac0` with both
daemons stable and official client/Memory Agent smoke passing.

This is an ownership cutover only; the rich Executive admission/execution
facade and final RA-05/XRET caller-zero gates remain open.

### 2026-08-12 Runtime recovery DTO ownership cutover

`AgentRecoveryRuntimeInput` is now a Runtime-owned recovery contract in
`crates/runtime/src/agent_recovery.rs`; Executive's execution module only
re-exports it for the compatibility launcher surface. The DTO carries the
identity-bearing host handle/request and opaque checkpoint reference without
owning launcher selection or concrete Pi behavior.

Focused Runtime/Executive recovery, spawn, and Pi RPC tests passed; architecture
acceptance passed. Installed verification passed at SHA
`4e6a8416f564f1350c37de3c75502d398071878ab0ab177a27bb095a1c9881ae`, with both
daemons stable and official client/Memory Agent smoke passing.

### 2026-08-12 Runtime context projection ownership cutover

The pure bounded Agent context projection/builder moved from Executive
`application/agent_control/context_fork.rs` to Runtime
`crates/runtime/src/agent_context.rs`. Executive's context module was removed;
root-level compatibility re-exports keep existing host callers source-compatible.
The Runtime module has no Executive/Kernel/concrete-adapter dependency and owns
context size, unsafe-content, artifact-reference, ordering, and omission rules.

Focused context, Runtime, spawn, and recovery tests passed; architecture
acceptance passed. Installed verification passed at SHA
`61c62c12765313239a37624af44fc6cb7156634def41698c592c0e7010e0bd80`, with both
daemons stable and official client/Memory Agent smoke passing.

### 2026-08-12 Runtime context projection ownership cutover

The pure bounded Agent context projection/builder moved from Executive
`application/agent_control/context_fork.rs` to Runtime
`crates/runtime/src/agent_context.rs`. Executive's context module was removed;
root-level compatibility re-exports keep existing host callers source-compatible.
Runtime now owns context size, unsafe-content, artifact-reference, ordering, and
omission rules without an Executive/Kernel/concrete-adapter dependency.

Focused context, Runtime, spawn, and recovery tests passed; architecture
acceptance passed. Installed verification passed at SHA
`61c62c12765313239a37624af44fc6cb7156634def41698c592c0e7010e0bd80`, with both
daemons stable and official client/Memory Agent smoke passing.

### 2026-08-12 Runtime live-Agent lifecycle ownership cutover

The pure live Agent lifecycle/resource owner moved from Executive
`application/agent_control/live_runs.rs` to Runtime
`crates/runtime/src/agent_live_runs.rs`. `LiveAgentRun`, `LiveAgentRuns`,
resource quiescing/reparenting, cancellation ownership, and the
`SettlementQuiescePort` implementation now live together in Runtime. Executive
keeps only root-level compatibility re-exports; workspace checkpoint remains a
local host trait implementation over the Runtime type.

The moved Runtime module contains no Executive/Kernel/concrete-adapter
dependency. Runtime tests (88), Executive admission/spawn/recovery tests,
architecture acceptance, and installed verification passed. Latest installed
SHA: `18c943d5df854947a817fb859ef588a27d6f5eddbe697198847810b0a0d924b9`.

### 2026-08-12 Runtime cleanup coordinator ownership cutover

The idempotent terminal Agent resource cleanup coordinator moved from Executive
`application/agent_control/cleanup.rs` to Runtime `crates/runtime/src/agent_cleanup.rs`.
Runtime now owns the lease expiry/terminal gate, worktree reclaimer port, bounded
batch, idempotent deletion, and cleanup report. Executive/adapters only consume
root-level compatibility exports and provide the concrete worktree adapter.

Runtime (88 tests), Executive cleanup/spawn/recovery tests, architecture
acceptance, and installed verification passed. Latest installed SHA:
`0151d09849e3fd995ab3ff41044a2a99d49478ca533e330c6fcd1a4b1ad95b13`.

### 2026-08-12 Runtime identity-contract ownership cutover

The pure validated Agent identity receipt and Fabric-to-Runtime capability
translation moved from Executive `application/agent_control/identity.rs` to
`crates/runtime/src/agent_identity.rs`. Runtime now owns the identity fields,
Runtime generation receipt, and capability mapping; Executive keeps only a
direct package re-export for its host orchestration call sites. Architecture
acceptance, Runtime (88 tests), Executive spawn/recovery/cleanup tests, and
`git diff --check` passed. Installed deployment passed with release and
`/usr/bin/aletheon` both at SHA
`1672f5a0e48fb3b41f31148398bbd908ca4feb1822d98167114b42876ca84ea6`;
core and user daemons are active with zero restarts.

### 2026-08-12 Runtime lifecycle-hook contract cutover

The Runtime-owned lifecycle hook contract, no-op sink, and bounded HookContext
builder moved to `crates/runtime/src/agent_lifecycle_hooks.rs`. Executive now
retains only the concrete Corpus sink adapter in
`application/agent_control/lifecycle_hooks.rs`; host execution passes the
Runtime Agent handle/task/workspace values into the Runtime context builder.
Runtime (88 tests), Executive spawn/recovery/cleanup tests, architecture
acceptance, and `git diff --check` passed. Installed deployment passed with
release and `/usr/bin/aletheon` at SHA
`4a587135bc7d586a834ae5504c21ff47bad18da4a79aa46988de6972aa0f4124`;
core and user daemons are active with zero restarts.

### 2026-08-12 Runtime bounded-admission policy ownership cutover

The bounded root-scoped Agent admission state machine moved from Executive
`application/agent_control/admission.rs` into
`crates/runtime/src/agent_admission_policy.rs`. Runtime now owns topology
capacity, FIFO fairness, depth/storage/budget policy, lease transitions,
settlement, revoke, and parent budget transfer. Executive retains only a thin
configuration conversion and Kernel budget-controller compatibility shell;
Runtime has no Kernel or Cognit dependency. Admission (10), Runtime (88),
Executive spawn/recovery/cleanup tests, architecture acceptance, and
`git diff --check` passed. Installed deployment passed with release and
`/usr/bin/aletheon` at SHA
`e6d9ea4fe9a2fc7fa0a01e2c02d9a17e1e9d0629d2126652a51fc78375f528c9`;
core and user daemons are active with zero restarts.

### 2026-08-12 Runtime settlement-adapter ownership cutover

Runtime now owns the concrete Agent settlement resource/lease adapters and
admission terminal helpers in `crates/runtime/src/agent_settlement_adapters.rs`:
`ManagedSettlementResourcePort`, `FailClosedSettlementResourcePort`,
`RepositorySettlementLeasePort`, `settle_admission`, and
`terminal_with_memory_flush`. Executive settlement retains only the concrete
Evaluation projection adapter and compatibility re-exports. Settlement
unit/audit tests (13), Runtime (88), architecture acceptance, and
`git diff --check` passed. Installed deployment passed with release and
`/usr/bin/aletheon` at SHA
`cbbf9187c071ff1920164b3023da6af576fad24c9eb7c397f44fd7be52ba61e7`;
core and user daemons are active with zero restarts.

### 2026-08-12 Runtime Agent candidate-contract ownership cutover

The bounded candidate projection contract moved from Executive
`application/conscious_core_ports.rs` into Runtime
`crates/runtime/src/agent_candidates.rs`. Runtime now owns
`CandidateCause`, `CandidateSubmission`, admission status/receipt, and
`AgentCandidateSubmissionPort`; Executive keeps the conscious Agora pool and
the `AgentCandidateProjector` as host/application adapters, with compatibility
re-exports. This is a contract ownership cutover, not a claim that the
conscious workspace implementation has moved. Runtime (90 tests), Executive
Agora/conscious focused tests (12), architecture acceptance, formatting, and
diff checks passed. Installed deployment and official client/Memory Agent smoke
passed at SHA `2196bdd5ef7cd94d29d8b17cb8b39bcb9ea72ec6b8fc9906c0952c247db5fa04`;
`target/release/aletheon`, `/usr/bin/aletheon`, and both running daemon
executables matched, with core/user services active and `NRestarts=0`.

### 2026-08-12 Runtime Agent wait-port ownership cutover

The bounded Agent state-change wait contract and production implementation
moved from Executive `application/agent_control/mod.rs` into
`crates/runtime/src/agent_wait.rs`. Runtime now owns the `AgentWaitTimer` port
and `SystemAgentWaitTimer` implementation using Tokio directly; Executive keeps
only a compatibility re-export for the rich host façade. Runtime wait-port
tests passed, and the architecture census records the Runtime owner as
`RA-A-17`.

Installed verification for the wait-port follow-up passed at SHA
`1a5e0c7b6df230b03d7022c056321fcb81c42384ea5e8fc63275a040f8e7fe4e`.
Runtime (92 tests), Executive package check, architecture acceptance,
format/diff checks, official client real-request smoke, and Memory Agent smoke
passed. `target/release/aletheon`, `/usr/bin/aletheon`, and both running daemon
executables matched this SHA; core and user services were active with
`ExecMainStatus=0` and `NRestarts=0`.

The AgentControl operation/spawn regression suite also passed after the
wait-port cutover: 6 operation tests and 8 spawn tests.

### 2026-08-12 Runtime Agent topology-route ownership cutover

Directional sibling-route state moved from Executive `AgentControlService` into
`crates/runtime/src/agent_topology.rs`. Runtime owns bounded idempotent
permit/revoke/lookup storage; Executive retains the host-side durable identity
and live-tree validation before consulting the Runtime route policy. Runtime
unit tests, AgentControl operations (6), Executive check, architecture, and
format/diff checks passed. Rich execution/mailbox façade and final RA-05/XRET
closure remain open.

Installed verification for the topology-route cutover passed at SHA
`e9e9d022ad2e0150bcd31d79615f2d85840f97c4a686f5736b5bdb1e32698d1c`; all four
executable digests matched, official client/Memory Agent smokes passed, and
core/user services remained active with `NRestarts=0`.
