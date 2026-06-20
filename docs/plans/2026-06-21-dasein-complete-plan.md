# Dasein Self-Awareness — Complete Implementation Plan

> **For agentic workers:** Use `workflow-feature` or `writing-plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Enable DaseinModule in production, fix 10 critical runtime issues, add LLM thinking capture, mood-aware reasoning, MetaCognition, and self-observation — closing all 7 feedback loops.

**Architecture:** Bottom-up: data layer → provider → flow → auto-fill → brain → persistence → decision → self-observation → coordination.

**Tech Stack:** Rust, tokio, serde, mpsc channels, RwLock, rusqlite

**Specs:**
- `docs/plans/2026-06-21-self-awareness-architecture-design.md`
- `docs/plans/2026-06-21-dasein-runtime-fix-design.md`

---

## Phase 1: Data Layer — ABI Types

### Task 1: Add `ContentBlock::Thinking` Variant

**Files:**
- Modify: `crates/aletheon-abi/src/message.rs:9-32`

- [ ] **Step 1: Add Thinking variant**

```rust
// crates/aletheon-abi/src/message.rs — after Text variant:
    /// LLM thinking/reasoning content (extended thinking, chain-of-thought)
    Thinking {
        text: String,
        /// Anthropic thinking signature for multi-turn verification
        signature: Option<String>,
    },
```

- [ ] **Step 2: Update estimate_chars to return 0 for Thinking**

In the `estimate_chars()` method:
```rust
ContentBlock::Thinking { .. } => 0,
```

- [ ] **Step 3: Add test**

```rust
#[test]
fn test_thinking_block_serde_roundtrip() {
    let block = ContentBlock::Thinking {
        text: "I need to think about this...".to_string(),
        signature: Some("sig_abc".to_string()),
    };
    let json = serde_json::to_string(&block).unwrap();
    assert!(json.contains("thinking"));
    let deserialized: ContentBlock = serde_json::from_str(&json).unwrap();
    match deserialized {
        ContentBlock::Thinking { text, signature } => {
            assert_eq!(text, "I need to think about this...");
            assert_eq!(signature, Some("sig_abc".to_string()));
        }
        _ => panic!("Expected Thinking variant"),
    }
}

#[test]
fn test_thinking_block_estimate_chars_zero() {
    let block = ContentBlock::Thinking {
        text: "Long thinking content...".to_string(),
        signature: None,
    };
    assert_eq!(block.estimate_chars(), 0);
}
```

- [ ] **Step 4: Run tests**

```bash
cargo test -p aletheon-abi
```

- [ ] **Step 5: Commit**

```bash
git add crates/aletheon-abi/src/message.rs
git commit -m "feat(abi): add ContentBlock::Thinking variant for LLM thinking capture"
```

---

### Task 2: Add Thinking-Related DaseinEvent Variants

**Files:**
- Modify: `crates/aletheon-abi/src/dasein.rs:261-291`

- [ ] **Step 1: Add new variants**

```rust
// crates/aletheon-abi/src/dasein.rs — add to DaseinEvent enum:
    // LLM thinking events
    ThinkingObserved { text: String, turn: usize },
    ReasoningObserved { text: String, turn: usize, has_tool_calls: bool },
    KnowledgeAsserted { assertions: Vec<String>, confidence: f64 },
```

- [ ] **Step 2: Add tests**

```rust
#[test]
fn test_thinking_observed_serde() {
    let event = DaseinEvent::ThinkingObserved {
        text: "Let me reason...".to_string(),
        turn: 42,
    };
    let json = serde_json::to_string(&event).unwrap();
    let deserialized: DaseinEvent = serde_json::from_str(&json).unwrap();
    match deserialized {
        DaseinEvent::ThinkingObserved { text, turn } => {
            assert_eq!(text, "Let me reason...");
            assert_eq!(turn, 42);
        }
        _ => panic!("Expected ThinkingObserved"),
    }
}
```

- [ ] **Step 3: Run tests**

```bash
cargo test -p aletheon-abi
```

- [ ] **Step 4: Commit**

```bash
git add crates/aletheon-abi/src/dasein.rs
git commit -m "feat(abi): add ThinkingObserved/ReasoningObserved/KnowledgeAsserted events"
```

---

### Task 3: Add `StreamChunk::ThinkingDelta`

**Files:**
- Modify: `crates/aletheon-brain/src/impl/llm/provider.rs:9-29`

- [ ] **Step 1: Add variant**

```rust
// crates/aletheon-brain/src/impl/llm/provider.rs — after TextDelta:
    /// Thinking/reasoning content delta (extended thinking)
    ThinkingDelta { text: String },
