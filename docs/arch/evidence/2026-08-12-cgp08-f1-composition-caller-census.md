# CGP08-F1 duplicate composition caller census

Date: 2026-08-12
Scope: audit only

## Result

The installed daemon is composed by `crates/aletheon/src/wiring/**`; no source
outside Executive imports `executive::host`, `executive::composition`,
`RuntimeCore`, `SystemCoreRuntime`, or Executive `UserRuntime`. Those trees are
inert compatibility/test candidates, but they are **not deletion-ready yet**:
the new composition still imports the production `executive::core::session_gateway`
implementation and several Executive application/adapters.

## Census

| Surface | Production callers | Writer / bind / spawn | Classification |
|---|---|---|---|
| `executive/src/host/launcher.rs` | none outside Executive | legacy exec idempotency DB | rollback/test-only; XRET inventory candidate |
| `executive/src/host/{mod,systemd,container}.rs` | none outside Executive | legacy RuntimeCore boot and workers | inert duplicate graph; XRET inventory candidate |
| `executive/src/host/daemon/**` | none outside Executive | legacy socket bind, stores and workers remain in source | inert duplicate graph; do not delete before remaining business dependencies are extracted |
| `executive/src/composition/**` | tests/Executive compatibility only | exec/session/evaluation test stores | compatibility/test-only |
| `executive/src/core/runtime_core.rs` | Executive legacy hosts only | constructs legacy RequestHandler and workers | no new-composition caller |
| `executive/src/core/system_core_runtime.rs` | Executive launcher/tests only | legacy inference service worker | no new-composition caller |
| `aletheon/src/wiring/**` | installed production | official socket, stores and workers | unique active full composition root |
| `executive/src/core/session_gateway/**` | **active `aletheon` production caller** | diagnostic/session projection facade | blocker to Executive core caller-zero |

Evidence commands:

```bash
rg -n 'executive::(host|composition|core)' crates --glob '!executive/**' --glob '*.rs'
rg -n 'RuntimeCore|SystemCoreRuntime|UserRuntime|RequestHandler|ExecutiveAcpBackend|HandlerPorts' \
  crates/aletheon crates/executive crates/interact --glob '*.rs'
rg -n 'UnixListener|::open\(|tokio::spawn|JoinSet' \
  crates/executive/src/{host,composition,core} --glob '*.rs'
```

## CGP08-F2 typed projection cutover

The installed typed Gateway snapshot/list/event paths now consume a narrow
`SessionProjectionPort` implemented by
`aletheon::wiring::daemon::session_projection::CanonicalSessionProjection`.
That adapter reuses the already composed canonical `SessionService`; it opens
no database and owns no writer. The Executive `SessionGateway` remains behind a
separate `LegacySessionGatewayPort` only for the still-inventoried
`session.*`/debug compatibility routes.

Focused evidence:

```bash
bash scripts/cargo-agent.sh check -p aletheon --all-targets
bash scripts/cargo-agent.sh test -p executive --test session_protocol_reconnect
ARCH_SKIP_DEPENDENCIES=1 bash scripts/aletheon.sh acceptance architecture
git diff --check
```

Result on 2026-08-12: Aletheon all-target check passed (one pre-existing
dead-code warning), all 8 session protocol reconnect tests passed, architecture
reported no additions, and diff check passed.

### CGP08-F3 typed memory projection cutover

The versioned `Query::MemorySnapshot` path no longer calls the compatibility
`SessionGateway::handle_method("session.memory", ...)` dispatcher. A narrow
`SessionMemoryProjectionPort` is composed from the existing `CoreMemory` and
`RecallMemory` handles and preserves the bounded typed result shape without
opening a store or adding a writer. The legacy dispatcher remains reachable
only through the separately named compatibility port.

Focused evidence:

```bash
bash scripts/cargo-agent.sh test -p aletheon --lib session_projection::tests
bash scripts/cargo-agent.sh check -p aletheon --all-targets
ARCH_SKIP_DEPENDENCIES=1 bash scripts/aletheon.sh acceptance architecture
git diff --check
```

Result: both typed memory projection tests passed; Aletheon all-target check,
architecture no-additions gate and diff check passed. Changed validation and
installed acceptance remain required for this production route cutover.

### CGP08-F4 legacy SessionGateway production retirement

The `aletheon` composition no longer constructs or imports the Executive
`SessionGateway`, `SessionStateRef`, `SessionDebugView`, or `ParamRegistry`.
Unversioned lifecycle commands continue through explicit typed use-case ports;
unknown legacy diagnostic `session.*` methods now fall through to the normal
method-not-found contract rather than retaining a second live read model.

The obsolete post-turn `TurnStateSink` existed only to update that transient
legacy debug snapshot. It and its composition input were removed rather than
replaced with a no-op facade. Canonical Session/Turn projections remain the
only production state shown by typed clients.

Focused evidence:

```bash
rg -n 'executive::core::session_gateway|SessionGateway|SessionStateRef|SessionDebugView|ParamRegistry' \
  crates/aletheon/src --glob '*.rs'
bash scripts/cargo-agent.sh test -p executive --test turn_pipeline_order
bash scripts/cargo-agent.sh test -p executive --test turn_coordinator_lifecycle
bash scripts/cargo-agent.sh test -p aletheon --lib wiring::daemon
ARCH_SKIP_DEPENDENCIES=1 bash scripts/aletheon.sh acceptance architecture
git diff --check
```

The concrete SessionGateway grep is empty except for the separately tracked
`ContextWorkingSet` module re-export. Focused turn and daemon tests and the
architecture no-additions gate pass. The repository-owned changed validation
passed all 91 steps. System deploy passed with release/install/running SHA
`87286e3785a61a48256d1081bd9d5e39a76ce497da678c5f19fb88d020234827`;
both daemons remained active with exit status zero and zero restarts, the
official Memory Agent smoke and official-client real request passed, and the
post-deploy forbidden-log scan was empty.

