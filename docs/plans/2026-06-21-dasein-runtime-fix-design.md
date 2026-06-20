# DaseinModule Runtime Fix Design

**Date**: 2026-06-21
**Status**: Design Complete, Pending Review
**Scope**: Fix 10 critical issues preventing DaseinModule from running

## Problem Statement

The DaseinModule is a well-designed but completely disconnected subsystem. Every module has clean implementations and passing unit tests, but in production:

1. **DaseinModule is disabled** — `enable_dasein=false`, daemon handler never enables it
2. **mood never updated** — SorgeLoop computes mood locally, never writes back to shared state
3. **Bewandtnisganzheit never populated** — zero calls to `add_entity()` in production
4. **MutableSelfModel never seeded** — zero calls to `assert()`, negation cycle has nothing to negate
5. **CareStructure never populated** — `determine_action()` zero callers
6. **ProtentionField never updated** — `update_from_patterns()` zero callers
7. **DaseinEventBridge never wired** — `wire_dasein_event_bridge()` zero callers
8. **No persistence** — `dasein_state` table exists but never read/written
9. **Context injection never consumed** — `dasein_prompt_injection()` never called by runtime
10. **`set_decay_rate()` is a no-op stub** — empty method body with TODO

## Design Decisions

| Decision | Choice | Rationale |
|---|---|---|
| Strategy | Three layers: connect → fill → persist | 10 problems too many to fix at once |
| First loop | LLM thinking → temporal → mood → reasoning injection | Most fundamental self-awareness loop |
| Data source | Auto-fill from system events | No manual seeding needed |
| Communication | Dual-layer: internal mpsc + cross-module EventBus | Internal fast, cross-module decoupled |
| Approach | Minimal viable self-loop (Plan C) | Close one loop first, verify, then expand |

## Layer 1: Connectivity — Fix 6 Issues

### Fix #1: Enable DaseinModule

File: `crates/aletheon-runtime/src/impl/daemon/handler.rs:400-404`

```rust
let self_field_config = SelfFieldConfig {
    db_path: Some(data_dir.join("self_field.db")),
    enable_dasein: true,  // FIX: enable DaseinModule
    ..Default::default()
};
```

### Fix #2: SorgeLoop Writes Mood Back

File: `crates/aletheon-self/src/dasein/sorge.rs`

The SorgeLoop needs access to `DaseinModule.mood: Arc<RwLock<Stimmung>>`. Pass it as a parameter to `start()`.

```rust
// In SorgeLoop::start(), after mood synthesis:
let new_mood = Stimmung::synthesize(
    world_mood.as_ref(),
    temporal_mood.as_ref(),
    care_mood.as_ref(),
    Some(&mood),
);
// FIX: Write back to shared mood
{
    let mut shared_mood = shared_mood_ref.write();
    *shared_mood = new_mood.clone();
}
mood = new_mood;
```

### Fix #6: ProtentionField Update

File: `crates/aletheon-self/src/dasein/sorge.rs`

After passive synthesis, update protentions from detected patterns:

```rust
if tick_count % 10 == 0 {
    let patterns = temporality.passive_synthesize();
    // FIX: Update protentions from detected patterns
    temporality.update_protentions_from_patterns(&patterns);
}
```

File: `crates/aletheon-self/src/dasein/temporality.rs`

Add `update_protentions_from_patterns()` method that converts `TemporalPattern` into `Protention` entries.

### Fix #7: Wire DaseinEventBridge

File: `crates/aletheon-runtime/src/impl/daemon/handler.rs`

After `self_field.init()`:

```rust
if let Some(dasein_tx) = self_field.dasein_event_tx() {
    let bridge = DaseinEventBridge::new(dasein_tx);
    if let Some(bus) = &event_bus {
        bridge.subscribe(&*bus)?;
    }
}
```

### Fix #9: Inject DaseinContext into Prompts

File: `crates/aletheon-runtime/src/core/react_loop.rs`

Replace `build()` with `build_with_dasein()` when DaseinModule is available:

```rust
let dasein_context = self.dasein.as_ref().map(|d| d.format_context());
let system_prompt = if let Some(ctx) = dasein_context {
    build_with_dasein(base_prompt, &ctx)
} else {
    build(base_prompt)
};
```