```

- [ ] **Step 2: Verify compilation**

```bash
cargo check -p aletheon-brain
```

- [ ] **Step 3: Commit**

```bash
git add crates/aletheon-brain/src/impl/llm/provider.rs
git commit -m "feat(brain): add StreamChunk::ThinkingDelta variant"
```

---

## Phase 2: Fix Runtime Issues

### Task 4: Implement set_decay_rate (#10)

**Files:**
- Modify: `crates/aletheon-self/src/dasein/temporality.rs:45,119-123`

- [ ] **Step 1: Make base_decay_rate mutable**

```rust
// RetentionField struct — change:
pub base_decay_rate: RwLock<f64>,  // was: f64
```

- [ ] **Step 2: Update constructor**

```rust
pub fn new(max_depth: usize, decay_rate: f64) -> Self {
    Self {
        moments: RwLock::new(Vec::new()),
        max_depth,
        base_decay_rate: RwLock::new(decay_rate),
    }
}
```

- [ ] **Step 3: Implement set_decay_rate**

```rust
pub fn set_decay_rate(&self, rate: f64) {
    *self.base_decay_rate.write() = rate.clamp(0.1, 1.0);
}
```

- [ ] **Step 4: Update all reads**

Search for `self.base_decay_rate` and change to `*self.base_decay_rate.read()`.

- [ ] **Step 5: Run tests**

```bash
cargo test -p aletheon-self -- dasein::temporality
```

- [ ] **Step 6: Commit**

```bash
git add crates/aletheon-self/src/dasein/temporality.rs
git commit -m "fix(self): implement set_decay_rate — make base_decay_rate mutable via RwLock"
```

---

### Task 5: Enable DaseinModule in Production (#1)

**Files:**
- Modify: `crates/aletheon-runtime/src/impl/daemon/handler.rs:404`

- [ ] **Step 1: Enable dasein**

```rust
// handler.rs — line ~404:
let self_field_config = SelfFieldConfig {
    db_path: Some(data_dir.join("self_field.db")),
    enable_dasein: true,  // ADD THIS LINE
    ..Default::default()
};
```

- [ ] **Step 2: Verify compilation**

```bash
cargo check -p aletheon-runtime
```

- [ ] **Step 3: Commit**

```bash
git add crates/aletheon-runtime/src/impl/daemon/handler.rs
git commit -m "fix(runtime): enable DaseinModule in production"
```

---

### Task 6: SorgeLoop Writes Mood Back (#2)

**Files:**
- Modify: `crates/aletheon-self/src/dasein/sorge.rs:48-56,113-121`
- Modify: `crates/aletheon-self/src/dasein/mod.rs:80-88`

- [ ] **Step 1: Add shared_mood parameter to start()**

```rust
// sorge.rs — change start() signature:
pub fn start(
    &self,
    temporality: Arc<TemporalStream>,
    world: Arc<Bewandtnisganzheit>,
    self_model: Arc<MutableSelfModel>,
    care: Arc<CareStructure>,
    negativity: Arc<NegativityEngine>,
    shared_mood: Arc<RwLock<Stimmung>>,  // NEW
) -> Option<tokio::task::JoinHandle<()>> {
```

- [ ] **Step 2: Write mood back after synthesis**

```rust
// sorge.rs — after mood synthesis (~line 121):
if new_mood != mood {
    mood = new_mood.clone();
    *shared_mood.write() = new_mood;
}
```

- [ ] **Step 3: Update DaseinModule::start_sorge_loop()**

```rust
// mod.rs — pass mood:
pub fn start_sorge_loop(&self) -> Option<tokio::task::JoinHandle<()>> {
    self.sorge.start(
        self.temporality.clone(),
        self.world.clone(),
        self.self_model.clone(),
        self.care.clone(),
        self.negativity.clone(),
        self.mood.clone(),
    )
}
```

- [ ] **Step 4: Add import to sorge.rs**

```rust
use std::sync::RwLock;
```

- [ ] **Step 5: Run tests**

```bash
cargo test -p aletheon-self -- dasein
```

- [ ] **Step 6: Commit**

```bash
git add crates/aletheon-self/src/dasein/sorge.rs crates/aletheon-self/src/dasein/mod.rs
git commit -m "fix(self): SorgeLoop writes mood back to DaseinModule"
```

---

### Task 7: ProtentionField Update from Patterns (#6)

**Files:**
- Modify: `crates/aletheon-self/src/dasein/temporality.rs` — add method to TemporalStream
- Modify: `crates/aletheon-self/src/dasein/sorge.rs` — call after passive_synthesize

- [ ] **Step 1: Add update_protentions_from_patterns to TemporalStream**

```rust
// temporality.rs — add to TemporalStream impl:
pub fn update_protentions_from_patterns(&self, patterns: &[TemporalPattern]) {
    self.protention.write().update_from_patterns(patterns);
}
```

- [ ] **Step 2: Verify passive_synthesize returns patterns**

Check that `passive_synthesize()` returns `Vec<TemporalPattern>`. If it returns `()`, change it to return patterns.

- [ ] **Step 3: Call in SorgeLoop**

```rust
// sorge.rs — change step 5:
if tick_count % 10 == 0 {
    let patterns = temporality.passive_synthesize();
    temporality.update_protentions_from_patterns(&patterns);
}
```

- [ ] **Step 4: Run tests**

```bash
cargo test -p aletheon-self -- dasein::temporality
```

- [ ] **Step 5: Commit**

```bash
git add crates/aletheon-self/src/dasein/temporality.rs crates/aletheon-self/src/dasein/sorge.rs
git commit -m "fix(self): update ProtentionField from detected patterns"
```

---

## Phase 3: Provider Layer — Capture Thinking

### Task 8: Preserve Thinking Blocks in Anthropic Provider

**Files:**
- Modify: `crates/aletheon-brain/src/impl/llm/anthropic.rs:260-264`

- [ ] **Step 1: Change thinking block handling**

```rust
// Change FROM:
"thinking" => {
    tracing::debug!("Skipping thinking block");
    None
}

// Change TO:
"thinking" => {
    tracing::debug!("Preserving thinking block (len={})", c.text.as_ref().map(|s| s.len()).unwrap_or(0));
    Some(ContentBlock::Thinking {
        text: c.text.unwrap_or_default(),
        signature: c.thinking_signature,
    })
}
```

- [ ] **Step 2: Handle thinking_delta in streaming**

In the streaming `content_block_delta` handler, add:
```rust
"thinking_delta" => {
    if let Some(delta) = c.delta.and_then(|d| d.text) {
        return Some(StreamChunk::ThinkingDelta { text: delta });
    }
}
```

- [ ] **Step 3: Verify compilation**

```bash
cargo check -p aletheon-brain
```

- [ ] **Step 4: Commit**

```bash
git add crates/aletheon-brain/src/impl/llm/anthropic.rs
git commit -m "feat(brain): preserve thinking blocks in Anthropic provider"
```

---

### Task 9: Preserve Reasoning Content in OpenAI Provider

**Files:**
- Modify: `crates/aletheon-brain/src/impl/llm/openai_provider.rs:379-385`

- [ ] **Step 1: Change reasoning_content handling**

```rust
// Change FROM:
let text = choice.message.content.filter(|s| !s.is_empty())
    .or(choice.message.reasoning_content.filter(|s| !s.is_empty()));
if let Some(text) = text {
    content.push(ContentBlock::Text { text });
}

// Change TO:
if let Some(thinking) = choice.message.reasoning_content.filter(|s| !s.is_empty()) {
    content.push(ContentBlock::Thinking { text: thinking, signature: None });
}
if let Some(text) = choice.message.content.filter(|s| !s.is_empty()) {
    content.push(ContentBlock::Text { text });
}
```

- [ ] **Step 2: Verify compilation**

```bash
cargo check -p aletheon-brain
```

- [ ] **Step 3: Commit**

```bash
git add crates/aletheon-brain/src/impl/llm/openai_provider.rs
git commit -m "feat(brain): preserve reasoning_content as ContentBlock::Thinking in OpenAI provider"
```

---

## Phase 4: Flow Layer — SorgeLoop + ReAct Loop

### Task 10: Process All DaseinEvent Variants in SorgeLoop

**Files:**
- Modify: `crates/aletheon-self/src/dasein/sorge.rs:79-106`

- [ ] **Step 1: Replace `_ => continue` with full handling**

Add handlers for all DaseinEvent variants. Key additions:

```rust
// ThinkingObserved → extract assertions + ingest
DaseinEvent::ThinkingObserved { text, turn } => {
    let assertions = extract_assertions_from_thinking(text);
    for assertion in assertions {
        self_model.assert(assertion, AssertionSource::Discovered);
    }
    ExperientialContent {
        semantic: format!("[thinking:turn_{}]", turn),
        action: Some(format!("llm_thinking_turn_{}", turn)),
        perception: Some(text.clone()),
        negation: None,
    }
}

// ReasoningObserved → ingest
DaseinEvent::ReasoningObserved { text, turn, .. } => {
    ExperientialContent {
        semantic: format!("[reasoning:turn_{}]", turn),
        action: Some(format!("llm_reasoning_turn_{}", turn)),
        perception: Some(text.clone()),
        negation: None,
    }
}

// KnowledgeAsserted → update self_model
DaseinEvent::KnowledgeAsserted { assertions, .. } => {
    for assertion in assertions {
        self_model.assert(assertion.clone(), AssertionSource::Discovered);
    }
    ExperientialContent {
        semantic: format!("[knowledge:{}]", assertions.join(",")),
        action: Some("knowledge_assertion".to_string()),
        perception: None,
        negation: None,
    }
}

// NegationCompleted → register possibilities
DaseinEvent::NegationCompleted { target, new_possibilities } => {
    for p in new_possibilities {
        self_model.add_possibility(p.clone(), 0.5, 0.5);
    }
    ExperientialContent {
        semantic: format!("[negation:{}]", target),
        action: Some("negation_completed".to_string()),
        perception: None,
        negation: Some(target.clone()),
    }
}

// MoodShift → log
DaseinEvent::MoodShift { from, to, reason } => {
    ExperientialContent {
        semantic: format!("[mood:{:?}->{:?}]", from, to),
        action: Some("mood_transition".to_string()),
        perception: Some(reason.clone()),
        negation: None,
    }
}

// BewandtnisChange → update world
DaseinEvent::BewandtnisChange { entity_id, new_state, .. } => {
    world.update_readiness(entity_id, new_state.clone());
    ExperientialContent {
        semantic: format!("[bewandtnis:{}]", entity_id),
        action: Some("world_change".to_string()),
        perception: None,
        negation: None,
    }
}

// TemporalEvent → ingest
DaseinEvent::TemporalEvent { kind, content } => {
    ExperientialContent {
        semantic: format!("[temporal:{:?}]", kind),
        action: Some("temporal_event".to_string()),
        perception: Some(content.clone()),
        negation: None,
    }
}
```

- [ ] **Step 2: Add extract_assertions_from_thinking helper**

```rust
fn extract_assertions_from_thinking(text: &str) -> Vec<String> {
    let patterns = ["I know that ", "This is ", "The fact that ", "I understand that "];
    let mut assertions = Vec::new();
    for sentence in text.split('.') {
        let trimmed = sentence.trim();
        for pattern in &patterns {
            if trimmed.starts_with(pattern) && trimmed.len() > pattern.len() + 5 {
                assertions.push(trimmed.to_string());
                break;
            }
        }
    }
    assertions
}
```

- [ ] **Step 3: Run tests**

```bash
cargo test -p aletheon-self -- dasein
```

- [ ] **Step 4: Commit**

```bash
git add crates/aletheon-self/src/dasein/sorge.rs
git commit -m "feat(self): process all DaseinEvent variants in SorgeLoop"
```

---

### Task 11: Auto-Fill Bewandtnis from Tool Events

**Files:**
- Modify: `crates/aletheon-self/src/dasein/sorge.rs` — SystemEvent handler
- Modify: `crates/aletheon-self/src/dasein/bewandtnis.rs` — add add_entity_if_absent

- [ ] **Step 1: Add add_entity_if_absent to Bewandtnisganzheit**

```rust
// bewandtnis.rs — add to impl:
pub fn add_entity_if_absent(
    &self,
    id: &EntityId,
    what_it_is: String,
    for_the_sake_of: Vec<String>,
    readiness: ReadinessState,
) {
    let mut nodes = self.nodes.write();
    if let Some(node) = nodes.get(id) {
        node.write().readiness = readiness;
    } else {
        nodes.insert(id.clone(), Arc::new(RwLock::new(BewandtnisNode {
            id: id.clone(),
            what_it_is,
            for_the_sake_of,
            readiness,
        })));
    }
}
```

- [ ] **Step 2: Add tool entity registration in SystemEvent handler**

```rust
// sorge.rs — in SystemEvent arm:
if source == "tool_execution" {
    let parts: Vec<&str> = content.splitn(2, ": ").collect();
    let tool_name = parts.first().unwrap_or(&"unknown");
    let status = parts.get(1).unwrap_or(&"unknown");
    let readiness = match *status {
        "success" => ReadinessState::ReadyToHand,
        "error" => ReadinessState::PresentAtHand,
        _ => ReadinessState::ReadyToHand,
    };
    world.add_entity_if_absent(
        &EntityId(tool_name.to_string()),
        format!("Tool: {}", tool_name),
        vec!["task_completion".to_string()],
        readiness,
    );
}
```

- [ ] **Step 3: Run tests**

```bash
cargo test -p aletheon-self -- dasein::bewandtnis
```

- [ ] **Step 4: Commit**

```bash
git add crates/aletheon-self/src/dasein/sorge.rs crates/aletheon-self/src/dasein/bewandtnis.rs
git commit -m "feat(self): auto-fill Bewandtnisganzheit from tool execution events"
```

---

### Task 12: Auto-Fill CareStructure from User Input

**Files:**
- Modify: `crates/aletheon-self/src/dasein/sorge.rs` — UserInput handler

- [ ] **Step 1: Add concern registration**

```rust
// sorge.rs — in UserInput arm:
DaseinEvent::UserInput { content } => {
    care.add_concern(content.clone(), 0.8);
    ExperientialContent { ... }
}
```

Check `add_concern` signature — may need a `Concern` struct.

- [ ] **Step 2: Run tests**

```bash
cargo test -p aletheon-self -- dasein::care_structure
```

- [ ] **Step 3: Commit**

```bash
git add crates/aletheon-self/src/dasein/sorge.rs
git commit -m "feat(self): auto-fill CareStructure from user input events"
```

---

### Task 13: Process Thinking Blocks in ReAct Loop

**Files:**
- Modify: `crates/aletheon-runtime/src/core/react_loop.rs:427-437`

- [ ] **Step 1: Add thinking_parts and Thinking handling**

```rust
// react_loop.rs — change block processing:
let mut text_parts = Vec::new();
let mut thinking_parts = Vec::new();
let mut tool_calls = Vec::new();
for block in &response.content {
    match block {
        ContentBlock::Text { text } => text_parts.push(text.clone()),
        ContentBlock::Thinking { text, .. } => {
            thinking_parts.push(text.clone());
            // Send to DaseinModule if available
            // (dasein_tx will be added in Task 18)
        }
        ContentBlock::ToolUse { id, name, input } => {
            tool_calls.push((id.clone(), name.clone(), input.clone()));
        }
        _ => {}
    }
}
```

- [ ] **Step 2: Verify compilation**

```bash
cargo check -p aletheon-runtime
```

- [ ] **Step 3: Commit**

```bash
git add crates/aletheon-runtime/src/core/react_loop.rs
git commit -m "feat(runtime): process ContentBlock::Thinking in ReAct loop"
```

---

## Phase 5: BrainCore — Mood-Aware Reasoning

### Task 14: Add think_with_stimmung()

**Files:**
- Modify: `crates/aletheon-brain/src/core/reasoner.rs`

- [ ] **Step 1: Add method**

```rust
use aletheon_abi::dasein::Stimmung;

pub fn think_with_stimmung(&self, intent: &str, stimmung: &Stimmung) -> String {
    let strategy = match stimmung {
        Stimmung::Angst { facing } => format!("Proceed with caution. Concern: {:?}", facing),
        Stimmung::Neugier { curiosity_about } => format!("Explore freely. Curious about: {}", curiosity_about),
        Stimmung::Langeweile { depth } => format!("Question assumptions. Boredom: {:?}", depth),
        Stimmung::Entschlossenheit { chosen_possibility } => format!("Act decisively. Chosen: {}", chosen_possibility),
        _ => "Balanced approach.".to_string(),
    };
    format!("Strategy: {}\nIntent: {}", strategy, intent)
}
```

- [ ] **Step 2: Run tests**

```bash
cargo test -p aletheon-brain
```

- [ ] **Step 3: Commit**

```bash
git add crates/aletheon-brain/src/core/reasoner.rs
git commit -m "feat(brain): add think_with_stimmung() for mood-aware reasoning"
```

---

### Task 15: Add generate_plan_with_stimmung()

**Files:**
- Modify: `crates/aletheon-brain/src/core/planner.rs`

- [ ] **Step 1: Add risk adjustment and method**

```rust
fn adjust_risk_for_stimmung(stimmung: &Stimmung) -> f64 {
    match stimmung {
        Stimmung::Angst { .. } => 0.3,
        Stimmung::Neugier { .. } => 0.8,
        Stimmung::Entschlossenheit { .. } => 0.9,
        Stimmung::Langeweile { .. } => 0.7,
        _ => 0.5,
    }
}

impl Planner {
    pub fn generate_plan_with_stimmung(&self, intent: &str, reasoning: &str, stimmung: &Stimmung) -> Plan {
        let risk = adjust_risk_for_stimmung(stimmung);
        let mut plan = self.generate_plan(intent, reasoning);
        plan.risk_tolerance = risk;
        plan
    }
}
```

- [ ] **Step 2: Run tests**

```bash
cargo test -p aletheon-brain
```

- [ ] **Step 3: Commit**

```bash
git add crates/aletheon-brain/src/core/planner.rs
git commit -m "feat(brain): add generate_plan_with_stimmung() for mood-aware planning"
```

---

## Phase 6: Wiring — Connect Everything

### Task 16: Wire DaseinEventBridge (#7)

**Files:**
- Modify: `crates/aletheon-runtime/src/impl/daemon/handler.rs`

- [ ] **Step 1: Add bridge wiring after self_field init**

```rust
// handler.rs — after self_field initialization:
if let Some(ref event_bus) = event_bus {
    let sf = self_field.lock().await;
    sf.wire_dasein_event_bridge(&**event_bus).await?;
}
```

- [ ] **Step 2: Verify compilation**

```bash
cargo check -p aletheon-runtime
```

- [ ] **Step 3: Commit**

```bash
git add crates/aletheon-runtime/src/impl/daemon/handler.rs
git commit -m "fix(runtime): wire DaseinEventBridge to EventBus"
```

---

### Task 17: Inject DaseinContext into Prompts (#9)

**Files:**
- Modify: `crates/aletheon-runtime/src/impl/daemon/handler.rs`

- [ ] **Step 1: Prepend dasein context to user input**

```rust
// handler.rs — before calling react_loop.run():
let enriched_input = {
    let sf = self_field.lock().await;
    if let Some(ref dasein) = sf.dasein() {
        let ctx = dasein.format_context();
        format!("{}\n\n---\n\n{}", ctx, user_input)
    } else {
        user_input.to_string()
    }
};
```

- [ ] **Step 2: Verify compilation**

```bash
cargo check -p aletheon-runtime
```

- [ ] **Step 3: Commit**

```bash
git add crates/aletheon-runtime/src/impl/daemon/handler.rs
git commit -m "fix(runtime): inject DaseinContext into LLM prompts"
```

---

### Task 18: Add quick_mood_update() to DaseinModule

**Files:**
- Modify: `crates/aletheon-self/src/dasein/mod.rs`

- [ ] **Step 1: Add method**

```rust
pub fn quick_mood_update(&self, turn_text: &str) -> Stimmung {
    let mut mood = self.mood.write();
    let new_mood = if turn_text.contains("error") || turn_text.contains("failed") {
        Stimmung::Geknickt { because: "turn had errors".to_string() }
    } else if turn_text.contains("success") || turn_text.contains("completed") {
        Stimmung::Gelaunt { toward: "successful completion".to_string() }
    } else {
        mood.clone()
    };
    let changed = std::mem::discriminant(&*mood) != std::mem::discriminant(&new_mood);
    if changed {
        let old = mood.clone();
        *mood = new_mood.clone();
        let _ = self.event_tx.try_send(DaseinEvent::MoodShift {
            from: old,
            to: new_mood.clone(),
            reason: "quick_update_after_turn".to_string(),
        });
    }
    new_mood
}
```

- [ ] **Step 2: Add tests**

```rust
#[test]
fn test_quick_mood_update_error() {
    let (module, _rx) = DaseinModule::new();
    let mood = module.quick_mood_update("operation failed with error");
    assert!(matches!(mood, Stimmung::Geknickt { .. }));
}

#[test]
fn test_quick_mood_update_success() {
    let (module, _rx) = DaseinModule::new();
    let mood = module.quick_mood_update("task completed successfully");
    assert!(matches!(mood, Stimmung::Gelaunt { .. }));
}
```

- [ ] **Step 3: Run tests**

```bash
cargo test -p aletheon-self -- dasein
```

- [ ] **Step 4: Commit**

```bash
git add crates/aletheon-self/src/dasein/mod.rs
git commit -m "feat(self): add quick_mood_update() for fast-path mood transitions"
```

---

## Phase 7: Persistence

### Task 19: Implement DaseinModule Persistence (#8)

**Files:**
- Create: `crates/aletheon-self/src/dasein/persistence.rs`
- Modify: `crates/aletheon-self/src/dasein/mod.rs` — add module + mood_raw()
- Modify: `crates/aletheon-self/src/core/store.rs` — add conn() accessor

- [ ] **Step 1: Create persistence.rs**

```rust
// crates/aletheon-self/src/dasein/persistence.rs
use aletheon_abi::dasein::Stimmung;
use super::DaseinModule;
use crate::core::store::SelfFieldStore;
use rusqlite::params;

pub fn save_dasein_state(dasein: &DaseinModule, store: &SelfFieldStore) -> anyhow::Result<()> {
    let conn = store.conn();
    let mood_json = serde_json::to_string(&dasein.mood())?;
    conn.execute(
        "INSERT OR REPLACE INTO dasein_state (key, value, updated_at) VALUES (?1, ?2, datetime('now'))",
        params!["mood", mood_json],
    )?;
    tracing::info!("DaseinModule state saved");
    Ok(())
}

pub fn load_dasein_state(dasein: &DaseinModule, store: &SelfFieldStore) -> anyhow::Result<()> {
    let conn = store.conn();
    let mut stmt = conn.prepare("SELECT value FROM dasein_state WHERE key = ?1")?;
    let mut rows = stmt.query(params!["mood"])?;
    if let Some(row) = rows.next()? {
        let mood_json: String = row.get(0)?;
        let mood: Stimmung = serde_json::from_str(&mood_json)?;
        *dasein.mood_raw().write() = mood;
        tracing::info!("DaseinModule mood loaded from database");
    }
    Ok(())
}
```

- [ ] **Step 2: Add mood_raw() to DaseinModule**

```rust
// mod.rs:
pub fn mood_raw(&self) -> &RwLock<Stimmung> { &self.mood }
```

- [ ] **Step 3: Register module and add conn() accessor**

```rust
// mod.rs: pub mod persistence;
// store.rs: pub fn conn(&self) -> &rusqlite::Connection { &self.conn }
```

- [ ] **Step 4: Run tests**

```bash
cargo test -p aletheon-self -- dasein::persistence
```

- [ ] **Step 5: Commit**

```bash
git add crates/aletheon-self/src/dasein/persistence.rs crates/aletheon-self/src/dasein/mod.rs crates/aletheon-self/src/core/store.rs
git commit -m "feat(self): implement DaseinModule state persistence"
```

---

### Task 20: Load/Save on Init/Shutdown

**Files:**
- Modify: `crates/aletheon-self/src/core/mod.rs`

- [ ] **Step 1: Add load on init**

```rust
// core/mod.rs — in init(), after starting sorge loop:
if let (Some(ref dasein), Some(ref store)) = (&self.dasein, &self.store) {
    if let Err(e) = crate::dasein::persistence::load_dasein_state(dasein, store) {
        tracing::warn!("Failed to load DaseinModule state: {}", e);
    }
}
```

- [ ] **Step 2: Add save on shutdown**

```rust
// core/mod.rs — in shutdown(), before stopping sorge:
if let (Some(ref dasein), Some(ref store)) = (&self.dasein, &self.store) {
    if let Err(e) = crate::dasein::persistence::save_dasein_state(dasein, store) {
        tracing::warn!("Failed to save DaseinModule state: {}", e);
    }
}
```

- [ ] **Step 3: Verify compilation**

```bash
cargo check -p aletheon-self
```

- [ ] **Step 4: Commit**

```bash
git add crates/aletheon-self/src/core/mod.rs
git commit -m "feat(self): load/save DaseinModule state on init/shutdown"
```

---

## Phase 8: Decision Layer + Self-Observation

### Task 21: Create MetaCognition Module

**Files:**
- Create: `crates/aletheon-meta/src/core/meta_cognition.rs`
- Modify: `crates/aletheon-meta/src/core/mod.rs`

- [ ] **Step 1: Create meta_cognition.rs**

```rust
// crates/aletheon-meta/src/core/meta_cognition.rs
use std::sync::RwLock;
use aletheon_abi::dasein::{DaseinContext, DaseinEvent, Stimmung, BoredomDepth, AngstSource};
use aletheon_abi::brain::MutationIntent;
use tokio::sync::mpsc;

pub struct MetaCognition {
    system_state: RwLock<SystemState>,
    decisions: RwLock<Vec<EvolutionDecision>>,
    thresholds: MetaCognitionThresholds,
    dasein_tx: Option<mpsc::Sender<DaseinEvent>>,
}

#[derive(Debug, Clone)]
pub struct SystemState {
    pub mood: Stimmung,
    pub turn_count: usize,
    pub last_evolution_turn: usize,
}

#[derive(Debug, Clone)]
pub struct EvolutionDecision {
    pub turn: usize,
    pub action: EvolutionAction,
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone)]
pub enum EvolutionAction {
    Observe,
    TriggerEvolution { intents: Vec<MutationIntent> },
    AdjustDasein { parameter: String, value: f64 },
    InjectReflection { content: String },
}

