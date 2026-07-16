# G04 Native Cognit Agent Runtime Implementation Plan

> **For agentic workers:** Use `workflow-feature` or `plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Execute child Agents through one Cognit `CognitiveSession` harness and delete the bootstrap-owned inline reasoning loop.

**Architecture:** `NativeCognitRuntime` implements the G03 launcher boundary. It creates one harness session from an `AgentProfile`, receives only a bounded context projection, invokes tools through `CapabilityService`, and returns a validated `AgentResult`; the runtime never owns lifecycle tables.

**Tech Stack:** Rust, Tokio, Cognit harness, Fabric Agent contracts, Executive capability service

**Prerequisites:** G03 and E03.

**Source requirements:** `docs/plans/2026-07-15-subagent-unified-harness-plan.md:508-549`; `docs/plans/2026-07-15-architecture-coupling-optimization-plan.md:1100-1104`.

---

## Current-code anchors

- Cognit exposes the session boundary at `crates/cognit/src/harness/session.rs:36`.
- Executive already has `CognitiveSessionFactory` at `crates/executive/src/service/harness_factory.rs:12`.
- `ProviderWorkerRuntime` duplicates a provider/tool loop at `crates/executive/src/impl/runtime/provider_worker.rs:18-150`.
- Bootstrap still builds an `ExecuteSubAgentFn` loop at `crates/executive/src/impl/daemon/bootstrap/runtime.rs:47`.
- The current runtime contract accepts only task text at `crates/executive/src/core/sub_agent.rs:49-93`.

## Invariants

- One child process owns one `CognitiveSession`; no session is reused across Agents.
- Profile model, tools, iteration and output bounds are authoritative.
- Context is labelled untrusted data and never raw hidden reasoning or unbounded history.
- Cancellation interrupts provider and tool calls; all tools use the governed capability service.

## Key contracts

```rust
#[async_trait]
pub trait AgentRuntimeLauncher: Send + Sync {
    async fn launch(&self, input: AgentRuntimeInput, events: Arc<dyn AgentEventSink>) -> Result<AgentResult, AgentControlError>;
}
pub struct AgentRuntimeInput { pub request: AgentSpawnRequest, pub handle: AgentHandle, pub context: AgentContextProjection, pub cancellation: CancellationToken }
pub struct AgentContextProjection { pub goal: Option<String>, pub constraints: Vec<String>, pub items: Vec<AgentContextItem>, pub broadcast_refs: Vec<ContentId>, pub omitted_count: usize }
```

### Task 1: Define the native runtime input and event sink

**Files:**
- Create: `crates/executive/src/impl/runtime/native_cognit.rs`
- Create: `crates/executive/src/service/agent_control/context_fork.rs`
- Modify: `crates/executive/src/impl/runtime/mod.rs`
- Modify: `crates/executive/src/service/agent_control/mod.rs`
- Create: `crates/executive/tests/native_cognit_runtime.rs`

- [ ] Add a failing constructor test that supplies a fake `CognitiveSessionFactory`, capability service and event sink without a `SubAgentSpawner`.
- [ ] Define `NativeCognitRuntimeResources { sessions, capabilities, clock }` and `AgentEventSink::emit(AgentRuntimeEvent)` with progress, tool and terminal variants carrying Agent/Process/Operation IDs.
- [ ] Change the G03 launcher input to include the validated `AgentSpawnRequest`, `AgentHandle`, projected context and operation cancellation token.
- [ ] Define the stable `AgentContextProjection`/`AgentContextItem` container here; G06 adds the Agora-aware builder without changing the launcher signature.
- [ ] Run `cargo test -p executive --test native_cognit_runtime`; expect FAIL before the launcher implementation.
- [ ] Commit with subject `test(agent): specify native Cognit runtime`.

### Task 2: Build one bounded harness session

**Files:**
- Modify: `crates/executive/src/impl/runtime/native_cognit.rs`
- Modify: `crates/executive/src/service/harness_factory.rs`
- Test: `crates/executive/tests/native_cognit_runtime.rs`

- [ ] Resolve one shared `AgentProfile` and reject unknown profile/model/tool entries before creating a session.
- [ ] Build `HarnessConfig` from profile limits and the child `AgentBudget`; use the stricter value for iterations, tokens, elapsed time and tool calls.
- [ ] Compose messages as profile system prompt, labelled `AgentContextProjection`, then task; never append parent transcript directly.
- [ ] Pass an execution context containing the persisted process and operation IDs to every capability call.
- [ ] Aggregate provider usage, tool evidence and artifact references into `AgentResult`, then call `validate()` before returning.
- [ ] Run `cargo test -p executive --test native_cognit_runtime parity`; expect final-text, one/multiple-tool and unknown-tool cases to pass.
- [ ] Commit with subject `feat(agent): run child through Cognit session`.

### Task 3: Complete failure, cancellation and policy parity

**Files:**
- Modify: `crates/executive/src/impl/runtime/native_cognit.rs`
- Test: `crates/executive/tests/native_cognit_runtime.rs`

- [ ] Add deterministic tests for max-iteration exhaustion, provider failure, cancellation during provider/tool calls, model enforcement and tool allow-list rejection.
- [ ] Map failures to typed `AgentControlErrorKind::Runtime` with bounded sanitized messages; preserve structured usage/evidence in terminal events.
- [ ] Select on cancellation around every awaited provider/tool operation and emit exactly one terminal event.
- [ ] Run `cargo test -p executive --test native_cognit_runtime`; expect all parity cases to pass.
- [ ] Commit with subject `feat(agent): enforce native runtime parity`.

### Task 4: Register native-cognit and delete duplicate loops

**Files:**
- Modify: `crates/executive/src/impl/daemon/bootstrap/runtime.rs`
- Modify: `crates/executive/src/impl/daemon/bootstrap/request.rs`
- Modify: `crates/executive/src/impl/runtime/provider_worker.rs`
- Modify: `crates/executive/src/core/sub_agent.rs`
- Modify: `scripts/architecture-check.sh`

- [ ] Register exactly one `RuntimeId("native-cognit")` through G03 and make it the default ordinary Agent runtime.
- [ ] Migrate goal worker/reviewer use separately; retain `ProviderWorkerRuntime` only for explicit goal attempt roles.
- [ ] Delete `register_agent_tool`'s inline `ExecuteSubAgentFn` loop and the generic SubAgent runtime fallback.
- [ ] Add a gate rejecting `ExecuteSubAgentFn` and direct `llm.complete` calls in daemon bootstrap and Agent control.
- [ ] Run `cargo test -p executive --test native_cognit_runtime --test agent_control_spawn && scripts/architecture-check.sh`; expect PASS.
- [ ] Commit with subject `refactor(agent): delete inline reasoning loop`.

## Final verification

Run `scripts/architecture-check.sh && cargo test --workspace --all-targets --no-fail-fast`; expect the architecture gate and complete workspace suite to pass before the final stage commit.

## Completion evidence

- [ ] All eight source parity scenarios pass.
- [ ] Child tools share the governed capability path and lifecycle IDs.
- [ ] Bootstrap contains no Agent LLM loop or `ExecuteSubAgentFn`.
- [ ] `native-cognit` is the ordinary Agent runtime authority.
