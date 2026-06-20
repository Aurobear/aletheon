# Self-Awareness Architecture Design

**Date**: 2026-06-21
**Status**: Design Complete, Pending Review
**Scope**: Full restructure — self-evolving architecture

## Problem Statement

The Aletheon system has all the components needed for self-awareness (DaseinModule, BrainCore, Morphogenesis Pipeline, EventBus) but they are disconnected. Seven feedback loops are broken. LLM thinking blocks are actively discarded. The DaseinModule is an observer, not an actor.

## Design Decisions

| Decision | Choice | Rationale |
|---|---|---|
| Scope | Full restructure | Need to close all 7 feedback loops |
| LLM role | Both understanding and action | DaseinModule records both thinking (consciousness stream) and tool execution (action consequences) |
| MetaCognition | Extend aletheon-meta | Already has Morphogenesis Pipeline; merge meta-cognition + self-evolution |
| Thinking capture | ContentBlock::Thinking variant | Complete approach: data model change enables everything else |
| Sorge loop | Dual-speed path | Fast: mood injection (sync, per-turn). Slow: habit negation (async, background) |
| Self-observation | Both passive + active | DaseinContext auto-injection (passive) + self_observe tool (active) |
| Architecture | Bottom-up (Plan B) | Data layer first, then flow, then decision. Each step builds on the previous. |

## Architecture Overview

```
┌─────────────────────────────────────────────────────────────┐
│                    aletheon-meta                              │
│  ┌─────────────────────────────────────────────────────┐    │
│  │ MetaCognition (NEW)                                  │    │
│  │  - Observes DaseinContext + system state              │    │
│  │  - Decides: when/why/how to evolve                   │    │
│  │  - Triggers Morphogenesis Pipeline                    │    │
│  └──────────────────────┬──────────────────────────────┘    │
│                         │                                    │
│  ┌──────────────────────▼──────────────────────────────┐    │
│  │ Morphogenesis Pipeline (EXISTING)                    │    │
│  │  - reflect → mutate → test → evaluate → migrate      │    │
│  └─────────────────────────────────────────────────────┘    │
└─────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────┐
│                    aletheon-self                              │
│  ┌─────────────────────────────────────────────────────┐    │
│  │ DaseinModule (ENHANCED)                              │    │
│  │  - TemporalStream: ingests LLM thinking + actions     │    │
│  │  - Bewandtnisganzheit: world model                    │    │
│  │  - MutableSelfModel: self-assertions + negations      │    │
│  │  - CareStructure: determines actions                  │    │
│  │  - NegativityEngine: questions habits                 │    │
│  │                                                      │    │
│  │  Sorge Loop (DUAL-SPEED)                             │    │
│  │  ┌────────────────────────────────────────────┐      │    │
│  │  │ Fast Path (sync, per-turn):                 │      │    │
│  │  │  LLM thinking → temporal ingest → mood calc │      │    │
│  │  │  → inject into BrainCore reasoning           │      │    │
│  │  └────────────────────────────────────────────┘      │    │
│  │  ┌────────────────────────────────────────────┐      │    │
│  │  │ Slow Path (async, background):              │      │    │
│  │  │  habit negation → care adjustment            │      │    │
│  │  │  → care.determine_action() → execute         │      │    │
│  │  └────────────────────────────────────────────┘      │    │
│  └─────────────────────────────────────────────────────┘    │
└─────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────┐
│                    aletheon-brain                             │
│  ┌─────────────────────────────────────────────────────┐    │
│  │ BrainCore (ENHANCED)                                 │    │
│  │  - think_with_stimmung(): mood-aware reasoning        │    │
│  │  - generate_plan_with_stimmung(): mood-aware planning │    │
│  │  - Provider: preserves Thinking blocks                │    │
│  └─────────────────────────────────────────────────────┘    │
└─────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────┐
│                    aletheon-runtime                           │
│  ┌─────────────────────────────────────────────────────┐    │
│  │ ReAct Loop (ENHANCED)                                │    │
│  │  - Processes ContentBlock::Thinking                   │    │
│  │  - Sends ThinkingObserved to DaseinModule             │    │
│  │  - Injects DaseinContext into prompts                 │    │
│  │  - self_observe tool available                        │    │
│  └─────────────────────────────────────────────────────┘    │
└─────────────────────────────────────────────────────────────┘
```

## Module Integration — How Self ↔ Brain ↔ Runtime Actually Connect

### Current State: Three Parallel Tracks