#[derive(Debug, Clone)]
pub struct MetaCognitionThresholds {
    pub evolution_interval: usize,
}

impl Default for MetaCognitionThresholds {
    fn default() -> Self { Self { evolution_interval: 20 } }
}

impl MetaCognition {
    pub fn new(dasein_tx: Option<mpsc::Sender<DaseinEvent>>) -> Self {
        Self {
            system_state: RwLock::new(SystemState {
                mood: Stimmung::Gelassenheit,
                turn_count: 0,
                last_evolution_turn: 0,
            }),
            decisions: RwLock::new(Vec::new()),
            thresholds: MetaCognitionThresholds::default(),
            dasein_tx,
        }
    }

    pub fn decide(&self, ctx: &DaseinContext, turn: usize) -> EvolutionAction {
        let mut state = self.system_state.write().unwrap();
        state.turn_count = turn;
        state.mood = ctx.mood.clone();

        let action = match &ctx.mood {
            Stimmung::Angst { facing } => EvolutionAction::TriggerEvolution {
                intents: vec![MutationIntent {
                    target: "care.priorities".to_string(),
                    action: "adjust".to_string(),
                    reason: format!("Angst: {:?}", facing),
                    magnitude: 0.1,
                }],
            },
            Stimmung::Langeweile { depth: BoredomDepth::Deep } => EvolutionAction::AdjustDasein {
                parameter: "curiosity_weight".to_string(),
                value: 0.8,
            },
            Stimmung::Neugier { curiosity_about } => EvolutionAction::InjectReflection {
                content: format!("Explore: {}", curiosity_about),
            },
            _ => {
                if turn - state.last_evolution_turn >= self.thresholds.evolution_interval {
                    state.last_evolution_turn = turn;
                    EvolutionAction::TriggerEvolution { intents: vec![] }
                } else {
                    EvolutionAction::Observe
                }
            }
        };

        self.decisions.write().unwrap().push(EvolutionDecision {
            turn,
            action: action.clone(),
            timestamp: chrono::Utc::now(),
        });

        action
    }
}
```

- [ ] **Step 2: Register in mod.rs**

```rust
// crates/aletheon-meta/src/core/mod.rs:
pub mod meta_cognition;
pub use meta_cognition::{MetaCognition, EvolutionAction, EvolutionDecision, SystemState};
```

- [ ] **Step 3: Verify compilation**

```bash
cargo check -p aletheon-meta
```

- [ ] **Step 4: Commit**

```bash
git add crates/aletheon-meta/src/core/meta_cognition.rs crates/aletheon-meta/src/core/mod.rs
git commit -m "feat(meta): add MetaCognition decision module"
```

---

### Task 22: Create self_observe Tool

**Files:**
- Create: `crates/aletheon-runtime/src/tools/self_observe.rs`

- [ ] **Step 1: Create tool**

```rust
// crates/aletheon-runtime/src/tools/self_observe.rs
use std::sync::Arc;
use aletheon_abi::dasein::DaseinOps;
use serde_json::json;

