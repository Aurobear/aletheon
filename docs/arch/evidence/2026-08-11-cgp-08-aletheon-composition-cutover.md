# CGP-08 continuation: binary-owned composition launcher

Date: 2026-08-11

## Requirement and current code anchors

- Requirement: `aletheon` must become the full-graph composition root and the
  Executive host/composition tree may remain only as a one-way rollback seam
  until XRET-04 (`composition-gateway-presentation-extraction.md:364-372`).
- Before this slice, the CLI entered `executive::host::launcher` directly
  (`config/architecture/composition-root-census.tsv:CGP-C-01`).
- After this slice, production CLI routes enter
  `crates/aletheon/src/main.rs:429-504,648-653` through
  `crates/aletheon/src/launcher.rs:34-176,394-416`; the Executive launcher is
  no longer a production caller.

## Change

`crates/aletheon/src/launcher.rs` now owns the binary-facing Core, Daemon,
Exec, and ensure-user-daemon lifecycle contracts. Core/daemon lifecycle
implementation has moved to `crates/aletheon/src/wiring.rs:28-113`; it opens
the runtime paths, acquires the daemon authority lock, composes the user
runtime, and starts the inference core. The UserRuntime composition itself is
now owned by `crates/aletheon/src/wiring/user_runtime.rs:22-360`; it is a
typed migration of the previous Executive composition module and retains the
same cleanup/shutdown contracts. The old
`crates/aletheon/src/wiring/exec.rs` remains the rollback implementation and
existing Executive test seam, but it is not referenced directly by
`aletheon/src/main.rs`. The wiring now owns the Core RPC client/protocol,
readiness lifecycle, and Exec host path (`crates/aletheon/src/wiring/core_rpc/`,
`crates/aletheon/src/wiring/readiness.rs`, and
`crates/aletheon/src/wiring/exec.rs`). Launcher request/error DTOs are
binary-owned in `crates/aletheon/src/launcher.rs`; the Exec implementation
uses the Core RPC client and Executive's public application/session contracts,
not the Executive host launcher. The daemon bootstrap, `RequestHandler`, and
`UnixServer` now live in the binary-owned
`crates/aletheon/src/wiring/daemon/` tree (49 modules); `user_runtime.rs`
invokes those local types rather than Executive host daemon construction.
`crates/aletheon/src/wiring/domain.rs` owns only binary composition handles;
Executive remains the owner of application ports and concrete domain services.
SessionGateway now consumes narrow `SessionContextView` and
`SessionDebugView` ports (`crates/executive/src/core/session_gateway/gateway.rs`),
so the binary daemon owns its DebugHandler implementation and has no production
`executive::host::*` import. The shared context projection is no longer hosted
under `executive::host::daemon`; the old host path is a one-way re-export only.
The doctor facade is implemented under `crates/aletheon/src/wiring/doctor.rs`.
The CLI Exec builder is now binary-owned in
`crates/aletheon/src/wiring/exec_session.rs`; it uses Executive application
ports and adapters but no Executive host launcher.
The machine-core production path is now binary-owned as well:
`crates/aletheon/src/wiring/core_runtime.rs` provides
`MachineInferenceRuntime`, while `crates/aletheon/src/wiring/core_rpc/server.rs`
provides the authenticated `CoreRpcServer`. `aletheon::wiring::run_core` no
longer calls `executive::core::SystemCoreRuntime` or
`executive::host::core_rpc`; the old implementation remains only in the
Executive rollback tree. The copied server seam is covered by
`crates/aletheon/tests/core_rpc_auth.rs` (five tests).

The production caller census is now empty for Executive host symbols outside
the Executive rollback tree:

```text
rg -n 'executive::host' crates -g '*.rs' -g '!**/tests/**'  # no matches
```

`WorkspaceArgs::exec_launch` (`crates/aletheon/src/workspace.rs:17-22`) now
returns the binary-owned launch request. Configuration, doctor, and extension
inspection also enter through `aletheon` facade modules rather than direct CLI
imports of Executive host/application symbols.

## Boundary decision: gateway crates

`gateway-protocol` and `gateway-client` are not remnants to merge into the
server `gateway` crate:

```text
interact / ACP / CLI -> gateway-client -> gateway-protocol
                                      ^
                                shared wire contract
server adapter -> gateway (TypedApplicationPort)
```

The protocol crate is transport-free schema/cursor/error vocabulary; the client
crate owns framing, correlation, and reconnect; `gateway` owns server/channel
routing and application ports. None depends on Executive. The remaining
Executive typed handler is an adapter implementation of the Gateway port, not a
reason to invert the dependency direction.

## Verification

```text
bash scripts/cargo-agent.sh check -p aletheon --bin aletheon   PASS
bash scripts/cargo-agent.sh test -p aletheon --lib             PASS (138)
bash scripts/cargo-agent.sh test -p executive --lib            PASS (792)
bash scripts/cargo-agent.sh test -p aletheon --test workspace_cli PASS (3)
bash scripts/cargo-agent.sh test -p gateway-client --lib       PASS (7)
bash scripts/cargo-agent.sh test -p gateway-protocol --lib     PASS (3)
bash scripts/cargo-agent.sh fmt --all -- --check                PASS
ARCH_SKIP_DEPENDENCIES=1 bash scripts/aletheon.sh acceptance architecture PASS
sudo bash scripts/aletheon.sh deploy                              PASS
bash scripts/aletheon.sh verify                                  PASS
```

Installed provenance after deploy: `target/release/aletheon`,
`/usr/bin/aletheon`, and `/home/aurobear/.cache/aletheon-cargo/target/release/aletheon`
all resolved to SHA-256
`67c340aa0d4da350bb7e52446ebcb9e0947927b38106bbacbf7bad01e4eecfca`.
Both `aletheon-core.service` and the user `aletheon.service` were active with
`NRestarts=0` and `ExecMainStatus=0`. Three fresh real-TUI captures completed
one terminal turn each, rendered `❯`, and contained no forbidden provider or
daemon errors:

```text
.scenario-runs/tui-events/full-daemon-r3-1/events.jsonl
.scenario-runs/tui-events/full-daemon-r3-2/events.jsonl
.scenario-runs/tui-events/full-daemon-r3-3/events.jsonl
```

Remaining CGP-08 work is now concentrated in the final Executive adapter
retirement inventory and the still-open RA-05 AgentSupervisor caller-zero
gate. The configuration, direct composition callers, and extension lifecycle
owner have moved to binary-owned seams in the continuations below. The
Gateway client/protocol split is intentionally retained; the remaining server
work is route-family caller-zero and narrowing the Aletheon composition
adapter behind `gateway-server::handlers::typed::TypedApplicationPort`, not
merging client and server packages. The compatibility roots remain
`IN_PROGRESS` in
`config/architecture/composition-root-census.tsv` because this slice does not
claim XRET-04 physical deletion of the rollback tree.