## CGP08-F5 / RA-03 ContextWorkingSet owner cutover

The last production `aletheon` import from `executive::core` was the
`ContextWorkingSet` re-export. The working cache now lives in
`runtime::context_working_set`; `aletheon` composes it directly. Runtime owns
messages, turn count, rewrite identity, hydration and cache mutation while a
narrow `ContextCompactor` port keeps the concrete Mnemosyne summarizer outside
Runtime's dependency graph. The Executive core file is now only a rollback
re-export and no longer contains a second implementation.

Focused evidence:

```bash
bash scripts/cargo-agent.sh test -p runtime context_working_set
bash scripts/cargo-agent.sh test -p executive --test session_use_case_port
bash scripts/cargo-agent.sh test -p aletheon --lib wiring::daemon::bootstrap::turn_runtime
bash scripts/aletheon.sh test architecture
rg -n 'executive::core' crates/aletheon/src/wiring/daemon --glob '*.rs'
```

The focused tests and architecture no-additions gate pass; the final grep is
empty. The repository-owned changed validation passed all 91 selected steps.
System deployment then passed at SHA
`515cef2fc182d8df73939bb675762177a4c32f5bebc6f137a544f953dbe01d2c`;
the release binary, `/usr/bin/aletheon`, and both running daemon executables
matched. Both services remained active with `ExecMainStatus=0`, stable
`NRestarts=0`, and the post-deploy forbidden-log scan was empty.

Installed scenarios used only `/usr/bin/aletheon` and the official user socket:

- a short real request completed with substantive output (`2+3=5`), one
  inference round, zero provider retries and zero tool calls;
- two turns in canonical Session
  `session-9c3a8de2-9d59-424a-b73b-e897655b82c7` retained the requested
  `蓝鲸` context across typed resume;
- a required `pi-coder` run reached durable Agent `ccdaa0e5-15de-4fe9-989b-5b414e19cca3`
  terminal status `succeeded`; the parent observed `agent_wait` before returning
  `AGENT_WAIT_OK` (three parent inference rounds, zero provider retries, two
  Agent tool calls);
- deploy's official Memory Agent protocol and official-client smoke both passed.

Cancellation, timeout, wrong-generation and restart/reconciliation retain their
deterministic acceptance coverage in the 91-step report; this cache-owner slice
did not change those state machines and did not manufacture a second installed
exercise as broader RA-04/RA-05 closure evidence.

## CGP08-F6 deletion-ready gate result

The four remaining composition census rows are now production caller-zero:

- `aletheon`, `interact`, `gateway-client`, and `gateway-protocol` have no
  `executive::host`, `executive::composition`, or `executive::core` import;
- Executive `host/**` and `composition/user_runtime` are private, inert
  rollback/test graphs with no production caller outside the Executive crate
  (one Aletheon integration test still characterizes Core RPC compatibility);
- Executive `turn_service`, `turn_coordinator`, and `exec_session` composition
  are reachable only through the explicitly test-only `executive::testing`
  facade;
- the installed daemon, official socket, stores, and workers are composed only
  by `crates/aletheon/src/wiring/**`.

The source remnants still contain bind/open/spawn operations, but source text
is not a production caller. They remain compiled solely to preserve the
countable rollback/test seam and are assigned to the existing XRET-04 deletion
inventory; CGP-08 does not physically delete them.

The last production `AgentControlService` construction occurs in Aletheon
wiring as an Executive application/host-effects adapter. It remains the
separate `RA-A-10` / RA-05 blocker and is not evidence of a second composition
root. CGP-08 closure therefore does not close RA-05 or authorize RA-06 deletion
of that application facade.

Validation evidence for the caller-zero state includes the repository-owned
91/91 changed validation, workspace all-target check, architecture gate, and
the installed SHA
`4f998cdeb0648b6e496b43ffa04c7716b78847538ebe46336313190bb2235a34`.
The release, `/usr/bin/aletheon`, and both running daemon executables matched;
both services were active with zero restarts. An official `pi-coder` request
reached durable Agent `bc9f2970-439f-4a19-a334-76a4b593f800` terminal status
`succeeded` after the parent observed the terminal result, with three parent
inference rounds, zero provider retries, and two Agent tool calls.

## Completed implementation slice

**CGP08-F2: extract the production SessionGateway protocol projection behind
a narrow `SessionProjectionPort`.**

Move only the typed session snapshot/list/event-page implementation needed by
the Gateway server into the `aletheon` composition (or a narrow neutral
application package if an existing owner is verified). Keep legacy debug
method dispatch and markdown snapshot compatibility in Executive until their
callers are separately zero.

Allowed files:

- `crates/aletheon/src/wiring/daemon/handler/ports.rs`
- a new narrowly named `aletheon/src/wiring/daemon/session_projection.rs`
- `crates/aletheon/src/wiring/daemon/bootstrap/{request,services,params}.rs`
- directly affected typed Gateway/session tests
- Executive SessionGateway only for deleting production imports after parity

Forbidden:

- deleting Executive host/composition trees in this slice;
- moving all 2,500 SessionGateway lines as a copy;
- changing Session/Turn writers or opening a second store;
- changing Gateway package boundaries;
- closing CGP-08 before every production Executive host/composition/core caller
  is zero and installed equivalence passes.

Focused validation is complete. Repository-owned changed validation and
system-installed multi-turn plus reconnect evidence remain required before this
production cutover is accepted. This slice does not close CGP-08: the remaining
legacy debug/context/parameter callers must be handled by separately bounded
caller-zero slices.