pub struct SelfObserveTool<T: DaseinOps> {
    dasein: Arc<T>,
}

impl<T: DaseinOps> SelfObserveTool<T> {
    pub fn new(dasein: Arc<T>) -> Self { Self { dasein } }

    pub fn definition(&self) -> serde_json::Value {
        json!({
            "name": "self_observe",
            "description": "Observe your own internal state: mood, experiences, world, self-model, care.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "enum": ["mood", "temporality", "world", "self_model", "care", "full"],
                        "description": "What to observe"
                    }
                },
                "required": ["query"]
            }
        })
    }

    pub fn execute(&self, input: &serde_json::Value) -> String {
        let query = input["query"].as_str().unwrap_or("full");
        let ctx = self.dasein.to_context_injection();
        match query {
            "mood" => format!("Mood: {:?}", ctx.mood),
            "temporality" => format!("Retentions: {}, Protentions: {}",
                ctx.temporality.recent_retentions.len(), ctx.temporality.protentions.len()),
            "world" => format!("Ready: {}, PresentAtHand: {}, Unavailable: {}",
                ctx.world.ready_to_hand.len(), ctx.world.present_at_hand.len(), ctx.world.unavailable.len()),
            "self_model" => format!("Assertions: {}, Negated: {}, Possibilities: {}",
                ctx.self_model.current_assertions.len(), ctx.self_model.negated_assertions.len(), ctx.self_model.possibilities.len()),
            "care" => format!("Concerns: {}, Fallenness: {:.2}, Rhythm: {}ms",
                ctx.care.concerns.len(), ctx.care.fallenness_depth, ctx.care.rhythm_interval_ms),
            "full" => format!("{:#?}", ctx),
            _ => format!("Unknown query: {}", query),
        }
    }
}
```

- [ ] **Step 2: Verify compilation**

```bash
cargo check -p aletheon-runtime
```

- [ ] **Step 3: Commit**

```bash
git add crates/aletheon-runtime/src/tools/self_observe.rs
git commit -m "feat(runtime): add self_observe tool for LLM self-observation"
```

---

### Task 23: Wire RequestHandler as Coordinator

**Files:**
- Modify: `crates/aletheon-runtime/src/impl/daemon/handler.rs`

- [ ] **Step 1: Add coordinate() method**

```rust
async fn coordinate(&self, self_field: &SelfField, turn: usize, turn_text: &str) {
    // Quick mood update
    if let Some(ref dasein) = self_field.dasein() {
        dasein.quick_mood_update(turn_text);
    }

    // MetaCognition decides
    if let (Some(ref mc), Some(ref dasein)) = (&self.meta_cognition, self_field.dasein()) {
        let ctx = dasein.to_context_injection();
        let action = mc.decide(&ctx, turn);
        match action {
            EvolutionAction::TriggerEvolution { .. } => {
                tracing::info!("MetaCognition: triggering evolution");
            }
            EvolutionAction::AdjustDasein { parameter, value } => {
                tracing::info!("MetaCognition: adjusting {}={}", parameter, value);
            }
            EvolutionAction::InjectReflection { content } => {
                tracing::info!("MetaCognition: injecting reflection: {}", content);
            }
            EvolutionAction::Observe => {}
        }
    }
}
```

- [ ] **Step 2: Call coordinate() after each turn**

In handle_request(), after the ReAct loop:
```rust
self.coordinate(&self_field, turn_count, &final_response).await;
```

- [ ] **Step 3: Verify compilation**

```bash
cargo check -p aletheon-runtime
```

- [ ] **Step 4: Commit**

```bash
git add crates/aletheon-runtime/src/impl/daemon/handler.rs
git commit -m "feat(runtime): wire RequestHandler as Self-Brain-Runtime coordinator"
```

---

## Phase 9: Final Verification

### Task 24: Full Workspace Verification

- [ ] **Step 1: Run full workspace check**

```bash
cargo check --workspace
```

Expected: No errors.

- [ ] **Step 2: Run full workspace tests**

```bash
cargo test --workspace
```

Expected: All tests pass.

- [ ] **Step 3: Run clippy**

```bash
cargo clippy --workspace -- -D warnings
```

Expected: No clippy warnings.

- [ ] **Step 4: Final commit**

```bash
git add -A
git commit -m "feat: Dasein self-awareness — complete implementation

