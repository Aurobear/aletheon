# Multi-User Runtime M0-M2 Implementation Plan

> **For agentic workers:** Use `workflow-feature` or `plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make Aletheon launch from any accessible directory while isolating each Linux user's runtime, state, approvals, workspace authority, and tool execution behind an authenticated system-core boundary.

**Architecture:** Keep machine-scoped inference in `aletheon-core`, and run client/session/tool work in a private socket-activated runtime owned by the login user. Introduce one immutable principal/workspace contract, propagate it through turns and approvals, and materialize the same canonical writable roots in structured tools and bubblewrap.

**Tech Stack:** Rust 2024, Tokio, serde/serde_json, clap, Unix domain sockets with `SO_PEERCRED`, systemd user units, bubblewrap, Cargo nextest-compatible tests.

---

## Scope and anchors

This plan implements M0-M2 only, as required by `docs/plans/2026-07-17-codex-inspired-multi-user-runtime-design.md:323-344,411`.

| Requirement | Spec anchor | Current code anchor | Owning tasks |
|---|---|---|---|
| Immutable authenticated principal | `docs/plans/2026-07-17-codex-inspired-multi-user-runtime-design.md:132-149,325-330` | peer credentials are discarded at `crates/executive/src/impl/daemon/server.rs:95-119` | 1, 3, 4 |
| Exact approval ownership | `docs/plans/2026-07-17-codex-inspired-multi-user-runtime-design.md:183-191,369-375` | session is used as principal at `crates/executive/src/impl/daemon/handler/rpc/rpc_approval.rs:17-23` | 4, 5 |
| Versioned handshake and terminal contract | `docs/plans/2026-07-17-codex-inspired-multi-user-runtime-design.md:20-21,327` | versioned protocol at `crates/fabric/src/protocol/client.rs:10-145` | 2 |
| Per-user private runtime | `docs/plans/2026-07-17-codex-inspired-multi-user-runtime-design.md:110-130,331-337` | fixed system paths at `crates/fabric/src/types/paths.rs:5-56`; early user unit at `config/aletheon.user.service:1-14` | 6, 7, 10 |
| System core cannot execute tools | `docs/plans/2026-07-17-codex-inspired-multi-user-runtime-design.md:92-108,334-336` | provider and handler coexist at `crates/executive/src/core/runtime_core.rs:36-45,224-236` | 8, 9 |
| Arbitrary cwd and add-dir | `docs/plans/2026-07-17-codex-inspired-multi-user-runtime-design.md:153-162,338-343` | fixed roots at `crates/executive/src/impl/daemon/handler/mod.rs:189-217` | 11, 12 |
| Same roots enforced everywhere | `docs/plans/2026-07-17-codex-inspired-multi-user-runtime-design.md:163-181,377-384` | single cwd in `crates/fabric/src/types/sandbox.rs:25-33`; structured write check at `crates/corpus/src/tools/tools/mutation_path.rs:5-44` | 13, 14 |
| Remove machine-specific deployment authority | `docs/plans/2026-07-17-codex-inspired-multi-user-runtime-design.md:31-33,340-343` | Bear-ws path at `config/aletheon.service:55-60` | 15, 16 |

M3-M5 are not implemented here. The M0 compatibility conversion from `SessionId` to `ThreadId` is named `LegacySessionThreadAdapter`; legacy JSON-RPC chat uses `LegacyClientHandshakeAdapter` to synthesize a capability-empty initialized connection. M3 removes both adapters together with default-session switching (`docs/plans/2026-07-17-codex-inspired-multi-user-runtime-design.md:345-350`).

## Fixed implementation decisions

- `PrincipalId` for a local user is encoded only by `PrincipalId::local_uid(uid)` as `local-uid:<decimal>`. No transport or JSON handler formats principals itself.
- `TurnRequest` owns one `PrincipalContext`; compatibility accessors derive legacy session/cwd values from that context. Duplicate client-supplied identity fields are rejected.
- Filesystem authority lives in new `types/local_authority.rs`; the existing cognitive `types/workspace.rs` is untouched.
- Client socket resolution order is `--socket`, `ALETHEON_SOCKET`, then `$XDG_RUNTIME_DIR/aletheon/aletheon.sock`.
- Core RPC authenticates UID/GID from `SO_PEERCRED`; wire requests contain no authoritative UID/GID fields.
- The shared daemon stays fail-closed for workspace tools until the per-user runtime is active. Removing the fixed root without M1 is not a deployable state.

### Task 1: Add principal and canonical workspace contracts

**Files:**
- Create: `crates/fabric/src/types/local_authority.rs`
- Create: `crates/fabric/tests/local_authority_contract.rs`
- Modify: `crates/fabric/src/types/admission.rs:31-37`
- Modify: `crates/fabric/src/types/mod.rs:1-45`
- Modify: `crates/fabric/src/lib.rs:174-269`

- [ ] **Step 1: Write the failing contract tests**

```rust
use fabric::{
    ApprovalPolicy, ConnectionId, LocalOsPrincipal, PermissionProfileId,
    PrincipalContext, PrincipalId, ThreadId, WorkspacePolicy,
};
use std::path::PathBuf;

#[test]
fn local_principal_encoding_is_stable() {
    assert_eq!(PrincipalId::local_uid(1001).0, "local-uid:1001");
}

#[test]
fn workspace_is_cwd_first_and_deduplicated() {
    let workspace = WorkspacePolicy::from_resolved_roots(
        PathBuf::from("/tmp/project"),
        vec![PathBuf::from("/tmp/extra"), PathBuf::from("/tmp/project")],
    ).unwrap();
    assert_eq!(workspace.writable_roots(), &[
        PathBuf::from("/tmp/project"),
        PathBuf::from("/tmp/extra"),
    ]);
}

#[test]
fn principal_context_round_trips_without_mutable_metadata() {
    let context = PrincipalContext::new(
        PrincipalId::local_uid(1001),
        LocalOsPrincipal { uid: 1001, gid: 1001 },
        ConnectionId::new(),
        ThreadId::from("thread-a"),
        WorkspacePolicy::from_resolved_roots(PathBuf::from("/tmp"), vec![]).unwrap(),
        PermissionProfileId::workspace_write(),
        ApprovalPolicy::OnRequest,
    );
    let json = serde_json::to_value(&context).unwrap();
    assert_eq!(json["os_principal"]["uid"], 1001);
    assert!(json.get("metadata").is_none());
}
```

- [ ] **Step 2: Run the test and verify the types are missing**

Run: `cargo test -p fabric --test local_authority_contract`

Expected: FAIL with unresolved imports for `PrincipalContext` and `WorkspacePolicy`.

- [ ] **Step 3: Implement the value objects and single principal conversion**

```rust
// crates/fabric/src/types/local_authority.rs
use serde::{Deserialize, Serialize};
use std::{collections::HashSet, path::PathBuf};
use uuid::Uuid;

use super::{PrincipalId, TurnId};

#[derive(Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ConnectionId(pub Uuid);
impl ConnectionId { pub fn new() -> Self { Self(Uuid::new_v4()) } }

