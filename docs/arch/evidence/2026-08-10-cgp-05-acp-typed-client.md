# Agent Kernel V2：CGP-05 ACP typed-client seam

Date: 2026-08-10
Baseline: `0bf690b2`
Plan: `docs/plans/2026-08-08-composition-gateway-presentation-extraction.md` §CGP-05
State: **production ACP typed cutover active**; `aletheon/src/acp.rs` constructs
`GatewayAcpBackend` over the official typed Gateway socket. No
`ExecutiveAcpBackend` symbol remains in the production graph.

## Context receipt (runbook §3.1)

```text
Slice: CGP-05 ACP typed client cutover (PR-A translation seam)
Baseline commit: 0bf690b2
Plan revision: CGP-05 (composition-gateway-presentation-extraction.md:340-346)
Direct prerequisites: CGP-02 (gateway-client) + CGP-03 (typed handlers) — done
Current authoritative writer: Runtime/Gateway behind `GatewayAcpBackend`
Target owner/writer: interact ACP over typed GatewayClient; switched in production
IDs minted here: none — consumes server-assigned SessionRef/TurnRef
Production callers: `crates/aletheon/src/acp.rs:33` (`GatewayAcpBackend::connect`)
Test-only callers: 1 typed_client test (InMemoryTransport)
Installed/config callers: none
Tables/files/wire schemas: none changed
External side effects: none
Compatibility seam: none
Deletion owner: n/a (ExecutiveAcpBackend deletion at CGP-05 PR-C)
Unknowns/blockers: none for the seam
Expected files: crates/interact/src/acp/typed_client.rs, acp/mod.rs, interact Cargo.toml, CGP-05 gate
Out-of-scope files: the ACP switch + reconnect/cancel concurrency, ExecutiveAcpBackend deletion
```

## 1. What was created

`crates/interact/src/acp/typed_client.rs`:

- **`AcpTypedClient<T: GatewayTransport>`** — ACP create/resume/prompt/cancel translated to typed Gateway commands.
- **`PendingTransport`** — dummy transport (real socket comes from composition root at PR-C).
- Test: `typed_aclient_create_returns_server_ref` (via `InMemoryTransport`).

## 2. Rules honoured (CGP-05)

- ACP create/prompt/cancel translated to typed Gateway commands (`CreateSession`/`SubmitPrompt`/`CancelActiveTurn`) — **no raw JSON-RPC business method, no `ExecutiveAcpBackend`, no `RequestHandler`, no `UnixStream`** (CGP-05 gate rejects these).
- Session/Turn IDs are **server-assigned opaque refs** consumed from `CommandOutcome` — the adapter never mints (CGP-05: "ACP 不得 mint Session").
- Reconnect/cancel concurrency is PR-C; the legacy backend stays authoritative until then.
- Interact source set 56→57 (the typed_client seam) — CGP-00 gate updated to 57 with the census.

## 3. CGP-05 gate (architecture-check.sh)

`ARCH_SKIP_CGP05_GATES` rejects `ExecutiveAcpBackend`/`RequestHandler`/`UnixStream`/raw-JSON-RPC in the typed adapter; requires `gateway_client` + `GatewayClient` usage.

## 4. Validation

```text
bash scripts/cargo-agent.sh check -p interact      PASS
bash scripts/cargo-agent.sh test -p interact --lib acp::typed_client  PASS (1 passed)
bash scripts/cargo-agent.sh fmt --all -- --check   PASS
git diff --check                                    PASS
ARCH_SKIP_DEPENDENCIES=1 bash scripts/aletheon.sh acceptance architecture  PASS (23 findings, 0 deps; interact 52/52)
```

## 5. Remaining CGP-05 acceptance hardening

The backend switch and direct-store removal are complete. Remaining evidence is
the installed ACP-specific real request, concurrent cancel during an active
prompt, and reconnect/cursor replay on the official socket. These are acceptance
hardening items, not a second ACP authority path.

## 6. Rollback

Delete `typed_client.rs` + the acp/mod.rs module line + interact gateway deps + the CGP-05 gate; revert interact 57→56 → exact baseline. No schema, no writer, no daemon change.

## 7. Next

CGP-06 (TUI/CLI command-only cutover) touches the in-flight TUI branch; CGP-07 (App split) and CGP-08 (host deletion) follow.