### Fix #10: Implement set_decay_rate

File: `crates/aletheon-self/src/dasein/temporality.rs`

```rust
impl RetentionField {
    pub fn set_decay_rate(&mut self, rate: f64) {
        self.decay_rate = rate.clamp(0.1, 1.0);
    }
}
```

## Layer 2: Auto-Fill — Extract Data from System Events

### Data Source Mapping

| Subsystem | Data Source | Event Type |
|---|---|---|
| Bewandtnisganzheit | Tool calls | `ToolObservation` → tool as entity, call relations as edges |
| MutableSelfModel | LLM thinking text | `ThinkingObserved` → extract assertions |
| CareStructure | User goals | `UserInput` → as Concern |
| CareStructure | Task progress | `ToolObservation` → update fallenness |

### Bewandtnis Auto-Fill

In `DaseinEventBridge`, when `ToolObservation` arrives:

```rust
EventType::ToolObservation => {
    let tool_name = data["tool_name"].as_str().unwrap_or("unknown");
    let status = data["status"].as_str().unwrap_or("unknown");

    // Register tool as Bewandtnis entity
    world.add_entity(BewandtnisNode {
        id: EntityId(tool_name.to_string()),
        what_it_is: format!("Tool: {}", tool_name),
        for_the_sake_of: vec!["task_completion".to_string()],
        readiness: match status {
            "success" => ReadinessState::ReadyToHand,
            "error" => ReadinessState::PresentAtHand,
            _ => ReadinessState::ReadyToHand,
        },
    });
}
```

### SelfModel Auto-Fill

In SorgeLoop, when processing `ThinkingObserved`:

```rust
DaseinEvent::ThinkingObserved { text, turn } => {
    let assertions = extract_assertions_from_thinking(&text);
    for assertion in assertions {
        self_model.assert(assertion, AssertionSource::Discovered);
    }
    ExperientialContent { ... }
}

fn extract_assertions_from_thinking(text: &str) -> Vec<String> {
    // Heuristic: sentences starting with "I know", "This is", "The fact that"
    let patterns = ["I know that ", "This is ", "The fact that "];
    let mut assertions = Vec::new();
    for pattern in &patterns {
        if let Some(pos) = text.find(pattern) {
            let end = text[pos..].find('.').unwrap_or(text.len() - pos);
            assertions.push(text[pos..pos + end].to_string());
        }
    }
    assertions
}
```

### CareStructure Auto-Fill

In SorgeLoop, when processing `UserInput`:

```rust
DaseinEvent::UserInput { content } => {
    care.add_concern(Concern {
        purpose: content.clone(),
        urgency: 0.8,
        mood_tone: Stimmung::Neugier { curiosity_about: content.clone() },
    });
    ExperientialContent { ... }
}
```

When tool succeeds:

```rust
if status == "success" {
    care.update_fallenness(0.1);  // Progress reduces fallenness
}
```

## Layer 2: Communication — Dual-Layer Architecture

### Cross-Module Layer (EventBus)

```
EventBus → DaseinEventBridge → mpsc::try_send(DaseinEvent::SystemEvent)
    → SorgeLoop.recv() → TemporalStream.ingest()

Events: ToolObservation, MemoryStored, EvolutionTriggered, AgentStarted
```

### Internal Layer (mpsc channel)

```
DaseinModule internal:
event_tx → SorgeLoop → subsystems

Events: ThinkingObserved, ReasoningObserved, KnowledgeAsserted,
        MoodShift, BewandtnisChange, NegationCompleted, TemporalEvent
```

### Data Flow Paths

```
Path 1: External → DaseinModule
EventBus → DaseinEventBridge → mpsc → SorgeLoop → TemporalStream

Path 2: LLM Thinking → DaseinModule
ReAct Loop → ContentBlock::Thinking → mpsc → SorgeLoop → TemporalStream + SelfModel

Path 3: DaseinModule → LLM Reasoning
DaseinModule.mood() → ReAct Loop → build_with_dasein() → system prompt

Path 4: DaseinModule → MetaCognition
DaseinModule.to_context_injection() → MetaCognition.decide() → EvolutionAction
```

### Channel Capacity and Backpressure