#[derive(Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ThreadId(pub String);
impl From<&str> for ThreadId { fn from(value: &str) -> Self { Self(value.to_owned()) } }

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct LocalOsPrincipal { pub uid: u32, pub gid: u32 }

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PermissionProfileId(pub String);
impl PermissionProfileId {
    pub fn workspace_write() -> Self { Self("workspace-write".into()) }
    pub fn danger_full_access() -> Self { Self("danger-full-access".into()) }
    pub fn permits_filesystem_root(&self) -> bool { self.0 == "danger-full-access" }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalPolicy { Never, OnRequest }

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct WorkspacePolicy { cwd: PathBuf, writable_roots: Vec<PathBuf> }
impl WorkspacePolicy {
    pub fn from_resolved_roots(cwd: PathBuf, extra: Vec<PathBuf>) -> Result<Self, String> {
        if !cwd.is_absolute() { return Err(format!("cwd is not absolute: {}", cwd.display())); }
        let mut seen = HashSet::new();
        let mut roots = Vec::new();
        for root in std::iter::once(cwd.clone()).chain(extra) {
            if !root.is_absolute() { return Err(format!("root is not absolute: {}", root.display())); }
            if seen.insert(root.clone()) { roots.push(root); }
        }
        Ok(Self { cwd, writable_roots: roots })
    }
    pub fn cwd(&self) -> &std::path::Path { &self.cwd }
    pub fn writable_roots(&self) -> &[PathBuf] { &self.writable_roots }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PrincipalContext {
    pub principal_id: PrincipalId,
    pub os_principal: LocalOsPrincipal,
    pub connection_id: ConnectionId,
    pub thread_id: ThreadId,
    pub turn_id: Option<TurnId>,
    pub workspace: WorkspacePolicy,
    pub permission_profile: PermissionProfileId,
    pub approval_policy: ApprovalPolicy,
}

impl PrincipalContext {
    pub fn new(
        principal_id: PrincipalId,
        os_principal: LocalOsPrincipal,
        connection_id: ConnectionId,
        thread_id: ThreadId,
        workspace: WorkspacePolicy,
        permission_profile: PermissionProfileId,
        approval_policy: ApprovalPolicy,
    ) -> Self {
        Self { principal_id, os_principal, connection_id, thread_id, turn_id: None, workspace, permission_profile, approval_policy }
    }
}
```

Add to `admission.rs`:

```rust
impl PrincipalId {
    pub fn local_uid(uid: u32) -> Self { Self(format!("local-uid:{uid}")) }
}
```

Export the module and types through `types/mod.rs` and `lib.rs` using the repository's existing `pub use` pattern.

- [ ] **Step 4: Run focused and crate tests**

Run: `cargo test -p fabric --test local_authority_contract && cargo test -p fabric --lib`

Expected: both commands PASS.

- [ ] **Step 5: Commit the contract**

```bash
git add crates/fabric/src/types/local_authority.rs crates/fabric/src/types/admission.rs crates/fabric/src/types/mod.rs crates/fabric/src/lib.rs crates/fabric/tests/local_authority_contract.rs
git diff --cached --check && git diff --cached --stat
git commit -F - <<'MSG'
feat(fabric): define local runtime authority

The runtime had no immutable value tying an authenticated Linux user to a
connection, thread, workspace, and permission policy.

- add stable local principal, connection, and thread identifiers
- add a canonical cwd-first workspace policy
- cover serialization and root deduplication contracts
MSG
```

### Task 2: Add initialize and terminal protocol contracts

**Files:**
- Modify: `crates/fabric/src/protocol/client.rs:10-156`
- Modify: `crates/fabric/src/types/turn.rs:8-25`
- Modify: `crates/fabric/tests/protocol_schema.rs:1-54`

- [ ] **Step 1: Add failing schema and terminal mapping tests**

```rust
use fabric::protocol::client::{ClientCapabilities, ClientRequest, InitializeParams};
use fabric::{TurnStop, TurnTerminalStatus};

#[test]
fn initialize_has_version_and_capabilities_but_no_uid() {
    let value = serde_json::to_value(ClientRequest::Initialize(InitializeParams {
        client_version: "0.1.0".into(),
        protocol_versions: vec![1],
        capabilities: ClientCapabilities { item_events: true, cursors: true },
    })).unwrap();
    assert_eq!(value["type"], "initialize");
    assert_eq!(value["data"]["protocol_versions"], serde_json::json!([1]));
    assert!(value.to_string().find("uid").is_none());
}

#[test]
fn initialized_is_a_distinct_client_message() {
    assert_eq!(serde_json::to_value(ClientRequest::Initialized).unwrap()["type"], "initialized");
}

#[test]
fn internal_stops_map_to_one_external_terminal_status() {
    assert_eq!(TurnTerminalStatus::from(TurnStop::Completed), TurnTerminalStatus::Completed);
    assert_eq!(TurnTerminalStatus::from(TurnStop::Cancelled), TurnTerminalStatus::Interrupted);
    assert_eq!(TurnTerminalStatus::from(TurnStop::Blocked), TurnTerminalStatus::Failed);
}
```

- [ ] **Step 2: Verify schema compilation fails**

Run: `cargo test -p fabric --test protocol_schema`

Expected: FAIL because `InitializeParams` and `TurnTerminalStatus` do not exist.

- [ ] **Step 3: Add the versioned handshake and external terminal enum**

```rust
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ClientCapabilities { pub item_events: bool, pub cursors: bool }

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct InitializeParams {
    pub client_version: String,
    pub protocol_versions: Vec<u16>,
    pub capabilities: ClientCapabilities,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct InitializedResult {
    pub protocol_version: u16,
    pub server_capabilities: ClientCapabilities,
    pub connection_id: ConnectionId,
    pub principal_id: PrincipalId,
    pub os_principal: LocalOsPrincipal,
    pub runtime_version: String,
}

// Add Initialize(InitializeParams) and Initialized to ClientRequest.
// Add InitializeResponse(InitializedResult) to ClientEvent.

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnTerminalStatus { Completed, Failed, Interrupted }

impl From<TurnStop> for TurnTerminalStatus {
    fn from(value: TurnStop) -> Self {
        match value {
            TurnStop::Completed => Self::Completed,
            TurnStop::Cancelled => Self::Interrupted,
            TurnStop::Blocked | TurnStop::Failed => Self::Failed,
        }
    }
}
```

Negotiate the highest shared protocol version (currently only `1`) and reject an empty/no-overlap list with the existing structured protocol error type. Do not add UID/GID to `InitializeParams`; effective identity is present only in the server response.

- [ ] **Step 4: Run protocol regression tests**

Run: `cargo test -p fabric --test protocol_schema && cargo test -p fabric --test protocol_e2e`

Expected: both tests PASS; serialized initialize requests contain no identity authority.

- [ ] **Step 5: Commit the protocol change**

```bash
git add crates/fabric/src/protocol/client.rs crates/fabric/src/types/turn.rs crates/fabric/tests/protocol_schema.rs
git diff --cached --check
git commit -F - <<'MSG'
feat(fabric): add runtime handshake contracts

Clients need explicit version negotiation and a single external terminal state
without being allowed to assert their operating-system identity.

- add initialize capabilities and server acknowledgement
- add completed failed and interrupted terminal states
- preserve internal stop variants through a compatibility mapping
MSG
```

### Task 3: Preserve peer identity for every connection request

**Files:**
- Modify: `crates/executive/src/impl/daemon/server.rs:95-262`
- Modify: `crates/executive/src/impl/daemon/handler/mod.rs:64-148`
- Modify: `crates/executive/src/impl/daemon/handler/rpc.rs:23-28`

- [ ] **Step 1: Add a failing forged-identity test**

```rust
#[test]
fn json_identity_cannot_replace_peer_identity() {
    let peer = LocalOsPrincipal { uid: 1001, gid: 100 };
    let connection = ConnectionContext::from_peer(peer);
    let request = serde_json::json!({"method":"chat","params":{"uid":0,"gid":0}});
    assert_eq!(connection.principal_id, PrincipalId::local_uid(1001));
    assert_eq!(connection.os_principal.uid, 1001);
    assert!(request["params"]["uid"] != connection.os_principal.uid);
}

#[test]
fn connection_requires_initialize_then_initialized_exactly_once() {
    let mut state = ConnectionProtocolState::New;
    assert!(state.accept(&ClientRequest::Snapshot(fixture_snapshot())).is_err());
    state.accept(&fixture_initialize()).unwrap();
    assert!(state.accept(&fixture_initialize()).is_err());
    state.accept(&ClientRequest::Initialized).unwrap();
    assert!(matches!(state, ConnectionProtocolState::Ready { .. }));
}
```

- [ ] **Step 2: Run the server test and observe the missing connection context**

Run: `cargo test -p executive --lib impl::daemon::server::tests::json_identity_cannot_replace_peer_identity`

Expected: FAIL because `ConnectionContext` does not exist.

- [ ] **Step 3: Return credentials and propagate immutable connection context**

```rust
#[derive(Clone, Debug)]
pub struct ConnectionContext {
    pub principal_id: PrincipalId,
    pub os_principal: LocalOsPrincipal,
    pub connection_id: ConnectionId,
}

impl ConnectionContext {
    fn from_peer(os_principal: LocalOsPrincipal) -> Self {
        Self {
            principal_id: PrincipalId::local_uid(os_principal.uid),
            os_principal,
            connection_id: ConnectionId::new(),
        }
    }
}

fn check_peer_cred(stream: &UnixStream, allowed: &PeerPolicy) -> anyhow::Result<LocalOsPrincipal> {
    let cred = stream.peer_cred()?;
    allowed.authorize(cred.uid(), cred.gid())?;
    Ok(LocalOsPrincipal { uid: cred.uid(), gid: cred.gid() })
}
```

Change `handle_connection` to accept `ConnectionContext`, and change `RequestHandler::handle`, `handle_chat`, and `handle_rpc` to accept `&ConnectionContext`. Construct it once after `check_peer_cred`; never parse identity from request JSON.

Add `ConnectionProtocolState::{New, AwaitingInitialized { negotiated }, Ready { negotiated }}` inside `server.rs`. On the versioned Fabric path, only `Initialize` is accepted in `New`, only `Initialized` is accepted in `AwaitingInitialized`, and application requests are dispatched only in `Ready`. Return `InitializedResult` using the authenticated connection context and keep negotiated capabilities on that connection task. For the adjacent M0-M2 compatibility window, `LegacyClientHandshakeAdapter` recognizes only the existing JSON-RPC envelope, synthesizes `Ready` with no negotiated capabilities, and still attaches transport identity; it is never used for versioned messages and is deleted in Task M3.

- [ ] **Step 4: Run server and production health tests**

Run: `cargo test -p executive --lib impl::daemon::server && cargo test -p executive --test production_health`

Expected: PASS, including existing unauthorized-peer behavior.

- [ ] **Step 5: Commit authenticated connection propagation**

```bash
git add crates/executive/src/impl/daemon/server.rs crates/executive/src/impl/daemon/handler/mod.rs crates/executive/src/impl/daemon/handler/rpc.rs
git diff --cached --check
git commit -F - <<'MSG'
feat(executive): retain authenticated peer context

The Unix server authorized peer credentials and then discarded them before
dispatch, forcing downstream code to infer identity from mutable state.

- return UID and GID from peer authorization
- attach a stable connection identifier to every request
- prevent JSON parameters from replacing transport identity
MSG
```

### Task 4: Make turn authority explicit at the use-case boundary

**Files:**
- Modify: `crates/fabric/src/types/turn.rs:8-17`
- Modify: `crates/executive/src/service/request_use_cases.rs:409-468`
- Modify: `crates/executive/src/service/legacy_session_service.rs:338-360`
- Modify: `crates/executive/src/service/daemon_turn/execute.rs:19-74`
- Modify: `crates/executive/src/service/turn_coordinator.rs:35-42,120-234`
- Create: `crates/executive/tests/principal_turn_isolation.rs`

- [ ] **Step 1: Add a failing two-principal turn test**

```rust
#[tokio::test]
async fn concurrent_principals_keep_distinct_thread_authority() {
    let harness = TurnHarness::new().await;
    let alice = harness.context(1001, "alice-thread", "/tmp/alice");
    let bob = harness.context(1002, "bob-thread", "/tmp/bob");
    let (a, b) = tokio::join!(harness.execute(alice.clone()), harness.execute(bob.clone()));
    assert_eq!(a.unwrap().principal_id, alice.principal_id);
    assert_eq!(b.unwrap().principal_id, bob.principal_id);
    assert_ne!(harness.active_key(&alice), harness.active_key(&bob));
}
```

- [ ] **Step 2: Verify the shared default-session race fails the test**

Run: `cargo test -p executive --test principal_turn_isolation -- --nocapture`

Expected: FAIL because turn execution has no principal context or observes the shared default session.

- [ ] **Step 3: Carry context instead of rereading the default**

```rust
#[derive(Clone, Debug)]
pub struct TurnRequest {
    pub operation_id: OperationId,
    pub process_id: ProcessId,
    pub context: PrincipalContext,
    pub input: String,
    pub model_policy: Option<String>,
    pub deadline: Option<MonoDeadlineMillis>,
}

#[derive(Clone, Debug, Eq, PartialEq, Hash)]
struct ActiveTurnKey { principal_id: PrincipalId, thread_id: ThreadId }

impl ActiveTurnKey {
    fn from_context(context: &PrincipalContext) -> Self {
        Self { principal_id: context.principal_id.clone(), thread_id: context.thread_id.clone() }
    }
}
```

Preserve the existing operation, process, input, model-policy, and deadline fields exactly. Replace only the duplicate `session_id` and `working_dir` authority with `context`. Make `LegacySessionService::route_workspace` return its session ID; convert it with a `LegacySessionThreadAdapter` in the handler. Pass the resulting `PrincipalContext` through `TurnUseCases::execute`, `ProductionTurnUseCases`, and `DaemonTurnOrchestrator::execute_turn`. Delete the `default_session_id.lock()` read and `PrincipalId(session_id)` construction in `daemon_turn/execute.rs`. Derive compatibility session/cwd accessors only from `TurnRequest.context`.

- [ ] **Step 4: Run turn and session lifecycle tests**

Run: `cargo test -p executive --test principal_turn_isolation --test session_lifecycle_commands --test turn_coordinator_lifecycle`

Expected: PASS; concurrent contexts retain their own principal/thread/workspace.

- [ ] **Step 5: Commit the explicit authority boundary**

```bash
git add crates/fabric/src/types/turn.rs crates/executive/src/service/request_use_cases.rs crates/executive/src/service/legacy_session_service.rs crates/executive/src/service/daemon_turn/execute.rs crates/executive/src/service/turn_coordinator.rs crates/executive/tests/principal_turn_isolation.rs
git diff --cached --check
git commit -F - <<'MSG'
refactor(executive): make turn authority explicit

Turn execution reread a process-wide default session after workspace routing,
which allowed concurrent requests to replace each other's authority.

- carry principal and thread context into turn use cases
- scope active turns by principal and thread
- keep legacy session routing behind a named adapter
MSG
```

### Task 5: Scope durable and transient approvals to principal and call

**Files:**
- Modify: `crates/executive/src/impl/daemon/handler/rpc/rpc_approval.rs:17-95`
- Modify: `crates/executive/src/impl/daemon/handler/rpc/rpc_admin.rs:43-65`
- Modify: `crates/executive/src/service/admin_service.rs:57-77,202-228`
- Modify: `crates/executive/src/service/turn_runtime_ports.rs:491-520`
- Modify: `crates/executive/src/impl/daemon/handler/tool_executor.rs:59-68,186-193`
- Modify: `crates/executive/tests/approval_service.rs:95-198`
- Modify: `crates/fabric/src/types/tool.rs:31-37`
- Modify: `crates/fabric/src/include/turn.rs:38-79`
- Modify: `crates/corpus/src/tools/capability_executor.rs:161-177`
- Modify: `crates/corpus/src/security/approval.rs`
- Modify: `crates/corpus/src/security/socket_approval.rs`
- Modify: `crates/corpus/src/security/runner.rs:54-85,263-278`

- [ ] **Step 1: Add cross-principal rejection tests**

```rust
#[tokio::test]
async fn another_principal_cannot_resolve_transient_approval() {
    let pending = PendingApprovals::default();
    let alice = ApprovalOwner::new(PrincipalId::local_uid(1001), ThreadId("a".into()));
    let bob = ApprovalOwner::new(PrincipalId::local_uid(1002), ThreadId("b".into()));
    let id = pending.insert(alice, TurnId::new(), "call-1".into()).await;
    let error = pending.resolve(&bob, &id, ApprovalDecision::AllowOnce).await.unwrap_err();
    assert!(error.to_string().contains("not owned by authenticated principal"));
}

#[tokio::test]
async fn session_grant_is_not_reused_by_another_principal() {
    let cache = ScopedApprovalCache::default();
    cache.allow_for_thread(PrincipalId::local_uid(1001), ThreadId("a".into()), "shell").await;
    assert!(!cache.is_allowed(&PrincipalId::local_uid(1002), &ThreadId("a".into()), "shell").await);
}
```

- [ ] **Step 2: Run focused approval tests**

Run: `cargo test -p executive --test approval_service && cargo test -p executive admin_service`

Expected: the new tests FAIL because pending and cached grants are not owner-scoped.

- [ ] **Step 3: Implement exact owner keys and remove client tool authority**

```rust
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct ApprovalOwner { pub principal_id: PrincipalId, pub thread_id: ThreadId }

#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct PendingApprovalKey {
    pub owner: ApprovalOwner,
    pub turn_id: TurnId,
    pub call_id: String,
    pub approval_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Hash)]
struct ThreadGrantKey { owner: ApprovalOwner, tool: String }

pub struct ToolContext {
    pub principal_id: PrincipalId,
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    pub call_id: String,
    pub workspace: WorkspacePolicy,
}
```

Build `ApprovalContext` from authenticated `PrincipalContext` in `rpc_approval.rs`; do not call `sessions.current()`. Carry the canonical `TurnId` created by `TurnCoordinator` into `CapabilityAuthority`, carry `call_id` from `CapabilityCall`, and construct the shown `ToolContext` in `CorpusToolExecutor` without substituting operation IDs. Pass it through `security/approval.rs`, `socket_approval.rs`, and the runner. Store pending senders by `PendingApprovalKey`, and require the caller's `ApprovalOwner` on resolve. Remove `tool_name` from `TransientApprovalRequest`; tool/call identity comes from the pending record. Replace both external and Corpus tool-name-only caches with `ThreadGrantKey`.

- [ ] **Step 4: Run approval and runner regressions**

Run: `cargo test -p executive --test approval_service && cargo test -p executive admin_service && cargo test -p corpus security::runner`

Expected: PASS; another principal cannot list, resolve, or reuse a grant.

- [ ] **Step 5: Commit approval isolation**

```bash
git add crates/executive/src/impl/daemon/handler/rpc/rpc_approval.rs crates/executive/src/impl/daemon/handler/rpc/rpc_admin.rs crates/executive/src/service/admin_service.rs crates/executive/src/service/turn_runtime_ports.rs crates/executive/src/impl/daemon/handler/tool_executor.rs crates/executive/tests/approval_service.rs crates/fabric/src/types/tool.rs crates/fabric/src/include/turn.rs crates/corpus/src/tools/capability_executor.rs crates/corpus/src/security/approval.rs crates/corpus/src/security/socket_approval.rs crates/corpus/src/security/runner.rs
git diff --cached --check
git commit -F - <<'MSG'
fix(approval): isolate owners and thread grants

Approval RPCs treated session IDs as users and transient grants were cached by
tool name, allowing authority to leak across local principals.

- derive approval owners from authenticated context
- bind pending approvals to principal thread turn and call
- scope reusable grants by principal and thread
MSG
```

### Task 6: Resolve private per-user runtime paths

**Files:**
- Modify: `crates/fabric/src/types/paths.rs:5-56,202-267`
- Create: `crates/fabric/tests/user_runtime_paths.rs`

- [ ] **Step 1: Add path priority and missing-runtime tests**

```rust
#[test]
fn resolves_xdg_user_locations() {
    let env = FakeEnv::new([
        ("XDG_RUNTIME_DIR", "/run/user/1001"),
        ("XDG_STATE_HOME", "/home/a/.local/state"),
        ("XDG_CACHE_HOME", "/home/a/.cache"),
    ]);
    let paths = UserRuntimePaths::resolve(&env).unwrap();
    assert_eq!(paths.socket_path(), Path::new("/run/user/1001/aletheon/aletheon.sock"));
    assert_eq!(paths.state_root, Path::new("/home/a/.local/state/aletheon"));
}

#[test]
fn missing_xdg_runtime_dir_is_an_exact_error() {
    let error = UserRuntimePaths::resolve(&FakeEnv::default()).unwrap_err();
    assert_eq!(error.to_string(), "XDG_RUNTIME_DIR is not set for the invoking user");
}
```

- [ ] **Step 2: Verify `UserRuntimePaths` is missing**

Run: `cargo test -p fabric --test user_runtime_paths`

Expected: FAIL with unresolved `UserRuntimePaths`.

- [ ] **Step 3: Add environment-injected path resolution and secure preparation**

```rust
pub trait RuntimeEnvironment { fn var_os(&self, key: &str) -> Option<std::ffi::OsString>; }

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UserRuntimePaths {
    pub runtime_root: PathBuf,
    pub state_root: PathBuf,
    pub cache_root: PathBuf,
}

impl UserRuntimePaths {
    pub fn resolve(env: &impl RuntimeEnvironment) -> Result<Self, UserPathError> {
        let runtime = env.var_os("XDG_RUNTIME_DIR").ok_or(UserPathError::MissingRuntimeDir)?;
        let home = env.var_os("HOME").map(PathBuf::from);
        let state = env.var_os("XDG_STATE_HOME").map(PathBuf::from)
            .or_else(|| home.as_ref().map(|h| h.join(".local/state")))
            .ok_or(UserPathError::MissingHome)?;
        let cache = env.var_os("XDG_CACHE_HOME").map(PathBuf::from)
            .or_else(|| home.map(|h| h.join(".cache"))).ok_or(UserPathError::MissingHome)?;
        Ok(Self { runtime_root: PathBuf::from(runtime).join("aletheon"), state_root: state.join("aletheon"), cache_root: cache.join("aletheon") })
    }
    pub fn socket_path(&self) -> PathBuf { self.runtime_root.join("aletheon.sock") }
}
```

Add `prepare()` using `DirBuilderExt::mode(0o700)` and reject an existing runtime directory not owned by the effective UID.

- [ ] **Step 4: Run path tests**

Run: `cargo test -p fabric --test user_runtime_paths`

Expected: PASS for XDG and HOME fallback state/cache cases; runtime dir remains mandatory.

- [ ] **Step 5: Commit per-user paths**

```bash
git add crates/fabric/src/types/paths.rs crates/fabric/tests/user_runtime_paths.rs
git diff --cached --check
git commit -F - <<'MSG'
feat(fabric): resolve private user runtime paths

The client and daemon shared fixed machine paths, preventing independent local
users from owning sockets and state.

- resolve runtime state and cache locations from XDG inputs
- require an explicit per-user runtime directory
- define secure directory preparation and ownership checks
MSG
```

### Task 7: Resolve the client endpoint and private socket mode

**Files:**
- Modify: `crates/bin/src/main.rs:16-49,104-186`
- Modify: `crates/executive/src/impl/daemon/server.rs:55-76`
- Create: `crates/bin/tests/user_runtime_layout.rs`
- Modify: `crates/bin/tests/integration/socket_auth.rs:1-38`

- [ ] **Step 1: Add endpoint priority and mode tests**

```rust
#[test]
fn endpoint_priority_is_cli_then_env_then_xdg() {
    let paths = fixture_paths("/run/user/1001");
    assert_eq!(resolve_socket(Some("/tmp/explicit.sock".into()), Some("/tmp/env.sock".into()), &paths), PathBuf::from("/tmp/explicit.sock"));
    assert_eq!(resolve_socket(None, Some("/tmp/env.sock".into()), &paths), PathBuf::from("/tmp/env.sock"));
    assert_eq!(resolve_socket(None, None, &paths), paths.socket_path());
}

#[test]
fn one_systemd_listener_is_adopted_instead_of_rebound() {
    let fixture = ActivationFixture::with_one_unix_listener();
    let listener = inherited_listener(&fixture.environment(), fixture.duplicate_fd()).unwrap().unwrap();
    assert_eq!(listener.local_addr().unwrap().as_pathname(), Some(fixture.socket_path()));
}

#[tokio::test]
async fn user_socket_is_private() {
    let server = spawn_user_server().await;
    assert_eq!(std::fs::metadata(server.socket()).unwrap().permissions().mode() & 0o777, 0o600);
    assert_eq!(std::fs::metadata(server.socket().parent().unwrap()).unwrap().permissions().mode() & 0o777, 0o700);
}
```

- [ ] **Step 2: Run the layout test**

Run: `cargo test -p aletheon-bin --test user_runtime_layout`

Expected: FAIL because CLI defaults to `/run/aletheon/aletheon.sock` and server mode is `0660`.

- [ ] **Step 3: Implement endpoint resolution and socket privacy**

```rust
fn resolve_socket(explicit: Option<PathBuf>, env: Option<OsString>, paths: &UserRuntimePaths) -> PathBuf {
    explicit.or_else(|| env.map(PathBuf::from)).unwrap_or_else(|| paths.socket_path())
}
```

Change `Cli.socket` to `Option<PathBuf>`. Resolve it once before dispatch. Add a `SocketPrivacy::{UserPrivate,SystemCore}` server option: user-private prepares parent `0700` and sets socket `0600`; core retains group-authorized policy. Update socket tests so user runtime no longer assumes group mode `0660`.

Adopt systemd socket activation before trying to bind a path. Read `LISTEN_PID`, require it to match the current PID, require exactly one `LISTEN_FDS`, validate fd 3 is a Unix stream listener, clear the activation variables, and convert the fd to nonblocking `tokio::net::UnixListener`. A malformed activation environment is a startup error; absence of activation falls back to the resolved socket path.

```rust
enum ListenerSource { Activated(std::os::fd::OwnedFd), Path(PathBuf) }

fn listener_source(env: &impl RuntimeEnvironment) -> Result<ListenerSource, ActivationError> {
    match (env.var_os("LISTEN_PID"), env.var_os("LISTEN_FDS")) {
        (None, None) => Ok(ListenerSource::Path(resolve_user_socket(env)?)),
        (Some(pid), Some(fds)) if pid == std::process::id().to_string() && fds == "1" => {
            Ok(ListenerSource::Activated(duplicate_and_validate_unix_listener(3)?))
        }
        values => Err(ActivationError::Invalid(values)),
    }
}
```

- [ ] **Step 4: Run binary routing and socket tests**

Run: `cargo test -p aletheon-bin --test user_runtime_layout --test host_routing && cargo test -p aletheon-bin socket_auth`

Expected: PASS; default client endpoint is per-user and explicit overrides still work.

- [ ] **Step 5: Commit endpoint isolation**

```bash
git add crates/bin/src/main.rs crates/executive/src/impl/daemon/server.rs crates/bin/tests/user_runtime_layout.rs crates/bin/tests/integration/socket_auth.rs
git diff --cached --check
git commit -F - <<'MSG'
feat(runtime): use private per-user sockets

Clients defaulted to the shared system socket and user runtimes inherited a
group-readable socket policy.

- resolve client endpoints from CLI environment and XDG paths
- create user socket directories with mode 0700
- create per-user sockets with mode 0600
MSG
```

### Task 8: Introduce an inference port and serializable core frames

**Files:**
- Create: `crates/executive/src/service/inference_port.rs`
- Modify: `crates/executive/src/service/mod.rs`
- Modify: `crates/fabric/src/types/llm_types.rs:20-43,93-116`
- Create: `crates/executive/tests/inference_port_contract.rs`

- [ ] **Step 1: Add a local adapter contract test**

```rust
#[tokio::test]
async fn local_inference_port_preserves_response_and_stream_frames() {
    let provider = Arc::new(FakeProvider::responding("ok"));
    let port = LocalInferencePort::new(provider);
    let response = port.complete(CoreInferenceRequest::fixture()).await.unwrap();
    assert_eq!(response.stop_reason, StopReason::EndTurn);
    let chunks = port.stream(CoreInferenceRequest::fixture()).await.unwrap().collect::<Vec<_>>().await;
    assert!(matches!(chunks.last().unwrap().as_ref().unwrap(), StreamChunk::Done { stop_reason: StopReason::EndTurn }));
}
```

- [ ] **Step 2: Verify the port does not exist**

Run: `cargo test -p executive --test inference_port_contract`

Expected: FAIL with unresolved `LocalInferencePort`.

- [ ] **Step 3: Add object-safe complete/stream methods and wire-safe LLM types**

```rust
#[async_trait::async_trait]
pub trait InferencePort: Send + Sync {
    async fn complete(&self, request: CoreInferenceRequest) -> Result<LlmResponse, InferenceError>;
    async fn stream(&self, request: CoreInferenceRequest) -> Result<LlmStream, InferenceError>;
}

pub struct LocalInferencePort { provider: Arc<dyn LlmProvider> }

#[async_trait::async_trait]
impl InferencePort for LocalInferencePort {
    async fn complete(&self, request: CoreInferenceRequest) -> Result<LlmResponse, InferenceError> {
        self.provider.complete(&request.messages, &request.tools).await.map_err(Into::into)
    }
    async fn stream(&self, request: CoreInferenceRequest) -> Result<LlmStream, InferenceError> {
        self.provider.complete_stream(&request.messages, &request.tools).await.map_err(Into::into)
    }
}
```

Add serde derives to `StreamChunk`, `LlmResponse`, `StopReason`, and `Usage`. `CoreInferenceRequest` contains messages, tool definitions, model selection, and stream mode; it contains no UID/GID and no workspace paths.

- [ ] **Step 4: Run contract and Fabric serialization tests**

Run: `cargo test -p executive --test inference_port_contract && cargo test -p fabric --lib llm_types`

Expected: PASS for complete and stream paths.

- [ ] **Step 5: Commit the inference seam**

```bash
git add crates/executive/src/service/inference_port.rs crates/executive/src/service/mod.rs crates/fabric/src/types/llm_types.rs crates/executive/tests/inference_port_contract.rs
git diff --cached --check
git commit -F - <<'MSG'
refactor(runtime): introduce the core inference port

Request handling constructed provider clients directly, preventing a hard
boundary between machine credentials and user-owned execution.

- add object-safe complete and streaming inference operations
- add a local compatibility adapter for focused tests
- make inference frames serializable without workspace authority
MSG
```

### Task 9: Add authenticated core Unix RPC

**Files:**
- Create: `crates/executive/src/impl/core_rpc/mod.rs`
- Create: `crates/executive/src/impl/core_rpc/protocol.rs`
- Create: `crates/executive/src/impl/core_rpc/server.rs`
- Create: `crates/executive/src/impl/core_rpc/client.rs`
- Modify: `crates/executive/src/impl/mod.rs`
- Create: `crates/executive/tests/core_rpc_auth.rs`

- [ ] **Step 1: Add framing and identity-forgery tests**

```rust
#[test]
fn core_request_schema_has_no_authoritative_identity() {
    let value = serde_json::to_value(CoreRequest::complete(7, CoreInferenceRequest::fixture())).unwrap();
    let wire = value.to_string();
    assert!(!wire.contains("uid"));
    assert!(!wire.contains("gid"));
}

#[tokio::test]
async fn server_uses_peer_credentials() {
    let harness = CoreRpcHarness::start().await;
    harness.client().complete(CoreInferenceRequest::fixture()).await.unwrap();
    assert_eq!(harness.observed_peer().uid, unsafe { libc::geteuid() });
}

#[test]
fn core_peer_policy_rejects_unlisted_users() {
    let policy = CorePeerPolicy::new(0, 991, [1001]);
    assert!(policy.authorize(LocalOsPrincipal { uid: 1001, gid: 100 }).is_ok());
    assert!(policy.authorize(LocalOsPrincipal { uid: 1002, gid: 100 }).is_err());
}

#[tokio::test]
async fn oversized_and_duplicate_frames_are_rejected() {
    let harness = CoreRpcHarness::start_with_limit(128).await;
    assert!(harness.write_raw(vec![b'x'; 129]).await.unwrap_err().to_string().contains("frame exceeds 128 bytes"));
    harness.write_frame(CoreRequest::complete(7, CoreInferenceRequest::fixture())).await.unwrap();
    assert!(harness.write_frame(CoreRequest::complete(7, CoreInferenceRequest::fixture())).await.unwrap_err().to_string().contains("duplicate request id 7"));
}
```

- [ ] **Step 2: Run the missing RPC test**

Run: `cargo test -p executive --test core_rpc_auth`

Expected: FAIL because `core_rpc` is absent.

- [ ] **Step 3: Implement newline-delimited typed frames and peer authentication**

```rust
#[derive(Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum CoreFrame {
    Request { id: u64, request: CoreInferenceRequest },
    Response { id: u64, response: LlmResponse },
    Chunk { id: u64, chunk: StreamChunk },
    Completed { id: u64 },
    Error { id: u64, message: String },
}
```

The server obtains `peer_cred()` immediately after accept and applies `CorePeerPolicy`: allow root, the configured `aletheon` group, or an explicitly configured UID; reject everyone else before reading a frame. Supplementary group membership is resolved from `/proc/<pid>/status` using the peer PID and is bounded to the single accepted connection. Pass `LocalOsPrincipal` only as non-model-visible audit context. The client correlates frames by request ID; streaming uses a bounded Tokio MPSC channel and yields `Chunk` frames until `Completed` or `Error`. Reject duplicate request IDs and frames larger than 8 MiB.

- [ ] **Step 4: Run RPC authentication and stream tests**

Run: `cargo test -p executive --test core_rpc_auth -- --nocapture`

Expected: PASS; peer UID equals effective UID and the request JSON has no identity field.

- [ ] **Step 5: Commit core RPC**

```bash
git add crates/executive/src/impl/core_rpc crates/executive/src/impl/mod.rs crates/executive/tests/core_rpc_auth.rs
git diff --cached --check
git commit -F - <<'MSG'
feat(runtime): authenticate internal core inference RPC

