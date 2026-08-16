# CGP-03/04/05 typed Gateway server and ACP slice — evidence

Requirement anchors: `docs/plans/2026-08-08-composition-gateway-presentation-extraction.md:325-341` (CGP-03 route handlers), `:335-341` (CGP-04 official socket cutover), and `:343-348` (CGP-05 ACP typed client).

## Implemented in this slice

- `crates/gateway-server/src/handlers/typed.rs` exposes the typed Application port, including a fail-closed projection query, and dispatches versioned Session/Turn/Approval commands and queries to that port. `crates/gateway/src/handlers/typed.rs` is now only its compatibility re-export.
- `crates/executive/src/host/daemon/handler/typed_gateway.rs:171-253` resolves connection-owned transient approvals through the authenticated `PendingApprovals`/`AdminUseCases` path before falling back to durable `ApprovalUseCases`; a transient UUID-shaped choice id is never misrouted solely by its string shape.
- `crates/aletheon/src/wiring/application/turn_coordinator.rs:636-710` now publishes the Runtime-minted Turn receipt immediately after admission, before running the cognitive pipeline; `crates/aletheon/src/wiring/application/daemon_turn/execute.rs:175-208,430-490` owns the detached execution and ordinary terminal fence.
- `crates/executive/src/host/daemon/handler/typed_gateway.rs:35-214` binds the authenticated `ConnectionContext` to existing session/turn/approval/session-gateway use-case ports. It does not mint IDs or open a store. Workspace/session routing, approval version checks, and principal-scoped projection reads use the existing canonical services.
- `crates/aletheon/src/wiring/daemon/server.rs:1022-1086` recognizes the versioned `WireRequest` envelope on the official socket before legacy parsing, dispatches it through `TypedRouteHandler`, and rejects mixed wire families. `crates/executive/Cargo.toml:9-13` records the direct protocol dependency.
- `crates/gateway-protocol/src/lib.rs:195-218` now carries a durable `(sequence,event_id)` cursor; `crates/executive/src/host/daemon/handler/typed_gateway.rs:229-274` authenticates it through the canonical event-page API and returns `snapshot`, `events`, and `next` instead of echoing an unused cursor.
- `crates/interact/src/acp/typed_client.rs:42-179` now provides the production `GatewayAcpBackend` and projection-based `GatewayAcpEvents`: ACP create/prompt/cancel go through `GatewayClient`, including the requested workspace, and recovery reads the typed snapshot rather than opening a Session store or calling Executive business methods.
- `crates/aletheon/src/acp.rs:1-58` is now a client composition root: it ensures the installed user socket, connects the typed ACP backend, and does not bootstrap `RuntimeCore` or open a canonical store. `crates/aletheon/src/main.rs:412` passes the neutral `interact::host::WorkspaceLaunch` into this path.

## Validation

```text
bash scripts/cargo-agent.sh check -p gateway -p executive       PASS
bash scripts/cargo-agent.sh test -p gateway --lib handlers::typed PASS (2)
bash scripts/cargo-agent.sh test -p executive --lib host::daemon::server::tests PASS (14)
bash scripts/cargo-agent.sh test -p executive --lib application::daemon_turn::execute::tests::async_submit_returns_runtime_receipt_before_terminal_settlement PASS (1)
bash scripts/cargo-agent.sh test -p gateway-client --lib       PASS (7)
bash scripts/cargo-agent.sh test -p interact --lib acp::typed_client PASS (1)
bash scripts/cargo-agent.sh test -p interact --lib single_message PASS (2)
bash scripts/cargo-agent.sh test -p aletheon --features acp --bin aletheon acp_cli_tests PASS (1)
bash scripts/cargo-agent.sh test -p interact --lib             PASS (179; current typed-only TUI test set)
ARCH_SKIP_DEPENDENCIES=1 bash scripts/aletheon.sh acceptance architecture PASS
git diff --check                                             PASS
```

### 2026-08-11 continuation: memory client framing moved behind Gateway client