### 2026-08-11 continuation: machine-core and extension inspection cutover

`aletheon::wiring::run_core` now starts the binary-owned
`MachineInferenceRuntime` and local authenticated `CoreRpcServer`; no
production source under `aletheon`, `interact`, `gateway-client`, or
`gateway-protocol` imports `executive::host`. Offline extension inspection is
also a binary-owned read-only seam (`aletheon::extension::inspect_archive`);
daemon lifecycle mutations continue through the typed Gateway application
port rather than a CLI-local store.

Focused validation passed: `aletheon` library (138), local Core RPC auth (5),
`gateway-client` (7), `gateway-protocol` (3), and Executive library (792), plus
formatting, diff checks, and architecture acceptance. Installed deployment
and verification passed with SHA-256
`7b0bd7b694bd4e64e206e42d40b26c13ac3adbb568277a693c8589ac6d35a52b` across
release, `/usr/bin/aletheon`, and both running daemons; both restart counters
remain zero. Three fresh official user-socket TUI runs completed with stable
prompt frames, one `turn_done` each, no render checks, and no provider or
inference errors:

```text
.scenario-runs/tui-events/full-daemon-r4-1/events.jsonl
.scenario-runs/tui-events/full-daemon-r4-2/events.jsonl
.scenario-runs/tui-events/full-daemon-r4-3/events.jsonl
```

### 2026-08-11 continuation: configuration and composition caller cutover

The typed layered configuration owner is now `aletheon-config` (`crates/aletheon-config/src/lib.rs:1-18`).
`aletheon` exposes it through its binary facade (`crates/aletheon/src/lib.rs:4-9`),
while the Executive config module is only a one-way compatibility re-export.
The owner crate's focused suite passed (27 tests); Executive's layered-config
contract (11), governed-review RPC (3), and Executive library (765) also pass.

The remaining direct Executive composition callers were moved behind binary-owned
wiring adapters. `ExecSessionBuilder` now calls local corpus and coordinator
composition (`crates/aletheon/src/wiring/exec_session.rs:140-146,228-235`);
daemon profile loading uses the local agent loader
(`crates/aletheon/src/wiring/daemon/bootstrap/runtime.rs:10-11`), and daemon
session/evaluation construction uses local coordinator seams
(`crates/aletheon/src/wiring/daemon/bootstrap/session_infrastructure.rs:61-65`,
`crates/aletheon/src/wiring/daemon/bootstrap/services.rs:831-838`). The adapter
module is enumerated at `crates/aletheon/src/wiring/composition/mod.rs:8-15`.

Evidence:

```text
rg -n 'executive::composition|executive::host' crates/aletheon/src \
  crates/interact/src crates/gateway-client/src crates/gateway-protocol/src
# only a documentation mention in wiring/composition/mod.rs; no production import
bash scripts/cargo-agent.sh check -p aletheon --bin aletheon       PASS
bash scripts/cargo-agent.sh test -p aletheon --lib                 PASS (162)
ARCH_SKIP_DEPENDENCIES=1 bash scripts/aletheon.sh acceptance architecture PASS
```

This reduces the CGP-08 production-caller surface, but does not claim XRET-04
physical deletion: Executive host/composition and application extension
implementations remain compiled rollback/application seams, and the installed
release/TUI acceptance must be rerun after this source cutover.

### Installed verification after this cutover

`sudo bash scripts/aletheon.sh deploy` and `bash scripts/aletheon.sh verify` both
passed. The release binary, `/usr/bin/aletheon`, and both running daemon
executables are SHA-256
`55b17dca6418ba3266c2b0c73eda707d29ee171ec9df656cac9c33f6319478d1`; the
machine and user units report `NRestarts=0` and `ExecMainStatus=0`, and the
official user-socket request smoke passed.

The diagnostic fresh TUI captures are under
`.scenario-runs/tui-events/full-daemon-r5-{1..6}/events.jsonl`. Runs r5-1,
r5-2, r5-4, and r5-6 reached an authoritative `turn_done`, stable prompt, and
empty render-check list. r5-3 exposed a duplicate-render check and r5-5 used an
isolated empty working directory, causing expected repository-tool read errors;
therefore this set is evidence of runtime viability, not a claimed three-run
acceptance streak. The existing r4 streak remains the latest clean three-run
official-repository acceptance, while a new clean streak should be rerun after
the extension lifecycle slice.

### 2026-08-11 continuation: extension lifecycle owner cutover

Production extension lifecycle now has a binary-adjacent owner crate,
`aletheon-extension`, covering install/manage, activation policy, immutable
runtime snapshots, runtime routing, and coordinator publication
(`crates/aletheon-extension/src/lib.rs:1-31`). Aletheon daemon bootstrap and
CLI exec paths use this owner; the production source census has no
`executive::application::extension_*` import under `crates/aletheon/src`.
Executive's extension modules remain only as compatibility/rollback sources and
are recorded as `SPLIT -> aletheon-extension` in
`config/architecture/executive-surface-ledger.tsv`.

The Executive admin use case now consumes the narrow
`ExtensionSkillCatalogPort` projection (`crates/aletheon/src/wiring/application/admin_service.rs:483-520`)
instead of a concrete snapshot type. This keeps the application route stable
while allowing the Aletheon-owned runtime view to be injected through
`with_extension_catalog`.

Validation:

```text
bash scripts/cargo-agent.sh test -p aletheon-extension --lib  PASS (7)
bash scripts/cargo-agent.sh test -p executive --lib             PASS (765)
bash scripts/cargo-agent.sh test -p aletheon --lib              PASS (162)
bash scripts/aletheon.sh acceptance architecture               PASS (36 dependencies)
```

### 2026-08-11 installed acceptance after extension cutover

The current release was deployed and verified with:

```text
sudo bash scripts/aletheon.sh deploy  PASS
bash scripts/aletheon.sh verify      PASS
SHA-256 (target/release/aletheon, /usr/bin/aletheon, and both running daemon executables):
9216287ba1b6d2643898a465c4735e43e8c26c593b6f47645ee8550080393d9b
user aletheon.service:      NRestarts=0, ExecMainStatus=0
system aletheon-core.service: NRestarts=0, ExecMainStatus=0
official user-socket request smoke: PASS
```

Three fresh official-repository TUI runs were then captured through the
official user socket:

```text
.scenario-runs/tui-events/full-daemon-r6-1/events.jsonl  turn_done=true, stable=true, prompt_visible=true, checks=[]
.scenario-runs/tui-events/full-daemon-r6-2/events.jsonl  turn_done=true, stable=true, prompt_visible=true, checks=[]
.scenario-runs/tui-events/full-daemon-r6-3/events.jsonl  turn_done=true, stable=true, prompt_visible=true, checks=[]
```

All three runs completed without `provider_unavailable`,
`provider_rejected_request`, or rendered inference errors. This is the current
installed-runtime acceptance evidence for the composition and extension-owner
cutovers; the compatibility Executive sources remain intentionally retained
until the XRET-04 deletion gate.