The user runtime needs inference without receiving machine provider credentials
or allowing a client-supplied identity to cross the core boundary.

- add bounded typed Unix RPC frames for complete and stream calls
- derive caller identity from Unix peer credentials
- reject oversized and duplicate request frames
MSG
```

### Task 10: Split system core and user runtime bootstrap

**Files:**
- Create: `crates/executive/src/core/system_core_runtime.rs`
- Create: `crates/executive/src/user_runtime/mod.rs`
- Modify: `crates/executive/src/core/mod.rs`
- Modify: `crates/executive/src/core/runtime_core.rs:36-64,224-236`
- Modify: `crates/executive/src/impl/daemon/bootstrap/request.rs:65-134,461-504`
- Modify: `crates/executive/src/impl/session/canonical_store.rs:21-40,75-108`
- Modify: `crates/executive/src/host/launcher.rs:14-77`
- Modify: `crates/bin/src/main.rs:52-186`
- Create: `crates/executive/tests/core_user_boundary.rs`

- [ ] **Step 1: Add compile-time ownership boundary tests**

```rust
#[tokio::test]
async fn user_runtime_builds_from_inference_port_without_provider_registry() {
    let runtime = UserRuntime::bootstrap(UserRuntimeConfig::fixture(), Arc::new(FakeInferencePort::default())).await.unwrap();
    runtime.health().await.unwrap();
}