`interact/src/memory_client.rs` now depends on `gateway_client::FabricProtocolClient`
instead of naming or driving `LegacyJsonRpcClient`; initialize/initialized negotiation,
numeric correlation, response decoding and fail-closed protocol errors are owned by
`crates/gateway-client/src/lib.rs`. The memory integration test passes (1/1), while the
legacy framing implementation remains confined to the gateway-client compatibility adapter.
This closes the remaining Interact memory-client framing item; it does not close CGP-08.

The latest installed deployment after this change passed with SHA-256
`7ab5feeac2b0e94cfede2c3f29f0a220f0e1f57b9da8e2f15ac6c0825a0a3b61`, including the
official client request, Memory Agent protocol smoke, and TUI command-surface smoke.

Installed evidence after the typed workspace/session and progress-event cutover:

- `sudo bash scripts/aletheon.sh deploy` passed; official client real-request and
  Memory Agent smoke passed; installed provenance SHA was
  `fd51336cf9d42f2bd00d28afdd9ea7c33eb897d23f67245ecee9920444be14ee`.
- `/usr/bin/aletheon --socket /run/user/1000/aletheon/aletheon.sock run ...`
  completed a fresh typed create/submit/async-query request (`ASYNC_TYPED_TURN_OK`).
- The same official binary completed `ACP_CURSOR_RECONNECT_OK`; target/release and
  `/usr/bin/aletheon` had identical SHA-256 digests, and the user daemon remained
  active with `NRestarts=0`.
- A typed official-socket probe observed seven `Progress` events while waiting
  for the correlated `SubmitPrompt` response; unsolicited progress uses the
  distinct `RuntimeProgressEvent` schema and is not treated as terminal state.

## Current remaining work (not claimed complete)

### 2026-08-10 continuation: typed-only production TUI boundary

- `crates/interact/src/tui/mod.rs:131-160` now creates only
  `GatewayClient<UnixSocketTransport>` for every real TUI invocation,
  including scripted acceptance mode; the legacy slot is `None` in the
  production composition root and is only constructed by `TuiModel::new` in
  unit-test fixtures (`crates/interact/src/tui/model.rs:166-181`).
- `crates/interact/src/tui/response.rs:9-31` treats the legacy response pump as
  a compatibility-only path; production drains typed Gateway events separately.
- `crates/gateway-protocol/src/lib.rs:220-232` and
  `crates/executive/src/host/daemon/handler/typed_gateway.rs:439-454` add the
  typed `AgentCatalog` projection, so production Agent Inspector refresh no
  longer sends the legacy `sub_agents` RPC (`crates/interact/src/tui/app/submit.rs:18-55`).
- `gateway-protocol` now also carries typed model/mode/profile commands and
  profile-catalog queries; full TUI and simple-line mode/profile actions use
  those receipts rather than legacy admin RPC (`crates/interact/src/tui/app/submit.rs:280-360`,
  `crates/interact/src/tui/app/lifecycle.rs:1030-1080`).
- `gateway-protocol` now also owns `InvokeSkill`, `MemoryStatus`, and
  `MemorySearch` (`crates/gateway-protocol/src/lib.rs:253-308`). The typed
  server delegates Skill admission and bounded memory recall through the
  authenticated `DaemonTypedApplication`
  (`crates/executive/src/host/daemon/handler/typed_gateway.rs:205-285,520-580`),
  and production TUI routes Skill invocation, `/memory search`, and
  `/memory status` through those typed commands/queries before any test-only
  legacy fallback
  (`crates/interact/src/tui/app/submit.rs:401-490,1100-1165`).
- Checkpoint recovery is now an additive typed family: `CheckpointListQuery`
  and `RestoreWorkspaceCheckpoint` with typed restore outcomes are defined in
  `crates/gateway-protocol/src/lib.rs:160-178,253-269,310-325`; the server validates authenticated
  thread authority and delegates list/rewind to the existing host ports in
  `crates/executive/src/host/daemon/handler/typed_gateway.rs:268-318,675-760`; TUI/CLI
  checkpoint list, picker rewind, deferred fork+rewind, and line-mode rewind
  use those routes in `crates/interact/src/tui/app/submit.rs:551-633,884-910`,
  `key_handler.rs:206-225`, and `lifecycle.rs:574-610,978-1075`.