### 2026-08-11 CGP-03 server-owner extraction

The typed Gateway dispatch owner is now the independent
`gateway-server` package (`crates/gateway-server/src/handlers/typed.rs`). Its
`TypedApplicationPort`/`TypedRouteHandler` surface depends on
`gateway-protocol` only; Aletheon’s official daemon server imports this owner
(`crates/aletheon/src/wiring/daemon/server.rs`) rather than the compatibility
`gateway::handlers::typed` module. The old module remains compiled only as a
compatibility surface for Executive’s retained host tree; it is not the
production Aletheon route.

Validation:

```text
bash scripts/cargo-agent.sh test -p gateway-server --lib PASS (7)
bash scripts/cargo-agent.sh check -p aletheon --bin aletheon PASS
bash scripts/aletheon.sh acceptance architecture PASS (36 dependencies)
```

The production profile path is also binary-owned: Aletheon’s
`MarkdownAgentProfileLoader` implements Runtime’s `AgentProfilePort`, while
the Executive loader is retained only as a split/XRET-04 compatibility source
(`crates/aletheon/src/wiring/composition/agent_loader/mod.rs:161-170`,
`config/architecture/composition-root-census.tsv:11`).

### 2026-08-11 installed cutover recheck

The current tree was rebuilt and deployed after the server-owner extraction and
typed adapter narrowing:

```text
target/release/aletheon    d976fcc6cb22b9c1af6c93528e10aa19961afe5f04d1b0ae16e205aa8e78d563
/usr/bin/aletheon          d976fcc6cb22b9c1af6c93528e10aa19961afe5f04d1b0ae16e205aa8e78d563
machine core executable    /usr/bin/aletheon (same digest)
user daemon executable     /usr/bin/aletheon (same digest)
```

`sudo bash scripts/aletheon.sh deploy` and `bash scripts/aletheon.sh verify`
passed. `aletheon-core.service` and the user `aletheon.service` are both
`active/running` with `NRestarts=0`; the official client real-request smoke and
Memory Agent protocol smoke passed. The typed adapter now carries
`HandlerPorts` plus the narrow `TypedDaemonUseCases` application port rather
than a `RequestHandler` field
(`crates/aletheon/src/wiring/daemon/handler/typed_gateway.rs:23-124`).
This closes the current installed provenance gate for this slice; active-child
reconnect/settlement, rollback-binary, and Executive XRET-04 physical deletion
remain open.

A final adapter-constructor correction was deployed afterward. The current
release and installed/runtime digest is `c1eb9eac6437218457d41b8e23e0aa012275208d957ac6e78847f8bd1eb6e5b9`; both daemon units remain active with `NRestarts=0`, and deployment verification passes.

After removing the unused session-list method from the typed application port,
`sudo bash scripts/aletheon.sh deploy` was rerun. Current release, installed,
machine-core, and user-daemon digest is
`6b3198939e0e809a5385be1a43c754bf7b62526424104727c8379ec30b4743ce`; both
units are active with `NRestarts=0`, and deployment verification plus official
client/Memory-Agent smoke pass.

### 2026-08-11 gateway server dependency convergence

The retained Executive host typed route now imports the independent
`gateway-server` owner directly (`crates/aletheon/src/wiring/daemon/server.rs:17`,
`crates/executive/src/host/daemon/handler/typed_gateway.rs:8`). The former
`gateway::handlers::typed` compatibility re-export and the `gateway ->
gateway-server` dependency were removed; `gateway` is now only the neutral
channel dispatcher/store/transport package. The Executive dependency is a
compatibility host edge, while the production Aletheon daemon already imports
`gateway-server` directly. Gateway (38), Executive (765), Aletheon checks,
formatting, and architecture acceptance pass.

### 2026-08-11 Turn/Gateway seam narrowing

`TurnPipeline` no longer imports or stores the concrete Executive
`SessionGateway`. Its post-turn update now consumes the narrow
`application::turn_pipeline::TurnStateSink`; the compatibility SessionGateway
implements that port in `core/session_gateway/session_state.rs`, while the
Aletheon composition supplies it through `TurnPipelineResources`.
This removes a direct Gateway concrete dependency from the shared application
pipeline without claiming that the compatibility read model itself has already
been physically deleted.

The TurnStateSink seam and direct Executive->gateway-server dependency were
then included in a fresh release deployment. Current release/installed digest:
`1797eb19662c03d3e16089d37b294002ee8af89918b41fce5c6238df3d8653dc`.
Both daemon units remain active with `NRestarts=0`; deployment verification,
official client request, and Memory-Agent protocol smoke pass.

### 2026-08-11 SessionGateway application-port narrowing

The Aletheon daemon handler ports no longer expose the concrete Executive
`SessionGateway`; handler consumers receive the local `SessionGatewayPort`
trait, with one composition adapter forwarding to the retained compatibility
gateway (`crates/aletheon/src/wiring/daemon/handler/ports.rs`). `TurnServices`
also stores that port, while the composition root still constructs the single
compatibility instance. This narrows the production handler boundary without
inventing a second session authority.

Focused Aletheon (162) and Executive (765) tests, formatting, architecture
acceptance, and `git diff --check` pass. The release was redeployed with
`sudo bash scripts/aletheon.sh deploy`; release, installed, machine-core, and
user-daemon binaries all report SHA-256
`3904e7809c5fa6ea6073ae56e6fd1e3294b7e6a291a01bd419e7c97f598df571`.
Both daemon units are `active/running` with `NRestarts=0`; official client and
Memory-Agent protocol smoke checks pass.

### 2026-08-11 Runtime Agent lifecycle reducer ownership

RA-05's pure Agent lifecycle reducer and daemon-generation fence now live in
Runtime (`crates/runtime/src/agent_lifecycle.rs` and
`crates/runtime/src/generation_fence.rs`). Executive retains compatibility
re-export modules only; it no longer contains a second implementation of these
state transitions. Runtime's library suite (77) and Executive/Aletheon focused
suites pass. The resulting release was deployed and verified at SHA
`c09a49b52f42fe0bf2991835c25c369a2406698c2a9543dcadfed791d3de5be4`, with both
daemons active and zero restarts.

### 2026-08-11 Telegram channel adapter extraction

The concrete Telegram HTTP/long-poll transport and DTOs moved out of
`crates/gateway/src/adapters/telegram/` into the independent
`gateway-channel-telegram` package. `gateway` now owns only neutral channel
contracts/dispatcher/store; Aletheon and the retained Executive bootstrap
select the Telegram adapter explicitly at composition. The old
`gateway::build_telegram_transport` facade was removed. The adapter's 25 tests,
Gateway/Executive/Aletheon checks, and architecture acceptance pass. This
closes the concrete Telegram transport split while CGP-08 Executive caller-zero
and XRET deletion remain open.