#[tokio::test]
async fn two_user_runtime_configs_never_share_state_paths() {
    let alice = UserRuntimeConfig::fixture_at("/tmp/alice-state");
    let bob = UserRuntimeConfig::fixture_at("/tmp/bob-state");
    let a = UserRuntime::bootstrap(alice, Arc::new(FakeInferencePort::default())).await.unwrap();
    let b = UserRuntime::bootstrap(bob, Arc::new(FakeInferencePort::default())).await.unwrap();
    assert_ne!(a.state_paths(), b.state_paths());
    assert!(a.state_paths().iter().all(|p| p.starts_with("/tmp/alice-state")));
    assert!(b.state_paths().iter().all(|p| p.starts_with("/tmp/bob-state")));
}

#[tokio::test]
async fn core_registry_resolves_requested_models_and_rejects_unknown_providers() {
    let port = RegistryInferencePort::fixture_with_alias("fast", "openai/gpt-test");
    assert_eq!(port.resolve_model("fast").unwrap().model, "gpt-test");
    assert!(port.resolve_model("missing-provider/model").unwrap_err().to_string().contains("Provider 'missing-provider' not found"));
}

#[test]
fn system_core_surface_exposes_no_request_handler() {
    fn accepts_core(_: &SystemCoreRuntime) {}
    accepts_core(&SystemCoreRuntime::fixture());
}
```

Add architecture grep assertions to the test: `system_core_runtime.rs` must not contain `RequestHandler`, `ToolRegistry`, or `Sandbox`; `user_runtime/mod.rs` must not contain `ProviderRegistry` or credential loading.

- [ ] **Step 2: Run the boundary test**

Run: `cargo test -p executive --test core_user_boundary`

Expected: FAIL because the two runtime types are absent.

- [ ] **Step 3: Split bootstrap ownership and route the user runtime through RPC**

```rust
pub struct SystemCoreRuntime {
    provider_registry: Arc<ProviderRegistry>,
    inference_server: CoreInferenceServer,
}

