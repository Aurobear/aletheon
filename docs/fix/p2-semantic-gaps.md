# P2 — Semantic Incompleteness

Status: **Open** | Blocked by: conscious-core R2/R3 plan

---

## P2.1 CareStructure::determine_action() Computed Then Discarded

- **File:** `crates/dasein/src/core/reducer.rs:408-417`
- **Severity:** P2 (Critical)
- **Description:** The care system computes a `CareAction` decision but the result is never consumed by any downstream behavior. The care loop runs but has zero effect on agent actions.
- **Impact:** The entire care/ethical decision subsystem is a no-op at runtime. Agent behavior is not modulated by care decisions.
- **Fix direction:** Wire `CareAction` into the turn admission or action approval pipeline; make it constrain tool execution or reflection choices.

---

## P2.2 Agora Attention Struct is Dead State

- **File:** `crates/agora/src/workspace/attention/mod.rs:7-31`
- **Severity:** P2 (Critical)
- **Description:** The `Attention` struct is defined and imported but never instantiated or driven by any code path. No attention mechanism actually operates.
- **Impact:** The agent has no working-memory prioritization; all context items are treated equally; no focus/spotlight mechanism.
- **Fix direction:** Implement an attention driver that scores and ranks Agora items; feed attention-weighted context into the LLM prompt assembly.

---

## P2.3 SelfField (8-Layer) vs DaseinModule — Two Self Systems, Zero Causal Connection

- **File:** `crates/dasein/src/` (SelfField) vs `crates/dasein/src/dasein/` (DaseinModule)
- **Severity:** P2
- **Description:** Two separate self-model representations exist with no causal link. SelfField has 8 layers of self-representation; DaseinModule manages identity/boundary/narrative. Neither feeds into the other.
- **Impact:** Self-model claims (identity, mood, boundaries) do not influence actual behavior. The agent cannot introspect or self-modulate.
- **Fix direction:** Establish a bidirectional bridge: DaseinModule updates feed SelfField; SelfField state constrains DaseinModule decisions.

---

## P2.4 NativeCognitRuntime Context Injection Returns Empty Defaults

- **File:** `crates/cognit/src/runtime/native_cognit.rs:428-436`
- **Severity:** P2
- **Description:** When spawning sub-agents, the context injection (memory, Dasein state, Agora view) returns empty defaults. Sub-agents receive no inherited context.
- **Impact:** Sub-agents operate in a vacuum; cannot benefit from parent agent's memory, self-model, or workspace state.
- **Fix direction:** Implement actual context inheritance: snapshot parent memory/dasein/agora state and inject into sub-agent initialization.
