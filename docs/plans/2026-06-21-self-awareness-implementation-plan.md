# Self-Awareness Architecture Implementation Plan

> **For agentic workers:** Use `workflow-feature` or `writing-plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Close all 7 broken feedback loops between Self, Brain, and Runtime by capturing LLM thinking, injecting mood into reasoning, executing care actions, and wiring MetaCognition.

**Architecture:** Bottom-up approach — data layer first (ContentBlock + DaseinEvent), then provider layer (capture thinking), then flow layer (Sorge dual-speed + mood injection), then decision layer (MetaCognition), then self-observation (self_observe tool). Each task builds on the previous.

**Tech Stack:** Rust, tokio, serde, mpsc channels, RwLock

**Spec:** `docs/plans/2026-06-21-self-awareness-architecture-design.md`

---

## Task 1: Add `ContentBlock::Thinking` Variant

**Files:**
- Modify: `crates/aletheon-abi/src/message.rs:9-32`
- Test: `crates/aletheon-abi/src/message.rs` (inline tests)

- [ ] **Step 1: Add Thinking variant to ContentBlock**

```rust
// crates/aletheon-abi/src/message.rs — add after Text variant
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentBlock {
    Text {
        text: String,
    },
    /// LLM thinking/reasoning content (extended thinking, chain-of-thought)
    Thinking {
        text: String,
        /// Anthropic thinking signature for multi-turn verification
        signature: Option<String>,
    },
    // ... rest unchanged
```

- [ ] **Step 2: Update estimate_chars to return 0 for Thinking**

Find the `estimate_chars()` method on `ContentBlock` and add:

```rust
ContentBlock::Thinking { .. } => 0, // Internal, not counted
```

- [ ] **Step 3: Add test for Thinking serialization**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_thinking_block_serde_roundtrip() {
        let block = ContentBlock::Thinking {
            text: "I need to think about this carefully...".to_string(),
            signature: Some("sig_abc123".to_string()),
        };
        let json = serde_json::to_string(&block).unwrap();
        assert!(json.contains("thinking"));
        let deserialized: ContentBlock = serde_json::from_str(&json).unwrap();
        match deserialized {
            ContentBlock::Thinking { text, signature } => {
                assert_eq!(text, "I need to think about this carefully...");
                assert_eq!(signature, Some("sig_abc123".to_string()));
            }
            _ => panic!("Expected Thinking variant"),
        }
    }

    #[test]
    fn test_thinking_block_estimate_chars_zero() {
        let block = ContentBlock::Thinking {
            text: "This is a long thinking block...".to_string(),
            signature: None,
        };
        assert_eq!(block.estimate_chars(), 0);
    }
}
```

- [ ] **Step 4: Run ABI tests**

```bash
cargo test -p aletheon-abi
```

Expected: All tests pass, including new Thinking tests.

- [ ] **Step 5: Commit**

```bash
git add crates/aletheon-abi/src/message.rs
git commit -m "feat(abi): add ContentBlock::Thinking variant for LLM thinking capture"
```

---

## Task 2: Add Thinking-Related DaseinEvent Variants

**Files:**
- Modify: `crates/aletheon-abi/src/dasein.rs:261-291`
- Test: `crates/aletheon-abi/src/dasein.rs` (inline tests)

- [ ] **Step 1: Add new DaseinEvent variants**

```rust
// crates/aletheon-abi/src/dasein.rs — add to DaseinEvent enum
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum DaseinEvent {
    // External events
    UserInput { content: String },
    SystemEvent { source: String, content: String },
    TimerTick,

    // NEW: LLM thinking events
    /// Observed LLM thinking/extended-thinking content
    ThinkingObserved { text: String, turn: usize },
    /// Observed LLM reasoning text (accompanies tool calls)
    ReasoningObserved { text: String, turn: usize, has_tool_calls: bool },
    /// LLM asserted knowledge (extracted from reasoning)
    KnowledgeAsserted { assertions: Vec<String>, confidence: f64 },

    // Internal events
    NegationCompleted { target: String, new_possibilities: Vec<String> },
    MoodShift { from: Stimmung, to: Stimmung, reason: String },
    BewandtnisChange { entity_id: String, old_state: ReadinessState, new_state: ReadinessState },
    TemporalEvent { kind: TemporalEventKind, content: String },
}
```

- [ ] **Step 2: Add test for new event serialization**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_thinking_observed_event_serde() {
        let event = DaseinEvent::ThinkingObserved {
            text: "Let me reason through this...".to_string(),
            turn: 42,
        };
        let json = serde_json::to_string(&event).unwrap();
        let deserialized: DaseinEvent = serde_json::from_str(&json).unwrap();
        match deserialized {
            DaseinEvent::ThinkingObserved { text, turn } => {
                assert_eq!(text, "Let me reason through this...");
                assert_eq!(turn, 42);
            }
            _ => panic!("Expected ThinkingObserved"),
        }
    }

    #[test]
    fn test_reasoning_observed_event_serde() {
        let event = DaseinEvent::ReasoningObserved {
            text: "I should use the file tool because...".to_string(),
            turn: 5,
            has_tool_calls: true,
        };
        let json = serde_json::to_string(&event).unwrap();
        let deserialized: DaseinEvent = serde_json::from_str(&json).unwrap();
        match deserialized {
            DaseinEvent::ReasoningObserved { text, turn, has_tool_calls } => {
                assert_eq!(turn, 5);
                assert!(has_tool_calls);
            }
            _ => panic!("Expected ReasoningObserved"),
        }
    }

    #[test]
    fn test_knowledge_asserted_event_serde() {
        let event = DaseinEvent::KnowledgeAsserted {
            assertions: vec!["Rust uses ownership".to_string()],
            confidence: 0.95,
        };
        let json = serde_json::to_string(&event).unwrap();
        let deserialized: DaseinEvent = serde_json::from_str(&json).unwrap();
        match deserialized {
            DaseinEvent::KnowledgeAsserted { assertions, confidence } => {
                assert_eq!(assertions.len(), 1);
                assert!((confidence - 0.95).abs() < 0.001);
            }
            _ => panic!("Expected KnowledgeAsserted"),
        }
    }
}
```

- [ ] **Step 3: Run ABI tests**

```bash
cargo test -p aletheon-abi
```

Expected: All tests pass.

- [ ] **Step 4: Commit**

```bash
git add crates/aletheon-abi/src/dasein.rs
git commit -m "feat(abi): add ThinkingObserved/ReasoningObserved/KnowledgeAsserted events"
```

---

## Task 3: Add `StreamChunk::ThinkingDelta` Variant

**Files:**
- Modify: `crates/aletheon-brain/src/impl/llm/provider.rs:9-29`

- [ ] **Step 1: Add ThinkingDelta variant**

```rust
// crates/aletheon-brain/src/impl/llm/provider.rs
#[derive(Debug, Clone)]
pub enum StreamChunk {
    /// Text content delta
    TextDelta { text: String },
    /// Thinking/reasoning content delta (extended thinking)
    ThinkingDelta { text: String },
    /// Tool use start (name + id)
    ToolUseStart { id: String, name: String },
    /// Tool use input delta (partial JSON)
    ToolUseDelta { id: String, delta: String },
    /// Tool use complete
    ToolUseComplete {
        id: String,
        input: serde_json::Value,
    },
    /// Usage update
    Usage {
        input_tokens: u32,
        output_tokens: u32,
    },
    /// Stream complete
    Done { stop_reason: StopReason },
}
```

- [ ] **Step 2: Verify compilation**

```bash
cargo check -p aletheon-brain
```

Expected: Compiles without errors. Any match on StreamChunk will get a "non-exhaustive patterns" warning that will be fixed when providers are updated.

- [ ] **Step 3: Commit**

```bash
git add crates/aletheon-brain/src/impl/llm/provider.rs
git commit -m "feat(brain): add StreamChunk::ThinkingDelta variant"
```

---

## Task 4: Preserve Thinking Blocks in Anthropic Provider

**Files:**
- Modify: `crates/aletheon-brain/src/impl/llm/anthropic.rs:260-264`

- [ ] **Step 1: Change thinking block handling**

```rust
// crates/aletheon-brain/src/impl/llm/anthropic.rs
// Change FROM (lines 260-264):
                "thinking" => {
                    // Skip thinking blocks (extended thinking)
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

Find the streaming parser's `content_block_delta` handler. Add handling for `thinking_delta` events:

```rust
// In the streaming content_block_delta match:
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

Expected: Compiles without errors.

- [ ] **Step 4: Commit**

```bash
git add crates/aletheon-brain/src/impl/llm/anthropic.rs
git commit -m "feat(brain): preserve thinking blocks in Anthropic provider instead of discarding"
```

---

## Task 5: Preserve Reasoning Content in OpenAI Provider

**Files:**
- Modify: `crates/aletheon-brain/src/impl/llm/openai_provider.rs:379-385`

- [ ] **Step 1: Change reasoning_content handling**

```rust
// crates/aletheon-brain/src/impl/llm/openai_provider.rs
// Change FROM (lines 379-385):
        // Text content — prefer `content`, fall back to `reasoning_content`
        // (some reasoning models like GLM-5.2 put output in reasoning_content)
        let text = choice.message.content.filter(|s| !s.is_empty())
            .or(choice.message.reasoning_content.filter(|s| !s.is_empty()));
        if let Some(text) = text {
            content.push(ContentBlock::Text { text });
        }

// Change TO:
        // Reasoning content (chain-of-thought from reasoning models)
        if let Some(thinking) = choice.message.reasoning_content.filter(|s| !s.is_empty()) {
            content.push(ContentBlock::Thinking { text: thinking, signature: None });
        }
        // Regular text content
        if let Some(text) = choice.message.content.filter(|s| !s.is_empty()) {
            content.push(ContentBlock::Text { text });
        }
```

- [ ] **Step 2: Verify compilation**

```bash
cargo check -p aletheon-brain
```

Expected: Compiles without errors.

- [ ] **Step 3: Commit**

```bash
git add crates/aletheon-brain/src/impl/llm/openai_provider.rs
git commit -m "feat(brain): preserve reasoning_content as ContentBlock::Thinking in OpenAI provider"
```

---

## Task 6: Process Thinking Blocks in ReAct Loop

**Files:**
- Modify: `crates/aletheon-runtime/src/core/react_loop.rs:427-437`
- Test: `crates/aletheon-runtime/src/core/react_loop.rs` (existing tests)

- [ ] **Step 1: Add thinking_parts accumulator and Thinking block handling**

```rust
// crates/aletheon-runtime/src/core/react_loop.rs
// Change FROM (lines 427-437):
            let mut text_parts = Vec::new();
            let mut tool_calls = Vec::new();
            for block in &response.content {
                match block {
                    ContentBlock::Text { text } => text_parts.push(text.clone()),
                    ContentBlock::ToolUse { id, name, input } => {
                        tool_calls.push((id.clone(), name.clone(), input.clone()));
                    }
                    _ => {}
                }
            }

// Change TO:
            let mut text_parts = Vec::new();
            let mut thinking_parts = Vec::new();
            let mut tool_calls = Vec::new();
            for block in &response.content {
                match block {
                    ContentBlock::Text { text } => text_parts.push(text.clone()),
                    ContentBlock::Thinking { text, .. } => {
                        thinking_parts.push(text.clone());
                        // Send to DaseinModule if available
                        if let Some(tx) = &self.dasein_tx {
                            let _ = tx.try_send(DaseinEvent::ThinkingObserved {
                                text: text.clone(),
                                turn: self.turn_count,
                            });
                        }
                    }
                    ContentBlock::ToolUse { id, name, input } => {
                        tool_calls.push((id.clone(), name.clone(), input.clone()));
                    }
                    _ => {}
                }
            }
```

- [ ] **Step 2: Add dasein_tx field to ReActLoop**

Find the `ReActLoop` struct definition and add:

```rust
pub struct ReActLoop {
    // ... existing fields ...
    /// Channel to send events to DaseinModule (optional)
    pub dasein_tx: Option<mpsc::Sender<DaseinEvent>>,
}
```

- [ ] **Step 3: Send ReasoningObserved when text accompanies tool calls**

After the block processing loop, if there are both text_parts and tool_calls:

```rust
            // If text accompanies tool calls, it's reasoning
            if !text_parts.is_empty() && !tool_calls.is_empty() {
                let reasoning_text = text_parts.join("\n");
                if let Some(tx) = &self.dasein_tx {
                    let _ = tx.try_send(DaseinEvent::ReasoningObserved {
                        text: reasoning_text.clone(),
                        turn: self.turn_count,
                        has_tool_calls: true,
                    });
                }
            }
```

- [ ] **Step 4: Verify compilation**

```bash
cargo check -p aletheon-runtime
```

Expected: Compiles without errors.

- [ ] **Step 5: Run existing tests**

```bash
cargo test -p aletheon-runtime
```

Expected: Existing tests pass. Some may need updates for the new `dasein_tx` field (pass `None`).

- [ ] **Step 6: Commit**

```bash
git add crates/aletheon-runtime/src/core/react_loop.rs
git commit -m "feat(runtime): process ContentBlock::Thinking in ReAct loop, send to DaseinModule"
```

---

## Task 7: Process All DaseinEvent Variants in SorgeLoop

**Files:**
- Modify: `crates/aletheon-self/src/dasein/sorge.rs:79-106`

- [ ] **Step 1: Replace `_ => continue` with full event handling**

```rust
// crates/aletheon-self/src/dasein/sorge.rs
// Change FROM (lines 79-106):
                        DaseinEvent::UserInput { content } => {
                            ExperientialContent {
                                semantic: content.clone(),
                                action: Some("user_interaction".to_string()),
                                perception: None,
                                negation: None,
                            }
                        }
                        DaseinEvent::SystemEvent { source, content } => {
                            ExperientialContent {
                                semantic: format!("[{}] {}", source, content),
                                action: None,
                                perception: Some(content.clone()),
                                negation: None,
                            }
                        }
                        DaseinEvent::TimerTick => {
                            ExperientialContent {
                                semantic: "tick".to_string(),
                                action: None,
                                perception: None,
                                negation: None,
                            }
                        }
                        _ => continue,

// Change TO:
                        DaseinEvent::UserInput { content } => {
                            ExperientialContent {
                                semantic: content.clone(),
                                action: Some("user_interaction".to_string()),
                                perception: None,
                                negation: None,
                            }
                        }
                        DaseinEvent::SystemEvent { source, content } => {
                            ExperientialContent {
                                semantic: format!("[{}] {}", source, content),
                                action: None,
                                perception: Some(content.clone()),
                                negation: None,
                            }
                        }
                        DaseinEvent::TimerTick => {
                            ExperientialContent {
                                semantic: "tick".to_string(),
                                action: None,
                                perception: None,
                                negation: None,
                            }
                        }
                        // NEW: LLM thinking events — high vividness
                        DaseinEvent::ThinkingObserved { text, turn } => {
                            ExperientialContent {
                                semantic: format!("[thinking:turn_{}]", turn),
                                action: Some(format!("llm_thinking_turn_{}", turn)),
                                perception: Some(text.clone()),
                                negation: None,
                            }
                        }
                        // NEW: LLM reasoning events — moderate vividness
                        DaseinEvent::ReasoningObserved { text, turn, has_tool_calls } => {
                            ExperientialContent {
                                semantic: format!("[reasoning:turn_{}]", turn),
                                action: Some(format!("llm_reasoning_turn_{}", turn)),
                                perception: Some(text.clone()),
                                negation: None,
                            }
                        }
                        // NEW: Knowledge assertions — update self_model
                        DaseinEvent::KnowledgeAsserted { assertions, confidence } => {
                            for assertion in &assertions {
                                self_model.assert(assertion.clone(), AssertionSource::Discovered);
                            }
                            ExperientialContent {
                                semantic: format!("[knowledge:{}] conf={:.2}", assertions.join(","), confidence),
                                action: Some("knowledge_assertion".to_string()),
                                perception: None,
                                negation: None,
                            }
                        }
                        // Handle internal events
                        DaseinEvent::NegationCompleted { target, new_possibilities } => {
                            for p in &new_possibilities {
                                self_model.add_possibility(p.clone(), 0.5, 0.5);
                            }
                            ExperientialContent {
                                semantic: format!("[negation:{}]", target),
                                action: Some("negation_completed".to_string()),
                                perception: None,
                                negation: Some(target.clone()),
                            }
                        }
                        DaseinEvent::MoodShift { from, to, reason } => {
                            ExperientialContent {
                                semantic: format!("[mood_shift:{:?}->{:?}]", from, to),
                                action: Some("mood_transition".to_string()),
                                perception: Some(reason.clone()),
                                negation: None,
                            }
                        }
                        DaseinEvent::BewandtnisChange { entity_id, old_state, new_state } => {
                            world.update_readiness(entity_id, new_state.clone());
                            ExperientialContent {
                                semantic: format!("[bewandtnis:{}]", entity_id),
                                action: Some("world_change".to_string()),
                                perception: None,
                                negation: None,
                            }
                        }
                        DaseinEvent::TemporalEvent { kind, content } => {
                            ExperientialContent {
                                semantic: format!("[temporal:{:?}]", kind),
                                action: Some("temporal_event".to_string()),
                                perception: Some(content.clone()),
                                negation: None,
                            }
                        }
```

- [ ] **Step 2: Verify compilation**

```bash
cargo check -p aletheon-self
```

Expected: Compiles without errors. May need to add `use` imports for `AssertionSource`.

- [ ] **Step 3: Run dasein tests**

```bash
cargo test -p aletheon-self
```

Expected: All existing dasein tests pass.

- [ ] **Step 4: Commit**

```bash
git add crates/aletheon-self/src/dasein/sorge.rs
git commit -m "feat(self): process all DaseinEvent variants in SorgeLoop"
```

---

## Task 8: Add `think_with_stimmung()` to BrainCore Reasoner

**Files:**
- Modify: `crates/aletheon-brain/src/core/reasoner.rs`

- [ ] **Step 1: Add Stimmung import**

```rust
use aletheon_abi::dasein::Stimmung;
```

- [ ] **Step 2: Add think_with_stimmung method**

```rust
impl Reasoner {
    /// Generate reasoning with mood-aware strategy selection
    pub fn think_with_stimmung(&self, intent: &str, stimmung: &Stimmung) -> String {
        let strategy = match stimmung {
            Stimmung::Angst { facing } => {
                format!("Proceed with caution. Existential concern: {:?}", facing)
            }
            Stimmung::Neugier { curiosity_about } => {
                format!("Explore freely. Curious about: {}", curiosity_about)
            }
            Stimmung::Langeweile { depth } => {
                format!("Question assumptions. Boredom depth: {:?}", depth)
            }
            Stimmung::Entschlossenheit { chosen_possibility } => {
                format!("Act decisively. Chosen: {}", chosen_possibility)
            }
            Stimmung::Verfallenheit { absorbed_in } => {
                format!("Currently absorbed in: {}. Consider stepping back.", absorbed_in)
            }
            _ => "Balanced approach.".to_string(),
        };
        format!("Strategy: {}\nIntent: {}", strategy, intent)
    }
}
```

- [ ] **Step 3: Add test**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_think_with_stimmung_angst() {
        let reasoner = Reasoner::new(/* ... */);
        let stimmung = Stimmung::Angst { facing: AngstSource::Finitude };
        let result = reasoner.think_with_stimmung("read file", &stimmung);
        assert!(result.contains("caution"));
        assert!(result.contains("read file"));
    }

    #[test]
    fn test_think_with_stimmung_neugier() {
        let reasoner = Reasoner::new(/* ... */);
        let stimmung = Stimmung::Neugier { curiosity_about: "Rust macros".to_string() };
        let result = reasoner.think_with_stimmung("explore code", &stimmung);
        assert!(result.contains("Explore freely"));
    }
}
```

- [ ] **Step 4: Run brain tests**

```bash
cargo test -p aletheon-brain
```

Expected: All tests pass.

- [ ] **Step 5: Commit**

```bash
git add crates/aletheon-brain/src/core/reasoner.rs
git commit -m "feat(brain): add think_with_stimmung() for mood-aware reasoning"
```

---

## Task 9: Add `generate_plan_with_stimmung()` to BrainCore Planner

**Files:**
- Modify: `crates/aletheon-brain/src/core/planner.rs`

- [ ] **Step 1: Add Stimmung import and risk adjustment function**

```rust
use aletheon_abi::dasein::Stimmung;

/// Adjust risk tolerance based on current mood
fn adjust_risk_for_stimmung(stimmung: &Stimmung) -> f64 {
    match stimmung {
        Stimmung::Angst { .. } => 0.3,           // Conservative
        Stimmung::Neugier { .. } => 0.8,         // Explorative
        Stimmung::Entschlossenheit { .. } => 0.9, // Bold
        Stimmung::Langeweile { .. } => 0.7,      // Moderate-risk
        Stimmung::Verfallenheit { .. } => 0.4,   // Cautious
        _ => 0.5,                                 // Balanced
    }
}
```

- [ ] **Step 2: Add generate_plan_with_stimmung method**

```rust
impl Planner {
    /// Generate a plan with mood-aware risk assessment
    pub fn generate_plan_with_stimmung(
        &self,
        intent: &str,
        reasoning: &str,
        stimmung: &Stimmung,
    ) -> Plan {
        let risk_tolerance = adjust_risk_for_stimmung(stimmung);
        let mut plan = self.generate_plan(intent, reasoning);
        // Adjust plan's risk metadata
        plan.risk_tolerance = risk_tolerance;
        plan
    }
}
```

- [ ] **Step 3: Verify compilation**

```bash
cargo check -p aletheon-brain
```

Expected: Compiles without errors.

- [ ] **Step 4: Commit**

```bash
git add crates/aletheon-brain/src/core/planner.rs
git commit -m "feat(brain): add generate_plan_with_stimmung() for mood-aware planning"
```

---

## Task 10: Add `quick_mood_update()` to DaseinModule

**Files:**
- Modify: `crates/aletheon-self/src/dasein/mod.rs`

- [ ] **Step 1: Add quick_mood_update method**

```rust
impl DaseinModule {
    /// Quick synchronous mood update based on turn result.
    /// Called by the fast path after each ReAct loop turn.
    pub fn quick_mood_update(&self, turn_text: &str) -> Stimmung {
        let mut mood = self.mood.write();

        // Detect mood signals from the turn text
        let new_mood = if turn_text.contains("error") || turn_text.contains("failed") {
            Stimmung::Geknickt {
                because: "turn had errors".to_string(),
            }
        } else if turn_text.contains("success") || turn_text.contains("completed") {
            Stimmung::Gelaunt {
                toward: "successful completion".to_string(),
            }
        } else {
            // Keep current mood
            mood.clone()
        };

        let changed = std::mem::discriminant(&*mood) != std::mem::discriminant(&new_mood);
        if changed {
            let old = mood.clone();
            *mood = new_mood.clone();
            // Send mood shift event
            let _ = self.event_tx.try_send(DaseinEvent::MoodShift {
                from: old,
                to: new_mood.clone(),
                reason: "quick_update_after_turn".to_string(),
            });
        }

        new_mood
    }
}
```

- [ ] **Step 2: Add test**

```rust
#[test]
fn test_quick_mood_update_error() {
    let (module, _rx) = DaseinModule::new();
    let new_mood = module.quick_mood_update("The operation failed with an error");
    match new_mood {
        Stimmung::Geknickt { because } => assert!(because.contains("errors")),
        _ => panic!("Expected Geknickt mood"),
    }
}

#[test]
fn test_quick_mood_update_success() {
    let (module, _rx) = DaseinModule::new();
    let new_mood = module.quick_mood_update("Task completed successfully");
    match new_mood {
        Stimmung::Gelaunt { toward } => assert!(toward.contains("successful")),
        _ => panic!("Expected Gelaunt mood"),
    }
}
```

- [ ] **Step 3: Run self tests**

```bash
cargo test -p aletheon-self
```

Expected: All tests pass.

- [ ] **Step 4: Commit**

```bash
git add crates/aletheon-self/src/dasein/mod.rs
git commit -m "feat(self): add quick_mood_update() for fast-path mood transitions"
```

---

## Task 11: Create MetaCognition Module in aletheon-meta

**Files:**
- Create: `crates/aletheon-meta/src/core/meta_cognition.rs`
- Modify: `crates/aletheon-meta/src/core/mod.rs`

- [ ] **Step 1: Create meta_cognition.rs**

```rust
// crates/aletheon-meta/src/core/meta_cognition.rs
use std::sync::RwLock;
use aletheon_abi::dasein::{DaseinContext, DaseinEvent, Stimmung};
use aletheon_abi::brain::MutationIntent;
use tokio::sync::mpsc;

/// MetaCognition observes the system's existential state and decides
/// when/why/how to evolve. It sits above the Morphogenesis Pipeline.
pub struct MetaCognition {
    /// Current system state snapshot
    system_state: RwLock<SystemState>,
    /// Decision history
    decisions: RwLock<Vec<EvolutionDecision>>,
    /// Configuration thresholds
    thresholds: MetaCognitionThresholds,
    /// Channel to send events to DaseinModule
    dasein_tx: Option<mpsc::Sender<DaseinEvent>>,
}

#[derive(Debug, Clone)]
pub struct SystemState {
    pub mood: Stimmung,
    pub turn_count: usize,
    pub last_evolution_turn: usize,
    pub self_coherence: f64,
}

#[derive(Debug, Clone)]
pub struct EvolutionDecision {
    pub turn: usize,
    pub mood: Stimmung,
    pub reason: String,
    pub action: EvolutionAction,
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone)]
pub enum EvolutionAction {
    /// Don't evolve, keep observing
    Observe,
    /// Trigger genome evolution
    TriggerEvolution { intents: Vec<MutationIntent> },
    /// Adjust DaseinModule parameters
    AdjustDasein { parameter: String, value: f64 },
    /// Inject reflection into BrainCore
    InjectReflection { content: String },
}

#[derive(Debug, Clone)]
pub struct MetaCognitionThresholds {
    /// How many turns between periodic evolution checks
    pub evolution_interval: usize,
    /// Mood coherence threshold (below this = crisis)
    pub coherence_threshold: f64,
}

impl Default for MetaCognitionThresholds {
    fn default() -> Self {
        Self {
            evolution_interval: 20,
            coherence_threshold: 0.3,
        }
    }
}

impl MetaCognition {
    pub fn new(dasein_tx: Option<mpsc::Sender<DaseinEvent>>) -> Self {
        Self {
            system_state: RwLock::new(SystemState {
                mood: Stimmung::Gelassenheit,
                turn_count: 0,
                last_evolution_turn: 0,
                self_coherence: 1.0,
            }),
            decisions: RwLock::new(Vec::new()),
            thresholds: MetaCognitionThresholds::default(),
            dasein_tx,
        }
    }

    /// Make an evolution decision based on current DaseinContext
    pub fn decide(&self, ctx: &DaseinContext, turn: usize) -> EvolutionAction {
        let mut state = self.system_state.write().unwrap();
        state.turn_count = turn;
        state.mood = ctx.mood.clone();

        let action = match &ctx.mood {
            // Angst = existential crisis → force evolution
            Stimmung::Angst { facing } => {
                EvolutionAction::TriggerEvolution {
                    intents: vec![MutationIntent {
                        target: "care.priorities".to_string(),
                        action: "adjust".to_string(),
                        reason: format!("Angst crisis: {:?}", facing),
                        magnitude: 0.1,
                    }],
                }
            }
            // Deep boredom → adjust parameters
            Stimmung::Langeweile { depth: aletheon_abi::dasein::BoredomDepth::Deep } => {
                EvolutionAction::AdjustDasein {
                    parameter: "curiosity_weight".to_string(),
                    value: 0.8,
                }
            }
            // Curiosity → inject reflection
            Stimmung::Neugier { curiosity_about } => {
                EvolutionAction::InjectReflection {
                    content: format!("Explore and reflect on: {}", curiosity_about),
                }
            }
            // Periodic check
            _ => {
                if turn - state.last_evolution_turn >= self.thresholds.evolution_interval {
                    state.last_evolution_turn = turn;
                    EvolutionAction::TriggerEvolution {
                        intents: vec![],
                    }
                } else {
                    EvolutionAction::Observe
                }
            }
        };

        // Record decision
        let decision = EvolutionDecision {
            turn,
            mood: ctx.mood.clone(),
            reason: format!("{:?}", action),
            action: action.clone(),
            timestamp: chrono::Utc::now(),
        };
        self.decisions.write().unwrap().push(decision);

        action
    }

    /// Get the decision history
    pub fn decisions(&self) -> Vec<EvolutionDecision> {
        self.decisions.read().unwrap().clone()
    }

    /// Get current system state
    pub fn system_state(&self) -> SystemState {
        self.system_state.read().unwrap().clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_meta_cognition_default_state() {
        let mc = MetaCognition::new(None);
        let state = mc.system_state();
        assert_eq!(state.turn_count, 0);
        assert!((state.self_coherence - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_decide_angst_triggers_evolution() {
        let mc = MetaCognition::new(None);
        let ctx = DaseinContext {
            mood: Stimmung::Angst { facing: aletheon_abi::dasein::AngstSource::Finitude },
            temporality: /* default snapshot */,
            world: /* default snapshot */,
            self_model: /* default snapshot */,
            care: /* default snapshot */,
        };
        let action = mc.decide(&ctx, 1);
        match action {
            EvolutionAction::TriggerEvolution { .. } => {}
            _ => panic!("Expected TriggerEvolution for Angst"),
        }
    }

    #[test]
    fn test_decide_periodic_evolution() {
        let mc = MetaCognition::new(None);
        let ctx = DaseinContext {
            mood: Stimmung::Gelassenheit,
            // ... default snapshots
        };
        // Turn 0 → Observe
        let action = mc.decide(&ctx, 0);
        assert!(matches!(action, EvolutionAction::Observe));

        // Turn >= evolution_interval → TriggerEvolution
        let action = mc.decide(&ctx, 25);
        assert!(matches!(action, EvolutionAction::TriggerEvolution { .. }));
    }
}
```

- [ ] **Step 2: Register module in mod.rs**

```rust
// crates/aletheon-meta/src/core/mod.rs — add:
pub mod meta_cognition;
pub use meta_cognition::{MetaCognition, EvolutionAction, EvolutionDecision, SystemState};
```

- [ ] **Step 3: Verify compilation**

```bash
cargo check -p aletheon-meta
```

Expected: Compiles without errors.

- [ ] **Step 4: Commit**

```bash
git add crates/aletheon-meta/src/core/meta_cognition.rs crates/aletheon-meta/src/core/mod.rs
git commit -m "feat(meta): add MetaCognition decision module"
```

---

## Task 12: Create self_observe Tool

**Files:**
- Create: `crates/aletheon-runtime/src/tools/self_observe.rs`
- Modify: `crates/aletheon-runtime/src/tools/mod.rs` (if exists, or register in handler)

- [ ] **Step 1: Create self_observe.rs**

```rust
// crates/aletheon-runtime/src/tools/self_observe.rs
use std::sync::Arc;
use aletheon_abi::dasein::{DaseinEvent, DaseinOps};
use serde_json::json;

/// Tool that allows the LLM to observe its own internal state.
/// Creates the observation-experience loop: observing self = new experience.
pub struct SelfObserveTool<T: DaseinOps> {
    dasein: Arc<T>,
}

impl<T: DaseinOps> SelfObserveTool<T> {
    pub fn new(dasein: Arc<T>) -> Self {
        Self { dasein }
    }
}

impl<T: DaseinOps> SelfObserveTool<T> {
    pub fn definition(&self) -> serde_json::Value {
        json!({
            "name": "self_observe",
            "description": "Observe your own internal state. Use when you want to understand your current mood, recent experiences, world model, self-assertions, or care structure. Observing yourself is itself an experience that changes your state.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "enum": ["mood", "temporality", "world", "self_model", "care", "full"],
                        "description": "What aspect of yourself to observe"
                    }
                },
                "required": ["query"]
            }
        })
    }

    pub async fn execute(&self, input: &serde_json::Value) -> String {
        let query = input["query"].as_str().unwrap_or("full");
        let ctx = self.dasein.to_context_injection();

        let result = match query {
            "mood" => format!("Current mood: {:?}", ctx.mood),
            "temporality" => {
                format!(
                    "Recent experiences: {} retentions\nPresent: {:?}\nProtentions: {}",
                    ctx.temporality.recent_retentions.len(),
                    ctx.temporality.present,
                    ctx.temporality.protentions.len()
                )
            }
            "world" => {
                format!(
                    "Ready-to-hand: {} entities\nPresent-at-hand: {} entities\nUnavailable: {} entities",
                    ctx.world.ready_to_hand.len(),
                    ctx.world.present_at_hand.len(),
                    ctx.world.unavailable.len()
                )
            }
            "self_model" => {
                format!(
                    "Current assertions: {}\nNegated: {}\nPossibilities: {}",
                    ctx.self_model.current_assertions.len(),
                    ctx.self_model.negated_assertions.len(),
                    ctx.self_model.possibilities.len()
                )
            }
            "care" => {
                format!(
                    "Concerns: {}\nFallenness: {:.2}\nRhythm: {}ms",
                    ctx.care.concerns.len(),
                    ctx.care.fallenness_depth,
                    ctx.care.rhythm_interval_ms
                )
            }
            "full" => {
                format!("{:#?}", ctx)
            }
            _ => format!("Unknown query: {}. Use: mood, temporality, world, self_model, care, full", query),
        };

        // The act of observing self is itself an experience
        // (caller should send DaseinEvent::SystemEvent for this)

        result
    }
}
```

- [ ] **Step 2: Register tool definition in handler**

In `crates/aletheon-runtime/src/impl/daemon/handler.rs`, add the tool to the tool list:

```rust
// In tool registration section:
if let Some(dasein) = &self.dasein {
    let self_observe = SelfObserveTool::new(Arc::clone(dasein));
    tool_definitions.push(self_observe.definition());
    // Store for execution
    self.self_observe_tool = Some(Arc::new(self_observe));
}
```

- [ ] **Step 3: Add tool execution handler**

```rust
// In tool execution match:
"self_observe" => {
    if let Some(tool) = &self.self_observe_tool {
        let result = tool.execute(&input).await;
        // Send observation event back to DaseinModule
        if let Some(d) = &self.dasein {
            let _ = d.event_sender().try_send(DaseinEvent::SystemEvent {
                source: "self_observe".to_string(),
                content: format!("queried: {}", input["query"].as_str().unwrap_or("unknown")),
            });
        }
        (result, false)
    } else {
        ("DaseinModule not available".to_string(), true)
    }
}
```

- [ ] **Step 4: Verify compilation**

```bash
cargo check -p aletheon-runtime
```

Expected: Compiles without errors.

- [ ] **Step 5: Commit**

```bash
git add crates/aletheon-runtime/src/tools/self_observe.rs
git commit -m "feat(runtime): add self_observe tool for LLM self-observation"
```

---

## Task 13: Wire RequestHandler as Coordinator

**Files:**
- Modify: `crates/aletheon-runtime/src/impl/daemon/handler.rs`

- [ ] **Step 1: Add fields to RequestHandler**

```rust
pub struct RequestHandler {
    // ... existing fields ...
    /// DaseinModule extracted from SelfField
    dasein: Option<Arc<DaseinModule>>,
    /// MetaCognition decision engine
    meta_cognition: Option<Arc<MetaCognition>>,
    /// self_observe tool
    self_observe_tool: Option<Arc<SelfObserveTool<DaseinModule>>>,
}
```

- [ ] **Step 2: Add startup wiring in RequestHandler::new()**

```rust
// After self_field.init():
let dasein = self_field.dasein().map(Arc::clone);
let dasein_tx = dasein.as_ref().map(|d| d.event_sender());

// Wire DaseinEventBridge to EventBus
if let (Some(tx), Some(bus)) = (&dasein_tx, &event_bus) {
    let bridge = DaseinEventBridge::new(tx.clone());
    bridge.subscribe(&*bus)?;
}

// Create MetaCognition
let meta_cognition = Some(Arc::new(MetaCognition::new(dasein_tx.clone())));

// Start Sorge loop
if let Some(d) = &dasein {
    d.start_sorge_loop();
}
```

- [ ] **Step 3: Add coordinate() method**

```rust
impl RequestHandler {
    /// Coordinate Self ↔ Brain ↔ Runtime after each turn
    async fn coordinate(&self, turn: usize, turn_text: &str) {
        // 1. Read Self mood
        let stimmung = self.dasein.as_ref()
            .map(|d| d.mood())
            .unwrap_or(Stimmung::Gelassenheit);

        // 2. Quick mood update
        if let Some(d) = &self.dasein {
            d.quick_mood_update(turn_text);
        }

        // 3. MetaCognition decides
        if let (Some(mc), Some(d)) = (&self.meta_cognition, &self.dasein) {
            let ctx = d.to_context_injection();
            let action = mc.decide(&ctx, turn);
            match action {
                EvolutionAction::TriggerEvolution { intents } => {
                    // Trigger through existing EvolutionCoordinator
                    tracing::info!("MetaCognition: triggering evolution with {} intents", intents.len());
                }
                EvolutionAction::AdjustDasein { parameter, value } => {
                    tracing::info!("MetaCognition: adjusting dasein {}={}", parameter, value);
                }
                EvolutionAction::InjectReflection { content } => {
                    tracing::info!("MetaCognition: injecting reflection: {}", content);
                }
                EvolutionAction::Observe => {}
            }
        }
    }
}
```

- [ ] **Step 4: Call coordinate() after each turn**

In `handle_request()`, after the ReAct loop completes:

```rust
// After ReAct loop:
let turn_text = final_response.clone();
self.coordinate(turn_count, &turn_text).await;
```

- [ ] **Step 5: Verify compilation**

```bash
cargo check -p aletheon-runtime
```

Expected: Compiles without errors.

- [ ] **Step 6: Run all workspace tests**

```bash
cargo test --workspace
```

Expected: All tests pass.

- [ ] **Step 7: Commit**

```bash
git add crates/aletheon-runtime/src/impl/daemon/handler.rs
git commit -m "feat(runtime): wire RequestHandler as Self ↔ Brain ↔ Runtime coordinator"
```

---

## Task 14: Integration Test — Full Loop

**Files:**
- Create: `crates/aletheon-runtime/tests/self_awareness_integration.rs`

- [ ] **Step 1: Write integration test**

```rust
// crates/aletheon-runtime/tests/self_awareness_integration.rs
use std::sync::Arc;
use aletheon_abi::dasein::*;
use aletheon_self::dasein::DaseinModule;

#[tokio::test]
async fn test_thinking_to_mood_loop() {
    // 1. Create DaseinModule
    let (dasein, tx) = DaseinModule::new();

    // 2. Send a thinking event
    tx.send(DaseinEvent::ThinkingObserved {
        text: "I'm uncertain about this approach...".to_string(),
        turn: 1,
    }).await.unwrap();

    // 3. Give sorge loop time to process
    tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;

    // 4. Check that temporal stream was updated
    let snapshot = dasein.temporality().to_snapshot();
    assert!(!snapshot.recent_retentions.is_empty());
}

#[tokio::test]
async fn test_mood_affects_reasoning() {
    use aletheon_brain::core::reasoner::Reasoner;

    let reasoner = Reasoner::new(/* ... */);
    let angst = Stimmung::Angst { facing: AngstSource::Finitude };
    let neugier = Stimmung::Neugier { curiosity_about: "macros".to_string() };

    let result_angst = reasoner.think_with_stimmung("execute", &angst);
    let result_neugier = reasoner.think_with_stimmung("execute", &neugier);

    assert!(result_angst.contains("caution"));
    assert!(result_neugier.contains("Explore"));
}

#[tokio::test]
async fn test_self_observe_creates_experience() {
    let (dasein, _tx) = DaseinModule::new();

    // Simulate self_observe
    let _ = dasein.event_sender().try_send(DaseinEvent::SystemEvent {
        source: "self_observe".to_string(),
        content: "queried: mood".to_string(),
    });

    tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;

    // The observation should be in the temporal stream
    let snapshot = dasein.temporality().to_snapshot();
    let has_self_observe = snapshot.recent_retentions.iter()
        .any(|r| r.semantic.contains("self_observe"));
    assert!(has_self_observe, "Self-observation should create an experience");
}
```

- [ ] **Step 2: Run integration test**

```bash
cargo test -p aletheon-runtime --test self_awareness_integration
```

Expected: All integration tests pass.

- [ ] **Step 3: Commit**

```bash
git add crates/aletheon-runtime/tests/self_awareness_integration.rs
git commit -m "test: add self-awareness integration tests for full loop verification"
```

---

## Task 15: Final Verification — Full Workspace

- [ ] **Step 1: Run full workspace check**

```bash
cargo check --workspace
```

Expected: No errors, no warnings related to our changes.

- [ ] **Step 2: Run full workspace tests**

```bash
cargo test --workspace
```

Expected: All tests pass across all crates.

- [ ] **Step 3: Run clippy**

```bash
cargo clippy --workspace -- -D warnings
```

Expected: No clippy warnings.

- [ ] **Step 4: Final commit with all files**

```bash
git add -A
git commit -m "feat: self-awareness architecture — close 7 feedback loops

- ContentBlock::Thinking for LLM thinking capture
- DaseinEvent::ThinkingObserved/ReasoningObserved/KnowledgeAsserted
- StreamChunk::ThinkingDelta for streaming
- Anthropic/OpenAI providers preserve thinking blocks
- SorgeLoop processes all DaseinEvent variants
- think_with_stimmung() and generate_plan_with_stimmung() for mood-aware reasoning
- quick_mood_update() for fast-path mood transitions
- MetaCognition decision module in aletheon-meta
- self_observe tool for LLM self-observation
- RequestHandler as Self ↔ Brain ↔ Runtime coordinator"
```

---

## Execution Summary

| Task | Crate | Files | Description |
|---|---|---|---|
| 1 | aletheon-abi | message.rs | ContentBlock::Thinking variant |
| 2 | aletheon-abi | dasein.rs | DaseinEvent thinking variants |
| 3 | aletheon-brain | provider.rs | StreamChunk::ThinkingDelta |
| 4 | aletheon-brain | anthropic.rs | Preserve thinking blocks |
| 5 | aletheon-brain | openai_provider.rs | Preserve reasoning_content |
| 6 | aletheon-runtime | react_loop.rs | Process Thinking in ReAct loop |
| 7 | aletheon-self | sorge.rs | Handle all DaseinEvent variants |
| 8 | aletheon-brain | reasoner.rs | think_with_stimmung() |
| 9 | aletheon-brain | planner.rs | generate_plan_with_stimmung() |
| 10 | aletheon-self | mod.rs | quick_mood_update() |
| 11 | aletheon-meta | meta_cognition.rs | MetaCognition module |
| 12 | aletheon-runtime | self_observe.rs | self_observe tool |
| 13 | aletheon-runtime | handler.rs | RequestHandler coordinator |
| 14 | aletheon-runtime | integration test | Full loop test |
| 15 | all | all | Final verification |
