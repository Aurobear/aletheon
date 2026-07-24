# Agent Runtime Correctness Implementation Plan

> **For agentic workers:** Use `flow-feature` or `plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Correct Plan Mode prompt visibility, preserve the selected model's real context limit across the core/daemon boundary, replace six-message replay with token-budget selection, and make installed-runtime acceptance the mandatory completion gate.

**Architecture:** Deliver four independently reviewable stages. Runtime model capabilities are resolved by the core that owns provider configuration and returned over the existing inference boundary; the daemon consumes that value for budgeting. Canonical history remains the source of truth, while a deterministic token-budget selector chooses the newest complete conversational units without splitting tool-use/result pairs. Installed deployment—not a development daemon—is the final acceptance environment.

**Tech Stack:** Rust, Tokio, serde, existing `fabric::Message::estimate_tokens`, existing Mnemosyne `ContextBudgetPlanner`, JSON-RPC/core RPC, shell/systemd acceptance scripts.

---

## Requirement and code anchors

The plan covers these approved requirements:

1. Plan Mode must be visible to the model, while mutation denial remains enforced at the permission layer (`docs/plans/gap-analysis-20260724.md:297-312`). Current `ReActLoop::run()` pushes raw input (`crates/cognit/src/harness/linear/step.rs:34`), while the marker is only produced by `compose_user_message()` (`crates/cognit/src/harness/linear/message_compose.rs:22-48`).
2. The daemon must use the effective selected model context limit rather than `128_000` (`docs/plans/gap-analysis-20260724.md:356-416`). The hard-coded value is at `crates/executive/src/application/inference_port.rs:51-65`; the value feeds session construction at `crates/executive/src/host/daemon/bootstrap/request.rs:269-281`.
3. History replay must be token-budget driven, not capped at six messages (`docs/plans/gap-analysis-20260724.md:267-295`). The cap is at `crates/executive/src/application/daemon_turn/helpers.rs:12,27-45`; the existing budget calculation already exposes `history_budget` through `crates/executive/src/host/daemon/bootstrap/turn_runtime.rs:250-278`.
4. Completion requires installed-runtime deployment and a real request. Existing provenance, stability, and official-client gates are at `scripts/lib/aletheon/runtime_gate.sh:46-143`.

## Selective-absorption constraints

The Codex/Pi ideas are not rejected, but they must extend current Aletheon
mechanisms in measured increments. Their separate plan is
`docs/plans/2026-07-24-selective-agent-pattern-absorption.md`.

- Use `Reasoner`/`Planner`/`Critic` only behind task/risk gates and benchmark each
  stage against the existing ReAct/verifier path.
- Extend the existing context-fragment and world-state paths; do not replace
  them with parallel abstractions.
- Improve the existing BM25 `tool_search`; do not add another implementation.
- Keep SQLite canonical and add JSONL only as a versioned import/export format.
- Add compaction-window identity only to persisted compaction metadata and
  diagnostics; do not inject it into every prompt without measured benefit.
- Improve the system prompt by responsibility and benchmark, not word count.
- Do not implement the evidence-gated backlog listed at the end of this document until its gate passes.

---

### Task 1: Route every live user input through the existing Plan Mode composer

**Files:**
- Modify: `crates/cognit/src/harness/linear/step.rs:29-35`
- Modify: `crates/cognit/src/harness/linear/message_compose.rs:22-48`
- Test: `crates/cognit/src/harness/linear/mod.rs` (existing `ReActLoop` unit-test module)

- [ ] **Step 1: Add a failing test proving the live `run()` path sends the marker**

Add a recording `LlmProvider`, enable plan mode, call `run("inspect only", ...)`, and assert its first captured user message begins with `PLAN_MODE_MARKER` and ends with `inspect only`. The provider must return a text-only response so no tool executor is invoked.

- [ ] **Step 2: Run the focused failing test**

Run:

```bash
bash scripts/cargo-agent.sh test -p cognit plan_mode_run_injects_marker -- --exact
```

Expected: FAIL because `step.rs:34` currently sends the raw input.

- [ ] **Step 3: Compose once at the production entry point**

Replace the raw push in `run()` with:

```rust
let user_message = if let Some(provider) = &self.dasein_ctx_provider {
    let dasein = provider();
    self.compose_user_message_with_dasein(user_input, dasein.as_deref())
} else {
    self.compose_user_message(user_input)
};
self.pending_memory.clear();
self.messages.push(Message::user(user_message));
```

Keep the marker in the user message, not the cache-stable system prefix. Do not weaken permission enforcement.

- [ ] **Step 4: Add regression cases**

Add exact tests for:

- plan mode on: one marker, before user text;
- plan mode off: no marker;
- a pending memory update is consumed once and absent on the following run;
- a Dasein provider is included on the live path;
- plan mode plus Dasein still emits exactly one marker.

- [ ] **Step 5: Run focused validation**

```bash
bash scripts/cargo-agent.sh test -p cognit plan_mode
bash scripts/cargo-agent.sh test -p cognit message_compose
```

Expected: all selected tests PASS.

- [ ] **Step 6: Stage review checkpoint**

Inspect `git diff -- crates/cognit/src/harness/linear/` and confirm there is no permission-policy change and no system-prefix mutation.

---

### Task 2: Add a typed model-capabilities query to the inference boundary

**Files:**
- Modify: `crates/executive/src/application/inference_port.rs:8-33`
- Modify: `crates/executive/src/host/core_rpc/protocol.rs` (the existing `CoreRequest`/response enums)
- Modify: `crates/executive/src/host/core_rpc/client.rs`
- Modify: `crates/executive/src/host/core_rpc/server.rs`
- Modify: `crates/executive/src/core/runtime_core.rs`
- Test: `crates/executive/tests/core_rpc_auth.rs`
- Test: `crates/executive/tests/inference_port_contract.rs`

- [ ] **Step 1: Add the wire-safe capability type**

Define beside `CoreInferenceRequest`:

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelCapabilities {
    pub model_spec: String,
    pub display_name: String,
    pub max_context_tokens: usize,
}
```

