# P3: Brain Layer Implementation Plan

> **For agentic workers:** Use `workflow-feature` or `writing-plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Wire awareness signals to EventBus for TUI display, add plan serialization for plan mode, and expose model info for status bar.

**Architecture:** Modify existing awareness_signal.rs to emit UiEvent via EventBus. Add plan serialization types. Expose model name from LLM provider.

**Tech Stack:** Rust, aletheon-abi types from P0

**Depends on:** P0 (ABI types)

---

### Task 1: Wire awareness signals to EventBus

**Files:**
- Modify: `crates/aletheon-brain/src/core/awareness_signal.rs`

- [ ] **Step 1: Add conversion from SelfState to AwarenessLevel**

```rust
use aletheon_abi::ui_event::AwarenessLevel;
use aletheon_abi::self_field::SelfState;

impl From<&SelfState> for AwarenessLevel {
    fn from(state: &SelfState) -> Self {
        match state {
            SelfState::Confident => AwarenessLevel::Confident,
            SelfState::Hesitant => AwarenessLevel::Hesitant,
            SelfState::Confused => AwarenessLevel::Confused,
            SelfState::Curious => AwarenessLevel::Curious,
            SelfState::Focused => AwarenessLevel::Confident,
            SelfState::Other(_) => AwarenessLevel::Confident,
        }
    }
}
```

- [ ] **Step 2: Add helper to emit awareness event**

```rust
/// Convert awareness signals to UiEvent for TUI display.
pub fn signals_to_ui_events(signals: &[AwarenessSignal]) -> Vec<(AwarenessLevel, String)> {
    signals.iter().filter_map(|s| {
        let state = s.detected_state.as_ref()?;
        let level = AwarenessLevel::from(state);
        let context = match state {
            SelfState::Confused => format!("Impasse detected at step {}", s.step),
            SelfState::Hesitant => format!("Uncertainty detected at step {}", s.step),
            SelfState::Curious => format!("Goal shift detected: {}", s.action),
            SelfState::Confident => format!("Confident at step {}", s.step),
            _ => return None,
        };
        Some((level, context))
    }).collect()
}
```

- [ ] **Step 3: Verify it compiles**

Run: `cargo check -p aletheon-brain`

- [ ] **Step 4: Commit**

```bash
git add crates/aletheon-brain/src/core/awareness_signal.rs
git commit -m "feat(brain): add AwarenessLevel conversion and signals_to_ui_events helper"
```

---

### Task 2: Wire awareness emission in the ReAct loop

**Files:**
- Modify: `crates/aletheon-runtime/src/core/react_loop.rs`

- [ ] **Step 1: Add awareness event emission**

In the `emit_signal()` method (line 256), after storing the signal, also emit a UiEvent:

```rust
fn emit_signal(&mut self, signal: AwarenessSignal) {
    self.signals.push(signal.clone());

    // Convert to AwarenessLevel for TUI
    let Some(ref state) = signal.detected_state else { return };
    let level = AwarenessLevel::from(state);
    let context = match state {
        SelfState::Confused => format!("Impasse detected at step {}", signal.step),
        SelfState::Hesitant => format!("Uncertainty detected"),
        SelfState::Curious => format!("Goal shift: {}", signal.action),
        _ => return, // Don't emit for Confident/Focused
    };

    // The event will be picked up by the handler and forwarded to TUI
    // Store for later retrieval
}
```

- [ ] **Step 2: Add method to drain awareness events**

```rust
impl ReActLoop {
    /// Drain accumulated awareness signals as UI events.
    pub fn drain_awareness_events(&mut self) -> Vec<(AwarenessLevel, String)> {
        let signals: Vec<_> = self.signals.drain(..).collect();
        awareness_signal::signals_to_ui_events(&signals)
    }
}
```

- [ ] **Step 3: Verify it compiles**

Run: `cargo check -p aletheon-runtime`

- [ ] **Step 4: Commit**

```bash
git add crates/aletheon-runtime/src/core/react_loop.rs
git commit -m "feat(runtime): wire awareness signal emission in ReAct loop"
```

---

### Task 3: Expose model info for status bar

**Files:**
- Modify: `crates/aletheon-brain/src/impl/llm/provider.rs`

- [ ] **Step 1: Add model_info() method to LlmProvider trait**

```rust
#[async_trait]
pub trait LlmProvider: Send + Sync {
    async fn complete(&self, messages: &[Message], tools: &[ToolDefinition]) -> Result<LlmResponse>;
    async fn complete_stream(&self, messages: &[Message], tools: &[ToolDefinition]) -> Result<LlmStream>;
    fn name(&self) -> &str;
    fn max_context_length(&self) -> usize;

    /// Human-readable model info for status bar display.
    fn model_info(&self) -> ModelInfo {
        ModelInfo {
            name: self.name().to_string(),
            max_context: self.max_context_length(),
        }
    }
}

/// Model information for TUI display.
#[derive(Debug, Clone)]
pub struct ModelInfo {
    pub name: String,
    pub max_context: usize,
}
```

- [ ] **Step 2: Implement for existing providers**

Add `model_info()` override to `AnthropicProvider`, `OpenAiProvider`, `OllamaProvider` that returns the specific model name (e.g., "claude-sonnet-4-6" instead of generic "anthropic").

- [ ] **Step 3: Verify it compiles**

Run: `cargo check -p aletheon-brain`

- [ ] **Step 4: Commit**

```bash
git add crates/aletheon-brain/src/impl/llm/provider.rs
git commit -m "feat(brain): add ModelInfo to LlmProvider for status bar display"
```

---

### Task 4: Run Brain tests

- [ ] **Step 1: Run tests**

Run: `cargo test -p aletheon-brain`

- [ ] **Step 2: Run clippy**

Run: `cargo clippy -p aletheon-brain -- -D warnings`

- [ ] **Step 3: Final commit**

```bash
git add -A
git commit -m "chore(brain): P3 Brain complete — awareness UI events, model info"
```

---

## Summary

P3 modifies 3 files:

| File | Action | What Added |
|------|--------|------------|
| `awareness_signal.rs` | MODIFY | `AwarenessLevel` conversion, `signals_to_ui_events()` |
| `react_loop.rs` | MODIFY | Awareness event emission + drain |
| `provider.rs` | MODIFY | `ModelInfo`, `model_info()` method |