Phase 1: Data Layer
- ContentBlock::Thinking for LLM thinking capture
- DaseinEvent::ThinkingObserved/ReasoningObserved/KnowledgeAsserted
- StreamChunk::ThinkingDelta

Phase 2: Runtime Fixes
- Fix #1: Enable DaseinModule (enable_dasein=true)
- Fix #2: SorgeLoop writes mood back
- Fix #6: ProtentionField updated from patterns
- Fix #10: set_decay_rate() implemented

Phase 3: Provider Layer
- Anthropic preserves thinking blocks
- OpenAI preserves reasoning_content as Thinking

Phase 4: Flow Layer
- SorgeLoop handles all DaseinEvent variants
- Auto-fill Bewandtnis/SelfModel/CareStructure from events

Phase 5: BrainCore
- think_with_stimmung() for mood-aware reasoning
- generate_plan_with_stimmung() for mood-aware planning

Phase 6: Wiring
- DaseinEventBridge wired to EventBus
- DaseinContext injected into LLM prompts
- quick_mood_update() for fast-path transitions

Phase 7: Persistence
- DaseinModule state saved/loaded via dasein_state table

Phase 8: Decision + Self-Observation
- MetaCognition decision module
- self_observe tool
- RequestHandler coordinator

Full self-awareness loop operational:
LLM thinking → temporal → mood → reasoning → action → consequence → loop"
```

---

## Execution Summary

| Task | Phase | Crate | Description |
|---|---|---|---|
| 1 | Data | aletheon-abi | ContentBlock::Thinking |
| 2 | Data | aletheon-abi | DaseinEvent thinking variants |
| 3 | Data | aletheon-brain | StreamChunk::ThinkingDelta |
| 4 | Fix | aletheon-self | set_decay_rate (#10) |
| 5 | Fix | aletheon-runtime | enable_dasein (#1) |
| 6 | Fix | aletheon-self | mood sync (#2) |
| 7 | Fix | aletheon-self | protention update (#6) |
| 8 | Provider | aletheon-brain | Anthropic thinking blocks |
| 9 | Provider | aletheon-brain | OpenAI reasoning_content |
| 10 | Flow | aletheon-self | SorgeLoop all events |
| 11 | Flow | aletheon-self | Bewandtnis auto-fill |
| 12 | Flow | aletheon-self | CareStructure auto-fill |
| 13 | Flow | aletheon-runtime | ReAct loop Thinking |
| 14 | Brain | aletheon-brain | think_with_stimmung |
| 15 | Brain | aletheon-brain | generate_plan_with_stimmung |
| 16 | Wire | aletheon-runtime | EventBridge wiring (#7) |
| 17 | Wire | aletheon-runtime | Context injection (#9) |
| 18 | Wire | aletheon-self | quick_mood_update |
| 19 | Persist | aletheon-self | Persistence module |
| 20 | Persist | aletheon-self | Init/shutdown hooks |
| 21 | Decision | aletheon-meta | MetaCognition |
| 22 | Decision | aletheon-runtime | self_observe tool |
| 23 | Decision | aletheon-runtime | RequestHandler coordinator |
| 24 | Final | all | Full verification |