Extend `InferencePort` with:

```rust
async fn capabilities(&self, model_spec: &str) -> Result<ModelCapabilities, InferenceError>;
```

Do not provide a silent 128K default in the trait.

- [ ] **Step 2: Add failing local-port and RPC round-trip tests**

The local-port test must wrap a provider reporting `1_000_000` and assert `capabilities("")` returns `1_000_000`. The RPC test must use the existing authenticated fixture, request capabilities for an alias, and assert the core-resolved canonical model name and context limit survive serialization.

- [ ] **Step 3: Run the focused failing tests**

```bash
bash scripts/cargo-agent.sh test -p executive --test inference_port_contract
bash scripts/cargo-agent.sh test -p executive --test core_rpc_auth
```

Expected: compile/test failure because the new method and wire variants are absent.

- [ ] **Step 4: Implement core-owned capability resolution**

Add a core request/response variant equivalent to:

```rust
CoreRequest::Capabilities { model_spec: String }
CoreResponse::Capabilities(ModelCapabilities)
```

Resolve aliases through the same provider registry/router used by `complete` and `stream`; return `provider.name()` and `provider.max_context_length()`. Unknown model specs must return the same typed provider-selection error used by inference, never fall back to 128K.

- [ ] **Step 5: Implement both inference-port adapters**

- `LocalInferencePort::capabilities()` returns values from its provider.
- `CoreRpcClient::capabilities()` sends the authenticated capabilities request and validates the response variant.

- [ ] **Step 6: Re-run boundary tests**

```bash
bash scripts/cargo-agent.sh test -p executive --test inference_port_contract
bash scripts/cargo-agent.sh test -p executive --test core_rpc_auth
```

Expected: PASS, including an explicit `1_000_000` context value.

