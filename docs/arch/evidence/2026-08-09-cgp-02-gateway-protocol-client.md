# Agent Kernel V2：CGP-02 gateway-protocol + gateway-client owner seam

Date: 2026-08-09
Baseline: `0bf690b2`
Plan: `docs/plans/2026-08-08-composition-gateway-presentation-extraction.md` §CGP-02, runbook §7.1 PR-A
State: owner seam established; **no traffic cut**; TUI/CLI/ACP still run the legacy path

## Context receipt (runbook §3.1)

```text
Slice: CGP-02 gateway-protocol + gateway-client owner seam
Baseline commit: 0bf690b2
Plan revision: CGP-02 (composition-gateway-presentation-extraction.md:313-320)
Direct prerequisites: CGP-00 (frozen route census) — done
Current authoritative writer: unchanged legacy Executive daemon/handler + interact raw socket
Target owner/writer: unchanged by this seam (contract defined, no writer)
IDs minted here: none — `SessionRef`/`TurnRef`/`AgentRef` are opaque client refs, not canonical IDs
Production callers: none yet — new crates are additive; legacy path untouched
Test-only callers: in-memory transport tests in gateway-client
Installed/config callers: none (no wiring)
Tables/files/wire schemas: none changed (new crates only)
External side effects: none
Compatibility seam: none (gateway-protocol is additive; legacy wire untouched)
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
- **Events**: `SessionCreated`, `TurnStarted`, `TurnSettled` (typed terminal), `Snapshot`.
- **Cursor**: `Cursor(pub u64)` opaque stream position.
- **Errors**: `ProtocolError` with typed `VersionMismatch`/`UnknownSchema`/`Timeout`/`ConnectionClosed`/`ProviderRejected`/`Server`/`Cancelled` — client must fail closed, never report success on these.
- `PROTOCOL_VERSION = 1`; additive-only versioning.

### `crates/gateway-client` (depends on `gateway-protocol` + `fabric`)

- `GatewayTransport` trait: `send_command`/`send_query`/`next_event` — the bounded framing/correlation seam.
- `GatewayClient<T>` typed facade.
- `InMemoryTransport` — contract verification without a socket (CGP-02 acceptance: "client contract 可用 in-memory transport 验证"). 4 tests pass.

## 2. Boundary discipline honored

- `gateway-protocol` depends only on `fabric` (ownerless primitives) — never on Executive, per plan §3.2 dependency direction (`contracts`/protocol → owner ports only).
- `gateway-client` depends only on `fabric` + `gateway-protocol` — **no `executive`**, no raw socket, no `executive::host` (CGP-02: "外层 ACP framing...留 interact::acp adapter，但该 adapter 不得出现 Gateway JSON-RPC/business method").
- No canonical ID mint, no effective-policy derivation, no terminal inference in the client.
- Both crates registered in `Cargo.toml` workspace members + `module-boundaries.txt`.

## 3. Not done (explicitly, per PR-A)

- No traffic cut: TUI/CLI/ACP still run the legacy Executive daemon + raw socket path.
- No server-side handler extraction (that's CGP-03).
- No socket transport yet (composition root supplies it).
- No ACP outer-framing move (that's CGP-05).

## 4. Validation

```text
bash scripts/cargo-agent.sh check -p gateway-protocol   PASS (0.26s)
bash scripts/cargo-agent.sh check -p gateway-client      PASS (0.16s)
bash scripts/cargo-agent.sh test -p gateway-client       PASS (4 passed)
  - create_session_returns_opaque_ref_without_client_mint
  - submit_prompt_returns_turn_ref
  - terminal_only_from_typed_settlement_event
  - event_stream_exhaustion_is_typed_connection_closed
bash scripts/cargo-agent.sh fmt --all -- --check         PASS
git diff --check                                          PASS
ARCH_SKIP_DEPENDENCIES=1 bash scripts/aletheon.sh acceptance architecture   PASS (23 findings, 0 deps)
```

## 5. Rollback

Both crates are additive and un-wired; removing them (plus the Cargo.toml + module-boundaries entries) restores the exact baseline. No published wire tag reused for changed semantics.