Canonical sudo deployment after the Telegram extraction passed at SHA
`641f0d87dde3102aefd15970029943ddde909fd50807bfad3b146d51d6932807` for
release and installed binaries. Both daemon units are active/running with
`NRestarts=0`; official client and Memory-Agent smoke checks passed.

### 2026-08-12 Approval application/delivery port split

The repository-shaped `ChannelApprovalPort` was removed from Gateway. Approval
callbacks and pending queries now consume `ApprovalApplicationPort`, while
notification outbox receipts consume the independent
`ChannelApprovalDeliveryPort`. Executive's `ApprovalRepositoryPort` is an
adapter implementing both capabilities; daemon bootstrap and the Telegram
poller inject the application port instead of reaching into the repository.
The approval-channel and Gmail goal-draft integration tests pass, and the
architecture gate reports no new dependency paths.

Canonical sudo deployment after this port split passed. Release and installed
`aletheon` binaries both have SHA-256
`44dac5fc90bac4b2fbda6b30b23e15782e938d1566d1f6d000a75da221c9f7f2`; the
system `aletheon-core.service` and user `aletheon.service` are active/running
with `NRestarts=0`. Official client and Memory-Agent protocol smoke checks
passed.

### 2026-08-12 ChannelStore compatibility removal

All production and test callers now construct `adapters_sqlite::ChannelStore`
directly. The `gateway::ChannelStore`/`InsertOutcome` compatibility re-export
was deleted; Gateway dispatch continues to consume only
`ChannelProjectionStore`. This closes the compatibility-removal item noted in
the earlier ChannelProjectionStore evidence.

### 2026-08-12 Gateway router and typed Application ports

The old `dispatcher.rs`/`ChannelDispatcher` symbol was replaced by the
`gateway::router::ChannelRouter` owner. Turn and Goal calls now cross
`ChannelTurnApplicationPort` and `ChannelGoalApplicationPort`; the channel
turn request is a typed `ChannelTurnRequest`, and Executive alone converts it
to the full `ClientIntent` (`crates/gateway/src/ports.rs:87-115`,
`crates/aletheon/src/wiring/adapters/channel/daemon_adapter.rs:348-376`). Goal
progress, approval resolver, and external-event capability contracts were
moved out of the mixed registry into the ports boundary. Telegram and
Executive/Aletheon callers no longer import `gateway::dispatcher`.

Focused channel, approval, Gmail, Google, Goal, and restart-recovery tests
pass; formatting, `git diff --check`, and architecture-check pass. This is an
incremental route cutover: the remaining work is to delete the legacy handler
business logic and move Session/Goal/Approval command mapping fully into the
Application route owner as required by CGP-03.

Canonical sudo deployment after the router/typed-port cutover passed at SHA
`bfb21cecdc47c27a46b418e76657b9e6c99021b6fb7c88ddabf5f7cff8022b4a`; release
and installed binaries match, both daemon units are active/running with
`NRestarts=0`, and official client/Memory-Agent smoke checks passed.

The follow-up external-read slice moved the preflight contract to
`ChannelReadApplicationPort`/`ChannelReadDecision`; `ChatHandler` no longer
imports the external-read contract or provider account interface, and the
Executive/Aletheon composition roots inject the typed port. Gateway unit and
Google Telegram query tests pass. Canonical sudo deployment after this slice
passed at SHA
`0a66d0f4df56e457370985439f679bc1918fcd860a04fdf3f5083a18a67c158d`, with
release/installed parity and both daemon units stable.

The channel chat route now emits `ChannelTurnRequest { principal,
correlation_id, SubmitPromptIntent { session_id: None, ... } }`; the Gateway
does not mint a session id. The Application adapter owns conversion to
`ClientIntent` and the remaining principal-scoped fallback, leaving the next
step (canonical Runtime session selection) at the Application/Runtime seam.
`ExternalAccountDirectory` also moved to the ports boundary. Focused channel
and external-read tests remain green.

Canonical sudo deployment after this session/read-port slice passed at SHA
`0ce8124d6350604f41cecaadf5b99de03696bdcc4fb2c50364dcb1b70a3df6fc`; release
and installed binaries match, both daemon units are active/running with
`NRestarts=0`, and official client/Memory-Agent smoke checks passed.

The provider-aware `ExternalReadPreprocessor` implementation was physically
moved out of `gateway/src/handlers` into
`crates/executive/src/adapters/channel/external_read.rs`; Gateway retains only
the typed `ChannelReadApplicationPort` contract. The old Gateway
`handlers/external_read.rs` file and module export are gone, and Executive /
Aletheon composition roots inject the adapter. Architecture inventory was
updated and passes with no new paths.

Canonical sudo deployment after the external-read adapter move passed at SHA
`faafbcf7a72951fcc6037516f244e3f033d4a5ebce5999a9aea14ff64600706d`; release
and installed binaries match, both daemon units are active/running with
`NRestarts=0`, and official client/Memory-Agent smoke checks passed.

### 2026-08-12 canonical channel session and projection adapter cutover

The owner-only channel no longer derives a `ThreadId` from the authenticated
principal. `ChannelChatAdapter` deliberately leaves `session_id` unset, and
the Executive channel Application adapter now requires either an explicit
session or the canonical session selected by daemon composition via
`with_default_session` (`crates/aletheon/src/wiring/adapters/channel/daemon_adapter.rs`).
Both Aletheon and retained Executive bootstrap pass the Runtime-created
initial canonical session into Telegram wiring; an unset production selection
fails closed instead of creating a principal-named session.

The physical SQLite `ChannelStore` binding was also moved out of Gateway's
crate dependency surface. Gateway owns only `ChannelProjectionStore` and its
neutral `ChannelInsertOutcome`; the concrete binding is the local Executive
composition wrapper `ExecutiveChannelProjectionStore` over `adapters-sqlite`
(`crates/executive/src/adapters/channel/channel_projection.rs`). This removes
the persistence adapter dependency from `crates/gateway/Cargo.toml` while
preserving one injected store instance at each composition root.

Gateway, adapter, client/protocol, channel-recovery, Goal, Approval, Gmail,
and Google query tests pass; formatting, diff checks, and architecture-check
pass with `22 findings, 36 dependencies, 4 paths; no additions`.

Canonical sudo deployment passed at SHA
`f08031fc2fb07cc5274c80ff7a782dd52bed849098f0d6b773cbfb857abca1b1`; release
and `/usr/bin/aletheon` match, both daemon units are `active/running` with
`NRestarts=0`, and official client plus Memory-Agent smoke checks passed.

The Goal and Approval business handlers are now Application adapters:
`ChannelGoalCommandPort` is implemented by
`DaemonChannelGoalCommandAdapter`, and approval callbacks use
`ChannelApprovalCallbackPort` implemented by
`DaemonChannelApprovalCallbackAdapter`. Gateway `handlers/goal.rs` is now only
a bounded syntax-to-port adapter; `handlers/approval.rs` was deleted. The
router no longer owns an approval repository or resolver registry. Focused
Goal, Approval, Gmail, and channel recovery tests pass.

