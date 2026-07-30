# Per-Child Capability Attenuation (E′)

**Date:** 2026-07-30
**Status:** Revised design; security decisions locked; implementation not started
**Scope:** Derive one authoritative effective child grant before admission and
enforce `child-effective ⊆ parent-runtime-effective` for tools, workspace policy,
and budget. This is the security prerequisite for Wave 1 multi-agent planning.

## 1. Current state and corrected problem statement

`AgentControlService::spawn()` currently validates parent identity at
`crates/executive/src/application/agent_control/mod.rs:596-665`, but constructs
the live authority directly from the requested workspace, tools, and budget at
`mod.rs:1074-1083`. The native runtime filters tools from that same request.
Nothing intersects the request with the parent's runtime-effective grant.

The existing `ReparentAuthority` is useful but is not yet a complete subset
contract:

- `covers()` checks tool membership and writable-root containment only
  (`live_runs.rs:46-62`).
- `WorkspacePolicy` also carries `protected_paths`
  (`crates/fabric/src/types/local_authority.rs:71-76`); a child that omits a
  parent's protected credential path is wider even when its writable roots are
  identical.
- `validated_parent()` supports an external root process that is live in Kernel
  but absent from the Agent repository/live registry (`mod.rs:625-648`). A
  `LiveAgentRuns`-only resolver would therefore break the first root-to-child
  spawn.
- Request budget is consumed by admission and Kernel deadline construction
  before `LiveAgentRun::new` (`mod.rs:876-900`), and the request is persisted at
  `mod.rs:1021-1032`. Narrowing only the launcher and live-run authority would
  leave admission, deadlines, hashes, and durable records inconsistent.

The effective grant consequently has four consumer classes, all of which must
receive exactly the same value:

```text
parent authority + child request
              |
              v
       attenuate once
              |
      +-------+----------+-------------+
      |                  |             |
 admission/deadline   durable record  runtime tool/workspace filter
                                         |
                                   LiveAgentRun authority
```

## 2. Goals and non-goals

### Goals

1. Resolve the parent's runtime-effective authority without trusting model input.
2. Compute the effective child authority once, immediately after parent identity
   validation and before request hashing, admission, process allocation, deadline
   construction, or persistence.
3. Enforce tools, writable roots, protected paths, and every `AgentBudget`
   dimension. More protected paths means less authority.
4. Apply the effective request to every downstream consumer and persist enough
   evidence to distinguish requested from effective authority.
5. Compose transitively for grandchildren.
6. Fail closed when a non-root parent authority cannot be resolved.

### Non-goals

- Aggregate reservation across multiple siblings. E′ is the per-child subset
  invariant; machine/provider concurrency and shared budget reservation remain
  owned by admission/backpressure.
- Per-agent MCP registration. MCP tools remain subject to the same effective
  name filter, but registry partitioning is separate.
- Changing the authority of a true root spawn with no parent.
- Any fallback that widens authority.

## 3. Typed authority source

### 3.1 `AgentDelegationAuthority`

Move the cross-boundary grant shape into Fabric rather than passing an
Executive-private `ReparentAuthority` through public structs:

```rust
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AgentDelegationAuthority {
    pub workspace: Option<WorkspacePolicy>,
    pub allowed_tools: Vec<String>,
    pub budget: AgentBudget,
}
```

`AgentSpawnIntent` and `AgentSpawnRequest` gain a host-only
`#[serde(skip)] delegator_authority: Option<AgentDelegationAuthority>`. Model or
wire JSON can never mint it.

The authoritative source is selected as follows:

1. **True root request:** both parent IDs absent; no attenuation.
2. **Managed child parent:** resolve the parent from `LiveAgentRuns` and use its
   already-effective authority. Ignore any presented copy for authorization.
3. **External root parent:** `validated_parent()` has proved that the Kernel
   process is the matching live root. Require the host-injected
   `delegator_authority`; if absent, reject `Forbidden`.

