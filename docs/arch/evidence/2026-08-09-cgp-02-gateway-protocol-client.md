# Agent Kernel V2：CGP-02 gateway-protocol + gateway-client owner seam

Date: 2026-08-09
Baseline: `0bf690b2`
State: owner seam established and now used by the official typed Session/Turn/ACP/one-shot paths; legacy framing remains only behind the compatibility client for extension/debug surfaces.

## Context receipt (runbook §3.1)

```text
Slice: CGP-02 gateway-protocol + gateway-client owner seam
Baseline commit: 0bf690b2
Direct prerequisites: CGP-00 (frozen route census) — done
Current authoritative writer: Runtime/Executive application ports behind the typed Gateway handler
Target owner/writer: `gateway-protocol` contract + `gateway-client` transport/correlation; server authority remains Runtime/application-owned
IDs minted here: none — `SessionRef`/`TurnRef`/`AgentRef` are opaque client refs, not canonical IDs
Production callers: official socket typed route, ACP typed backend, one-shot client, and compatibility clients
Test-only callers: in-memory transport tests in gateway-client
Installed/config callers: official user daemon socket; latest installed provenance is recorded in the CGP-03 evidence
Tables/files/wire schemas: versioned typed envelope and durable `(sequence,event_id)` cursor
External side effects: none
Compatibility seam: `LegacyJsonRpcClient` owns legacy line framing and subscription decoding
Deletion owner: n/a (new crate, no deletion)
Unknowns/blockers: none
Expected files: crates/gateway-protocol/, crates/gateway-client/, Cargo.toml, module-boundaries.txt
Out-of-scope files: all legacy route/socket behavior, CGP-03+
```

## 1. What was created

### `crates/gateway-protocol` (depends only on `fabric`)

Versioned typed contract shared by client and server:

- **Opaque references**: `SessionRef(pub String)`, `TurnRef(pub String)`, `AgentRef(pub String)` — client-side correlation only, never canonical `SessionId`/`TurnId`/`AgentId` mints (plan §5.4: "Presentation 临时对象只能使用 UiOverlayId... overlay 与 durable ID 必须类型不可互换").
- **Requested preferences**: `RequestedExecutionTarget` (Automatic/Runtime), `RequestedPermissionMode` (Inherit/Safe/Full) — the client submits what it *requests*; effective policy is computed host-side (CGP-02: "客户端 authority fields 改为 requested preferences").
- **Commands**: `CreateSession`, `ResumeSession`, `SubmitPrompt`, `CancelActiveTurn`, `SubmitApproval`.
- **Queries**: `SessionSnapshot(after_cursor)`.
- **Events**: `SessionCreated`, `TurnStarted`, `TurnSettled` (typed terminal), `Snapshot`, distinct `Progress`, and connection-owned `ApprovalRequested`.
- **Cursor**: durable `Cursor { sequence, event_id }`; legacy numeric cursors still decode during additive rollout.
- **Errors**: `ProtocolError` with typed `VersionMismatch`/`UnknownSchema`/`Timeout`/`ConnectionClosed`/`ProviderRejected`/`Server`/`Cancelled` — client must fail closed, never report success on these.
- `PROTOCOL_VERSION = 1`; additive-only versioning.

### `crates/gateway-client` (depends on `gateway-protocol` + `fabric`)

- `GatewayTransport` trait: `send_command`/`send_query`/`next_event` — the bounded framing/correlation seam.
- `GatewayClient<T>` typed facade.
- `InMemoryTransport` — contract verification without a socket (CGP-02 acceptance: "client contract 可用 in-memory transport 验证"). The focused gateway-client contract suite passes.
- `UnixSocketTransport` — versioned JSON-line request/response transport with
  request correlation, interleaved typed events, and fail-closed version/schema
  checks; it is bound to the official socket by the daemon composition.

## 2. Boundary discipline honored

- `gateway-protocol` depends only on `fabric` (ownerless primitives) — never on Executive, per plan §3.2 dependency direction (`contracts`/protocol → owner ports only).
- `gateway-client` depends only on `fabric` + `gateway-protocol` — **no `executive`** or `executive::host`; socket framing is encapsulated by the client transport (CGP-02: "外层 ACP framing...留 interact::acp adapter，但该 adapter 不得出现 Gateway JSON-RPC/business method").
- No canonical ID mint, no effective-policy derivation, no terminal inference in the client.
- Both crates registered in `Cargo.toml` workspace members + `module-boundaries.txt`.

## 3. Remaining work (outside the CGP-02 owner seam)

- Full TUI command-only cutover and `TuiModel/TuiController/TuiRenderer` split remain open under CGP-06/07.
- Connection-owned approval requests are now persisted as a distinct `ApprovalRequested` protocol event and replayed by the canonical cursor/page path. This is durable request evidence, not a durable `ApprovalSnapshot`; approval resolution still uses the canonical approval repository.
- Legacy extension/debug routes remain compatibility-only and do not constitute CGP-06 completion.