Canonical sudo deployment after the Goal/Approval route cutover passed at SHA
`dfa40d5016d97211bdf274f041323a2a2f1ba2e85f098bfa101fde990bf7a39c`; release
and installed binaries match, both daemon units are active/running with
`NRestarts=0`, and official client/Memory-Agent smoke checks passed.

### 2026-08-11 Channel SQLite adapter extraction

The concrete `ChannelStore` and channel schema implementation moved from
`crates/adapters/sqlite/src/channel.rs` to
`crates/adapters/sqlite/src/channel.rs`. Gateway now consumes the persistence
adapter through a compatibility re-export and no longer reaches into the
SQLite connection directly; reject/fail/mark-outbox operations are explicit
store methods. Six adapter tests and seven Gateway unit tests pass. The next
CGP store slice can replace the compatibility concrete surface with a
`ChannelProjectionStore` port; this extraction does not claim that port cutover
is complete.

Canonical sudo deployment after the ChannelStore extraction passed at SHA
`9858bd01ad5fbcc811583cffbd0aa107f0bb9de2ead56377f3aa9a4356135684` for
release and installed binaries. Both daemon services are active/running with
`NRestarts=0`; official client and Memory-Agent smoke checks passed.

### 2026-08-12 ChannelProjectionStore port cutover

Gateway dispatch now consumes the Gateway-owned `ChannelProjectionStore` port
rather than a concrete SQLite connection. `adapters-sqlite::ChannelStore`
implements the port, while `gateway::ChannelStore`/`InsertOutcome` remain
compatibility re-exports for existing callers. Reject/fail/outbox persistence
is fully expressed through the port. Gateway, adapter, and Executive channel
recovery/approval/goal tests pass; the remaining work is removing the
compatibility re-export and moving schema migration invocation to the canonical
Aletheon migration phase.

Canonical sudo deployment after the ChannelProjectionStore port cutover passed
at SHA `3dd84c19b77f720e3149b84bb8a08686a4a6950eea0784331ce35bfa801665e5` for
release and installed binaries. Both daemon services are active/running with
`NRestarts=0`; official client and Memory-Agent smoke checks passed.

### 2026-08-12 Typed chat route and direct Application invocation

`ChatHandler` is now a thin Gateway capability over `ChannelChatCommandPort`;
read preflight, `SubmitPromptIntent` construction, workspace policy, and
session-neutral request shaping live in `ChannelChatAdapter` at
`crates/gateway/src/ports.rs`. The router and both daemon composition roots
inject that adapter, while the Google/Telegram query fixture uses the same
typed seam. The Executive channel turn adapter no longer re-enters the
presentation `CommandDispatcher`; it invokes its typed `CommandUseCases`
implementation directly after Gateway classification
(`crates/gateway/src/handlers/chat.rs`, `crates/gateway/src/router.rs`,
`crates/aletheon/src/wiring/adapters/channel/daemon_adapter.rs`).

Focused Gateway/Executive checks, Google Telegram, channel recovery, Goal,
Approval, and Gmail tests pass; formatting, `git diff --check`, and the
architecture census pass (`22 findings, 36 dependencies, 4 paths`).

Canonical sudo deployment passed at SHA
`dda07fa069425218486c53a3bf889129c375582fca51c1c2878a7a636cd008dd`; the
release and `/usr/bin/aletheon` binaries match, both daemon units are
`active/running` with `NRestarts=0`, and official client plus Memory-Agent
smoke checks passed.

### 2026-08-12 Executive core/adapter configuration-owner convergence

The remaining non-host Executive runtime and adapter modules no longer reach
through `executive::composition::config`: core orchestration, RuntimeCore,
SystemCoreRuntime, SessionGateway snapshots, supplemental-memory adapters, and
the Pi runtime now consume `aletheon_config` directly. The public Executive
configuration re-export also points at `aletheon_config`; the old composition
module remains only as a rollback/test surface (`crates/executive/src/lib.rs:26-31`,
`crates/executive/src/core/mod.rs:18-21`). This closes the configuration-owner
dependency in the code that remains behind the Executive application/runtime
facade without deleting the host rollback tree required by CGP-08/XRET-04.

Evidence commands:

```text
rg -n 'crate::composition::config|executive::composition::config' \
  crates/executive/src/application crates/executive/src/adapters crates/executive/src/core
# no matches
bash scripts/cargo-agent.sh check -p executive -p aletheon              PASS
bash scripts/cargo-agent.sh test -p executive --lib                    PASS (751)
bash scripts/cargo-agent.sh fmt --all -- --check                       PASS
bash scripts/libexec/aletheon/architecture-check.sh                    PASS
git diff --check                                                        PASS
```

Canonical sudo deployment passed at SHA
`a7c9c3a0a7eb9c17d2d4b25500eb6ee8fd1175864a0f94673f6d4857aa6cb619`;
`target/release/aletheon` and `/usr/bin/aletheon` match, the system core and
user daemon are active with `NRestarts=0`, and official client plus
Memory-Agent smoke checks passed.

### 2026-08-12 Aletheon exec single-coordinator composition

The binary-owned `TurnService` no longer creates an in-memory
`TurnCoordinator`/`CanonicalSessionStore` and then replaces it. Its constructor
now requires the already-composed coordinator, and `ExecSessionBuilder` passes
the same coordinator used for the CLI turn (`crates/aletheon/src/wiring/composition/turn_service.rs:31-51`,
`crates/aletheon/src/wiring/exec_session.rs:251-258`). The dead in-memory and
alternate event-spine helpers were removed from the binary composition module;
the only production session-store composition seam is now the injected
`compose_session_store` used by `SessionInfrastructure`
(`crates/aletheon/src/wiring/composition/turn_coordinator.rs:37-58`). This
eliminates a second in-memory composition root during every CLI exec startup.

`aletheon` library tests pass (151), formatting, architecture-check, and diff
checks pass. Canonical sudo deployment passed at SHA
`a617a596917ef9f0628dbda6d57372128466d6c207afb040cf33242d8048738c`; release
and installed binaries match, system core and user daemon are active/running
with zero restarts, and official client plus Memory-Agent smoke checks passed.

### 2026-08-12 Canonical channel session and persistence dependency cleanup

Telegram channel composition now receives the Runtime-minted canonical initial
session and injects it into `DaemonChannelTurnApplicationPort`; a channel
principal is no longer silently reused as a session identifier. Gateway's
projection contract owns `ChannelInsertOutcome`, while the concrete SQLite
mapping lives in `ExecutiveChannelProjectionStore`. This removes the remaining
Gateway-to-SQLite dependency, leaving physical persistence at the composition
boundary.