struct RegistryInferencePort { registry: Arc<ProviderRegistry> }

#[async_trait::async_trait]
impl InferencePort for RegistryInferencePort {
    async fn complete(&self, request: CoreInferenceRequest) -> Result<LlmResponse, InferenceError> {
        let provider = self.registry.resolve_and_create(&request.model_spec)?;
        provider.complete(&request.messages, &request.tools).await.map_err(Into::into)
    }
    async fn stream(&self, request: CoreInferenceRequest) -> Result<LlmStream, InferenceError> {
        let provider = self.registry.resolve_and_create(&request.model_spec)?;
        provider.complete_stream(&request.messages, &request.tools).await.map_err(Into::into)
    }
}

pub struct UserRuntime {
    request_handler: Arc<RequestHandler>,
    server: UnixServer,
}

impl UserRuntime {
    pub async fn bootstrap(config: UserRuntimeConfig, inference: Arc<dyn InferencePort>) -> Result<Self> {
        let request = config.request.with_data_dir(config.paths.state_root.clone());
        let request_handler = RequestHandler::new(request, inference).await?;
        Ok(Self { server: UnixServer::user_private(config.socket, request_handler.clone())?, request_handler })
    }
}
```

Change `RequestHandler::new` to accept `Arc<dyn InferencePort>` and remove provider construction from user bootstrap. `RegistryInferencePort` is the only adapter allowed to call `ProviderRegistry::resolve_and_create`; it resolves `CoreInferenceRequest.model_spec` for every call and returns the registry's exact unknown-provider error. Add `core` and `daemon` launch paths: `aletheon core` starts only `SystemCoreRuntime`; `aletheon daemon` starts only `UserRuntime` and connects to the core socket. `aletheon exec` uses an ephemeral `UserRuntime` with the same core client and user paths; it must not recreate a provider registry. Do not load project configuration in system-core bootstrap.

Replace every `config.data_dir`/`xdg_data_dir()` state constructor in request bootstrap and canonical store with the injected `UserRuntimePaths.state_root`: sessions, canonical records, self field, recall/episodic/consolidation/retention memory, approvals/audit, lineage, channel state, goals, agents, and external artifacts. Cache-only artifacts use `cache_root`. No user store may fall back to a machine path.

Classify integrations explicitly in config: provider connections/model catalog are `Machine`; Telegram, Google, gbrain/MCP credentials and their durable state default to `User` and remain in user bootstrap. A machine integration must opt in with `scope = "machine"` and must expose no workspace tool capability. Add config tests that system core rejects user-scoped credential fields and user runtime config contains no machine provider API keys.

- [ ] **Step 4: Run boundary and daemon host tests**

Run: `cargo test -p executive --test core_user_boundary --test production_health && cargo test -p aletheon-bin --test host_routing`

Expected: PASS; the user runtime has no provider registry and core has no tool handler.

- [ ] **Step 5: Commit runtime separation**

```bash
git add crates/executive/src/core/system_core_runtime.rs crates/executive/src/user_runtime/mod.rs crates/executive/src/core/mod.rs crates/executive/src/core/runtime_core.rs crates/executive/src/impl/daemon/bootstrap/request.rs crates/executive/src/impl/session/canonical_store.rs crates/executive/src/host/launcher.rs crates/bin/src/main.rs crates/executive/tests/core_user_boundary.rs
git diff --cached --check
git commit -F - <<'MSG'
refactor(runtime): split system core from user execution