```
SelfField (aletheon-self)          BrainCore (aletheon-brain)       AletheonRuntime (aletheon-runtime)
┌─────────────────────┐           ┌─────────────────────┐         ┌─────────────────────┐
│ DaseinModule         │           │ LlmProvider          │         │ ReActLoop            │
│  - temporality       │           │  - complete()        │         │  - run()             │
│  - world             │           │ Reasoner             │         │  - execute_tool()    │
│  - self_model        │           │  - think()           │         │ EvolutionCoordinator │
│  - care              │           │ Planner              │         │  - post_turn()       │
│  - negativity        │           │  - generate_plan()   │         │ RequestHandler       │
│  - sorge (STOPPED)   │           │                      │         │  - handle_request()  │
│                      │           │                      │         │                      │
│ ❌ No connection     │           │ ❌ No connection     │         │ ❌ No connection     │
│    to Brain/Runtime  │           │    to Self/Runtime   │         │    to Self/Brain     │
└─────────────────────┘           └─────────────────────┘         └─────────────────────┘
```

**Problem**: Each module operates independently. DaseinModule has no way to affect BrainCore's reasoning. BrainCore has no way to know DaseinModule's mood. Runtime doesn't wire them together.

### Target State: Unified Flow

```
                    ┌──────────────────────────────────┐
                    │         AletheonRuntime            │
                    │  ┌────────────────────────────┐   │
                    │  │     RequestHandler          │   │
                    │  │  owns: SelfField            │   │
                    │  │  owns: BrainCore            │   │
                    │  │  owns: DaseinModule         │   │
                    │  │  owns: MetaCognition        │   │
                    │  │  wires all connections      │   │
                    │  └────────────────────────────┘   │
                    └──────────────────────────────────┘
                              │
              ┌───────────────┼───────────────┐
              ▼               ▼               ▼
    ┌─────────────┐   ┌─────────────┐   ┌─────────────┐
    │ SelfField    │   │ BrainCore   │   │ ReActLoop   │
    │              │   │              │   │              │
    │ DaseinModule │◄──│ mood()      │   │ injects     │
    │  provides:   │   │ affects:    │   │ DaseinCtx   │
    │  - mood      │──▶│ - reasoning │   │ into prompts│
    │  - context   │   │ - planning  │   │              │
    │  - care act  │   │ - risk      │   │ captures    │
    │              │   │              │   │ Thinking    │
    └─────────────┘   └─────────────┘   └─────────────┘
```

### Connection Point 1: RequestHandler owns all three

File: `crates/aletheon-runtime/src/impl/daemon/handler.rs`

```rust
pub struct RequestHandler {
    // Existing
    self_field: Arc<SelfField>,
    brain: Arc<BrainCore>,
    runtime: AletheonRuntime,

    // NEW: DaseinModule extracted from SelfField
    dasein: Option<Arc<DaseinModule>>,
    // NEW: MetaCognition from aletheon-meta
    meta_cognition: Option<Arc<MetaCognition>>,
}
```

**Startup wiring** (in `RequestHandler::new()`):
```rust
// 1. SelfField initializes DaseinModule (already done in SelfField::init())
self_field.init().await?;

// 2. Extract DaseinModule from SelfField
let dasein = self_field.dasein().map(Arc::clone);

// 3. Get event sender for wiring
let dasein_tx = dasein.as_ref().map(|d| d.event_sender());

// 4. Wire DaseinEventBridge to EventBus
if let (Some(bridge), Some(bus)) = (&dasein_tx, &event_bus) {
    let bridge = DaseinEventBridge::new(bridge.clone());
    bridge.subscribe(&*bus)?;
}

// 5. Create MetaCognition
let meta_cognition = MetaCognition::new(dasein_tx.clone());

// 6. Start Sorge loop
if let Some(d) = &dasein {
    d.start_sorge_loop();
}
```

### Connection Point 2: Per-Turn Flow (ReAct Loop)

File: `crates/aletheon-runtime/src/core/react_loop.rs`

