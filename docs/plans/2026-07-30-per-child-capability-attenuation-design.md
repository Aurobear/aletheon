# Per-Child Capability Attenuation (E′)

**Date:** 2026-07-30
**Status:** Design approved (decisions locked); implementation not started
**Scope:** Enforce that a spawned child agent's effective capabilities are a
subset of its parent's runtime-effective grant, at the spawn choke point.
This is the security prerequisite for Wave 1 (B, multi-agent planning).

> Roadmap context: this is the sole remaining item of the E′ hardening wave.
> The second candidate item — a suspected `Retry-After` backoff bug — was
> **verified to be a false positive** during design: the streaming retry path
> honors `Retry-After` via `streaming_retry_delay_ms`
> (`crates/cognit/src/harness/linear/tool_exec.rs:1255-1262`,
> `exponential.max(provider_advised)`), proven by the test
> `streaming_retry_honors_a_longer_provider_retry_after`
> (`tool_exec.rs:1323-1338`); the non-streaming path treats it as authoritative
> at `crates/cognit/src/adapters/inference/scheduler.rs:316-332`. No change is
> needed there.

## 1. Background & Problem

When a parent agent spawns a child, the requested tool set is validated **only
against the child's own `AgentProfile` ceiling** — never intersected with the
parent's actual runtime-effective grant.

Current flow (verified):

```
parent -> AgentControlService::spawn(request)          crates/executive/src/application/agent_control/mod.rs:855
       -> validated_parent(&request)                    mod.rs (identity: agent_id, depth, parent_profile)
       -> launcher.launch(AgentRuntimeInput{request})   crates/executive/src/application/agent_control/execution.rs:368
       -> NativeCognitRuntimeLauncher::execute()         crates/executive/.../native_cognit.rs:203
       -> validate_requested_tools(request.allowed_tools, profile)  native_cognit.rs:209 (def 751-762)
       -> tools.filter(|t| request.allowed_tools.contains(t.name))   native_cognit.rs:254
```

The gap:

- `validate_requested_tools` checks `requested ⊆ child.profile.allowed_tools`,
  a static role ceiling. It does **not** check `requested ⊆ parent-effective`.
- The child's live-run authority is constructed from **its own** request, not a
  narrowed set: `ReparentAuthority::new(request.trusted_workspace,
  request.allowed_tools, request.budget)` at `mod.rs:1078-1082`.
- Therefore a parent that was itself narrowed at runtime can still spawn a child
  requesting more, as long as the child's profile permits it. The invariant
  `child-effective ⊆ parent-effective` is not enforced at spawn.

What already exists (and is reused, not rebuilt):

- `ReparentAuthority` = `{ workspace: Option<WorkspacePolicy>, allowed_tools:
  Vec<String>, budget: AgentBudget }` — `crates/executive/.../live_runs.rs:27-31`.
- `ReparentAuthority::covers(child)` checks **tools ⊆ and workspace-roots
  covered** — `live_runs.rs:46-62`.
- `ReparentAuthority::accepts_budget(child)` checks every budget dimension —
  `live_runs.rs:64-80`.
- Parent authority is stored per live run (`LiveAgentRun.reparent_authority:
  Arc<ReparentAuthority>`, `live_runs.rs:23`) and is lookup-able:
  `LiveAgentRuns::get(agent).await -> Option<LiveAgentRun>` (`live_runs.rs:277`)
  then `.reparent_authority()` (`live_runs.rs:145`).
- `AgentControlService` owns the registry: `live: Arc<LiveAgentRuns>`
  (`mod.rs:146`).

So `covers()` + `accepts_budget()` already express the full subset predicate for
tools + workspace + budget. They are only enforced at reparent/settlement time,
**not at spawn**.

### 1.1 Critical subtlety — two grant consumers

The granted tool set is read in **two** places, and narrowing must be applied to
**both** or it is cosmetic:

1. `LiveAgentRun.reparent_authority` — used to `covers()`-check *grandchildren*
   (transitive delegation) and at settlement.
2. `request.allowed_tools` carried into `AgentRuntimeInput` and consumed by the
   runtime tool filter at `native_cognit.rs:254` — this is what actually decides
   which tools the child can call.

Narrowing only (1) would let the child still call every requested tool via (2).
The effective set must feed both.

## 2. Goals / Non-goals

**Goals**

1. At spawn, enforce `child-effective ⊆ parent-effective` for **tools,
   workspace, and budget** (all three; the primitives already exist so it is the
   same cost as tools-only).
2. Attenuation composes transitively: a grandchild is bounded by the (already
   narrowed) child, which is bounded by the parent.
3. On over-request, **silently narrow to the intersection** and emit a durable
   observability event — never widen, never (for tool over-request) fail the
   spawn. Consistent with the codebase's recoverable-over-terminal ethos.
4. **Fail closed**: if a non-root spawn cannot resolve its parent's authority,
   reject — never fall through to an ungated grant.

**Non-goals**

- The typed `CapabilityGrant` newtype (approach B). Deferred; may be revisited if
  Wave 1 (B) needs correct-by-construction guarantees.
- Changing root-agent grant semantics (the root is the trust anchor).
- Per-agent MCP tool scoping (separate concern; MCP tools are registered
  globally in bootstrap today).
- Any behavior that could *widen* an existing grant.

## 3. Design

### 3.1 Enforcement point

Inside `AgentControlService::spawn()` (`mod.rs:855`), after `validated_parent`
and before the `LiveAgentRun::new(...)` construction at `mod.rs:1074-1083`.

### 3.2 New primitive — `ReparentAuthority::attenuate`

Add to `live_runs.rs`:

```rust
/// What was removed while narrowing a child's requested authority to fit its
/// parent's effective grant. Empty report == no narrowing occurred.
#[derive(Debug, Default, Clone)]
pub struct AttenuationReport {
    pub dropped_tools: Vec<String>,
    pub dropped_roots: Vec<std::path::PathBuf>,
    pub budget_capped: bool,
}

impl AttenuationReport {
    pub fn is_noop(&self) -> bool {
        self.dropped_tools.is_empty()
            && self.dropped_roots.is_empty()
            && !self.budget_capped
    }
}

impl ReparentAuthority {
    /// Intersect a child's `requested` authority with `self` (the parent's
    /// effective grant). The result is always covered by the parent. Never
    /// widens.
    pub fn attenuate(&self, requested: &Self) -> (Self, AttenuationReport);
}
```

Per-field semantics:

- **tools**: keep each requested tool iff `self.allowed_tools.contains(it)`;
  the rest go to `dropped_tools`.
- **workspace**: keep each child writable root iff it is covered by some parent
  root (`child_root.starts_with(parent_root)`) — mirrors the `covers()` logic at
  `live_runs.rs:51-58`. Uncovered roots go to `dropped_roots`. If the parent's
  workspace is `None` (no filesystem authority), the result is `None`.
- **budget**: per-dimension `min(requested, parent)` over exactly the dimensions
  `accepts_budget` checks (`max_input_tokens`, `max_output_tokens`,
  `max_tool_calls`, `max_elapsed_ms`, `max_depth`, and `max_cost_usd` where the
  parent's `Some` bound caps a child's). `budget_capped = true` if any dimension
  was reduced.

Post-condition (add a `debug_assert!`): for any inputs,
`self.covers(&effective) && self.accepts_budget(&effective)` holds.

### 3.3 `spawn()` wiring

```
if request has no parent (root / trust anchor):
    unchanged — no attenuation, no event.
else:
    parent_run = self.live.get(parent_agent_id).await
    if parent_run is None:
        return control_error(Forbidden, "parent authority unavailable for attenuation")  // FAIL CLOSED
    requested  = ReparentAuthority::new(request.trusted_workspace, request.allowed_tools, request.budget)
    (effective, report) = parent_run.reparent_authority().attenuate(&requested)
    if !report.is_noop():
        emit AgentRuntimeEvent::CapabilityAttenuated { agent_id, parent_agent_id, report }
    // Apply `effective` to BOTH consumers (§1.1):
    //   (a) the request/AgentRuntimeInput handed to the launcher  -> narrows native_cognit.rs:254 filter
    //   (b) LiveAgentRun::new(..., effective)                     -> replaces the raw request-derived authority at mod.rs:1078
```

"No parent" is determined by the existing root signal on the request
(`parent_agent_id` absent). The exact field/optionality is pinned in the plan.

### 3.4 New event variant

Add `CapabilityAttenuated` to `AgentRuntimeEvent` (`execution.rs:21`), carrying
`agent_id`, `parent_agent_id`, and the `AttenuationReport` fields. It flows
through the existing `AgentEventSink` chain
(`MemoryRecordingAgentEventSink` / `SpineAgentEventSink`) for durable audit —
no new transport.

## 4. Error handling

- **Missing parent authority** on a non-root spawn → `Forbidden` (fail closed).
  Never degrade to an ungated grant.
- **Empty effective tool set** after narrowing → allowed. The child spawns; any
  tool call returns the normal, recoverable "tool not found" result and the
  model can adapt. (Chosen policy: silent-narrow, not empty→reject.)
- `attenuate()` is pure and infallible.
- Root-agent path introduces **no new failure mode**.

## 5. Verification

Unit tests in `live_runs.rs`:

- `attenuate` drops tools the parent does not hold; `dropped_tools` lists them.
- `attenuate` narrows workspace roots to the covered subset; parent-`None` →
  child-`None`.
- `attenuate` caps each budget dimension to the parent min; `budget_capped` set.
- Property/post-condition: for arbitrary inputs,
  `parent.covers(&effective) && parent.accepts_budget(&effective)`.

Integration tests in `agent_control`:

- Parent tools `{a,b}` spawns child requesting `{a,b,c}` → child effective
  `{a,b}`; a `CapabilityAttenuated` event is emitted; the runtime filter yields
  only `{a,b}`.
- Transitive: grandchild ⊆ child ⊆ parent.
- Fail-closed: `parent_agent_id` present but no live run → spawn `Forbidden`.
- Root spawn (no parent) unchanged; no event emitted.
- Pre-existing `covers()` / `accepts_budget()` / reparent tests stay green.

Commands (via the wrapper, narrowest first):

```
bash scripts/cargo-agent.sh test -p executive
bash scripts/cargo-agent.sh fmt --all -- --check
bash scripts/aletheon.sh test architecture
```

## 6. Files touched

- `crates/executive/src/application/agent_control/live_runs.rs`
  — add `AttenuationReport` + `ReparentAuthority::attenuate` + unit tests.
- `crates/executive/src/application/agent_control/execution.rs`
  — add `AgentRuntimeEvent::CapabilityAttenuated` variant.
- `crates/executive/src/application/agent_control/mod.rs`
  — `spawn()`: parent lookup, attenuate, emit event, apply the effective set to
  both the launcher input and `LiveAgentRun::new`.
- `native_cognit.rs` is **unchanged**: it already filters by
  `request.allowed_tools`; it simply receives the narrowed set.

## 7. Scope boundary

This is E′ (Wave 0). It unblocks Wave 1 (B — multi-agent planning), which spawns
Planner/Executor/Reviewer children that must be attenuated. The typed
`CapabilityGrant` newtype (approach B) is intentionally deferred.
