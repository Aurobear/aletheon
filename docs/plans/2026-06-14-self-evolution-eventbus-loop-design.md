# Self-Evolution EventBus Loop Design

> EventBus-driven closed loop: Experience → Reflection → Learning → Evolution → Morphogenesis

## Problem

The self-evolution pipeline has three structural breaks that prevent it from working end-to-end:

1. **Engine ↔ BrainCore disconnected**: `Engine` (in `aletheon-runtime`) uses `aletheon_brain::impl::learning` (OutcomeRecorder/PatternExtractor/RuleStore) but does NOT use `core::` modules (Reflector/Learner/EvolutionTrigger). Two parallel architectures exist.

2. **Engine ↔ MetaRuntime disconnected**: Learning outcomes never flow to the Morphogenesis Pipeline. `MutationIntentGenerator` uses keyword heuristics (`contains("fail")`), not LLM-driven analysis.

3. **Direct coupling**: The initial design had Engine directly calling LearningBridge → MetaPipeline, violating the EventBus architecture defined in `docs/arch.md` §7.3.

## Solution: EventBus-Driven Closed Loop

All inter-module communication flows through EventBus. Components subscribe to events, process them, and emit new events. Zero direct calls between SelfField/BrainCore/BodyRuntime/MetaRuntime.

### Architecture

```text
User Input
    ↓
Runtime → EventBus ← UserIntentEvent
    ↓
EngineHook (subscribe) → Engine.run_turn()
    ↓ tool execution completes
    ↓ emits ToolObservationEvent
    ↓
EventBus ← ToolObservationEvent
    ↓ parallel dispatch
    ├→ ToolObservationHandler (BrainCore)
    │   → LLM reflect → LlmRequestEvent → LlmScheduler → DeepSeek
    │   → ReflectionEvent → EventBus
    │   → RuleExtractedEvent → EventBus (when batch ready)
    │   → EvolutionTriggeredEvent → EventBus (when threshold met)
    │
    ├→ Memory (persist to EpisodicMemory)
    │
    ↓
EventBus ← EvolutionTriggeredEvent
    ↓
MutationApprover (SelfField)
    → validate boundary / identity
    → MutationIntentEvent → EventBus
    ↓
MutationExecutor (MetaRuntime)
    → Morphogenesis Pipeline
    → EvolutionResultEvent → EventBus
    ↓
Memory (persist to SelfMemory) + LineageRecorder
```

### Principle

- `aletheon-brain` handles "thinking" (LLM calls for reflection/learning)
- `aletheon-meta` handles "mutation" (code-level morphogenesis)
- `aletheon-self` handles "approval" (boundary/identity validation)
- `aletheon-runtime` handles "lifecycle" (session, daemon, config)
- `aletheon-comm` provides EventBus (the nervous system)
- `aletheon-abi` defines event types (shared vocabulary)
- All modules depend only on `aletheon-abi` + `aletheon-comm`

---

## Part 1: Event Type Definitions

Location: `crates/aletheon-abi/src/events/`

New event types for the self-evolution loop:

### ToolObservationEvent

Emitted by Engine after a tool call completes.

```rust
pub struct ToolObservationEvent {
    pub turn_id: Uuid,
    pub tool_name: String,
    pub input: serde_json::Value,
    pub output: serde_json::Value,
    pub duration_ms: u64,
    pub error: Option<String>,
    pub rules_applied: Vec<LearnedRule>,
}
```

### ReflectionEvent

Emitted by BrainCore's ToolObservationHandler after LLM reflection.

```rust
pub struct ReflectionEvent {
    pub turn_id: Uuid,
    pub assessment: Assessment,        // Success | PartialSuccess | Failure
    pub root_cause: Option<String>,    // LLM-generated
    pub suggested_rule: Option<LearnedRule>,
    pub confidence: f64,
}
```

### RuleExtractedEvent