The monolithic runtime owned provider credentials, sessions, approvals, and
workspace tools in one shared process.

- keep providers and machine integrations in SystemCoreRuntime
- keep sessions approvals memory sandbox and tools in UserRuntime
- route user inference through the authenticated core port
MSG
```

### Task 11: Resolve `-C` and repeatable `--add-dir` deterministically

**Files:**
- Modify: `crates/fabric/src/types/local_authority.rs`
- Create: `crates/fabric/tests/workspace_resolution.rs`
- Modify: `crates/bin/src/main.rs:16-49,74-97`
- Create: `crates/bin/tests/workspace_cli.rs`
- Modify: `crates/executive/src/host/launcher.rs:68-90`

- [ ] **Step 1: Add canonicalization and CLI parsing tests**

```rust
#[test]
fn relative_add_dir_uses_final_cwd_and_deduplicates() {
    let tree = TempWorkspace::new(["project", "project/shared"]);
    let policy = WorkspaceSelection::new(Some(PathBuf::from("project")), vec![PathBuf::from("shared"), PathBuf::from("shared")])
        .resolve(tree.root()).unwrap();
    assert_eq!(policy.cwd(), tree.path("project").canonicalize().unwrap());
    assert_eq!(policy.writable_roots().len(), 2);
}

#[test]
fn filesystem_root_requires_explicit_selection_and_permitting_profile() {
    let implicit = WorkspaceSelection::new(None, vec![]).resolve_with_profile(Path::new("/"), &PermissionProfileId::workspace_write());
    assert!(matches!(implicit, Err(WorkspaceResolveError::ImplicitFilesystemRoot)));
    let explicit_denied = WorkspaceSelection::new(Some(PathBuf::from("/")), vec![])
        .resolve_with_profile(Path::new("/tmp"), &PermissionProfileId::workspace_write());
    assert!(matches!(explicit_denied, Err(WorkspaceResolveError::FilesystemRootDenied { .. })));
}