---

### Task 3: Construct `PortLlmProvider` from resolved capabilities

**Files:**
- Modify: `crates/executive/src/application/inference_port.rs:40-106`
- Modify: `crates/executive/src/composition/exec_session.rs:88-107`
- Modify: `crates/executive/src/host/daemon/model_router.rs:22-53`
- Modify: `crates/executive/src/host/daemon/bootstrap/inference.rs:9-22`
- Modify: `crates/executive/src/host/daemon/bootstrap/runtime.rs` at every `PortLlmProvider::new` call
- Modify: `crates/executive/src/host/daemon/bootstrap/agents.rs` at its provider construction
- Test: `crates/executive/src/host/daemon/bootstrap/inference.rs:24-52`
- Test: `crates/executive/src/host/daemon/bootstrap/runtime/runtime_tests.rs`

- [ ] **Step 1: Make provider construction explicit**

Change the constructor to accept resolved capabilities:

```rust
pub fn new(
    inference: Arc<dyn InferencePort>,
    capabilities: ModelCapabilities,
) -> Self {
    Self {
        inference,
        model_spec: capabilities.model_spec,
        display_name: capabilities.display_name,
        max_context: capabilities.max_context_tokens,
    }
}
```

Reject zero context tokens before construction with an `anyhow`/typed bootstrap error. Remove the `128_000` literal from this adapter.

- [ ] **Step 2: Add failing construction tests**

Update the inference composition test to supply `1_000_000`; assert `name()` and `max_context_length()` exactly match. Add a zero-context rejection test.

- [ ] **Step 3: Resolve capabilities before every construction**

Because capability lookup is async, make affected composition/router functions async where necessary. Each model selection—including task routing and model switching—must call:

```rust
let capabilities = inference.capabilities(&model_spec).await?;
let provider = PortLlmProvider::new(inference.clone(), capabilities)?;
```

Never reuse capabilities from a previously selected model.

- [ ] **Step 4: Assert the value reaches session budgeting**

Add a bootstrap/runtime test with a fake inference port reporting `1_000_000`; verify the composed provider and session/context budget use `1_000_000`, not `128_000`.

- [ ] **Step 5: Run focused validation**

```bash
bash scripts/cargo-agent.sh test -p executive inference_port
bash scripts/cargo-agent.sh test -p executive model_router
bash scripts/cargo-agent.sh test -p executive bootstrap::inference
bash scripts/cargo-agent.sh test -p executive runtime_tests
```

Expected: PASS and `rg -n 'max_context: 128_000' crates/executive/src/application/inference_port.rs` returns no matches.

---

### Task 4: Replace six-message replay with a token-budget history selector

**Files:**
- Modify: `crates/executive/src/application/daemon_turn/helpers.rs:10-45`
- Modify: `crates/executive/src/application/context_assembler.rs:173-209`
- Modify: `crates/executive/src/application/turn_pipeline.rs:444-460`
- Modify: `crates/executive/src/host/daemon/bootstrap/turn_runtime.rs:250-278`
- Test: `crates/executive/src/application/daemon_turn/helpers.rs:76-130`
- Test: `crates/executive/tests/context_assembler.rs`
- Test: `crates/executive/tests/turn_pipeline_order.rs`

- [ ] **Step 1: Replace the constant API with an explicit budget API**

Delete `MAX_HISTORY_MESSAGES`. Introduce:

```rust
pub(crate) fn select_text_history(
    history: &[Message],
    max_tokens: usize,
) -> Vec<Message>
```

Selection rules, in order:

1. Start at the newest canonical item and walk backward.
2. Include only complete conversational units.
3. Never include a `tool_result` without its preceding assistant `tool_use`; never split a multi-tool assistant/result group.
4. Exclude restored injected payloads using the current text-only filtering policy.
5. Stop before adding a unit that would exceed `max_tokens`.
6. Return selected messages in chronological order.
7. If the newest single user message exceeds the budget, include its UTF-8-safe truncated form so the current turn is never represented by an empty history selection.

