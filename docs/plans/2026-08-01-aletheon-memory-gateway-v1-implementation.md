# Aletheon Memory Gateway v1 Implementation Plan

> **For agentic workers:** Use `flow-feature` or `plans` to execute this plan task by task. Preserve the order: wire contract, durable ledger, host use case, protocol composition, then installed-runtime acceptance.

**Goal:** Provide Claude, Codex, and Aletheon with a negotiated, durable, host-authorized Memory Gateway for observe, lifecycle receipt, recall, and feedback operations.

**Architecture:** Fabric owns additive v1 wire types and capability negotiation. Mnemosyne owns a WAL-backed intake/lifecycle ledger but does not see client credentials or raw socket identity. Executive resolves the peer principal and canonical workspace, derives the opaque workspace key from a durable installation ID, validates and clamps client inputs, and invokes Mnemosyne through an application service. Versioned daemon dispatch returns typed terminal responses; clients without `memory_gateway_v1` cannot invoke memory methods.

**Tech Stack:** Rust 1.85, serde/schemars, Tokio, rusqlite WAL, Fabric versioned JSON-RPC, Executive dependency injection, Mnemosyne `MemoryService`.

---

## Requirement and code anchors

- The approved observe, receipt, recall, and feedback contracts are at `docs/plans/2026-08-01-unified-memory-gateway-design.md:section 5.1` through `section 5.4`.
- Capability negotiation and legacy compatibility are required by `docs/plans/2026-08-01-unified-memory-gateway-design.md:section 5`.
- Peer-authenticated identity is held by `crates/executive/src/host/daemon/server.rs:205-219`.
- Versioned request parsing and dispatch are at `crates/executive/src/host/daemon/server.rs:253-260` and `crates/executive/src/host/daemon/server.rs:330-460`.
- The existing capability shape and request enum are at `crates/fabric/src/protocol/client.rs:800-824` and `crates/fabric/src/protocol/client.rs:899-929`.
- The common host-filtered recall entry point is `MemoryService::recall_with_prefilter`; unfiltered `recall` currently derives session-only ancestry at `crates/mnemosyne/src/service.rs:920-936`.

### Task 1: Define and negotiate Memory Gateway v1 contracts

**Files:**
- Create: `crates/fabric/src/protocol/memory.rs`
- Modify: `crates/fabric/src/protocol/mod.rs`
- Modify: `crates/fabric/src/protocol/client.rs`
- Modify: `crates/fabric/src/lib.rs`
- Modify: `crates/fabric/tests/protocol_schema.rs`
- Modify: `crates/executive/src/host/daemon/protocol.rs`
- Modify: `crates/executive/src/host/daemon/server.rs`

- [ ] Add failing round-trip/schema tests for observation, receipt lookup, recall, feedback, and their typed result events.
- [ ] Add `memory_gateway_v1` to `ClientCapabilities` with serde default false so older initialize payloads remain decodable.
- [ ] Define bounded request validation and product-neutral response enums/records in `protocol/memory.rs`; do not expose Mnemosyne concrete types.
- [ ] Add four `ClientRequest` variants and JSON-RPC method names: `memory.observe/v1`, `memory.receipt.get/v1`, `memory.recall/v1`, and `memory.feedback/v1`.
- [ ] Add typed `ClientEvent` result variants.
- [ ] Add a pure protocol-state test proving a versioned client without the memory capability is rejected before dispatch, while old non-memory requests remain valid.
- [ ] Keep server capability false until the composed gateway exists; switch it on only in Task 4.
- [ ] Run `bash scripts/cargo-agent.sh test -p fabric --test protocol_schema -- --nocapture`.
- [ ] Run `bash scripts/cargo-agent.sh test -p executive host::daemon::protocol::tests -- --nocapture`.

### Task 2: Persist idempotent observation and lifecycle receipts

**Files:**
- Create: `crates/mnemosyne/src/intake.rs`
- Modify: `crates/mnemosyne/src/lib.rs`
- Create: `crates/mnemosyne/tests/memory_intake_ledger.rs`