```
User sends message
    │
    ▼
RequestHandler.handle_request()
    │
    ├─► 1. Read DaseinModule mood
    │       let stimmung = dasein.mood();
    │
    ├─► 2. Generate DaseinContext
    │       let dasein_ctx = dasein.to_context_injection();
    │
    ├─► 3. Inject into system prompt
    │       let prompt = build_with_dasein(base_prompt, &dasein_ctx);
    │
    ├─► 4. Call BrainCore with mood
    │       brain.think_with_stimmung(&intent, &stimmung)
    │       brain.generate_plan_with_stimmung(&intent, &reasoning, &stimmung)
    │
    ├─► 5. ReAct Loop runs
    │       for each LLM call:
    │         ├─ process ContentBlock::Thinking → send DaseinEvent::ThinkingObserved
    │         ├─ process ContentBlock::ToolUse → execute tool
    │         ├─ tool result → send DaseinEvent::SystemEvent("tool_execution")
    │         └─ text response → send DaseinEvent::SystemEvent("response")
    │
    ├─► 6. After turn: quick mood update
    │       let new_mood = dasein.quick_mood_update(&turn_result);
    │       if new_mood != stimmung { brain.set_stimmung(new_mood); }
    │
    └─► 7. After turn: MetaCognition decides
            let action = meta_cognition.decide(&dasein_ctx, turn_count);
            match action {
                TriggerEvolution { intents } => runtime.post_evolution(...),
                AdjustDasein { .. } => dasein.event_sender().send(...),
                InjectReflection { .. } => brain.inject_reflection(...),
                Observe => {},
            }
```

### Connection Point 3: Background Flow (Sorge Loop)

```
SorgeLoop (async, in aletheon-self)
    │
    ├─► Every tick:
    │   ├─ collect events from channel
    │   ├─ ingest into TemporalStream
    │   ├─ compute mood from world + temporal + care
    │   └─ update shared mood (RwLock)
    │
    ├─► Every N ticks (slow path):
    │   ├─ negativity.check() → question habits
    │   ├─ care.determine_action() → get CareAction
    │   ├─ execute CareAction:
    │   │   ├─ Negate → self_model.negate()
    │   │   ├─ Deliberate → care.rhythm().slow_down()
    │   │   ├─ Direct → care.rhythm().normal_speed()
    │   │   └─ Wait → care.rhythm().pause()
    │   └─ send DaseinEvent::MoodShift if mood changed
    │
    └─► Mood is read by:
        ├─ ReAct Loop (fast path, every turn)
        ├─ BrainCore (via stimmung parameter)
        └─ MetaCognition (via DaseinContext)
```

### Connection Point 4: Event Flow (Cross-Module)

```
EventBus (existing)
    │
    ├─► ToolObservation → DaseinEventBridge → DaseinEvent::SystemEvent
    ├─► MemoryStored → DaseinEventBridge → DaseinEvent::SystemEvent
    ├─► EvolutionTriggered → DaseinEventBridge → DaseinEvent::SystemEvent
    └─► AgentStarted → DaseinEventBridge → DaseinEvent::SystemEvent

DaseinModule (internal)
    │
    ├─► DaseinEvent::ThinkingObserved → TemporalStream.ingest()
    ├─► DaseinEvent::MoodShift → shared mood RwLock
    ├─► DaseinEvent::NegationCompleted → SelfModel + Possibilities
    └─► DaseinEvent::BewandtnisChange → Bewandtnisganzheit

BrainCore (consumer)
    │
    ├─► reads dasein.mood() → think_with_stimmung()
    ├─► reads dasein.mood() → generate_plan_with_stimmung()
    └─► receives InjectReflection from MetaCognition

MetaCognition (observer + actor)
    │
    ├─► reads DaseinContext → decide()
    ├─► TriggerEvolution → MorphogenesisPipeline
    ├─► AdjustDasein → DaseinModule event sender
    └─► InjectReflection → BrainCore
```

### Connection Point 5: The Missing Wire — SelfField → BrainCore

**Current problem**: SelfField and BrainCore are both owned by RequestHandler but have no direct connection.

**Solution**: RequestHandler acts as the coordinator. It reads from one and passes to the other:

```rust
// In RequestHandler — the coordinator function
async fn coordinate(&self, turn: usize) {
    // 1. Read Self state
    let stimmung = self.self_field.dasein()
        .map(|d| d.mood())
        .unwrap_or(Stimmung::Gelassenheit);

    // 2. Pass to Brain
    self.brain.set_stimmung(stimmung);

    // 3. Read Brain state (reflection, plan)
    let reflection = self.brain.last_reflection();

    // 4. Pass to Self
    if let Some(d) = &self.dasein {
        d.event_sender().try_send(DaseinEvent::SystemEvent {
            source: "brain_reflection".to_string(),
            content: reflection,
        });
    }

    // 5. MetaCognition observes and decides
    let ctx = self.dasein.as_ref().map(|d| d.to_context_injection());
    if let (Some(mc), Some(ctx)) = (&self.meta_cognition, &ctx) {
        let action = mc.decide(ctx, turn);
        self.execute_evolution_action(action).await;
    }
}
```