Emitted when BrainCore accumulates enough reflections to extract generalized rules.

```rust
pub struct RuleExtractedEvent {
    pub rules: Vec<LearnedRule>,
    pub source_reflections: Vec<Uuid>,
}
```

### EvolutionTriggeredEvent

Emitted when BrainCore detects evolution conditions are met.

```rust
pub struct EvolutionTriggeredEvent {
    pub trigger_reason: String,  // "consecutive_failures" | "confidence_drop" | "periodic"
    pub recent_reflections: Vec<Uuid>,
    pub current_rules_snapshot: Vec<LearnedRule>,
}
```

### MutationIntentEvent

Emitted by SelfField's MutationApprover after validating mutation intents.

```rust
pub struct MutationIntentEvent {
    pub intents: Vec<MutationIntent>,
    pub approved_by: String,
}
```

### EvolutionResultEvent

Emitted by MetaRuntime after Morphogenesis Pipeline completes.

```rust
pub struct EvolutionResultEvent {
    pub recommendation: Recommendation,  // Adopt | PartialAdopt | Reject
    pub genome_version_before: String,
    pub genome_version_after: Option<String>,
    pub summary: String,
}
```

### LlmRequestEvent / LlmResponseEvent

For LLM Scheduler routing. Uses `request_id` correlation — the requester registers a oneshot receiver locally before emitting the request. The Scheduler sends the response back as an `LlmResponseEvent` with the same `request_id`. The requester's local receiver resolves it.

```rust
pub struct LlmRequestEvent {
    pub request_id: Uuid,
    pub purpose: LlmPurpose,           // Reflect | ExtractRules | GenerateMutations | Execute
    pub messages: Vec<Message>,
}

pub struct LlmResponseEvent {
    pub request_id: Uuid,
    pub content: String,
    pub usage: TokenUsage,
}

pub enum LlmPurpose {
    Reflect,
    ExtractRules,
    GenerateMutations,
    Execute,
}
```

The LlmScheduler maintains an internal `pending: HashMap<Uuid, oneshot::Sender<LlmResponseEvent>>` map. Callers use a helper:

```rust
// crates/aletheon-brain/src/impl/llm/scheduler.rs
impl LlmScheduler {
    /// Convenience: emit request and wait for response.
    /// Registers a oneshot receiver, emits LlmRequestEvent, awaits response.
    pub async fn request(&self, purpose: LlmPurpose, messages: Vec<Message>, bus: &EventBus) -> Result<LlmResponseEvent> {
        let (tx, rx) = oneshot::channel();
        let request_id = Uuid::new_v4();
        self.pending.lock().await.insert(request_id, tx);
        bus.emit(Event::LlmRequest(LlmRequestEvent { request_id, purpose, messages })).await?;
        Ok(rx.await?)
    }
}
```

This keeps the EventBus purely event-based (no closures/callbacks in events) while supporting request-response patterns.

---

## Part 2: LLM Scheduler

Location: `crates/aletheon-brain/src/impl/llm/scheduler.rs`

Centralized LLM routing. Other modules do NOT hold `LlmProvider` directly. They emit `LlmRequestEvent` to EventBus; the Scheduler subscribes, routes to the right provider, and sends the response back via `oneshot::Sender`.

```rust
pub struct LlmScheduler {
    providers: HashMap<String, Box<dyn LlmProvider>>,
    routing_rules: Vec<RoutingRule>,
}

pub struct RoutingRule {
    pub purpose: LlmPurpose,
    pub provider_name: String,   // "deepseek" | "claude" | ...
    pub model_override: Option<String>,
}
```

Default routing:
- `LlmPurpose::Reflect` → DeepSeek (cheap, prefix-cache-friendly)
- `LlmPurpose::ExtractRules` → DeepSeek
- `LlmPurpose::GenerateMutations` → DeepSeek
- `LlmPurpose::Execute` → Claude (strong reasoning)

Config in `config.toml`:

