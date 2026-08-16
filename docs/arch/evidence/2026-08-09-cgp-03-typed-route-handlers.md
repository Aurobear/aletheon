# Agent Kernel V2：CGP-03 typed Gateway route handlers

Date: 2026-08-09
Baseline: `0bf690b2`
Plan: `docs/plans/2026-08-08-composition-gateway-presentation-extraction.md` §CGP-03
State: typed Session/Turn/Approval route handlers established; **legacy daemon handlers still authoritative (route cutover is separate)**

## Context receipt (runbook §3.1)

```text
Slice: CGP-03 typed route handlers (Session/Turn/Approval first)
Baseline commit: 0bf690b2
Plan revision: CGP-03 (composition-gateway-presentation-extraction.md:322-329)
Direct prerequisites: CGP-02 (gateway-protocol) + CGP-01 + APX-01 (Application facade) — done
Current authoritative writer/handler: unchanged legacy Executive daemon rpc handlers
Target owner/writer: Gateway typed handlers over Application/Runtime ports; not yet routed
IDs minted here: none — SessionRef/TurnRef opaque refs
Production callers: none yet (typed handlers additive; legacy route untouched)
Test-only callers: 2 handler tests (fake application port + legacy adapter)
Installed/config callers: none
Tables/files/wire schemas: none changed
External side effects: none
Compatibility seam: LegacyJsonRpcAdapter (translation only)
Deletion owner: n/a
Unknowns/blockers: none
Expected files: crates/gateway/src/handlers/typed.rs, gateway Cargo.toml, module-boundaries
Out-of-scope files: the actual route cutover, CGP-04+
```

## 1. What was created

`crates/gateway/src/handlers/typed.rs`:

- **`TypedApplicationPort`** — the Application/Runtime trait handlers may call (create/resume/submit/cancel/approval); no repository/concrete-adapter access.
- **`TypedRouteHandler`** — dispatches typed `gateway_protocol::Command` to the port; translation only.
- **`CommandOutcome`** — Session/Turn/Ok typed results.
- **`LegacyJsonRpcAdapter`** — one-way translation from legacy JSON-RPC method names to typed commands; **no business logic** (CGP-03: "LegacyJsonRpcAdapter 只翻译，不执行业务").

## 2. Rules honoured (CGP-03)

- Session/Turn/Approval handled first (Health/Admin and extension route families come later).
- Each handler only calls an Application trait (`TypedApplicationPort`) — no Kernel/domain store/concrete adapter import (CGP-03 gate rejects `use crate::(kernel|executive)`/`AgentRunRepository`/`SessionAppendStore`/`Sqlite` in the typed module).
- `LegacyJsonRpcAdapter` only translates; it dispatches through the typed handler (gate requires `translate_and_dispatch`).
- Unknown/old versions fail typed: `ProtocolError::UnknownSchema` for unmapped legacy methods.
- Same command → same Runtime receipt: the typed handler and legacy route both resolve to the Application port (the legacy adapter maps onto the same typed path).
- No Kernel/domain store/concrete adapter: verified.

## 3. CGP-03 gate (architecture-check.sh)

`ARCH_SKIP_CGP03_GATES` rejects any `use crate::(kernel|executive)` / `AgentRunRepository` / `SessionAppendStore` / `Sqlite` import in the typed handler; requires `LegacyJsonRpcAdapter` + `translate_and_dispatch` (translation-only).

## 4. Validation

```text
bash scripts/cargo-agent.sh check -p gateway        PASS
bash scripts/cargo-agent.sh test -p gateway --lib handlers::typed  PASS (2 passed)
  - typed_handler_creates_session_via_application_port
  - legacy_adapter_translates_and_dispatches
bash scripts/cargo-agent.sh fmt --all -- --check   PASS
git diff --check                                    PASS
ARCH_SKIP_DEPENDENCIES=1 bash scripts/aletheon.sh acceptance architecture  PASS (23 findings, 0 deps)
```

## 5. Not done (explicitly)

- No route cutover: legacy daemon rpc handlers remain authoritative; old/new routes are not yet the same writer (that's CGP-04/CGP-06).
- No peer identity/rate-limit/protocol-version into Gateway yet (CGP-04).
- Health/Admin/extension typed handlers deferred (CGP-03 order: Session/Turn/Approval/Projection first).

## 6. Rollback

Delete `typed.rs` + the gateway Cargo.toml gateway-protocol dep + the CGP-03 gate → exact baseline. No schema, no writer, no socket change.

## 7. Next

CGP-04 (official daemon/socket cutover) is the deployment-gated route cutover; CGP-05 (ACP typed client) and CGP-06 (TUI/CLI command-only) follow.