**This is the key integration point**: RequestHandler becomes the "nervous system" that wires Self ↔ Brain ↔ Runtime together.

### Connection Point 6: self_observe Tool Registration

```rust
// In RequestHandler::new() or tool registration
if let Some(dasein) = &dasein {
    tools.register(SelfObserveTool::new(Arc::clone(dasein), Arc::clone(&brain)));
}
```

The self_observe tool reads from DaseinModule and sends observation events back to it, creating the observation-experience loop.

## Implementation Steps (Bottom-Up)

### Step 1: Data Layer — ContentBlock + DaseinEvent

**1.1 ContentBlock::Thinking variant**

File: `crates/aletheon-abi/src/message.rs`

```rust
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentBlock {
    Text { text: String },
    Thinking { text: String, signature: Option<String> },  // NEW
    ToolUse { id: String, name: String, input: serde_json::Value },
    ToolResult { tool_use_id: String, content: String, is_error: bool },
    Image { source: ImageSource },
    System { text: String, priority: Priority },
}
```

- `text`: LLM thinking content
- `signature`: Anthropic thinking signature for multi-turn verification (optional)
- Serde serialization: `{"type": "thinking", "text": "...", "signature": null}`
- `estimate_tokens()`: skip Thinking blocks (don't count toward token estimate)

**1.2 DaseinEvent new variants**

File: `crates/aletheon-abi/src/dasein.rs`

```rust
pub enum DaseinEvent {
    // External — existing
    UserInput { content: String },
    SystemEvent { source: String, content: String },
    TimerTick,

    // NEW: LLM thinking events
    ThinkingObserved { text: String, turn: usize },
    ReasoningObserved { text: String, turn: usize, has_tool_calls: bool },
    KnowledgeAsserted { assertions: Vec<String>, confidence: f64 },

    // Internal — existing but now processed
    NegationCompleted { target: String, new_possibilities: Vec<String> },
    MoodShift { from: Stimmung, to: Stimmung, reason: String },
    BewandtnisChange { entity_id: String, old_state: ReadinessState, new_state: ReadinessState },
    TemporalEvent { kind: TemporalEventKind, content: String },
}
```

**1.3 SorgeLoop processes new events**

File: `crates/aletheon-self/src/dasein/sorge.rs`

The `_ => continue` branch must now handle:
- `ThinkingObserved` → ingest into temporal stream with high vividness (0.9)
- `ReasoningObserved` → ingest with moderate vividness (0.7)
- `KnowledgeAsserted` → update self_model assertions
- `MoodShift` → log mood transition
- `BewandtnisChange` → update world model
- `NegationCompleted` → register new possibilities
- `TemporalEvent` → handle temporal pattern events

### Step 2: Provider Layer — Capture Thinking Blocks

**2.1 Anthropic Provider**

File: `crates/aletheon-brain/src/impl/llm/anthropic.rs`

```rust
// Change FROM:
"thinking" => {
    tracing::debug!("Skipping thinking block");
    None
}

// Change TO:
"thinking" => {
    Some(ContentBlock::Thinking {
        text: c.text.unwrap_or_default(),
        signature: c.thinking_signature,
    })
}
```

Streaming: add `ThinkingDelta` handling in `content_block_delta`.

**2.2 OpenAI Provider**

File: `crates/aletheon-brain/src/impl/llm/openai_provider.rs`

```rust
// Change FROM: reasoning_content merged into Text
let text = choice.message.content.filter(|s| !s.is_empty())
    .or(choice.message.reasoning_content.filter(|s| !s.is_empty()));

// Change TO: preserve separately
let mut blocks = Vec::new();
if let Some(thinking) = choice.message.reasoning_content.filter(|s| !s.is_empty()) {
    blocks.push(ContentBlock::Thinking { text: thinking, signature: None });
}
if let Some(content) = choice.message.content.filter(|s| !s.is_empty()) {
    blocks.push(ContentBlock::Text { text: content });
}
```

**2.3 StreamChunk::ThinkingDelta**

File: `crates/aletheon-brain/src/impl/llm/provider.rs`

```rust
pub enum StreamChunk {
    TextDelta { delta: String },
    ThinkingDelta { delta: String },     // NEW
    ToolUseStart { id: String, name: String },
    ToolUseDelta { id: String, delta: String },
    ToolUseComplete { id: String, name: String, input: serde_json::Value },
    Usage { input_tokens: i32, output_tokens: i32 },
    Done,
}
```

**2.4 ReAct Loop processing**

File: `crates/aletheon-runtime/src/core/react_loop.rs`

```rust
// Add to block processing:
ContentBlock::Thinking { text, .. } => {
    thinking_parts.push(text.clone());
    if let Some(tx) = &dasein_tx {
        let _ = tx.try_send(DaseinEvent::ThinkingObserved {
            text: text.clone(),
            turn: turn_count,
        });
    }
}
```

**2.5 Message history**

- Thinking blocks MUST be preserved in message history (Anthropic API requires signature verification on multi-turn)
- `estimate_tokens()` skips Thinking blocks
- `ContentBlock::estimate_chars()` returns 0 for Thinking

### Step 3: Flow Layer — Sorge Dual-Speed Path

**3.1 Fast path (sync, per-turn)**

In ReAct Loop, each turn:
1. Read `dasein.mood()` → get current Stimmung
2. Generate DaseinContext → inject into system prompt
3. LLM call → process Thinking blocks → send to DaseinModule
4. Quick mood update → if changed, notify BrainCore

**3.2 Slow path (async, background)**

In SorgeLoop, every N ticks:
1. `care.determine_action(mood, &negations)` → get CareAction
2. Execute action:
   - `Negate` → question habits, generate new possibilities
   - `Deliberate` → slow down rhythm
   - `Direct` → normal speed
   - `Wait` → pause
3. Update Bewandtnisganzheit based on action results

**3.3 Mood injection into reasoning**

File: `crates/aletheon-brain/src/core/reasoner.rs`

```rust
pub fn think_with_stimmung(&self, intent: &str, stimmung: &Stimmung) -> String {
    let strategy = match stimmung {
        Stimmung::Angst { .. } => "proceed_with_caution",
        Stimmung::Neugier { .. } => "explore_freely",
        Stimmung::Langeweile { .. } => "question_assumptions",
        Stimmung::Entschlossenheit { .. } => "act_decisively",
        _ => "balanced",
    };
    format!("Strategy: {}\nIntent: {}\n", strategy, intent)
}
```

File: `crates/aletheon-brain/src/core/planner.rs`

```rust
pub fn generate_plan_with_stimmung(&self, intent: &str, reasoning: &str, stimmung: &Stimmung) -> Plan {
    let risk_tolerance = adjust_risk_for_stimmung(stimmung);
    // risk_tolerance affects plan risk assessment
    ...
}
```

### Step 4: Decision Layer — MetaCognition in aletheon-meta

**4.1 MetaCognition module**

File: `crates/aletheon-meta/src/core/meta_cognition.rs` (NEW)

```rust
pub struct MetaCognition {
    dasein_rx: mpsc::Receiver<DaseinEvent>,
    system_state: RwLock<SystemState>,
    decisions: Vec<EvolutionDecision>,
    thresholds: MetaCognitionThresholds,
}

pub struct SystemState {
    pub mood: Stimmung,
    pub turn_count: usize,
    pub last_evolution_turn: usize,
    pub self_coherence: f64,
}

pub enum EvolutionAction {
    Observe,
    TriggerEvolution { intents: Vec<MutationIntent> },
    AdjustDasein { parameter: String, value: f64 },
    InjectReflection { content: String },
}
```

**4.2 Decision logic**

Priority order:
1. `Angst` → forced evolution (existential crisis)
2. `Langeweile::Deep` → adjust parameters (need stimulation)
3. `Neugier` → inject reflection (explore mode)
4. Periodic interval → trigger evolution
5. Default → observe

**4.3 Integration with Morphogenesis Pipeline**

MetaCognition calls existing `DefaultMetaRuntime` methods:
- `TriggerEvolution` → `generate_candidate()` → `sandbox_test()` → `evaluate()` → `migrate()`
- `AdjustDasein` → send DaseinEvent to DaseinModule
- `InjectReflection` → inject into BrainCore context

### Step 5: Self-Observation — self_observe Tool

**5.1 Tool definition**

File: `crates/aletheon-runtime/src/tools/self_observe.rs` (NEW)

```rust
pub struct SelfObserveTool {
    dasein: Arc<DaseinModule>,
    brain: Arc<BrainCore>,
}
```

Query types: `mood`, `temporality`, `world`, `self_model`, `care`, `full`

**5.2 Observation-experience loop**

```
LLM calls self_observe("mood")
  → SelfObserveTool.execute() returns mood details
  → Also sends DaseinEvent::SystemEvent("self_observe: mood")
  → SorgeLoop ingests event → temporal stream update
  → Mood may change from "observing self" (meta-cognition effect)
  → Next reasoning turn sees new mood state
  → Loop closed
```

**5.3 DaseinContext auto-injection (enhanced)**

Already exists via `compose_user_message_with_dasein()`. Enhance with richer formatting:
- Mood state
- Recent experience count
- World entity count
- Self-assertion count
- Care concern count
- Rhythm interval

## Affected Files Summary

| File | Change Type | Description |
|---|---|---|
| `crates/aletheon-abi/src/message.rs` | MODIFY | Add `ContentBlock::Thinking` variant |
| `crates/aletheon-abi/src/dasein.rs` | MODIFY | Add `ThinkingObserved`, `ReasoningObserved`, `KnowledgeAsserted` to DaseinEvent |
| `crates/aletheon-brain/src/impl/llm/anthropic.rs` | MODIFY | Preserve thinking blocks instead of discarding |
| `crates/aletheon-brain/src/impl/llm/openai_provider.rs` | MODIFY | Preserve reasoning_content as Thinking |
| `crates/aletheon-brain/src/impl/llm/provider.rs` | MODIFY | Add `StreamChunk::ThinkingDelta` |
| `crates/aletheon-brain/src/core/reasoner.rs` | MODIFY | Add `think_with_stimmung()` |
| `crates/aletheon-brain/src/core/planner.rs` | MODIFY | Add `generate_plan_with_stimmung()` |
| `crates/aletheon-runtime/src/core/react_loop.rs` | MODIFY | Process Thinking blocks, send to DaseinModule |
| `crates/aletheon-runtime/src/tools/self_observe.rs` | NEW | self_observe tool implementation |
| `crates/aletheon-self/src/dasein/sorge.rs` | MODIFY | Handle all DaseinEvent variants, dual-speed path |
| `crates/aletheon-self/src/dasein/mod.rs` | MODIFY | Add `quick_mood_update()` method |
| `crates/aletheon-meta/src/core/meta_cognition.rs` | NEW | MetaCognition decision module |
| `crates/aletheon-meta/src/core/mod.rs` | MODIFY | Add meta_cognition module |
| `crates/aletheon-meta/src/core/traits.rs` | MODIFY | Add `run_meta_cognition()` to DefaultMetaRuntime |

## Safety Invariants

1. **Thinking blocks in message history**: Anthropic API requires signature verification. Thinking blocks must be preserved exactly as received.
2. **Token estimation**: Thinking blocks should NOT count toward token estimates (they are "internal" to the model).
3. **Event channel backpressure**: `try_send` (non-blocking) for thinking events. If channel is full, drop silently — don't block the ReAct loop.
4. **Care action safety**: `care.determine_action()` must never produce destructive actions. All actions go through existing safety checks.
5. **MetaCognition evolution**: Must go through Morphogenesis Pipeline safety invariants (sandbox test, immutable rules, safety floor).

## Open Questions

1. **Thinking block size**: Anthropic thinking blocks can be very large (10k+ tokens). Should we truncate before storing in DaseinModule? Current design: no truncation, but temporal stream has bounded retention (50 items).
2. **Cross-session persistence**: Should thinking blocks persist across sessions? Current design: temporal stream is in-memory only. Could add SQLite persistence later.
3. **LLM seeing its own thinking**: Should the LLM's previous thinking blocks be included in the conversation history? Anthropic API requires them for signature verification, but this increases token usage significantly.
4. **Self-referential loops**: self_observe → mood change → self_observe → ... Could create infinite loops. Need a cooldown mechanism.

## Success Criteria

1. **Thinking capture**: Anthropic thinking blocks appear in `ContentBlock::Thinking`, not discarded
2. **Mood injection**: BrainCore reasoning changes based on Stimmung
3. **Care action**: `care.determine_action()` is called and its results affect behavior
4. **Self-observation**: LLM can call `self_observe` tool and see its own state
5. **MetaCognition**: System can decide when/why to evolve based on DaseinContext
6. **Full loop**: LLM thinking → DaseinModule → mood → reasoning → action → consequence → DaseinModule