```toml
[llm_scheduler.executor]
provider = "anthropic"
model = "claude-sonnet-4-6"
api_key_env = "ANTHROPIC_API_KEY"

[llm_scheduler.reflector]
provider = "openai"
base_url = "https://api.deepseek.com/v1"
model = "deepseek-chat"
api_key_env = "DEEPSEEK_API_KEY"
```

---

## Part 3: BrainCore — ToolObservationHandler

Location: `crates/aletheon-brain/src/impl/event_handlers/tool_observer.rs`

Subscribes to `ToolObservationEvent`. Uses LLM (via LlmScheduler) to reflect, extract rules, and detect evolution triggers.

```rust
pub struct ToolObservationHandler {
    reflection_buffer: Arc<Mutex<Vec<ReflectionEvent>>>,
    config: BrainHandlerConfig,
}

impl ToolObservationHandler {
    pub async fn handle(&self, event: ToolObservationEvent, scheduler: &LlmScheduler, bus: &EventBus) -> Result<()> {
        // 1. Request LLM reflection via scheduler
        let llm_response = scheduler.request(
            LlmPurpose::Reflect,
            self.build_reflection_prompt(&event),
            bus,
        ).await?;
        let reflection = self.parse_reflection(&event, &llm_response)?;

        // 2. Emit ReflectionEvent
        bus.emit(Event::Reflection(reflection.clone())).await?;

        // 3. Buffer and check batch threshold
        let mut buffer = self.reflection_buffer.lock().await;
        buffer.push(reflection.clone());
        if buffer.len() >= self.config.batch_size {
            let rules = self.extract_rules_from_batch(&buffer, scheduler).await?;
            bus.emit(Event::RuleExtracted(rules)).await?;
            buffer.clear();
        }

        // 4. Check evolution trigger conditions
        if self.should_trigger_evolution(&reflection, &buffer).await? {
            bus.emit(Event::EvolutionTriggered(EvolutionTriggeredEvent {
                trigger_reason: self.detect_trigger_reason(&reflection, &buffer),
                recent_reflections: buffer.iter().map(|r| r.turn_id).collect(),
                current_rules_snapshot: self.current_rules.clone(),
            })).await?;
        }

        Ok(())
    }
}
```

Trigger conditions (same as `core::EvolutionTrigger`):
- Consecutive failures ≥ 3
- Rule confidence drop > 20%
- Periodic interval (6 hours, configurable)

---

## Part 4: SelfField — MutationApprover

Location: `crates/aletheon-self/src/impl/mutation/approver.rs`

Subscribes to `EvolutionTriggeredEvent`. SelfField's Mutation Layer validates whether evolution is safe.

```rust
pub struct MutationApprover {
    genome: Arc<RwLock<Genome>>,
    config: MutationConfig,
}

impl MutationApprover {
    pub async fn handle(&self, event: EvolutionTriggeredEvent, scheduler: &LlmScheduler, bus: &EventBus) -> Result<()> {
        // 1. Request LLM to generate mutation intents via scheduler
        let llm_response = scheduler.request(
            LlmPurpose::GenerateMutations,
            self.build_mutation_prompt(&event).await,
            bus,
        ).await?;
        let intents = self.parse_intents(&llm_response)?;

        // 2. Validate each intent against boundary rules
        let genome = self.genome.read().await;
        let approved: Vec<_> = intents.into_iter()
            .filter(|i| self.validate_boundary(i, &genome).is_ok())
            .filter(|i| self.validate_identity_continuity(i, &genome).is_ok())
            .collect();

        if approved.is_empty() {
            return Ok(());  // All rejected by SelfField, silent
        }

        // 3. Emit approved intents
        bus.emit(Event::MutationIntent(MutationIntentEvent {
            intents: approved,
            approved_by: "self_field.mutation_layer".into(),
        })).await?;

        Ok(())
    }
}
```