The agent-control tool boundary extends `AgentToolContext`
(`crates/fabric/src/types/tool.rs:10`, today only `caller_root_agent_id` /
`parent_agent_id` / `parent_process_id`) with this host-only authority. The mint
sites are the runtime composition points that already build `AgentToolContext`:
the turn runtime for the root/main agent
(`crates/executive/src/application/turn_pipeline.rs:699`) and the child runtime
launcher (`crates/executive/src/adapters/runtime/native_cognit.rs:261`). Each
must populate the delegable authority from the *actual installed tool set,
effective `WorkspacePolicy`, and delegable budget* at that site — never from
profile defaults. The corpus tool boundary that forwards a spawn is
`crates/corpus/src/tools/tools/agent_control.rs:295`, where the model supplies
`allowed_tools`; the parent grant must therefore arrive out-of-band through the
host-only `delegator_authority` field, not this input. The sandbox-runner mirror
is `crates/corpus/src/security/runner.rs:1709`. Host-driven workflow callers
obtain the same typed authority from the turn runtime; they must not reconstruct
it from profile defaults.

### 3.2 One subset contract (single owner)

`AgentDelegationAuthority` is the **only** type that implements the subset
contract — `covers()`, `accepts_budget()`, and `attenuate()` (§4). These are
pure functions defined in Fabric next to the type
(`crates/fabric/src/types/agent_control.rs`), with per-field workspace logic
delegated to `WorkspacePolicy` helpers in
`crates/fabric/src/types/local_authority.rs` (which already ships
`narrow_writable_roots`, `local_authority.rs:116`). The Executive-private
`ReparentAuthority` (`crates/executive/src/application/agent_control/live_runs.rs:27`,
whose `covers()` / `accepts_budget()` live at `:46` / `:64`) is retired:
`LiveAgentRun` stores an `AgentDelegationAuthority` directly. To bound call-site
churn, `live_runs.rs` may keep `ReparentAuthority` **only** as
`pub type ReparentAuthority = AgentDelegationAuthority` — never a second struct
with its own subset or attenuation logic. Executive calls the Fabric functions;
per §9 it must not reimplement them. This single home is a hard invariant of the
design: two subset implementations is precisely the failure mode E′ exists to
prevent.

## 4. Attenuation semantics

Add a pure operation returning both the effective grant and an audit report:

```rust
pub struct AttenuationReport {
    pub dropped_tools: Vec<String>,
    pub dropped_writable_roots: Vec<PathBuf>,
    pub inherited_protected_paths: Vec<PathBuf>,
    pub budget_capped_fields: Vec<AgentBudgetField>,
    pub requested_sha256: String,
    pub effective_sha256: String,
}

pub fn attenuate(
    parent: &AgentDelegationAuthority,
    requested: &AgentDelegationAuthority,
) -> Result<(AgentDelegationAuthority, AttenuationReport), AgentControlError>;
```

Field rules:

- **Tools:** stable intersection in child-request order; duplicate names are
  rejected by existing request validation.
- **Writable roots:** retain a child root only when it is inside a parent root.
  `child.workspace = None` remains no filesystem authority and is always narrower
  than a parent workspace. A parent `None` forces child `None`.
- **Protected paths:** the effective policy contains the union of parent and
  child protected paths, canonicalized through `ProtectedPathPolicy::new`.
  Parent protections may never be dropped by a child that retains filesystem
  authority. A child with `workspace=None` has no filesystem authority, so no
  protection list needs to be materialized.
- **Working directory:** retain the child's cwd because cwd is independent of
  writable authority; it grants no write access by itself
  (`local_authority.rs:111-116`).
- **Budget:** take the per-field minimum for input/output tokens, tool calls,
  elapsed time, depth, and cost. For cost, parent `None` is unbounded; parent
  `Some(x)` caps child `None` to `Some(x)`.

`attenuate()` is total except for one fallible step: materializing the unioned
protected paths via `ProtectedPathPolicy::new`
(`crates/fabric/src/types/local_authority.rs:165`, which returns `Result`). That
is the only `Err` source — every other field rule is a total intersection or
minimum — which is why the signature returns `Result`.

Extend the single `covers()` predicate (on `AgentDelegationAuthority`, §3.2) so
its filesystem truth table treats `None` as no authority and verifies both
writable containment and inherited protection. Required post-condition:

```text
parent.covers(effective)
&& parent.accepts_budget(effective)
&& (effective.workspace.is_none()
    || effective.protected_paths ⊇ parent.protected_paths)
```

## 5. Spawn ordering