## 4. Validation

```text
bash scripts/cargo-agent.sh check -p gateway-protocol   PASS (0.26s)
bash scripts/cargo-agent.sh check -p gateway-client      PASS (0.16s)
bash scripts/cargo-agent.sh test -p gateway-client       PASS (7 passed)
  - create_session_returns_opaque_ref_without_client_mint
  - submit_prompt_returns_turn_ref
  - terminal_only_from_typed_settlement_event
  - event_stream_exhaustion_is_typed_connection_closed
bash scripts/cargo-agent.sh fmt --all -- --check         PASS
git diff --check                                          PASS
ARCH_SKIP_DEPENDENCIES=1 bash scripts/aletheon.sh acceptance architecture   PASS (23 findings, 0 deps)
```

## 5. Rollback

The protocol remains additive and versioned. Rollback can remove the typed client/server route and return callers to the one-way legacy adapter without changing Runtime state or reusing a published tag with different semantics.

2026-08-11 continuation validation: the typed client/protocol contracts remain
green after the Runtime maintenance changes. `bash scripts/cargo-agent.sh test
-p gateway-client --lib` passes 7 tests and `bash scripts/cargo-agent.sh test
-p gateway-protocol --lib` passes 3 tests; architecture acceptance remains at
22 findings with no new dependencies or paths.
## 2026-08-11 typed extension fixture closure

The extension lifecycle acceptance fixture now speaks the same typed envelope
as production `ExtensionRpcClient`: it decodes `WireRequest.body.Command` /
`ManageExtension` and returns `WireResponse.body.Command.ExtensionResult` or a
typed `ProtocolError::Server`. It no longer assumes a retired JSON-RPC
`method`/`params` shape. This keeps the acceptance transport aligned with the
CGP-02 protocol owner rather than preserving a second wire contract.

Evidence:

```text
bash tests/suites/operations/extension_runtime_test.sh
# extension runtime operations acceptance passed
bash scripts/aletheon.sh test operations
# pass (including completion, extension lifecycle and filtered validation)
```

## 2026-08-12 CGP-02 client recovery/framing follow-up

`gateway-client` now owns the remaining transport mechanics required by the
CGP-02 seam instead of leaving them in presentation callers:

- `ReconnectableGatewayTransport` and bounded `ReconnectPolicy` reconnect an
  existing transport without minting a new Session/Turn identity;
- `SessionSubscription` carries the server-issued typed `Cursor`, requests a
  snapshot after that cursor, and provides reconnect-and-snapshot recovery;
- `UnixSocketTransport` retains its socket path for reconnect and enforces a
  bounded JSON-line frame before decoding or allocating an unbounded payload;
- `ProtocolError::FrameTooLarge` is a distinct fail-closed transport error.

The subscription still returns the existing projection JSON for compatibility,
but cursor state and transport recovery are typed and owned by the client
package. No Runtime repository, Executive type, effective policy, or canonical
ID mint was added.

The TUI projection loop now consumes that reconnect capability: a typed socket
closure retries the same authenticated `SessionSnapshot` query after the
current server cursor instead of opening a second transport or replaying local
events (`crates/interact/src/tui/app/lifecycle.rs`).

Validation:

```text
bash scripts/cargo-agent.sh test -p gateway-client       PASS (8 passed)
bash scripts/cargo-agent.sh test -p interact --lib       PASS (180 passed)
bash scripts/cargo-agent.sh check -p gateway-protocol -p gateway-client -p gateway-server -p interact -p aletheon PASS
bash scripts/cargo-agent.sh fmt --all                    PASS
git diff --check                                          PASS
ARCH_SKIP_DEPENDENCIES=1 bash scripts/aletheon.sh acceptance architecture PASS
```

Installed verification for this follow-up:

```text
sudo bash scripts/aletheon.sh deploy                              PASS
official client real-request smoke                               PASS
official Memory Agent protocol smoke                             PASS
target/release/aletheon == /usr/bin/aletheon == running core/user  PASS
sha256: 190a98ec6775a06916e2e053e475fa3ac974694f11655f099d01fa4f89070d65
aletheon-core.service: active, ExecMainStatus=0, NRestarts=0       PASS
user aletheon.service: active, ExecMainStatus=0, NRestarts=0       PASS
```

After the TUI reconnect integration, the required installed deployment was
rerun successfully. Latest installed/runtime SHA is
`710a5cda9c79195b8ac2de7e8da669a7be4f28d22f9674b4bec98fffdca9451b`; the
official client request and Memory Agent smoke both passed, and core/user
restart counters remained zero.