Validation rules:
- Boundary check: mutation must not violate `boundary.rules` in genome
- Identity continuity: mutation must not change identity fields by more than `max_identity_delta`
- Magnitude clamp: `intent.magnitude` must be ≤ `config.max_magnitude`

---

## Part 5: MetaRuntime — MutationExecutor

Location: `crates/aletheon-meta/src/impl/event_handlers/mutation_executor.rs`

Subscribes to `MutationIntentEvent`. Executes the Morphogenesis Pipeline.

```rust
pub struct MutationExecutor {
    pipeline: MorphogenesisPipeline,
}

impl MutationExecutor {
    pub async fn handle(&self, event: MutationIntentEvent, bus: &EventBus) -> Result<()> {
        let result = self.pipeline.run_with_intents(event.intents).await?;

        bus.emit(Event::EvolutionResult(EvolutionResultEvent {
            recommendation: result.recommendation,
            genome_version_before: result.version_before,
            genome_version_after: result.version_after,
            summary: result.summary,
        })).await?;

        Ok(())
    }
}
```

The existing Morphogenesis Pipeline (`pipeline.rs`, `candidate.rs`, `sandbox_runner.rs`, `evaluator.rs`, `migration.rs`, `rollback.rs`) stays unchanged. Only the input source changes: instead of `KeywordMutationSource`, it receives intents from EventBus.

New trait in `aletheon-meta/src/core/`:

```rust
#[async_trait]
pub trait MutationSource: Send + Sync {
    async fn generate_intents(&self, context: &EvolutionContext) -> Result<Vec<MutationIntent>>;
}

/// EventBus-driven implementation: receives intents from MutationIntentEvent
pub struct EventBusMutationSource {
    intent_rx: Arc<Mutex<mpsc::Receiver<MutationIntentEvent>>>,
}
```

---

## Part 6: Runtime ↔ Engine Decoupling

Location: `crates/aletheon-runtime/src/impl/hooks/engine_hook.rs`

Engine is no longer directly called by Runtime. It becomes a Hook handler on EventBus.

```rust
pub struct EngineHook {
    engine: Arc<Mutex<Engine>>,
}

impl EngineHook {
    pub async fn on_event(&self, event: &Event, bus: &EventBus) -> Result<HookAction> {
        match event {
            Event::UserIntent(intent) => {
                let mut engine = self.engine.lock().await;
                let observations = engine.run_turn(&intent.content).await?;

                // Emit each tool observation as an event
                for obs in observations {
                    bus.emit(Event::ToolObservation(obs)).await?;
                }

                Ok(HookAction::Continue)
            }
            _ => Ok(HookAction::Continue)
        }
    }
}
```

Engine changes:
- Remove `learning_bridge` and `meta_pipeline` fields (no longer needed)
- `run_turn` returns `Vec<ToolObservationEvent>` instead of directly recording outcomes
- LLM provider for task execution obtained from LlmScheduler, not held directly
- Existing `outcome_recorder` / `pattern_extractor` / `rule_store` remain as local caching; the authoritative learning path is through EventBus

---

## Part 7: Engine Changes Summary

File: `crates/aletheon-runtime/src/impl/engine/cognitive_loop.rs`

| Before | After |
|--------|-------|
| `llm: Box<dyn LlmProvider>` held directly | Obtained from LlmScheduler per-turn |
| `learning_bridge: Option<Arc<LearningBridge>>` | Removed — learning via EventBus |
| `meta_pipeline: Option<Arc<MorphogenesisPipeline>>` | Removed — evolution via EventBus |
| `run_turn` records outcomes inline | `run_turn` returns `Vec<ToolObservationEvent>` |
| Post-turn: direct call to Reflector/Learner | Post-turn: emit events to EventBus |

The core ReAct loop (lines 196-568) stays unchanged. Only the tail (line 492+ outcome recording) changes to emit events instead of direct calls.

---

## Part 8: Dual-Model Configuration