The focused `interact` library suite passes (180 tests), Gateway/adapter/
Executive focused suites pass, `gateway` check passes, formatting and
architecture checks pass, and `git diff --check` is clean.

Canonical sudo deployment passed at SHA
`b54557935a02c0bff858befd0f0c217e913aaeb584dc9d9ec11f2ecdb0bf904c`; release
and `/usr/bin/aletheon` match, both daemon units are `active/running` with
`NRestarts=0`, and official client plus Memory-Agent smoke checks passed.

### 2026-08-12 Gateway capability namespace cutover

The remaining Gateway capability handlers moved from the legacy
`crates/gateway/src/handlers/` namespace to `crates/gateway/src/capability/`.
`ChatHandler`, `GoalHandler`, and `GreetingHandler` remain thin adapters over
Gateway-owned typed ports; the old handler namespace shell was deleted. Router,
Aletheon daemon composition, Executive compatibility tests, and architecture
module inventory now reference `capability` directly. Gateway no longer has a
physical handler namespace or a SQLite dependency.

Focused Gateway and Google/Telegram tests pass, along with the Aletheon/Executive
compile check, formatting, architecture-check, and `git diff --check`.

### 2026-08-12 Gateway client compatibility isolation

The typed `gateway-client` facade now exposes only the versioned Gateway
transport/client at its root. Pre-CGP-02 JSON-line adapters used by the Memory
Agent and deterministic TUI fixtures live under the explicit
`gateway_client::legacy` module; callers must name that compatibility boundary
instead of importing legacy framing through the typed client surface.

The `interact` library suite (180 tests), Gateway client tests, focused Gateway/
Executive route tests, formatting, architecture-check, and diff checks pass.
Canonical sudo deployment passed at SHA
`39626ce3a5960dcb0a13db05998cb7f01c94f2906446fafa61f8c15799c335ab`; release
and `/usr/bin/aletheon` match, both daemon units are `active/running` with
`NRestarts=0`, and official client plus Memory-Agent smoke checks passed.

### 2026-08-12 Compatibility session facade config-owner cutover

`executive::compatibility::LegacySessionService` now consumes the canonical
`aletheon_config::SessionWriterMode` directly instead of reaching through the
legacy `executive::composition::config` facade. This keeps the rollback/read-only
session facade compatible while removing one configuration-owner dependency from
Executive composition. The session-use-case compatibility contract (6 tests)
and architecture checks pass.

Canonical sudo deployment after the compatibility configuration-owner cutover
passed at SHA `c991ef384a78b7a51961190de03d3adcc75930461922f2950e940d471ecda5b`;
release and installed binaries match, daemon services remain active/running with
zero restarts, and official client/Memory-Agent smoke checks passed.

### 2026-08-12 Executive Application configuration-owner cutover

Application-owned Executive modules no longer import configuration through the
legacy `executive::composition::config` facade. Turn, evaluation, policy,
maintenance, extension, and harness application ports now consume typed
`aletheon_config` values directly; only host/compatibility composition remains
on the legacy facade. This advances the CGP-08 boundary without deleting the
rollback host tree before its caller-zero gate.

Focused session/turn compatibility and lifecycle tests (45 tests) pass;
formatting, architecture-check, and diff checks pass. Canonical sudo deployment
passed at SHA
`5edafd6b1169f5c0a4dee2ff264faffe39c3e51e844573621a01d1280ff09598`.

### 2026-08-12 TUI legacy socket compatibility removal

`TuiController` now owns only the typed `GatewayClient`; `TuiModel` construction
no longer accepts or stores a raw `UnixStream`, and the TUI lifecycle no longer
contains the test-only JSON-line socket pump. Production and test event draining
therefore use the typed Gateway event path. The remaining `process_response` /
V0 JSON reducer is explicitly `cfg(test)` compatibility coverage and is not
reachable from the installed TUI loop.

The focused `interact` library suite passes (180 tests), `interact` and
`gateway-client` checks pass, formatting, architecture-check, and `git diff
--check` pass. Canonical sudo deployment passed at SHA
`8707a0145310f9befc7168c41aa1eb1c76d53bef82cd8a1b5072f9a48f8168fb`; the
release and `/usr/bin/aletheon` binaries match, the system core and user daemon
are active/running with zero restarts, and official client plus Memory-Agent
smoke checks passed.

### 2026-08-12 Executive host/config compatibility seam convergence

All remaining Executive `host/**` and `composition/**` configuration imports
now resolve directly from `aletheon_config`; the historical
`executive::composition::config` module is a one-way re-export only. The same
owner is used by Executive core, adapters, host bootstrap, legacy composition,
and test fixtures. This removes the last configuration-owner split while
leaving the host/composition implementation tree available as the explicitly
bounded rollback seam required before XRET-04.

Verification:

```text
rg -n 'crate::composition::config|executive::composition::config' \
  crates/executive/src --glob '*.rs'
# no matches
bash scripts/cargo-agent.sh test -p executive --lib                    PASS (751)
bash scripts/cargo-agent.sh check -p executive -p aletheon             PASS
bash scripts/cargo-agent.sh fmt --all -- --check                       PASS
bash scripts/libexec/aletheon/architecture-check.sh                    PASS
git diff --check                                                        PASS
```

Canonical sudo deployment passed at SHA
`b36ad010dae4d45a7c8c33e05734c927a03bdc55d73cbf82fbb705fca82f47c4`;
`target/release/aletheon` and `/usr/bin/aletheon` match, the system core and
user daemon are active/running with zero restarts, and official client plus
Memory-Agent smoke checks passed.

### 2026-08-12 Runtime identity binding for Aletheon Exec events

The standalone Aletheon Exec composition no longer mints a client-side
`TurnId` for event envelopes or capability authority. Pre-admission paths use
explicit non-canonical placeholders (`runtime-pending-turn` and a nil typed
turn); `TurnService::submit` binds the admitted Runtime `OperationId` and
`TurnId` into `ExecTurnEventWriter` before the cognitive session emits events.
This closes the remaining Aletheon-side CGP-06/CGP-08 identity seam while
preserving the Executive compatibility tree for the bounded RA-04/XRET-04
cutover.

Anchors: `crates/aletheon/src/wiring/composition/turn_service.rs:31-35,67-83`,
`crates/aletheon/src/wiring/exec.rs:261-269,141-147`,
`crates/aletheon/src/wiring/exec_session.rs:204-213`, and
`crates/aletheon/src/wiring/daemon/handler/tool_executor.rs:550-555`.

Verification:

```text
bash scripts/cargo-agent.sh test -p aletheon --lib                  PASS (151)
bash scripts/cargo-agent.sh check -p aletheon                      PASS
bash scripts/cargo-agent.sh fmt --all -- --check                    PASS
bash scripts/libexec/aletheon/architecture-check.sh                 PASS
git diff --check                                                     PASS
```