- [ ] Add failing tests for first insert, exact idempotent duplicate, conflicting observation-ID reuse, restart persistence, and principal/workspace isolation.
- [ ] Add failing tests for monotonic revisions and legal lifecycle transitions from `observed` to evaluation/local/remote terminal states.
- [ ] Store normalized request hash, host principal, workspace key, client/session correlation, scrubbed content, sensitivity, lifecycle JSON, and timestamps in a WAL/FULL SQLite database.
- [ ] Make the insert transaction return `observed` only after commit and `duplicate` only for an identical normalized request under the same host authority.
- [ ] Reject cross-principal/workspace receipt reads and conflicting idempotency reuse without revealing whether another caller's ID exists.
- [ ] Keep `projection_failed` as a remote terminal outcome without deleting the local promotion record IDs.
- [ ] Run `bash scripts/cargo-agent.sh test -p mnemosyne --test memory_intake_ledger -- --nocapture`.

### Task 3: Implement the host-authorized application service

**Files:**
- Create: `crates/executive/src/application/memory_gateway.rs`
- Modify: `crates/executive/src/application/mod.rs`
- Create: `crates/executive/tests/memory_gateway.rs`

- [ ] Add failing tests that working-directory aliases are canonicalized, nonexistent/non-directory paths fail, and request content cannot choose principal/workspace/scope.
- [ ] Persist a durable random installation ID under the configured state root with owner-only permissions and atomic create semantics; never return it in a response.
- [ ] Derive `WorkspaceIdentity` using the existing read-only repository fingerprint resolver and `WorkspaceMemoryKey` from the durable installation ID.
- [ ] Validate observation bounds and force initial workspace state to `local_only` until a verified binding exists.
- [ ] Map feedback into a new durable observation/revision rather than mutating target content directly.
- [ ] Clamp recall item/byte budgets, build exact principal/workspace/session ancestry, and call `recall_with_prefilter`.
- [ ] Convert recall items into product-neutral wire views; supplemental items are always marked `untrusted_reference`.
- [ ] Run `bash scripts/cargo-agent.sh test -p executive --test memory_gateway -- --nocapture`.

### Task 4: Compose and dispatch the gateway

**Files:**
- Modify: `crates/executive/src/host/daemon/handler/ports.rs`
- Modify: `crates/executive/src/host/daemon/handler/mod.rs`
- Modify: `crates/executive/src/host/daemon/bootstrap/request.rs`
- Modify: `crates/executive/src/host/daemon/server.rs`
- Modify: `crates/executive/src/host/daemon/protocol.rs`
- Modify: `crates/executive/src/host/daemon/server.rs` tests

- [ ] Open the lifecycle ledger beneath the configured daemon state root and inject one `MemoryGatewayService` into request ports.
- [ ] Dispatch each typed memory request with the peer-authenticated `ConnectionContext`; never deserialize principal or workspace authority from client JSON.
- [ ] Return typed result events and stable sanitized errors.
- [ ] Advertise `memory_gateway_v1: true` only after composition succeeds.
- [ ] Add server tests for missing capability rejection, host identity precedence, duplicate observation receipts, and authoritative receipt lookup.
- [ ] Run `bash scripts/cargo-agent.sh test -p executive host::daemon::server::tests -- --nocapture`.

### Task 5: Verify, commit, and defer deployment until client wiring

**Files:**
- Verify all files above.

- [ ] Run all four focused commands above.
- [ ] Run `bash scripts/cargo-agent.sh check -p executive`.
- [ ] Run `bash scripts/cargo-agent.sh fmt --all -- --check`.
- [ ] Run `git diff --check` and inspect the full staged diff.
- [ ] Commit wire contracts/ledger and host integration as separate conventional commits if both stages remain independently reviewable.
- [ ] Do not claim installed-runtime acceptance yet: CLI and Aurb clients are wired in later plans, after which `sudo bash scripts/aletheon.sh deploy`, digest equality, stable systemd counters, and a real official-socket LLM request are mandatory.

## Acceptance

- Old clients decode and operate without memory capability; they cannot invoke memory methods.
- An accepted observation is durable before `observed` is returned and exact retries are idempotent.
- Principal and workspace come only from peer identity and canonical host resolution.
- Lifecycle reads are revisioned, authority-scoped, and terminal-state accurate.
- Recall uses exact principal/workspace/session ancestry and clamped budgets.
- Feedback creates governed evidence rather than directly rewriting durable memory.
- No token, machine installation ID, canonical path, or untrusted authority field appears in receipts.