- Focused validation passed: gateway-server typed-handler (7), gateway-client (7),
  interact (182), including the checkpoint route tests, formatting, diff
  check, and architecture acceptance.

- The former `compat_transcript: ChatWidget` slot is now a system-notice-only,
  non-rendering `SystemNoticeQueue` (`crates/interact/src/tui/model.rs:31-63`).
  It has no renderer, scroll state, persistence, socket, or authority
  constructor; assistant/tool/user entries are no longer retained there and
  the typed projection remains the production conversation surface. This is
  an intermediate CGP-07 reduction, not deletion of the compatibility log:
  system notices and test-only legacy adapters still require final cleanup.
- Revalidated after this reduction: `bash scripts/cargo-agent.sh test -p interact --lib` (182),
  gateway-protocol (3), gateway-client (7), formatting, diff check, and the
  full architecture gate (23 findings, 36 reviewed dependencies, 4 paths).
- `sudo bash scripts/aletheon.sh deploy` passed at 2026-08-10 12:43 UTC;
  target/release/aletheon and `/usr/bin/aletheon` matched SHA-256
  `b1d4f1aa743ae189fc8acab33726fe688b60ff9340b3bf00f3d207ecf786435f`,
  official client and Memory Agent smoke passed, and runtime stability held
  with a 7-second interval.
- The same installed `/usr/bin/aletheon` typed line client successfully
  executed `/memory status`, `/memory search xyz`, and `/skills` over the
  official user socket; the former printed the typed composite health
  projection and the latter returned the typed Skill catalog without opening
  a legacy socket.

This closes the production **connection-opening** portion of CGP-06, but not
the complete command-only gate: unsupported TUI commands still fail closed
instead of silently using legacy wire, and the legacy compatibility buffer,
ACP backend, and remaining typed command families are still tracked below.

### Latest installed verification (2026-08-10 12:51 UTC)

- `sudo bash scripts/aletheon.sh deploy` passed after the authenticated
  `memory_gateway.recall` hardening in `typed_gateway.rs:585-620`.
- `target/release/aletheon`, `/usr/bin/aletheon`, and the running daemon
  matched SHA-256 `8617b6018a2df1322cb7dcabaf0eb7988b2985229b30a038ce08a4a6b6503770`.
- The official user socket completed `/memory status` and `/memory search xyz`
  through typed Gateway routes; deployment verification, the Memory Agent
  smoke, the real-request smoke, and the 7-second runtime-stability check all
  passed.

### Latest typed checkpoint cutover verification (2026-08-10 13:02 UTC)

- Installed deployment and official client/Memory Agent/stability checks passed
  with matching SHA-256
  `29133c26732b0312dbe8fadf48f29ed2cd5e20e1437bde7fb6f9bc407f81e20f`.
- Official line mode `/rewind` reached the typed checkpoint query and returned
  the daemon's typed `workspace checkpoint feature is disabled` error; it did
  not fall back to the legacy checkpoint socket. This is fail-closed evidence,
  not a claim that the optional checkpoint feature is enabled in the installed
  configuration.

### Latest typed memory snapshot verification (2026-08-10 13:08 UTC)

- The installed binary and running daemons matched SHA-256
  `acb49cbd6d17da22ce82b079faf3303eb4eecdde8c87d826038e3f5cd7ce733f` after
  adding `MemorySnapshotQuery` for the former `/memory` command.
- Official line mode `/memory`, `/memory status`, and `/memory search xyz`
  completed over the typed Gateway; the first returned the daemon-owned core
  and recent-memory projection, while the latter two returned typed health and
  bounded recall projections.

### Typed governed shell route (current source)

- `ExecuteShellRequest`/`Command::ExecuteShell` is now part of the typed
  protocol (`crates/gateway-protocol/src/lib.rs:160-181,264-270`). The host
  adapter bounds input, resolves authenticated workspace/session authority,
  and submits the command as an `exec_command` capability requirement rather
  than executing locally (`crates/executive/src/host/daemon/handler/typed_gateway.rs`).
