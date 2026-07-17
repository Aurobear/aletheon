# Codex-Inspired Multi-User Runtime Design

**Date:** 2026-07-17

**Status:** Approved for implementation planning

**Scope:** Local Linux multi-user execution, workspace authority, client protocol, durable turn recovery, and operator diagnostics

## 1. Decision summary

Aletheon will support multiple local Linux users without granting a shared daemon their filesystem identity or authority.

The selected architecture is a hybrid:

- a system-scoped core owns shared inference and machine-wide background services;
- a per-user runtime owns local client connections, user state, approvals, sessions, sandboxing, and tool execution;
- the invoking user's canonical current directory becomes the primary workspace root regardless of where it is located;
- explicitly added directories become additional writable roots;
- sandbox policy, approval policy, workspace selection, and OS identity remain separate concepts;
- the versioned Fabric client protocol becomes the only external turn/item event protocol;
- durable terminal boundaries are written and flushed before terminal client notification.

This design follows Codex's user-process and workspace-capability principles while retaining Aletheon's shared long-running core. It does not copy Codex's Responses API history representation or create a second source of truth beside Aletheon's canonical Session/Turn/Item records.

## 2. Motivation and current-code reality

The immediate failure is that a client launched from `/home/aurobear` is rejected even though the directory exists. The rejection is only the visible symptom of a broader authority mismatch.

| Concern | Current code | Required behavior |
|---|---|---|
| Launch directory | Two hard-coded allowed roots in `crates/executive/src/impl/daemon/handler/mod.rs:189-217` | Accept any existing canonical cwd that the invoking user can access |
| Host write boundary | Fixed `ReadWritePaths` in `config/aletheon.service:55-60` | User workspace authority is established by a per-user runtime, not a machine-specific systemd path list |
| Tool OS identity | System daemon runs as `aletheon:aletheon` in `config/aletheon.service:9-23` | Workspace tools run with the requesting user's UID/GID |
| Client identity | Peer UID is checked and then discarded in `crates/executive/src/impl/daemon/server.rs:95-119` | Principal identity remains attached to connection, thread, turn, approval, and tool call |
| Session selection | Workspace routing mutates a shared current session in `crates/executive/src/service/legacy_session_service.rs:338-360` | Every request carries an explicit user-scoped thread/session identity |
| Turn authority | Turn execution rereads the shared default session in `crates/executive/src/service/daemon_turn/execute.rs:62-71` | Turn authority comes from immutable request context |
| Approval principal | Local approval principal is constructed from current session ID in `crates/executive/src/impl/daemon/handler/rpc/rpc_approval.rs:17-23` | Approval principal is the authenticated user and exact thread/turn/call |
| Client events | Legacy wire events live in `crates/fabric/src/events/ui_event.rs:204-249` while versioned events live in `crates/fabric/src/protocol/client.rs:10-145` | One versioned external protocol with one terminal state machine |

The user has explicitly superseded the earlier fixed-Bear-ws behavior: Aletheon must be launchable from any directory and must be designed for multiple Linux users.

## 3. Codex reference principles

The design is based on the local Codex source at commit `5bed6447998c754d154dbd796517310b8f04d4ce`.

The transferable principles are:

1. **User identity is the outer security boundary.** Codex state and sockets are user-private; sandboxing narrows the authority of a process already running as that user.
2. **cwd is not an allowlist.** `-C` selects the primary cwd and `--add-dir` adds writable roots (`codex-rs/utils/cli/src/shared_options.rs:56-62`, `codex-rs/core/src/config/mod.rs:3206-3227`).
3. **Workspace and sandbox are distinct.** Workspace roots describe potential project authority; the sandbox policy determines effective access and re-protects metadata (`codex-rs/protocol/src/permissions.rs:887-939`).
4. **Approval is call-scoped.** Approval carries command, cwd, turn/call identity, and requested permissions (`codex-rs/protocol/src/approvals.rs:217-275`).
5. **Thread, turn, and item have explicit lifecycles.** The protocol exposes start/resume/fork, streaming items, and one terminal turn status (`codex-rs/app-server/README.md:64-81`).
6. **Durable history and model context are separate.** Append-only rollout remains durable while model-visible history is normalized and compacted (`codex-rs/core/src/context_manager/history.rs:121-134`, `codex-rs/core/src/compact.rs:323-368`).
7. **Connections have capabilities and bounded queues.** Initialization is explicit and overload is a retryable protocol result (`codex-rs/app-server/README.md:49-85`).
8. **Diagnostics are structured and redacted.** Doctor results contain status, evidence, remedy, and duration in both human and JSON forms (`codex-rs/cli/src/doctor.rs:146-225`).