`spawn()` must use this order:

```text
request.validate()
launcher/profile resolution
validated_parent()
resolve_parent_authority()
attenuate() -> effective request + report
hash effective request
admission.reserve(effective)
Kernel process/deadline from effective budget
persist AgentRunRecord containing effective request
construct AgentRuntimeInput from effective request
construct LiveAgentRun from effective authority
emit CapabilityAttenuated after durable AgentRunRecord creation
launch runtime
```

The durable record stores the effective request as the executable truth, and the
existing `request_hash` (`agent_spawn_request_hash`,
`crates/executive/src/application/agent_control/mod.rs:48,867`) is computed over
that **effective** request, not the original. The `requested_sha256` /
`effective_sha256` in `AttenuationReport` are digests of the delegation-authority
grant only (tools + workspace + budget) and are distinct from `request_hash`;
they let the audit event show exactly what narrowed. The attenuation event
carries those digests and the removed fields; it does not persist secrets or
unrestricted workspace content.

## 6. Error handling

- Missing managed-parent live authority: `Forbidden`.
- External root identity validated but host delegation authority absent:
  `Forbidden`.
- Protected-path canonicalization failure: reject before admission.
- Empty effective tools or writable roots: allowed; the child runs with less
  authority.
- Any post-condition failure: return an internal authority error in release and
  trigger `debug_assert!` in debug builds; never continue with the request.
- Root request with no parent: unchanged and emits no attenuation event.

## 7. Verification

Unit/contract tests:

- Tool, root, budget, and cost intersections.
- Parent protected paths are inherited even when absent in the child request.
- `Some(parent workspace)` covers a child `None`; parent `None` rejects/forces
  any child workspace to `None`.
- Property test for the complete post-condition.

AgentControl integration tests:

- Managed parent `{a,b}` and child `{a,b,c}` produces `{a,b}` in admission,
  deadline, persisted request, runtime input, and live authority.
- External Kernel root plus host delegation authority can spawn its first child.
- The same external root without a host delegation authority fails closed.
- A grandchild cannot regain a dropped tool, root, protected path, or budget.
- Replaying/observing `CapabilityAttenuated` does not change an existing grant.

Validation commands:

```bash
bash scripts/cargo-agent.sh test -p fabric --lib agent_control
bash scripts/cargo-agent.sh test -p executive application::agent_control
bash scripts/cargo-agent.sh fmt --all -- --check
bash scripts/aletheon.sh test architecture
```

Installed acceptance is required because the change affects tool context,
AgentControl, persistence, daemon bootstrap, and runtime launch. Run
`sudo bash scripts/aletheon.sh deploy`, verify binary digests and stable restart
counters, then prove root -> child -> grandchild attenuation through the official
user socket during a real LLM-backed request.

Status caveat: to date only the evaluation-kernel base has a historical
production-acceptance record. E′ — like the L2 permission closed-loop and the
other roadmap workstreams (A/B/C/D) — is to-be-implemented design and must not be
described as production-wired until this installed acceptance passes.

## 8. Files touched

- `crates/fabric/src/types/agent_control.rs` — `AgentDelegationAuthority` and
  host-only authority fields.
- `crates/fabric/src/types/tool.rs` — trusted delegation authority on
  `AgentToolContext`.
- `crates/fabric/src/types/local_authority.rs` — complete protected-path-aware
  narrowing/subset helpers.
- `crates/corpus/src/tools/tools/agent_control.rs` — forward only the host-minted
  delegator authority.
- `crates/executive/src/application/agent_control/live_runs.rs` — complete
  attenuation and subset predicate.
- `crates/executive/src/application/agent_control/mod.rs` — resolve, attenuate,
  and replace the request before all consumers.
- `crates/executive/src/application/agent_control/execution.rs` — durable
  `CapabilityAttenuated` event.
- Runtime/tool-context composition sites
  (`crates/executive/src/application/turn_pipeline.rs:699`,
  `crates/executive/src/adapters/runtime/native_cognit.rs:261`) — mint authority
  from effective runtime state, never from requested profile defaults.

## 9. Dependency boundary

E′ remains Wave 0 and must pass installed acceptance before the multi-agent
planning loop is enabled. B may use the new typed grant but may not introduce a
second attenuation implementation.