- TUI `!command` prefers this typed route and retains the old intent adapter
  only for test/compatibility construction (`crates/interact/src/tui/app/submit.rs`).
- Latest formal deployment after this route addition passed at 2026-08-10
  13:15 UTC with matching SHA-256
  `7b7d9ed23eb1689acefcfdc790a47739d402025e95a62245066fd5bf325f5ebd`.

### Typed transaction-review CLI migration (current source)

- `ReviewTransactionRequest`, `TransactionReviewed`, and
  `TransactionSettlementQuery` now carry review/show through the typed
  Gateway (`crates/gateway-protocol/src/lib.rs`).
- The host route verifies authenticated session authority, loads canonical
  review findings, and delegates settlement/rollback decisions to
  `TransactionReviewService` (`crates/executive/src/host/daemon/handler/typed_gateway.rs`).
- `crates/aletheon/src/review_cli.rs` no longer opens a legacy JSON-RPC client;
  it uses `GatewayClient<UnixSocketTransport>` for both queries and commands.
- Formal deployment after the review migration passed at 2026-08-10 13:25 UTC
  with matching SHA-256
  `8800da13f9300b41aeaaf797fee340c0a3c4abd186fec8f658f8e8bdfa5d66a5`.
- Official `aletheon review show session-a transaction-a` reached the typed
  route and failed closed with `session workspace authority not found`; it did
  not open the legacy JSON-RPC client.

- The typed turn adapter now returns the Runtime admission receipt before terminal projection (`typed_gateway.rs:126-159`); one-shot clients poll the typed projection until an authoritative terminal phase rather than treating the accepted receipt as completion (`interact/src/single_message.rs:125-185`). ACP recovery now consumes the canonical event page after its authenticated `(sequence,event_id)` cursor and advances one event at a time (`interact/src/acp/typed_client.rs:163-270`), so reconnect cannot skip a page prefix. Three fresh official client requests with `--require-agent-runtime pi-coder` also reached `agent_wait` terminal receipts; this is client-route evidence, not a substitute for the real-TUI gate.
- Approval now carries server-issued `choice_id` plus optimistic `version` (`gateway-protocol/src/lib.rs:146-165`). Connection-owned approval notifications are translated into the typed `ApprovalRequestedEvent` (`gateway-protocol/src/lib.rs:282-306`, `crates/aletheon/src/wiring/daemon/server.rs:37-70`) and appended to the canonical protocol journal before live delivery (`crates/runtime/src/session_service.rs`, `crates/aletheon/src/wiring/application/turn_pipeline.rs`). Reconnects replay the request through the durable cursor path; the event remains a request projection and is not a replacement for the canonical approval repository or its resolution state (`typed_gateway.rs:171-253`).
- Fresh and resumed one-shot CLI prompts now use typed create/resume/submit/query, including a requested workspace (`interact/src/single_message.rs:29-119`); TUI and extension compatibility routes remain under the CGP-06/07 migration. The caller-zero debug shells were retired separately. Official installed socket drain/rollback and final installed acceptance remain open for the historical slice recorded here.
- Legacy framing for the remaining extension/debug surface is now centralized in
  `gateway-client::LegacyJsonRpcClient` (`gateway-client/src/lib.rs:19-80`);
  `interact::memory_client`, review CLI, and extension CLI consume that adapter
  instead of opening their own line-framed sockets. This is compatibility
  consolidation, not the CGP-06 typed command-only completion gate.
- The previously censused debug subscription paths were unreferenced stale
  shells; caller-zero evidence was completed and those files were deleted.
  Supported observability remains a separate typed Gateway work item rather
  than an undocumented raw-RPC path.

## CGP-06 preparation evidence