The following Codex details are intentionally not copied:

- Responses API `ResponseItem` as Aletheon's durable history type;
- remote compaction endpoints and Codex-specific window payloads;
- cwd equality as the complete workspace identity;
- the full app-server surface before Aletheon's existing Fabric protocol is unified;
- sandbox escalation as a substitute for OS-user isolation.

## 4. Target architecture

```text
                                 machine scope
                      +-------------------------------+
                      | aletheon-core.service         |
                      | inference, shared integrations|
                      | no direct user-workspace tools|
                      +---------------+---------------+
                                      |
                         authenticated internal RPC
                                      |
            +-------------------------+-------------------------+
            |                                                   |
 user A     v                                      user B       v
+---------------------------+                    +---------------------------+
| aletheon-user runtime     |                    | aletheon-user runtime     |
| UID/GID A                 |                    | UID/GID B                 |
| private socket and state  |                    | private socket and state  |
| threads/approvals/sandbox |                    | threads/approvals/sandbox |
+-------------+-------------+                    +-------------+-------------+
              |                                                |
       cwd + add-dir                                    cwd + add-dir
              |                                                |
       user A filesystem                               user B filesystem
```

### 4.1 System core

The system core owns only machine-scoped concerns:

- inference provider connections and model catalog;
- system integrations explicitly configured as machine-wide;
- shared health and deployment metadata;
- serving shared inference requests from authenticated user runtimes.

It must not:

- execute workspace filesystem or shell tools;
- own a user's thread, approval cache, or workspace memory;
- translate sandbox escalation into root or service-account execution;
- use a client-supplied UID without transport authentication.

The core exposes a local Unix socket only to authorized Aletheon users. It derives the caller UID/GID from `SO_PEERCRED`, rejects any conflicting identity field, and returns data only for that principal. Machine-wide provider credentials remain in the core; user-scoped integration credentials remain in the per-user runtime and are never forwarded as model-visible data.

### 4.2 Per-user runtime

Each local user receives a socket-activated systemd user service. The service starts and executes as that login user; no root broker or shared service account executes its workspace tools. Packaging may provide a system template only to install or enable the user unit, not to own the runtime process.

Its runtime locations are user-private:

```text
$XDG_RUNTIME_DIR/aletheon/aletheon.sock   directory 0700, socket 0600
$XDG_STATE_HOME/aletheon/                 state owned by the user
$XDG_CACHE_HOME/aletheon/                 cache owned by the user
```

The per-user runtime owns:

- client initialization and capability negotiation;
- threads, turns, item subscriptions, and event cursors;
- canonical workspace identity;
- approval state and session-scoped permission grants;
- tool processes and their cleanup;
- sandbox policy materialization;
- user-scoped goals, memory, agent runs, and audit records.

### 4.3 Principal context

Every external request is resolved to an immutable principal context before reaching a use case:

```text
PrincipalContext
  uid
  gid
  connection_id
  thread_id
  turn_id (when a turn exists)
  canonical_cwd
  workspace_roots[]
  permission_profile
  approval_policy
```

The transport creates `uid/gid/connection_id`. Thread state supplies workspace and permission settings. A turn may apply an allowed sticky override. Model-visible input can never supply or replace the principal fields.

## 5. Workspace and permission model

### 5.1 Workspace selection

- Launching `aletheon` uses the client's canonical current directory as the primary workspace root.
- `-C <dir>` selects a different primary cwd.
- Repeated `--add-dir <dir>` values add writable roots without changing cwd.
- Relative `--add-dir` paths resolve against the final canonical cwd.
- All roots are canonicalized, deduplicated, and stored as absolute paths.
- A missing or unresolvable cwd fails initialization with the exact failing path and OS error.
- `/` is allowed only when explicitly selected and the effective permission profile permits it; it is not silently inferred.

### 5.2 Effective authority

Filesystem authority is the intersection of:

```text
requesting user's Linux DAC/ACL authority
  INTERSECT workspace roots
  INTERSECT permission profile
  INTERSECT sandbox backend enforcement
```

The default `workspace-write` profile provides:

- read access according to the selected filesystem policy;
- write access to cwd and explicit add-dir roots;
- protected metadata subpaths such as `.git`, `.aletheon`, and configured credential locations kept read-only unless an explicit narrower capability permits a mutation;
- no direct network access unless separately authorized.

`danger-full-access` means no Aletheon filesystem sandbox for that user process. It never changes UID/GID, grants root, invokes sudo, or inherits the system core's authority.

### 5.3 Approval identity

Approval keys are exact:

```text
(uid, thread_id, turn_id, call_id, approval_id)
```

An approval records command/tool identity, cwd, requested additional roots or network permissions, and the active permission profile. “For session” grants remain limited to the same user and thread. The existing global tool-name approval cache is retired from the external authorization path.

## 6. Thread, turn, and item protocol

The versioned types in `crates/fabric/src/protocol/client.rs` are extended rather than replaced.

### 6.1 Connection handshake

Every client connection performs:

```text
initialize(client version, protocol versions, capabilities)
initialize response(effective user/runtime identity and supported features)
initialized
```

Requests before initialization and repeated initialization are rejected. Negotiated capabilities are connection-scoped and cannot change another client's thread behavior.

### 6.2 Thread settings

A thread has sticky settings:

- authenticated principal UID;
- canonical cwd and workspace roots;
- permission and approval policies;
- selected model and provider policy;
- normalized workspace identity and optional repository identity.

Thread identity is never inferred solely by cwd. The lookup key is at least `(uid, thread_id)`; cwd is a filter and consistency check, not the session primary key.

### 6.3 Turn and item lifecycle

Each turn has one authoritative terminal event:

```text
TurnCompleted {
  turn_id,
  status: completed | failed | interrupted,
  error: optional structured error,
  retryable: bool,
  usage
}
```

Each item follows:

```text
started -> zero or more streaming deltas -> completed | failed
```

Stable `TurnId`, `ItemId`, and event cursor values drive live streaming, replay, reconnect deduplication, and pagination. The legacy `Error -> TurnDone` pair becomes an internal compatibility projection only during migration and is removed after the TUI uses the versioned protocol.

Turn interruption requires the exact `(thread_id, turn_id, operation_id)` precondition so a delayed cancel cannot terminate a newer turn.

## 7. Durable history, compaction, and recovery

Aletheon's canonical Session/Turn/Item store remains the durable authority. Journals and projections support replay and recovery but do not become competing histories.

### 7.1 Persistence order

For terminal settlement:

```text
finish item lifecycle
  -> persist terminal turn boundary
  -> flush durable writer
  -> clear active operation
  -> emit TurnCompleted
  -> publish idle/runtime status
```

If durable persistence fails, the client receives a terminal storage error rather than an apparently successful completion. The writer retains a queryable terminal failure so later append/flush calls cannot silently succeed.

### 7.2 Recovery

Startup and resume scan for turns that have a start boundary but no completed, failed, or interrupted boundary. Such turns are closed with a synthetic interrupted boundary carrying recovery provenance; they are not automatically rerun.

Resume selects workspace state using the latest persisted turn context, falling back to thread metadata only when no turn context exists. The persisted identity contains:

```text
principal uid
canonical cwd
canonical workspace roots
permission profile identity
optional repository/worktree identity
```

### 7.3 Context shaping

Model-visible history is a bounded derivative of canonical history:

- complete tool call/result pairs remain causally paired;
- a missing tool result is normalized to an aborted result;
- orphan results are removed from model input but remain in canonical audit history;
- compaction checkpoints receive stable IDs and lineage;
- real user messages receive an independent retention budget;
- no injected context fragment is unbounded.

Codex-specific token constants are not copied; Aletheon's model and configuration determine budgets.

## 8. Backpressure, cleanup, and error handling

- Transport ingress, per-thread work, durable writer, and outbound event queues are bounded.
- Saturation returns a typed retryable overload error with bounded retry guidance.
- Provider and compaction retry states are visible to clients and use finite attempts with backoff.
- A user runtime disconnect terminates connection-owned foreground processes and cancels orphaned approval requests.
- Explicitly backgrounded processes remain thread-owned and are discoverable and terminable after reconnect.
- Tool launch, sandbox setup, persistence, provider, protocol, and authorization errors use distinct typed categories.
- Observability failures degrade diagnostics but do not crash the runtime unless the failed component is required for safe execution.

## 9. Configuration and diagnostics

The existing layered loader and provenance implementation are retained (`crates/executive/src/core/config/mod.rs:94-193`, `crates/executive/src/core/config/provenance.rs:57-125`). The work adds read-only diagnostic surfaces:

```text
aletheon config effective
aletheon config layers
aletheon doctor
aletheon doctor --json
```

Doctor output is bounded and redacted. Each check contains status, measured evidence, expected value, remediation, and duration. It covers:

- installed and running binary SHA/version;
- system core and user runtime compatibility;
- socket paths, ownership, and modes;
- effective cwd, workspace roots, permission profile, and sandbox backend;
- provider, MCP, gbrain, and required integration health;
- canonical database, journal writer, compaction checkpoint, and unfinished-turn health;
- filesystem DAC checks for cwd and additional roots;
- effective systemd security constraints.

## 10. Migration stages

### M0: Contracts and identity

- Add `PrincipalContext`, workspace policy, typed terminal status, and protocol handshake contracts.
- Preserve existing behavior behind adapters while adding contract and two-principal tests.
- Stop deriving approval principals from session IDs.

### M1: Per-user runtime boundary

- Add private per-user socket/state layout and runtime service.
- Route system-core requests through an authenticated internal interface.
- Ensure tool subprocess UID/GID and created-file ownership match the client user.
- Prohibit shared-daemon unsandboxed execution.

### M2: Arbitrary cwd and additional roots

- Remove machine-specific root constants.
- Add `-C` and repeatable `--add-dir` semantics.
- Materialize protected writable roots in the sandbox.
- Replace the fixed Bear-ws systemd write list with the per-user execution boundary.

### M3: Explicit threads and one client protocol

- Remove global default-session switching from turn execution.
- Make thread identity explicit in chat, approval, cancel, snapshot, and subscribe requests.
- Project live turns and tools into versioned Item events.
- Move TUI resume, replay, interrupt, and terminal handling to the versioned protocol.

### M4: Durable recovery and context integrity

- Add observable bounded writer failure and terminal flush ordering.
- Close unfinished turns on recovery.
- Persist latest workspace context per turn.
- Add tool call/result normalization and compaction lineage.

### M5: Diagnostics and operational hardening

- Add effective-config and doctor commands.
- Add overload/backpressure behavior and connection-owned process cleanup.
- Add exact deployment verification and rollback checks for core/user runtime version mismatch.

Each stage is independently reviewable and deployable. Compatibility adapters may exist only between adjacent stages and must have a named removal stage.

## 11. Verification and acceptance criteria

### Security and multi-user

- Two Linux users can run Aletheon concurrently from the same path without sharing thread, memory, approval, goal, or agent state.
- A user cannot list, resolve, or reuse another user's approval.
- Files created by tools have the requesting user's UID/GID.
- `danger-full-access` cannot exceed that user's host authority.
- A client cannot forge UID/GID through JSON fields.

### Workspace behavior

- Launch succeeds from `/home/<user>`, `/tmp`, a repository, and a non-repository directory.
- `-C` changes the primary cwd.
- repeated `--add-dir` roots are writable and do not change cwd.
- paths outside effective roots are not writable in `workspace-write` mode.
- protected metadata remains read-only unless specifically authorized.
- missing, inaccessible, and symlink-sensitive paths return deterministic errors.

### Session and protocol

- Concurrent threads never change another thread's effective session or workspace.
- Every started turn reaches exactly one terminal status.
- Every started item reaches completed or failed exactly once.
- reconnect from a cursor produces no missing or duplicate completed items.
- interrupt targets only the identified active operation.

### Recovery and context

- Killing the runtime during model streaming, tool execution, compaction, and terminal persistence produces a recoverable explicit interrupted/failed turn.
- Durable canonical history remains unchanged by model-context compaction.
- tool call/result normalization never exposes an orphan result to the model.
- writer failure is visible in doctor output and prevents false success.

### Operations

- `doctor --json` is schema-stable, bounded, and secret-redacted.
- deployment verifies the actual installed SHA and both runtime versions.
- rollback restores both system-core and per-user-runtime compatible binaries/configuration.

## 12. Scope boundaries

This design does not add remote multi-tenant JWT identities, cross-user thread sharing, root tool execution, automatic sudo, or cross-user memory federation. Those require separate requirements and threat models.

The first implementation plan covers M0 through M2 as the minimum coherent fix for arbitrary-directory multi-user execution. M3 through M5 remain required follow-on plans under this same target architecture; the fixed-root workaround is not considered an acceptable intermediate deployment.