Canonical sudo deployment passed at SHA
`adc9c728bda005ebb1d854857d5ddce7bfed7487d438db6dd1baedf9eb55b88a`;
`target/release/aletheon` and `/usr/bin/aletheon` match, both daemon units are
active with `NRestarts=0`, and the official client plus Memory-Agent smoke
checks passed.

### 2026-08-12 Runtime delegate parent-turn reference cleanup

`runtime::DelegateSpawnRequest` and `SpawnAgentRunCommand` now model
`parent_turn` as an optional canonical reference. Background Goal attempts and AgentControl child admission
do not fabricate a `TurnId` from a goal label or root `AgentId`; they leave the
parent reference absent. The convenience Runtime spawn API still accepts a
real parent `TurnId` and wraps it as `Some`, preserving canonical ownership for
turn-scoped delegation. This removes two remaining client-side core-ID mint
sites without changing the Runtime-owned AgentRun/Generation allocation.

Anchors: `crates/runtime/src/agent_supervisor.rs:50-67`,
`crates/runtime/src/agent_writer.rs:574-587`,
`crates/executive/src/application/agent_control/spawning.rs:20-31`, and
`crates/executive/src/application/goal/runtime_executor.rs:188-203`.

Verification:

```text
bash scripts/cargo-agent.sh test -p runtime --lib                   PASS (80)
bash scripts/cargo-agent.sh test -p executive --lib                 PASS (751)
bash scripts/cargo-agent.sh check -p runtime -p executive -p aletheon PASS
bash scripts/cargo-agent.sh fmt --all -- --check                     PASS
bash scripts/libexec/aletheon/architecture-check.sh                  PASS
git diff --check                                                      PASS
```

Canonical sudo deployment after this cutover passed at SHA
`46486932eb3137d9cc902f4269e4bcdfd978e232756ef4358e8a548698d3f7dd`;
release and installed binaries match, both daemon units are active with
`NRestarts=0`, and official client/Memory-Agent smoke checks passed.

### 2026-08-12 Turn writer constructor injection

The official Aletheon daemon path now constructs `TurnCoordinator` with the
already-composed `RuntimeTurnWriter` via
`from_components_with_runtime_turn_writer`; it no longer constructs a
process-local writer and replaces it later through a builder mutation. The
legacy `from_components` constructor remains only for isolated compatibility
fixtures. This makes the RA-04 single-writer invariant explicit at the
production composition boundary.

Anchors: `crates/aletheon/src/wiring/application/turn_coordinator.rs:365-431` and
`crates/aletheon/src/wiring/daemon/bootstrap/services.rs:875-887`.

Verification: Executive 751 tests, Aletheon 151 tests, focused checks,
architecture-check, formatting, and diff checks pass. The subsequent sudo
deployment passed with the installed runtime provenance verification.

### 2026-08-12 Runtime-admitted Agent backend pinning

`AgentControlService` now fails closed when a Runtime-admitted child reaches
the rich host adapter without a composition-pinned launcher. It no longer
falls back to the legacy runtime catalog after Runtime has assigned the
AgentRun/Generation, preventing a reload or compatibility catalog from
silently swapping the backend generation. Legacy catalog resolution remains
available only for explicit non-Runtime compatibility fixtures.

Anchor: `crates/executive/src/application/agent_control/spawning.rs:101-124`.
Executive focused tests (751) and formatting pass; deployment follows the
same installed-runtime acceptance recorded above.

### 2026-08-12 Rollback host writer injection parity

The retained Executive daemon rollback seam now uses the same constructor-time
`RuntimeTurnWriter` injection as the official Aletheon composition. It no
longer creates a coordinator with a process-local writer and replaces that
writer through a later mutation. This keeps the rollback path single-writer
compatible while it remains inert from the official socket topology.

Anchor: `crates/executive/src/host/daemon/bootstrap/services.rs:871-883`.

The obsolete `TurnCoordinator::with_runtime_turn_writer` post-construction
mutator was removed; production and rollback composition now have only the
constructor-time injection path. Compatibility `from_components` remains
explicitly limited to isolated fixtures.

### 2026-08-12 Binary-owned composition groups

The concrete `MemoryGroup`, `SecurityGroup`, `CorpusGroup`, and `SessionGroup`
handles moved to `aletheon::wiring::domain`; the corresponding Executive core
modules are now `pub(crate)`. The official binary no longer reaches into
`executive::core` to assemble its component graph. Daemon turn resources are
also kept crate-private behind the explicit `DaemonTurnOrchestrator::compose`
boundary rather than crossing as a public resource bag.

Anchors: `crates/aletheon/src/wiring/domain.rs:9-65`,
`crates/aletheon/src/wiring/daemon/bootstrap/request.rs:863-925`,
`crates/executive/src/core/mod.rs:1-31`, and
`crates/aletheon/src/wiring/application/daemon_turn/orchestrator.rs:30-89`.

The remaining Aletheon composition imports from `executive::core` now use
Executive's narrow root exports (`AletheonExecutive`, evolution/permission
contracts, and deployment verification types). The corresponding core modules
are crate-private; no binary caller reaches into the Executive component tree.

Verification for the completed composition-surface cutover:

```text
bash scripts/cargo-agent.sh check -p executive -p aletheon              PASS
bash scripts/cargo-agent.sh test -p executive --lib                     PASS (751)
bash scripts/cargo-agent.sh test -p executive --tests                  PASS
bash scripts/cargo-agent.sh test -p aletheon --lib                     PASS (151)
bash scripts/libexec/aletheon/architecture-check.sh                    PASS
git diff --check                                                        PASS
sudo bash scripts/aletheon.sh deploy                                   PASS
```

The installed-runtime acceptance at SHA
`11ca206c3eb364c9811840caf43462d12ae54209c37f10b1721a6572503ed106`
confirmed release/installed digest parity, both daemon units active with
stable restart counters, and successful official client plus Memory-Agent
smoke requests. The remaining work is the deeper RA-04 turn reducer/writer
cutover, RA-05 `AgentControlService` authority retirement, and the gated
XRET-04/XRET-05 deletion of retained Executive host/composition compatibility
trees.

### 2026-08-12 Executive host/composition visibility cutover

The retained Executive `composition` and `host` trees are now crate-private;
black-box fixtures reach them only through the explicitly hidden
`executive::testing::{composition,host}` compatibility namespace. The public
Executive surface exports owner-crate configuration and narrow deployment /
permission contracts instead of exposing the old composition root. Workspace
production sources contain no `executive::composition::*` or
`executive::host::*` caller. The neutral `gateway` crate also no longer carries
a direct `gateway-protocol` dependency; protocol/client ownership remains in
their independent crates. `interact::MemoryClient` uses the separate
`fabric-client` compatibility crate for the non-Gateway Memory Agent protocol;
that traffic is not a typed Gateway Session/Turn route.