- [ ] **Step 2: Add deterministic failing tests**

Add tests proving:

- 20 short alternating messages fit and are all selected when the token budget permits;
- selection stops by token cost, not message count;
- chronological order is preserved;
- tool-use/result groups are atomic;
- oversized UTF-8 content is safely truncated;
- a zero budget returns no prior history;
- no injected `<memory-context>`/`<skills>` payload is replayed as history.

- [ ] **Step 3: Run the focused failing tests**

```bash
bash scripts/cargo-agent.sh test -p executive daemon_turn::helpers
```

Expected: FAIL until the selector replaces `bounded_text_history`.

- [ ] **Step 4: Thread `history_budget` into context assembly**

Add `history_budget_tokens: usize` to the context assembly input/request contract. Use the already computed `ContextBudgetPlan.history_budget` from `turn_runtime.rs:269-278`; do not recompute a competing budget in `ContextAssembler`.

Change assembly to:

```rust
let history = select_text_history(canonical_history, request.history_budget_tokens);
```

Canonical SQLite history remains unchanged; only the model projection is bounded.

- [ ] **Step 5: Keep compaction and recall responsibilities separate**

The selector must not trigger compaction, write summaries, or query recall. Hard/soft compaction remains owned by `SessionManager` and `ContextBudgetPlanner`; recall remains a context source. Add an assertion/test that selection performs no persistence mutation.

- [ ] **Step 6: Run focused integration validation**

```bash
bash scripts/cargo-agent.sh test -p executive --test context_assembler
bash scripts/cargo-agent.sh test -p executive --test turn_pipeline_order
bash scripts/cargo-agent.sh test -p executive session_use_case_port
```

Expected: PASS with long-history fixtures and intact tool pairs.

---

### Task 5: Add observability needed to prove context correctness

**Files:**
- Modify: `crates/executive/src/host/daemon/bootstrap/request.rs:269-281`
- Modify: `crates/executive/src/host/daemon/bootstrap/turn_runtime.rs:250-278`
- Modify: `crates/executive/src/host/daemon/model_router.rs:45-53`
- Test: nearest existing bootstrap/runtime test modules

- [ ] **Step 1: Emit structured, non-secret fields**

At provider selection emit `model_spec`, resolved `display_name`, and `max_context_tokens`. At budget planning emit `history_budget`, `current_history_tokens`, `projected_history_tokens`, and budget action. Do not log prompts, credentials, or tool payloads.

- [ ] **Step 2: Add log-capture assertions**

Use the repository's existing tracing test pattern to assert a fake 1M provider produces `max_context_tokens=1000000` and a history budget derived from that window.

- [ ] **Step 3: Run focused tests**

```bash
bash scripts/cargo-agent.sh test -p executive context_window
bash scripts/cargo-agent.sh test -p executive budget_plan
```

Expected: PASS.

---

### Task 6: Deterministic repository validation

**Files:** none

- [ ] **Step 1: Format**

```bash
bash scripts/cargo-agent.sh fmt --all -- --check
```

Expected: PASS.

- [ ] **Step 2: Run narrow package checks**

```bash
bash scripts/cargo-agent.sh check -p cognit
bash scripts/cargo-agent.sh check -p executive
```

Expected: PASS.

- [ ] **Step 3: Run the focused regression set**

Run all commands from Tasks 1–5 again. Expected: PASS with no ignored test used as evidence.

- [ ] **Step 4: Inspect staged diff before any commit**

```bash
git diff --check
git diff --stat
git diff -- crates/cognit/src/harness/linear crates/executive/src/application crates/executive/src/host
```

Expected: no whitespace errors and no changes outside the declared paths except necessary protocol exports/tests.

---

### Task 7: Installed-runtime deployment acceptance