- `crates/interact/src/host.rs:1-146` no longer imports Executive or starts a daemon; it resolves the official socket and passes typed client inputs to the presentation adapters.
- `crates/aletheon/src/main.rs:637-653` owns the lifecycle boundary and calls Executive's launcher before invoking Interact. This keeps daemon bootstrap in the executable composition root rather than in the presentation crate.
- `crates/interact/Cargo.toml` no longer depends on `executive` even for tests; the
  progress projection test consumes the canonical `TurnEventV1` stream directly,
  so Executive is absent from Interact's normal and test dependency graph.
- This is not the CGP-06 completion gate: TUI raw socket paths and protected compatibility reducers remain to be migrated.

## Typed extension lifecycle route (current source)

- `ExtensionRequest` and `Command::ManageExtension` now cover install/list/show,
  enable/disable, upgrade/rollback, remove/purge, and doctor in the versioned
  Gateway protocol (`crates/gateway-protocol/src/lib.rs`).
- The daemon host validates the authenticated actor, canonicalizes install and
  upgrade paths, and delegates lifecycle work to `ExtensionCoordinator`; the
  CLI no longer opens a raw Unix stream or constructs the legacy extension
  JSON-RPC envelope (`crates/executive/src/host/daemon/handler/typed_gateway.rs`,
  `crates/aletheon/src/extension_cli.rs`).
- Compile, focused Gateway/Gateway-client/Interact tests, formatting, diff, and
  architecture checks pass. Installed deployment and official extension CLI
  smoke completed at 2026-08-10 13:35 UTC. `target/release/aletheon`,
  `/usr/bin/aletheon`, and both running daemon executables matched SHA-256
  `6082975501699d7425b190fad917821d05956361fd3a2ca8682f0ade45e19745`;
  `extension list` completed over the typed route, and both user/core services
  remained active with `NRestarts=0` during the stability observation.
- A real official-socket request through `/usr/bin/aletheon exec` returned the
  typed terminal receipt with output `OK` (one inference round, zero tool calls,
  zero provider retries). This proves the deployed path is live, but does not
  close the multi-turn/rollback gates in RA-04/RA-05.

### TUI command cutover slice (current source)

- Session-picker resume, checkpoint fork, transaction review/settlement, and
  approval-adjacent review actions now prefer typed Gateway commands/queries in
  the controller path (`interact/src/tui/app/key_handler.rs`); the old
  `ClientRpcRequest` calls remain only as compatibility branches when a typed
  client is absent.
- The production composition continues to construct only the typed client;
  compatibility sockets are still available to deterministic test fixtures.
  Therefore this slice advances CGP-06 but does not claim the final
  command-only/CGP-07 deletion gate.
- Installed verification after this slice passed at 2026-08-10 13:45 UTC:
  release artifact, `/usr/bin/aletheon`, and running daemon executables matched
  SHA-256 `2907144b3ec6c93e32a1c6a7642481bd98e132fb3711e6adb4e07d618bf66338`;
  user/core services stayed active with `NRestarts=0`, official `extension list`
  used the typed route, and an official-socket real request returned a
  completed typed terminal receipt (`OK`).

### 2026-08-10 continuation: installed TUI is typed-only and records overlays

- `crates/interact/src/tui/mod.rs:131-150` now opens only
  `GatewayClient<UnixSocketTransport>` for both interactive and scripted TUI
  invocations. `gateway-client::LegacyJsonRpcClient` is constructed only by
  the unit-test fixture constructor (`crates/interact/src/tui/model.rs:166-181`).
- `crates/interact/src/tui/app/lifecycle.rs:324-357,455-463` fails closed when
  typed session admission fails, removes the startup legacy Session RPC pump,
  and drains only typed Gateway events in the installed loop. The old socket
  reader/response reducer is compiled under `cfg(test)` only
  (`crates/interact/src/tui/response.rs:12-18,615-636`).
- `crates/interact/src/tui/render/draw.rs:41-145` keeps overlay rendering on
  the common frame-recording path; `crates/interact/src/tui/test_infra.rs:42-89`
  normalizes wide-cell continuation padding in snapshots. This makes the
  command catalog and picker views observable rather than silently dropping
  modal frames.