Two LLM providers, routed by LlmScheduler:

| Purpose | Provider | Rationale |
|---------|----------|-----------|
| Task execution (Engine) | Claude (Anthropic) | Strong reasoning for tool use |
| Reflection (BrainCore) | DeepSeek | Cheap, prefix-cache-friendly |
| Rule extraction (BrainCore) | DeepSeek | Lightweight analysis |
| Mutation generation (SelfField) | DeepSeek | Pattern matching on genome |

This matches Reasonix's dual-model design: each model's prefix cache stays stable because they handle different concerns.

---

## Part 9: End-to-End Demo

Location: `examples/self-evolution-loop/`

### File Structure

```
examples/self-evolution-loop/
├── Cargo.toml
├── config.toml           # dual-model config + engine config
├── genome.yaml           # initial genome
├── src/
│   └── main.rs           # wire EventBus + all handlers + run demo
└── README.md
```

### Demo Flow

1. Wire EventBus with all subscribers (ToolObservationHandler, MutationApprover, MutationExecutor, Memory)
2. Start LlmScheduler with DeepSeek + Claude providers
3. Create EngineHook, register on EventBus
4. Send UserIntentEvent: "format syslog by severity"
5. Engine executes → ToolObservationEvent → BrainCore reflects → ReflectionEvent
6. Send 3 more tasks with intentional failures
7. After 3 consecutive failures → EvolutionTriggeredEvent → SelfField approves → MutationIntentEvent → MetaRuntime evolves genome
8. Print genome diff showing version increment and new care/boundary rules

### config.toml

```toml
[llm_scheduler.executor]
provider = "anthropic"
model = "claude-sonnet-4-6"
api_key_env = "ANTHROPIC_API_KEY"

[llm_scheduler.reflector]
provider = "openai"
base_url = "https://api.deepseek.com/v1"
model = "deepseek-chat"
api_key_env = "DEEPSEEK_API_KEY"

[engine]
learning_enabled = true

[evolution]
consecutive_failure_threshold = 3
batch_size = 3
evolution_interval_hours = 0   # disabled for demo
```

### Acceptance Criteria

1. Run 1 → ReflectionEvent emitted with LLM-generated assessment (not keyword)
2. After batch → RuleExtractedEvent with generalized rules
3. After 3 failures → EvolutionTriggeredEvent → MutationIntentEvent (LLM-generated, not keyword)
4. Morphogenesis Pipeline executes → genome.yaml version increments
5. All LLM call failures have graceful fallback
6. No `unwrap()` in event handler paths
7. Zero direct calls between BrainCore/SelfField/MetaRuntime — all through EventBus

---

## Implementation Order

1. **Event types** in `aletheon-abi` (new events module)
2. **LlmScheduler** in `aletheon-brain` (new scheduler module)
3. **ToolObservationHandler** in `aletheon-brain` (new event_handlers module)
4. **MutationApprover** in `aletheon-self` (new mutation module)
5. **EventBusMutationSource** in `aletheon-meta` (adapt existing pipeline)
6. **MutationExecutor** in `aletheon-meta` (new event_handlers module)
7. **EngineHook** in `aletheon-runtime` (new hooks module)
8. **Engine changes** — remove direct learning fields, emit events
9. **Demo** — `examples/self-evolution-loop/`
10. **Integration tests** — verify full loop with MockLlm

---

## Crate Dependency Graph (unchanged)

```text
aletheon-abi        ← all crates depend on this (event types)
aletheon-comm       ← all crates depend on this (EventBus)
aletheon-brain      ← depends on abi, comm
aletheon-self       ← depends on abi, comm
aletheon-meta       ← depends on abi, comm
aletheon-runtime    ← depends on abi, comm, brain, self, meta, body, memory
```

No new cross-crate dependencies introduced. Brain and Meta still only depend on abi + comm. Runtime is the only crate that knows about all others (as before).