**Files:**
- Modify only if a gap is found: `scripts/lib/aletheon/runtime_gate.sh:46-143`
- Modify only if documentation is inaccurate: `docs/testing/production-scenarios.md`

- [ ] **Step 1: Capture pre-deploy restart counters**

```bash
systemctl show aletheon-core.service -p NRestarts --value
systemctl --user show aletheon.service -p NRestarts --value
```

Record both values in the implementation report.

- [ ] **Step 2: Deploy the reviewed source**

```bash
bash scripts/aletheon.sh deploy
```

Expected: build, installation, restart, runtime provenance verification, stability verification, and official-client smoke request all PASS. This is mandatory for completion under repository policy.

- [ ] **Step 3: Independently verify executable provenance**

```bash
candidate=$(sha256sum target/release/aletheon | awk '{print $1}')
installed=$(sha256sum /usr/bin/aletheon | awk '{print $1}')
core=$(sha256sum "/proc/$(systemctl show aletheon-core.service -p MainPID --value)/exe" | awk '{print $1}')
user=$(sha256sum "/proc/$(systemctl --user show aletheon.service -p MainPID --value)/exe" | awk '{print $1}')
printf 'candidate=%s\ninstalled=%s\ncore=%s\nuser=%s\n' "$candidate" "$installed" "$core" "$user"
test "$candidate" = "$installed" && test "$candidate" = "$core" && test "$candidate" = "$user"
```

Expected: all four SHA-256 values are identical.

- [ ] **Step 4: Prove restart counters remain stable**

Capture the two counters, wait the runtime gate's configured stability interval, capture again, and assert equality. Expected: core and user values do not increase.

- [ ] **Step 5: Make a real request through the official client/socket**

```bash
timeout 120 /usr/bin/aletheon \
  --socket "${XDG_RUNTIME_DIR}/aletheon/aletheon.sock" \
  -m 'Reply with exactly: installed-runtime-ok'
```

Expected: non-empty model response containing `installed-runtime-ok`. A direct provider request, alternate socket, or temporary daemon does not satisfy this step.

- [ ] **Step 6: Exercise Plan Mode and long context in the installed runtime**

Use the official TUI/socket to verify: Plan Mode produces no mutation attempts after the model sees the marker; the effective context log reports the selected model's configured limit; a conversation longer than six messages retains an early unique fact while remaining under the computed history budget.

- [ ] **Step 7: Final acceptance record**

Report exact commands, hashes, restart counters, selected model/context limit, and real-request result. Do not claim completion if `deploy` or any installed-runtime assertion fails.

---

## Evidence-gated backlog (separate future plans only)

No item below may enter implementation merely because another agent product supports it.

| Candidate | Evidence required before a plan is opened | Minimum acceptance signal |
|---|---|---|
| TUI footer cost/cache metrics | Installed runtime exposes trustworthy per-turn input/output/cache-read/cache-write/cost fields | Three real turns reconcile TUI totals with provider usage events |
| Session naming | Operator test demonstrates UUID-only selection causes wrong-session choice or measurable delay | Named session persists across daemon restart and appears in list/resume UI |
| Provider reasoning parameters | A supported configured provider/model demonstrably ignores needed reasoning control | Typed per-provider capability validation plus successful installed request |
| Agora workspace persistence | Restart test proves required active workspace state is lost and not reconstructible from canonical stores | State survives daemon restart without duplicating canonical truth |
| CoreMemory prompt injection | Task benchmark proves tool-only retrieval misses relevant stable persona/preference facts | Bounded, attributed injection improves benchmark without leaking other principals |
| `workflow.run` | A concrete workflow is selected as a supported user-facing contract | RPC has durable lifecycle, cancellation, restart recovery, and installed-runtime test |

When a gate passes, first write a dedicated design and implementation plan with fresh `path:line` anchors. Do not append speculative implementation steps here.

## Completion definition

This plan is complete only when Tasks 1–7 pass. Evidence-gated backlog items and
the selective-absorption companion plan have independent completion gates.