Anchors: `crates/executive/src/lib.rs:18-41,78-102`,
`crates/gateway/Cargo.toml:9-19`,
`crates/cognit/src/harness/mod.rs:38-39`, and
`crates/interact/src/memory_client.rs:19-48`.

Verification:

```text
bash scripts/cargo-agent.sh check -p executive -p aletheon  PASS
bash scripts/cargo-agent.sh test -p executive --tests       PASS (751 unit + integration suites)
bash scripts/cargo-agent.sh test -p gateway --lib           PASS (7)
bash scripts/cargo-agent.sh fmt --all -- --check             PASS
bash scripts/libexec/aletheon/architecture-check.sh          PASS
git diff --check                                              PASS
sudo bash scripts/aletheon.sh deploy                         PASS
```

Installed-runtime SHA parity at
`68716f6d138a8595ea49c6470b0e12b2dcf668bd3d39f3e7317aacde3ce3112d`, active
system/user daemons with `NRestarts=0`, and official client plus Memory-Agent
smoke checks also passed.

### 2026-08-12 Legacy Fabric client ownership split

The old Fabric JSON-line transport implementation moved out of
`gateway-client` into the new `fabric-client` crate. The `gateway-client`
legacy module and its compatibility tests were removed; `gateway-client` now
owns only typed Gateway framing/correlation. `interact::MemoryClient` imports
`fabric_client::FabricProtocolClient` directly, so the non-Gateway Memory Agent
protocol no longer makes the typed Gateway client its implementation owner.

Anchors: `crates/fabric-client/src/lib.rs:1-12`,
`crates/gateway-client/src/lib.rs:1-18`,
`crates/interact/src/memory_client.rs:19-22`, and
`Cargo.toml:19-24`.

Verification:

```text
bash scripts/cargo-agent.sh check -p fabric-client -p gateway-client -p interact PASS
bash scripts/cargo-agent.sh test -p fabric-client --lib                         PASS (2)
bash scripts/cargo-agent.sh test -p gateway-client --lib                        PASS (5)
bash scripts/cargo-agent.sh test -p interact --lib                              PASS (180)
sudo bash scripts/aletheon.sh deploy                                         PASS
```

Installed-runtime SHA parity at
`288b79adc4149c8e4d4261d62484ed9583c8d5abcfa5e0a1ee322cd76f57cd1b`, active
system/user daemons with `NRestarts=0`, and official client plus Memory-Agent
smoke checks also passed.

## Profile loader composition shim retirement (2026-08-12)

Both daemon bootstrap trees now import `MarkdownAgentProfileLoader` directly
from its `adapters-agent-profile` owner. The two one-line composition re-export
modules were deleted; focused `executive --lib` and `aletheon --lib` checks
passed through `scripts/cargo-agent.sh`.

## Executive composition-only tree reduction (2026-08-12)

The caller-zero Executive copies of `config`, `user_runtime`, `exec_session`,
and `exec_corpus` were deleted. Configuration fixtures now re-export directly
from `aletheon-config`, and the multi-user boundary fixture moved to the
Aletheon package where `wiring/core_runtime.rs` and `wiring/user_runtime.rs`
are actually owned. `executive --all-targets` and the relocated Aletheon test
compile passed through `scripts/cargo-agent.sh`.

## Prefix and skill-admin composition copy retirement (2026-08-12)

The Executive copies of `prefix_builder` and `skill_admin` are no longer part
of its composition tree. Production remains owned by Aletheon wiring. The
retained rollback bootstrap keeps a host-local skill admin and uses the stable
configured system prompt directly; its isolated admin-service test now owns a
minimal test adapter instead of importing a production composition type.
`executive --all-targets` passed through `scripts/cargo-agent.sh`.

## Turn-service composition copy retirement (2026-08-12)

The Executive production `composition/turn_service.rs` copy was deleted.
Projection artifact materialization moved to the Corpus artifact adapter, and
Executive application/adapters now call that owner directly. Legacy behavior
fixtures use a test-local harness rather than exposing an Executive production
composition facade. `corpus --all-targets` and `executive --all-targets` checks
passed through `scripts/cargo-agent.sh`.

## Executive composition root retirement (2026-08-12)

The final production composition module, `turn_coordinator`, reached zero
Executive production callers and the entire `crates/executive/src/composition`
tree was deleted. Retained bootstrap construction is host-local; isolated
integration builders are explicitly test-scoped under the session adapter.
The shared in-memory Session authority constructor is owned by
`EventSourcedSessionStore`. `executive --all-targets` passed through
`scripts/cargo-agent.sh` after the deletion.

## Context compactor compatibility retirement (2026-08-12)

`MnemosyneContextCompactorFactory` now lives beside its concrete compressor at
`crates/mnemosyne/src/context_compactor.rs`. Executive session consumers import
the owner directly, and the compatibility copy was deleted. Focused Mnemosyne
all-target and Executive library checks passed through `scripts/cargo-agent.sh`.

## Legacy fixture ownership reductions (2026-08-12)

The canonical legacy-session facade test moved into the Aletheon package and
now validates `wiring/daemon/legacy_session.rs`, the production owner. The
Executive compatibility copy has no external test caller; it remains only
because the old Executive host tree still compiles. The legacy runtime registry
was reclassified and moved to an explicit Executive test-adapter module; no
installed production path consumes it. Executive all-target checks and the
relocated Aletheon test compile passed through `scripts/cargo-agent.sh`.

## Executive host retirement (2026-08-12)

All remaining owner-specific host tests now compile against Aletheon wiring,
and non-host Executive code no longer imports `crate::host`. The complete
`crates/executive/src/host` tree was deleted. Its legacy-session compatibility
copy became caller-zero and was deleted with it; the sole production facade is
Aletheon wiring. Executive all-target and Aletheon test checks passed through
`scripts/cargo-agent.sh`.

## 2026-08-12 XRET-04 hard-zero closure

- `crates/executive/src/host`: absent.
- `crates/executive/src/composition`: absent.
- `crates/executive/src/compatibility`: absent.
- The retained shared SQLite migration bundle now lives at `crates/adapters/sqlite/src/executive_migrations.rs`; Executive callers import the concrete adapter owner directly.
- Obsolete Executive-only `MemoryGroup`, `SessionGroup`, and MCP identity conversion shims were deleted; the live groups remain composition-private at `crates/aletheon/src/wiring/domain.rs`.
- Architecture locators and deletion gates now inspect the authoritative `crates/aletheon/src/wiring/**` paths and enforce Executive compatibility cardinality hard zero.

Verification:

```text
bash scripts/cargo-agent.sh check -p adapters-sqlite --all-targets  PASS
bash scripts/cargo-agent.sh check -p executive --all-targets        PASS
bash scripts/cargo-agent.sh fmt --all -- --check                    PASS
ARCH_SKIP_DEPENDENCIES=1 bash scripts/aletheon.sh acceptance architecture
  PASS: 4 existing findings, 0 dependencies, 2 paths; no additions
```