- Focused validation passed: `bash scripts/cargo-agent.sh test -p interact
  --lib` (182 passed), formatting, `git diff --check`, and architecture
  acceptance. Installed `bash tests/suites/operations/tui_command_surface_test.sh`
  passed for `/help` and retired governance commands on the official socket.
- Installed deployment at 2026-08-10 15:39 UTC passed; `target/release/aletheon`,
  `/usr/bin/aletheon`, and running daemon executables matched SHA-256
  `7ed0b6b90a1ab953e6dfa96aa27369d5333a66d068980c7b6db0d469a3c0bfda`, with
  stable restart counters and the official real-request smoke passing.

### 2026-08-10 continuation: production controller/model boundary

- `crates/interact/src/tui/controller.rs:12-40` now exposes the legacy JSON-RPC
  client field and constructor only under `cfg(test)`; production construction
  accepts only the typed Gateway client.
- `crates/interact/src/tui/app/lifecycle.rs:289-309` selects the matching
  production/test model constructor, while `submit.rs` and `key_handler.rs`
  refuse legacy dispatch in production rather than silently falling back.
- Focused validation passed: `interact` check and 182 unit tests; installed
  TUI command-surface verification passed again. Deployment at 2026-08-10
  15:47 UTC passed with SHA-256
  `50d9b6bde2ad38f0faad8dc0d020aa564096431f4e1553f5298dd5fed6efedd1`,
  stable restart counters, and official real-request smoke passing.

### 2026-08-11 continuation: typed TUI terminal correlation and rendered completion

- `crates/interact/src/tui/model.rs` now records the opaque, server-assigned
  `GatewayCommandOutcome::Submitted.turn` as `active_turn_ref`; the TUI does
  not mint or interpret a turn identifier. `crates/interact/src/tui/app/submit.rs`
  clears stale turn state on session create/resume and marks a submitted turn
  before live notifications can arrive.
- `crates/interact/src/tui/response.rs` settles presentation state only when a
  typed `SessionSnapshot` contains the matching submitted step and a terminal
  phase. A completed older step cannot clear a newer spinner. The production
  path has no task-level terminal fallback; the fallback is retained only for
  `cfg(test)` reducer fixtures.
- `crates/interact/src/tui/app/lifecycle.rs` marks a successful typed projection
  query as a frame mutation, closing the stale-render gap where durable terminal
  evidence had arrived but the visible footer still showed a spinner.
- Focused validation passed: the response projection target (18 tests), the TUI
  reducer integration target (10 tests), formatting, diff checks, and architecture
  acceptance. Installed deployment passed with release/installed/running daemon
  SHA-256 `16b4e9284e2a1adeb6e1f1bad8e65a8a817d43b6b409fd3c2a772d8eb3e08eee`.
- A real two-turn official TUI run over the official user socket rendered both
  answers and ended without a spinner in the final frame; the typed snapshot
  showed all submitted steps `completed`. Evidence: `/tmp/aletheon-tui-frames-verify3.jsonl`,
  `/tmp/aletheon-tui-events-verify3.jsonl`. This closes the rendered typed-TUI
  stale-completion defect, but not RA-04 failure/rollback or RA-05/E6 gates.

### 2026-08-11 continuation: installed two-turn TUI recheck

- After removing temporary diagnostic output from the typed submission/projection
  path, an official user-socket run submitted two scripted prompts and rendered
  both answers: the README heading and `[workspace]`.
- The final recorded frame in `/tmp/tui-current-18581.frames.jsonl` had no
  spinner; `/tmp/tui-current-18581.events.jsonl` contained two terminal
  `turn_done` events and no rendered inference error.
- The preceding installed deployment passed release, `/usr/bin/aletheon`, and
  running-daemon SHA parity at
  `e6da9553d65b6b41c86cd851c5a4b750e1bcad8583d3585b6108576b7ae71d3a`.
  This re-confirms the CGP-07 presentation slice only; RA-04 failure/rollback,
  RA-05 supervisor equivalence, E6 process cleanup, and RA-06 deletion remain
  open.