#[test]
fn cli_accepts_global_workspace_options() {
    let cli = Cli::try_parse_from(["aletheon", "-C", "/tmp", "--add-dir", "/var/tmp", "--add-dir", "/opt/work"]).unwrap();
    assert_eq!(cli.workspace.add_dirs.len(), 2);
}
```

- [ ] **Step 2: Run resolver and CLI tests**

Run: `cargo test -p fabric --test workspace_resolution && cargo test -p aletheon-bin --test workspace_cli`

Expected: FAIL because `WorkspaceSelection` and global flags are absent.

- [ ] **Step 3: Implement exact resolution with no fallback**

```rust
impl WorkspaceSelection {
    pub fn resolve_with_profile(self, process_cwd: &Path, profile: &PermissionProfileId) -> Result<WorkspacePolicy, WorkspaceResolveError> {
        let explicitly_selected = self.cwd.is_some();
        let requested = self.cwd.unwrap_or_else(|| process_cwd.to_path_buf());
        let cwd_input = if requested.is_absolute() { requested } else { process_cwd.join(requested) };
        let cwd = canonical_directory(&cwd_input)?;
        if cwd == Path::new("/") && !explicitly_selected {
            return Err(WorkspaceResolveError::ImplicitFilesystemRoot);
        }
        if cwd == Path::new("/") && !profile.permits_filesystem_root() {
            return Err(WorkspaceResolveError::FilesystemRootDenied { profile: profile.clone() });
        }
        let mut roots = Vec::new();
        for raw in self.add_dirs {
            let input = if raw.is_absolute() { raw } else { cwd.join(raw) };
            roots.push(canonical_directory(&input)?);
        }
        WorkspacePolicy::from_resolved_roots(cwd, roots).map_err(WorkspaceResolveError::Policy)
    }
}
```

Define global clap `WorkspaceArgs { #[arg(short='C', long="cd")] cwd, #[arg(long="add-dir")] add_dirs }`. Resolve once before TUI, message, daemon exec dispatch. Remove exec's `-d` path and the fallback chain that replaces a failed `current_dir()` with `/tmp`. Preserve exact input path and `io::Error` in `WorkspaceResolveError`.

- [ ] **Step 4: Run resolver, CLI, and host tests**

Run: `cargo test -p fabric --test workspace_resolution && cargo test -p aletheon-bin --test workspace_cli --test host_routing`

Expected: PASS for cwd override, relative add-dir, deduplication, missing path, file-not-directory, and symlink canonicalization.

- [ ] **Step 5: Commit workspace selection**

```bash
git add crates/fabric/src/types/local_authority.rs crates/fabric/tests/workspace_resolution.rs crates/bin/src/main.rs crates/bin/tests/workspace_cli.rs crates/executive/src/host/launcher.rs
git diff --cached --check
git commit -F - <<'MSG'
feat(cli): resolve cwd and additional workspace roots

Workspace selection was split across clients and silently fell back when path
resolution failed.

- add global -C and repeatable --add-dir options
- canonicalize relative roots against the final cwd
- return deterministic path errors instead of changing directories silently
MSG
```

### Task 12: Carry the resolved workspace through chat and capability authority

**Files:**
- Modify: `crates/interact/src/tui/mod.rs:42-61,366-377`
- Modify: `crates/interact/src/tui/app/submit.rs:328-343`
- Modify: `crates/interact/src/tui/app/lifecycle.rs:269-277`
- Modify: `crates/interact/src/tui/cli.rs:454-491`
- Modify: `crates/executive/src/impl/daemon/handler/mod.rs:143-219`
- Modify: `crates/executive/src/service/daemon_turn/execute.rs:19-27`
- Modify: `crates/executive/src/service/turn_pipeline.rs:243-326`
- Modify: `crates/executive/src/service/governed_capability.rs:20-33,227-276`
- Modify: `crates/executive/src/impl/daemon/handler/tool_executor.rs:76-100`
- Create: `crates/executive/src/service/thread_authority.rs`
- Modify: `crates/executive/tests/governed_capability_path.rs`

- [ ] **Step 1: Add an immutable envelope/authority test**

```rust
#[test]
fn chat_envelope_contains_the_resolved_workspace() {
    let workspace = fixture_workspace("/tmp/project", &["/tmp/shared"]);
    let value = chat_request("hello", &workspace);
    assert_eq!(value["params"]["working_dir"], "/tmp/project");
    assert_eq!(value["params"]["workspace_roots"], serde_json::json!(["/tmp/project", "/tmp/shared"]));
}

#[tokio::test]
async fn model_arguments_cannot_replace_capability_workspace() {
    let authority = fixture_authority("/tmp/project", &["/tmp/shared"]);
    let prepared = prepare_with_model_path(authority.clone(), "/etc").await.unwrap();
    assert_eq!(prepared.authority.workspace, authority.workspace);
}

#[test]
fn thread_workspace_is_bound_once_per_authenticated_user() {
    let store = ThreadAuthorityStore::in_memory();
    let key = ThreadAuthorityKey::new(PrincipalId::local_uid(1001), ThreadId("thread-a".into()));
    let first = fixture_thread_settings("/tmp/project", &["/tmp/shared"]);
    store.bind_or_verify(&key, &first).unwrap();
    let changed = fixture_thread_settings("/etc", &[]);
    assert!(matches!(store.bind_or_verify(&key, &changed), Err(ThreadAuthorityError::Conflict { .. })));
}
```

- [ ] **Step 2: Run interact and authority tests**

Run: `cargo test -p interact working_dir_tests && cargo test -p executive --test governed_capability_path`

Expected: FAIL because the envelope and authority carry only a single implicit cwd.

- [ ] **Step 3: Replace implicit cwd reads and fixed-root checks**

Change `chat_request(message, workspace)` to serialize canonical cwd and roots. Store `WorkspacePolicy` in immutable client/App configuration and pass it from full TUI, line mode, and `-m`; remove `client_working_dir()`.

In the daemon, deserialize roots, recanonicalize them with `WorkspacePolicy::verify_existing()`, and compare them to thread authority. Delete `LOCAL_WORKSPACE_ROOT`, `LEGACY_WORKING_DIR`, and `validate_working_dir_against_roots`. Pass `WorkspacePolicy` through turn, capability, authority provider, and tool executor signatures.

Implement `ThreadAuthorityStore::bind_or_verify` keyed by `(PrincipalId, ThreadId)`. The first initialized request persists workspace, permission/approval policy, and model policy under the injected user state root; later requests must match byte-for-byte after canonical normalization or return `ThreadAuthorityError::Conflict` without changing the stored record. Cwd is only a consistency field, never the lookup key. `LegacySessionThreadAdapter` supplies the thread ID but cannot overwrite stored settings.

```rust
pub struct CapabilityExecutionContext {
    pub principal: PrincipalContext,
    pub workspace: WorkspacePolicy,
    pub operation_id: OperationId,
}
```

- [ ] **Step 4: Run client, handler, and authority regressions**

Run: `cargo test -p interact working_dir_tests --test tui_reducer && cargo test -p executive working_dir_tests --test governed_capability_path --test turn_use_case_ports`

Expected: PASS; no fixed machine root remains and all capability calls retain the resolved roots.

- [ ] **Step 5: Commit workspace propagation**

```bash
git add crates/interact/src/tui crates/executive/src/impl/daemon/handler/mod.rs crates/executive/src/service/daemon_turn/execute.rs crates/executive/src/service/turn_pipeline.rs crates/executive/src/service/governed_capability.rs crates/executive/src/service/thread_authority.rs crates/executive/src/impl/daemon/handler/tool_executor.rs crates/executive/tests/governed_capability_path.rs
git diff --cached --check
git commit -F - <<'MSG'
feat(runtime): propagate explicit workspace authority

Clients reread process cwd per message and the daemon reduced workspace
authority to a hard-coded single-root check.

- keep resolved workspace selection stable for the client lifetime
- revalidate canonical roots at the user runtime boundary
- carry one workspace policy through turns capabilities and tools
MSG
```

### Task 13: Enforce multi-root policy in structured mutation tools

**Files:**
- Modify: `crates/fabric/src/types/tool.rs:31-37`
- Modify: `crates/corpus/src/tools/tools/mutation_path.rs:3-118`
- Modify: `crates/corpus/src/tools/tools/file_write.rs:50`
- Modify: `crates/corpus/src/tools/tools/apply_patch.rs:69-79`
- Modify: `crates/corpus/tests/controlled_apply.rs`

- [ ] **Step 1: Add add-dir, protected metadata, and symlink tests**

```rust
#[test]
fn add_dir_is_writable_but_metadata_and_symlink_escape_are_rejected() {
    let tree = MutationTree::new();
    let policy = tree.workspace_with_add_dir();
    assert!(validate_mutation_path(&policy, &tree.add_dir().join("ok.txt")).is_ok());
    assert!(validate_mutation_path(&policy, &tree.add_dir().join(".git/config")).is_err());
    tree.symlink_from_add_dir_to_outside("escape");
    assert!(validate_mutation_path(&policy, &tree.add_dir().join("escape/file")).is_err());
}
```

- [ ] **Step 2: Run mutation tests**

Run: `cargo test -p corpus mutation_path && cargo test -p corpus --test controlled_apply`

Expected: FAIL because validation accepts only `working_dir`.

- [ ] **Step 3: Validate against canonical roots and shared protected names**

```rust
pub fn validate_mutation_path(policy: &WorkspacePolicy, requested: &Path) -> Result<PathBuf, String> {
    let candidate = absolute_candidate(policy.cwd(), requested);
    reject_protected_components(&candidate)?;
    let (ancestor, suffix) = nearest_existing_ancestor(&candidate)?;
    let canonical = ancestor.canonicalize().map_err(|e| format!("{}: {e}", ancestor.display()))?.join(suffix);
    if !policy.writable_roots().iter().any(|root| canonical.starts_with(root)) {
        return Err(format!("path is outside writable roots: {}", candidate.display()));
    }
    Ok(canonical)
}
```

Put the protected metadata names (`.git`, `.aletheon`) and explicit configured credential paths in one `ProtectedPathPolicy`. Do not infer extra protected basenames or extensions. Change `ToolContext`, `file_write`, and `apply_patch` to pass both `WorkspacePolicy` and that materialized protected-path policy.

- [ ] **Step 4: Run structured-tool tests**

Run: `cargo test -p corpus mutation_path && cargo test -p corpus --test controlled_apply`

Expected: PASS; ordinary add-dir writes succeed while metadata and symlink escapes fail.

- [ ] **Step 5: Commit structured write enforcement**

```bash
git add crates/fabric/src/types/tool.rs crates/corpus/src/tools/tools/mutation_path.rs crates/corpus/src/tools/tools/file_write.rs crates/corpus/src/tools/tools/apply_patch.rs crates/corpus/tests/controlled_apply.rs
git diff --cached --check
git commit -F - <<'MSG'
feat(corpus): enforce structured multi-root writes

Structured mutation tools bypass process sandboxing and only recognized the
primary working directory.

- validate writes against every canonical workspace root
- reject symlink escapes through the nearest existing ancestor
- keep protected metadata read-only in every writable root
MSG
```

### Task 14: Materialize the same policy in production bubblewrap

**Files:**
- Modify: `crates/fabric/src/types/sandbox.rs:25-33,90-99`
- Modify: `crates/corpus/src/security/runner.rs:365-415`
- Modify: `crates/corpus/src/security/sandbox/policy.rs:13-77`
- Modify: `crates/corpus/src/security/sandbox/bwrap_builder.rs:24-71,290-307`
- Modify: `crates/corpus/src/security/sandbox/bubblewrap.rs:76-149,240-306`
- Modify: `crates/corpus/src/security/sandbox/executor.rs:11-39`
- Create: `crates/corpus/tests/workspace_sandbox.rs`

- [ ] **Step 1: Add mount-order and real behavior tests**

```rust
#[test]
fn protected_mounts_follow_every_writable_bind_and_cwd_is_not_rebound() {
    let args = build_args(fixture_two_root_policy());
    for root in ["/tmp/project", "/tmp/shared"] {
        let writable = position(&args, &["--bind", root, root]);
        let protected = position(&args, &["--ro-bind", &format!("{root}/.git"), &format!("{root}/.git")]);
        assert!(writable < protected);
    }
    assert_eq!(count_triplet(&args, "--bind", "/tmp/project", "/tmp/project"), 1);
}
```

The Linux integration test runs `touch` in cwd and add-dir, then asserts outside-root and `.git/.aletheon` writes fail. It prints `SKIP: bwrap unavailable` only when `bwrap --version` or an initial user-namespace probe fails; argv tests always run.

- [ ] **Step 2: Run sandbox tests and expose the duplicate cwd bind**

Run: `cargo test -p corpus sandbox::bwrap_builder && cargo test -p corpus sandbox::bubblewrap && cargo test -p corpus --test workspace_sandbox -- --nocapture`

Expected: mount-order test FAIL because cwd is rebound after protected metadata; real test may print the explicit skip message on unsupported hosts.

- [ ] **Step 3: Use one mount plan in production**

```rust
pub struct SandboxConfig {
    pub workspace: WorkspacePolicy,
    pub environment: BTreeMap<String, String>,
}

fn append_mount_plan(args: &mut Vec<OsString>, policy: &FilesystemPolicy) {
    push(args, ["--ro-bind", "/", "/"]);
    for root in &policy.writable_roots { push_bind(args, &root.path); }
    for root in &policy.writable_roots {
        for relative in &root.read_only_subpaths {
            push_ro_bind_if_exists(args, &root.path.join(relative));
        }
    }
    for glob in &policy.unreadable_globs { append_read_masks_for_glob(args, glob); }
}
```

Make `BubblewrapBackend` and `BwrapBuilder` share `append_mount_plan`. Required order is read-only `/`, every writable root, protected subpaths for every root, masks, then command. Do not bind cwd again when it is already a writable root. Keep argv-based execution and fail closed when bubblewrap setup fails.

- [ ] **Step 4: Run unit and real sandbox tests**

Run: `cargo test -p corpus sandbox::bwrap_builder && cargo test -p corpus sandbox::bubblewrap && cargo test -p corpus --test workspace_sandbox -- --nocapture`

Expected: PASS, or the real behavior test reports only the documented environment skip while argv security tests PASS.

- [ ] **Step 5: Commit sandbox materialization**

```bash
git add crates/fabric/src/types/sandbox.rs crates/corpus/src/security/runner.rs crates/corpus/src/security/sandbox/policy.rs crates/corpus/src/security/sandbox/bwrap_builder.rs crates/corpus/src/security/sandbox/bubblewrap.rs crates/corpus/src/security/sandbox/executor.rs crates/corpus/tests/workspace_sandbox.rs
git diff --cached --check
git commit -F - <<'MSG'
fix(sandbox): materialize canonical workspace roots

Production bubblewrap ignored the declarative multi-root policy and rebound the
cwd after protected metadata, weakening the intended mount boundary.

- share one ordered mount plan with the production backend
- bind all writable roots before re-protecting metadata
- verify allowed denied and symlink-sensitive writes
MSG
```

### Task 15: Install system-core and socket-activated user units

**Files:**
- Create: `config/aletheon-core.service`
- Create: `config/aletheon.user.socket`
- Modify: `config/aletheon.user.service:1-14`
- Modify: `config/aletheon.service:1-60`
- Modify: `setup.sh:444-458`
- Modify: `scripts/install-systemd.sh:16-65`
- Create: `tests/systemd_runtime_boundary.sh`

- [ ] **Step 1: Add static unit boundary checks**

```bash
#!/usr/bin/env bash
set -euo pipefail
! grep -R '/home/aurobear/Bear-ws' config/*.service config/*.socket
grep -q '^ListenStream=%t/aletheon/aletheon.sock$' config/aletheon.user.socket
grep -q '^SocketMode=0600$' config/aletheon.user.socket
grep -q '^RuntimeDirectoryMode=0700$' config/aletheon.user.service
! grep -Eq 'ReadWritePaths=.*(/home|/tmp)' config/aletheon-core.service
grep -q 'ExecStart=.*aletheon core' config/aletheon-core.service
grep -q 'ExecStart=.*aletheon daemon' config/aletheon.user.service
```

- [ ] **Step 2: Run the unit boundary test**

Run: `bash tests/systemd_runtime_boundary.sh`

Expected: FAIL because the core/socket split and private socket unit are absent.

- [ ] **Step 3: Add exact systemd units and installer behavior**

```ini
# config/aletheon.user.socket
[Unit]
Description=Aletheon per-user runtime socket
[Socket]
ListenStream=%t/aletheon/aletheon.sock
DirectoryMode=0700
SocketMode=0600
[Install]
WantedBy=sockets.target
```

```ini
# essential config/aletheon.user.service fields
[Service]
ExecStart=%h/.local/bin/aletheon daemon
RuntimeDirectory=aletheon
RuntimeDirectoryMode=0700
StateDirectory=aletheon
CacheDirectory=aletheon
```

`aletheon daemon` adopts fd 3 from `aletheon.user.socket` through Task 7 and therefore must not bind `%t/aletheon/aletheon.sock` again. `aletheon-core.service` runs `aletheon core`, exposes only its group-authorized internal socket, and grants write access only to core-owned `/run`, `/var/lib`, and `/var/cache` locations. Replace `config/aletheon.service` with a compatibility alias or core-only unit; it must not execute tools or contain user workspace paths. Install `config/aletheon.user.socket` as the user unit name `aletheon.socket` and enable that socket, not a permanently running user service.

- [ ] **Step 4: Verify units and installer tests**

Run: `bash tests/systemd_runtime_boundary.sh && systemd-analyze verify config/aletheon-core.service config/aletheon.user.socket config/aletheon.user.service`

Expected: PASS with no Bear-ws path or broad `/home`/`/tmp` write authority.

- [ ] **Step 5: Commit deployment units**

```bash
git add config/aletheon-core.service config/aletheon.user.socket config/aletheon.user.service config/aletheon.service setup.sh scripts/install-systemd.sh tests/systemd_runtime_boundary.sh
git diff --cached --check
git commit -F - <<'MSG'
fix(deploy): install core and per-user runtimes

The system service owned workspace execution and encoded a machine-specific
Bear-ws write path, while the early user unit lacked socket activation.

- install a core-only system service
- install a private socket-activated user runtime
- remove host-specific and broad user workspace write paths
MSG
```

### Task 16: Verify multi-user ownership, arbitrary cwd, and fail-closed rollout

**Files:**
- Create: `scripts/verify-multi-user-runtime.sh`
- Create: `crates/bin/tests/arbitrary_workspace_e2e.rs`
- Modify: `tests/architecture_check.sh`
- Modify: `docs/plans/2026-07-17-codex-inspired-multi-user-runtime-design.md:323-344`

- [ ] **Step 1: Add deterministic single-user E2E coverage**

```rust
#[tokio::test]
async fn launches_from_repo_non_repo_and_tmp() {
    for cwd in fixture_dirs(["repo/.git", "plain", "tmp-like"]) {
        let result = TestAletheon::spawn().cwd(&cwd).message("pwd").await;
        assert!(result.is_success(), "{}: {}", cwd.display(), result.stderr());
        assert_eq!(result.created_file_owner(), (unsafe { libc::geteuid() }, unsafe { libc::getegid() }));
    }
}

#[tokio::test]
async fn danger_full_access_never_changes_os_identity() {
    let result = TestAletheon::spawn().args(["--sandbox", "danger-full-access"]).message("id -u").await;
    assert_eq!(result.stdout().trim(), unsafe { libc::geteuid() }.to_string());
}
```

- [ ] **Step 2: Run E2E tests before deployment**

Run: `cargo test -p aletheon-bin --test arbitrary_workspace_e2e -- --nocapture`

Expected: PASS in the build tree; missing/inaccessible paths return their exact path and OS error.

- [ ] **Step 3: Add the privileged two-user verifier without embedding credentials**

```bash
#!/usr/bin/env bash
set -euo pipefail
: "${ALETHEON_TEST_USER_A:?set an existing unprivileged user}"
: "${ALETHEON_TEST_USER_B:?set an existing unprivileged user}"
for user in "$ALETHEON_TEST_USER_A" "$ALETHEON_TEST_USER_B"; do
  uid=$(id -u "$user")
  runuser -u "$user" -- systemctl --user start aletheon.socket
  runuser -u "$user" -- test -S "/run/user/$uid/aletheon/aletheon.sock"
  mode=$(stat -c %a "/run/user/$uid/aletheon/aletheon.sock")
  test "$mode" = 600
done
test "$(stat -c %u "/run/user/$(id -u "$ALETHEON_TEST_USER_A")/aletheon/aletheon.sock")" = "$(id -u "$ALETHEON_TEST_USER_A")"
test "$(stat -c %u "/run/user/$(id -u "$ALETHEON_TEST_USER_B")/aletheon/aletheon.sock")" = "$(id -u "$ALETHEON_TEST_USER_B")"
```

Extend the script to submit distinct thread names and approval IDs through each private socket, assert cross-user list/resolve returns not-found/forbidden, run a tool that creates a file in each user's fixture directory, and compare file UID/GID with `id -u/-g` for that user. The script exits before mutation if either account or user manager is unavailable.

- [ ] **Step 4: Run the full deterministic regression set and deployment verifier**

Run:

```bash
cargo test -p fabric --all-targets --no-fail-fast
cargo test -p corpus --all-targets --no-fail-fast
cargo test -p executive --all-targets --no-fail-fast
cargo test -p interact --all-targets --no-fail-fast
cargo test -p aletheon-bin --all-targets --no-fail-fast
cargo clippy -p fabric -p corpus -p executive -p interact -p aletheon-bin --all-targets -- -D warnings
bash tests/architecture_check.sh
test -n "${ALETHEON_TEST_USER_A:-}" && test -n "${ALETHEON_TEST_USER_B:-}"
# Manual deployment-host gate after explicit operator approval:
sudo --preserve-env=ALETHEON_TEST_USER_A,ALETHEON_TEST_USER_B bash scripts/verify-multi-user-runtime.sh
```

Expected: all Cargo and architecture commands PASS. The final command PASSes on a deployment host with two named existing accounts; otherwise it exits before changing services and prints the missing prerequisite.

- [ ] **Step 5: Record M0-M2 verification and commit tests**

Update the migration checklist in the design with the exact successful commands, date, and deployed binary SHA; do not mark M3-M5 complete.

```bash
git add scripts/verify-multi-user-runtime.sh crates/bin/tests/arbitrary_workspace_e2e.rs tests/architecture_check.sh docs/plans/2026-07-17-codex-inspired-multi-user-runtime-design.md
git diff --cached --check
git commit -F - <<'MSG'
test(runtime): verify multi-user workspace isolation

The runtime split needs deployment-level proof that sockets, approvals, tools,
and created files remain owned by the requesting Linux user.

- cover arbitrary repository and non-repository working directories
- verify private sockets and cross-user approval isolation
- record full M0-M2 regression and deployed binary identity
MSG
```

## Final review gate

- [ ] `git grep -nE 'LOCAL_WORKSPACE_ROOT|LEGACY_WORKING_DIR|/home/aurobear/Bear-ws' -- crates config scripts tests` returns no matches.
- [ ] `git grep -n 'PrincipalId(session_id)' -- crates` returns no matches.
- [ ] `git grep -n 'default_session_id.lock' -- crates/executive/src/service/daemon_turn` returns no matches.
- [ ] User runtime source contains no `ProviderRegistry`; system-core source contains no `RequestHandler`, sandbox, or tool registry.
- [ ] Every client and tool path uses one resolved `WorkspacePolicy`; no canonicalization failure falls back to `.`, process cwd, or `/tmp`.
- [ ] Two-user verifier proves private state/approvals and requesting UID/GID file ownership.
- [ ] M3 removal owner is explicit: `LegacySessionThreadAdapter` is removed when the versioned protocol becomes the only client protocol.