```rust
// Internal channel: capacity 256, non-blocking send
let (event_tx, event_rx) = mpsc::channel(256);

// Non-blocking: drop if full (don't block ReAct Loop)
event_tx.try_send(event).ok();

// 100ms timeout: avoid permanent block
tokio::time::timeout(Duration::from_millis(100), event_rx.recv())
```

## Layer 3: Persistence — Save/Load DaseinModule State

### What to Persist

| Subsystem | Content | Frequency |
|---|---|---|
| TemporalStream | retention (last 50 experiences) | Every shutdown |
| Bewandtnisganzheit | entities and edges | Every change |
| MutableSelfModel | assertions and possibilities | Every change |
| CareStructure | concerns and projections | Every change |
| mood | current mood | Every shutdown |

### Implementation

Use existing `dasein_state` table (key/value store):

```rust
impl Persistable for DaseinModule {
    fn save_to_store(&self, store: &SelfFieldStore) -> anyhow::Result<()> {
        let conn = store.conn();

        let mood_json = serde_json::to_string(&*self.mood.read())?;
        conn.execute(
            "INSERT OR REPLACE INTO dasein_state (key, value) VALUES (?1, ?2)",
            params!["mood", mood_json],
        )?;

        let temporal_json = serde_json::to_string(&self.temporality.to_snapshot())?;
        conn.execute(
            "INSERT OR REPLACE INTO dasein_state (key, value) VALUES (?1, ?2)",
            params!["temporality", temporal_json],
        )?;

        // ... self_model, world, care ...
        Ok(())
    }

    fn load_from_store(&mut self, store: &SelfFieldStore) -> anyhow::Result<()> {
        let conn = store.conn();

        if let Some(mood_json) = conn.query_row(
            "SELECT value FROM dasein_state WHERE key = 'mood'",
            [],
            |row| row.get::<_, String>(0),
        ).optional()? {
            let mood: Stimmung = serde_json::from_str(&mood_json)?;
            *self.mood.write() = mood;
        }

        // ... temporal, self_model, world, care ...
        Ok(())
    }
}
```

### Save/Load Timing

```rust
impl SelfField {
    pub fn init(&mut self) -> anyhow::Result<()> {
        // ... existing init ...
        if let (Some(dasein), Some(store)) = (&mut self.dasein, &self.store) {
            dasein.load_from_store(store)?;
        }
        Ok(())
    }

    pub fn shutdown(&mut self) -> anyhow::Result<()> {
        if let (Some(dasein), Some(store)) = (&self.dasein, &self.store) {
            dasein.save_to_store(store)?;
        }
        // ... existing shutdown ...
        Ok(())
    }
}
```

## Affected Files Summary

| File | Change | Issues Fixed |
|---|---|---|
| `crates/aletheon-runtime/src/impl/daemon/handler.rs` | Enable dasein, wire bridge | #1, #7 |
| `crates/aletheon-self/src/dasein/sorge.rs` | Write mood back, process new events | #2, auto-fill |
| `crates/aletheon-self/src/dasein/temporality.rs` | Implement set_decay_rate, update_protentions | #6, #10 |
| `crates/aletheon-self/src/dasein/event_bridge.rs` | Auto-fill Bewandtnis | auto-fill |
| `crates/aletheon-runtime/src/core/react_loop.rs` | Inject dasein context | #9 |
| `crates/aletheon-self/src/core/store.rs` | Implement Persistable for DaseinModule | #8 |
| `crates/aletheon-self/src/core/mod.rs` | Load/save on init/shutdown | #8 |

## Success Criteria

1. **DaseinModule is enabled** — `enable_dasein=true` in production config
2. **Mood updates** — `DaseinModule.mood()` returns non-stale values
3. **ProtentionField works** — patterns detected → protentions updated
4. **EventBridge wired** — real system events flow into DaseinModule
5. **Context injected** — LLM sees DaseinContext in system prompt
6. **set_decay_rate works** — mood-based decay rate adjustment functional
7. **Auto-fill works** — Bewandtnis, SelfModel, CareStructure populated from events
8. **Persistence works** — state survives restarts
9. **Full loop** — LLM thinking → temporal → mood → reasoning → LLM sees mood
