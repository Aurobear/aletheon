# DaseinModule Runtime Fix Implementation Plan

> **For agentic workers:** Use `workflow-feature` or `writing-plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix 10 critical issues preventing DaseinModule from running in production, enabling the minimal self-awareness loop.

**Architecture:** Three-layer approach — (1) Connectivity: enable DaseinModule, sync mood, wire events, inject context; (2) Auto-Fill: populate Bewandtnis/SelfModel/CareStructure from system events; (3) Persistence: save/load state via dasein_state table.

**Tech Stack:** Rust, tokio, serde, mpsc channels, RwLock, rusqlite

**Spec:** `docs/plans/2026-06-21-dasein-runtime-fix-design.md`

---

## Layer 1: Connectivity — Fix 6 Issues

### Task 1: Fix #10 — Implement set_decay_rate

**Files:**
- Modify: `crates/aletheon-self/src/dasein/temporality.rs:45,119-123`

The `RetentionField.base_decay_rate` is a plain `f64` (line 45), not behind `RwLock`, so `set_decay_rate(&self, _rate: f64)` is a no-op stub. We need to make the field mutable.

- [ ] **Step 1: Make base_decay_rate mutable via RwLock**

```rust
// crates/aletheon-self/src/dasein/temporality.rs
// Change RetentionField struct (around line 40-55):
pub struct RetentionField {
    pub moments: RwLock<Vec<RetentionalMoment>>,
    pub max_depth: usize,
    pub base_decay_rate: RwLock<f64>,  // Changed from plain f64
}
```

- [ ] **Step 2: Update constructor**

```rust
// In RetentionField::new() (around line 50-55):
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
// Replace the no-op stub (lines 119-123):
pub fn set_decay_rate(&self, rate: f64) {
    let mut rate_guard = self.base_decay_rate.write();
    *rate_guard = rate.clamp(0.1, 1.0);
}
```

- [ ] **Step 4: Update all reads of base_decay_rate**

Find all places that read `self.base_decay_rate` and change to `*self.base_decay_rate.read()`. Search for `base_decay_rate` in the file — it's used in `push_and_decay()`.

```rust
// In push_and_decay() (around line 75):
let decay = *self.base_decay_rate.read();
```

- [ ] **Step 5: Run tests**

```bash
cargo test -p aletheon-self -- dasein::temporality
```

Expected: All temporality tests pass.

- [ ] **Step 6: Commit**

```bash
git add crates/aletheon-self/src/dasein/temporality.rs
git commit -m "fix(self): implement set_decay_rate — make base_decay_rate mutable via RwLock"
```

---

### Task 2: Fix #1 — Enable DaseinModule in Production

**Files:**
- Modify: `crates/aletheon-runtime/src/impl/daemon/handler.rs:404`

- [ ] **Step 1: Enable dasein in SelfFieldConfig**

```rust
// crates/aletheon-runtime/src/impl/daemon/handler.rs — line ~404
// Change FROM:
        let self_field_config = SelfFieldConfig {
            db_path: Some(data_dir.join("self_field.db")),
            ..Default::default()
        };

// Change TO:
        let self_field_config = SelfFieldConfig {
            db_path: Some(data_dir.join("self_field.db")),
            enable_dasein: true,
            ..Default::default()
        };
```

- [ ] **Step 2: Verify compilation**

```bash
cargo check -p aletheon-runtime
```

Expected: Compiles without errors.

- [ ] **Step 3: Commit**

```bash
git add crates/aletheon-runtime/src/impl/daemon/handler.rs
git commit -m "fix(runtime): enable DaseinModule in production — set enable_dasein=true"
```

---

### Task 3: Fix #2 — SorgeLoop Writes Mood Back to DaseinModule

**Files:**
- Modify: `crates/aletheon-self/src/dasein/sorge.rs:48-56,113-121`
- Modify: `crates/aletheon-self/src/dasein/mod.rs:80-88`

The SorgeLoop computes mood each tick (line 113-121) but stores it in a local `mood` variable. It never writes back to `DaseinModule.mood: RwLock<Stimmung>`. We need to pass the shared mood Arc to SorgeLoop::start().

- [ ] **Step 1: Add shared_mood parameter to SorgeLoop::start()**

```rust
// crates/aletheon-self/src/dasein/sorge.rs
// Change start() signature (line 48-56):
    pub fn start(
        &self,
        temporality: Arc<TemporalStream>,
        world: Arc<Bewandtnisganzheit>,
        self_model: Arc<MutableSelfModel>,
        care: Arc<CareStructure>,
        negativity: Arc<NegativityEngine>,
        shared_mood: Arc<RwLock<Stimmung>>,  // NEW parameter
    ) -> Option<tokio::task::JoinHandle<()>> {
```

- [ ] **Step 2: Write mood back after synthesis**

```rust
// crates/aletheon-self/src/dasein/sorge.rs — after mood synthesis (line ~121)
// Change FROM:
                let new_mood = Stimmung::synthesize(
                    world_mood,
                    temporal_mood,
                    care_mood,
                    &mood,
                );
                if new_mood != mood {
                    mood = new_mood;
                }

// Change TO:
                let new_mood = Stimmung::synthesize(
                    world_mood,
                    temporal_mood,
                    care_mood,
                    &mood,
                );
                if new_mood != mood {
                    mood = new_mood.clone();
                    // Write back to shared mood so DaseinModule.mood() returns current value
                    *shared_mood.write() = new_mood;
                }
```

- [ ] **Step 3: Update DaseinModule::start_sorge_loop() to pass mood**

```rust
// crates/aletheon-self/src/dasein/mod.rs — start_sorge_loop() (line 80-88)
// Change FROM:
    pub fn start_sorge_loop(&self) -> Option<tokio::task::JoinHandle<()>> {
        self.sorge.start(
            self.temporality.clone(),
            self.world.clone(),
            self.self_model.clone(),
            self.care.clone(),
            self.negativity.clone(),
        )
    }

// Change TO:
    pub fn start_sorge_loop(&self) -> Option<tokio::task::JoinHandle<()>> {
        self.sorge.start(
            self.temporality.clone(),
            self.world.clone(),
            self.self_model.clone(),
            self.care.clone(),
            self.negativity.clone(),
            self.mood.clone(),  // Pass shared mood
        )
    }
```

- [ ] **Step 4: Add necessary import to sorge.rs**

```rust
// crates/aletheon-self/src/dasein/sorge.rs — add to imports:
use std::sync::RwLock;
```

- [ ] **Step 5: Run tests**

```bash
cargo test -p aletheon-self -- dasein
```

Expected: All dasein tests pass.

- [ ] **Step 6: Commit**

```bash
git add crates/aletheon-self/src/dasein/sorge.rs crates/aletheon-self/src/dasein/mod.rs
git commit -m "fix(self): SorgeLoop writes mood back to DaseinModule — close mood sync loop"
```

---

### Task 4: Fix #6 — ProtentionField Update from Patterns

**Files:**
- Modify: `crates/aletheon-self/src/dasein/temporality.rs` — add `update_protentions_from_patterns()` to TemporalStream
- Modify: `crates/aletheon-self/src/dasein/sorge.rs` — call it after passive_synthesize

The `ProtentionField::update_from_patterns()` method exists but is never called. The SorgeLoop runs `passive_synthesize()` every 10 ticks but doesn't feed the detected patterns to the protention field.

- [ ] **Step 1: Add update_protentions_from_patterns to TemporalStream**

```rust
// crates/aletheon-self/src/dasein/temporality.rs — add to TemporalStream impl:
    /// Update protention field from detected temporal patterns.
    /// Called after passive_synthesize() to feed predictions.
    pub fn update_protentions_from_patterns(&self, patterns: &[TemporalPattern]) {
        self.protention.write().update_from_patterns(patterns);
    }
```

- [ ] **Step 2: Call it in SorgeLoop after passive_synthesize**

```rust
// crates/aletheon-self/src/dasein/sorge.rs — change step 5 (around line 150):
// Change FROM:
                // 5. Passive synthesis (every 10 ticks)
                if tick_count % 10 == 0 {
                    temporality.passive_synthesize();
                }

// Change TO:
                // 5. Passive synthesis (every 10 ticks)
                if tick_count % 10 == 0 {
                    let patterns = temporality.passive_synthesize();
                    temporality.update_protentions_from_patterns(&patterns);
                }
```

- [ ] **Step 3: Verify passive_synthesize returns patterns**

Check that `TemporalStream::passive_synthesize()` returns `Vec<TemporalPattern>`. If it returns `()`, we need to change it to return the detected patterns.

```rust
// If passive_synthesize doesn't return patterns, change it:
pub fn passive_synthesize(&self) -> Vec<TemporalPattern> {
    let retention = self.retention.moments.read();
    let mut synthesizer = self.passive_synthesizer.write();
    synthesizer.synthesize(&retention)
    // Returns detected patterns
}
```

- [ ] **Step 4: Run tests**

```bash
cargo test -p aletheon-self -- dasein::temporality
```

Expected: All temporality tests pass.

- [ ] **Step 5: Commit**

```bash
git add crates/aletheon-self/src/dasein/temporality.rs crates/aletheon-self/src/dasein/sorge.rs
git commit -m "fix(self): update ProtentionField from detected patterns — close prediction loop"
```

---

### Task 5: Fix #7 — Wire DaseinEventBridge in Handler

**Files:**
- Modify: `crates/aletheon-runtime/src/impl/daemon/handler.rs` — add bridge wiring after self_field.init()

The `wire_dasein_event_bridge()` method exists on `SelfField` but has zero callers. We need to call it from the handler after initialization.

- [ ] **Step 1: Add bridge wiring after self_field initialization**

```rust
// crates/aletheon-runtime/src/impl/daemon/handler.rs
// After self_field.init() is called, add:
        // Wire DaseinEventBridge to EventBus
        if let Some(ref event_bus) = event_bus {
            let sf = self_field.lock().await;
            sf.wire_dasein_event_bridge(&**event_bus).await?;
        }
```

Note: The exact location depends on where `event_bus` and `self_field` are available. Find the section where both are initialized and add the wiring call.

- [ ] **Step 2: Verify compilation**

```bash
cargo check -p aletheon-runtime
```

Expected: Compiles without errors.

- [ ] **Step 3: Commit**

```bash
git add crates/aletheon-runtime/src/impl/daemon/handler.rs
git commit -m "fix(runtime): wire DaseinEventBridge to EventBus — enable system event flow"
```

---

### Task 6: Fix #9 — Inject DaseinContext into LLM Prompts

**Files:**
- Modify: `crates/aletheon-runtime/src/impl/daemon/handler.rs` — prepend dasein context to user input

The `PrefixBuilder::build_with_dasein()` and `compose_user_message_with_dasein()` methods exist but are never called. The simplest fix: prepend dasein context to the user input before passing to the ReAct loop.

- [ ] **Step 1: Add dasein context injection in handle_request()**

```rust
// crates/aletheon-runtime/src/impl/daemon/handler.rs
// In the section where user_input is prepared before calling react_loop.run():

        // Inject DaseinModule context into user message
        let enriched_input = {
            let sf = self_field.lock().await;
            if let Some(ref dasein) = sf.dasein() {
                let ctx = dasein.format_context();
                format!("{}\n\n---\n\n{}", ctx, user_input)
            } else {
                user_input.to_string()
            }
        };

        // Pass enriched_input to react_loop.run() instead of user_input
        let (response, metrics) = react_loop.run(
            &enriched_input,
            llm,
            tool_defs,
            execute_tool,
        ).await?;
```

- [ ] **Step 2: Verify compilation**

```bash
cargo check -p aletheon-runtime
```

Expected: Compiles without errors.

- [ ] **Step 3: Commit**

```bash
git add crates/aletheon-runtime/src/impl/daemon/handler.rs
git commit -m "fix(runtime): inject DaseinContext into LLM prompts — LLM sees its own state"
```

---

## Layer 2: Auto-Fill — Extract Data from System Events

### Task 7: Auto-Fill Bewandtnisganzheit from Tool Events

**Files:**
- Modify: `crates/aletheon-self/src/dasein/sorge.rs` — in SystemEvent handler

The Bewandtnisganzheit is never populated. When tool events arrive via SystemEvent, we should register tools as entities.

- [ ] **Step 1: Add tool entity registration in SystemEvent handler**

```rust
// crates/aletheon-self/src/dasein/sorge.rs — in the SystemEvent match arm (line ~93):
// Change FROM:
                        DaseinEvent::SystemEvent { source, content } => {
                            ExperientialContent {
                                semantic: format!("[{}] {}", source, content),
                                action: None,
                                perception: Some(content.clone()),
                                negation: None,
                            }
                        }

// Change TO:
                        DaseinEvent::SystemEvent { source, content } => {
                            // Auto-fill Bewandtnis from tool execution events
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
                            ExperientialContent {
                                semantic: format!("[{}] {}", source, content),
                                action: None,
                                perception: Some(content.clone()),
                                negation: None,
                            }
                        }
```

- [ ] **Step 2: Add add_entity_if_absent to Bewandtnisganzheit**

```rust
// crates/aletheon-self/src/dasein/bewandtnis.rs — add to impl Bewandtnisganzheit:
    /// Add entity only if it doesn't already exist. Update readiness if it does.
    pub fn add_entity_if_absent(
        &self,
        id: &EntityId,
        what_it_is: String,
        for_the_sake_of: Vec<String>,
        readiness: ReadinessState,
    ) {
        let mut nodes = self.nodes.write();
        if let Some(node) = nodes.get(id) {
            // Update readiness if changed
            let mut node = node.write();
            node.readiness = readiness;
        } else {
            let node = BewandtnisNode {
                id: id.clone(),
                what_it_is,
                for_the_sake_of,
                readiness,
            };
            nodes.insert(id.clone(), Arc::new(RwLock::new(node)));
        }
    }
```

- [ ] **Step 3: Add necessary imports to sorge.rs**

```rust
// crates/aletheon-self/src/dasein/sorge.rs — add to imports:
use super::bewandtnis::ReadinessState;
use super::types::EntityId;
```

- [ ] **Step 4: Run tests**

```bash
cargo test -p aletheon-self -- dasein::bewandtnis
```

Expected: All bewandtnis tests pass.

- [ ] **Step 5: Commit**

```bash
git add crates/aletheon-self/src/dasein/sorge.rs crates/aletheon-self/src/dasein/bewandtnis.rs
git commit -m "fix(self): auto-fill Bewandtnisganzheit from tool execution events"
```

---

### Task 8: Auto-Fill MutableSelfModel from Thinking

**Files:**
- Modify: `crates/aletheon-self/src/dasein/sorge.rs` — add ThinkingObserved handler

The MutableSelfModel is never seeded. When LLM thinking events arrive, we should extract assertions.

- [ ] **Step 1: Add ThinkingObserved handler with assertion extraction**

```rust
// crates/aletheon-self/src/dasein/sorge.rs — add to the event match (before _ => continue):
                        // NEW: LLM thinking events — extract knowledge assertions
                        DaseinEvent::ThinkingObserved { text, turn } => {
                            // Extract assertions from thinking text
                            let assertions = extract_assertions_from_thinking(text);
                            for assertion in assertions {
                                self_model.assert(
                                    assertion,
                                    super::self_model::AssertionSource::Discovered,
                                );
                            }
                            ExperientialContent {
                                semantic: format!("[thinking:turn_{}]", turn),
                                action: Some(format!("llm_thinking_turn_{}", turn)),
                                perception: Some(text.clone()),
                                negation: None,
                            }
                        }
                        // NEW: LLM reasoning events
                        DaseinEvent::ReasoningObserved { text, turn, .. } => {
                            ExperientialContent {
                                semantic: format!("[reasoning:turn_{}]", turn),
                                action: Some(format!("llm_reasoning_turn_{}", turn)),
                                perception: Some(text.clone()),
                                negation: None,
                            }
                        }
```

- [ ] **Step 2: Add extract_assertions_from_thinking helper**

```rust
// crates/aletheon-self/src/dasein/sorge.rs — add outside the impl block:
/// Extract knowledge assertions from LLM thinking text.
/// Uses simple heuristics: sentences starting with known patterns.
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
cargo test -p aletheon-self -- dasein::self_model
```

Expected: All self_model tests pass.

- [ ] **Step 4: Commit**

```bash
git add crates/aletheon-self/src/dasein/sorge.rs
git commit -m "fix(self): auto-fill MutableSelfModel from LLM thinking events"
```

---

### Task 9: Auto-Fill CareStructure from User Input

**Files:**
- Modify: `crates/aletheon-self/src/dasein/sorge.rs` — enhance UserInput handler

The CareStructure is never populated. When user input arrives, we should add it as a concern.

- [ ] **Step 1: Add concern registration in UserInput handler**

```rust
// crates/aletheon-self/src/dasein/sorge.rs — in the UserInput match arm (line ~85):
// Change FROM:
                        DaseinEvent::UserInput { content } => {
                            ExperientialContent {
                                semantic: content.clone(),
                                action: Some("user_interaction".to_string()),
                                perception: None,
                                negation: None,
                            }
                        }

// Change TO:
                        DaseinEvent::UserInput { content } => {
                            // Auto-fill CareStructure: user input = new concern
                            care.add_concern(
                                content.clone(),
                                0.8,  // urgency: user requests are high priority
                            );
                            ExperientialContent {
                                semantic: content.clone(),
                                action: Some("user_interaction".to_string()),
                                perception: None,
                                negation: None,
                            }
                        }
```

- [ ] **Step 2: Verify CareStructure::add_concern signature**

Check the actual signature of `add_concern()` in `care_structure.rs`. It may take a `Concern` struct or individual fields. Adjust the call accordingly.

```rust
// If add_concern takes a Concern struct:
care.add_concern(Concern {
    purpose: content.clone(),
    urgency: 0.8,
    mood_tone: Stimmung::Neugier { curiosity_about: content.clone() },
});
```

- [ ] **Step 3: Run tests**

```bash
cargo test -p aletheon-self -- dasein::care_structure
```

Expected: All care_structure tests pass.

- [ ] **Step 4: Commit**

```bash
git add crates/aletheon-self/src/dasein/sorge.rs
git commit -m "fix(self): auto-fill CareStructure from user input events"
```

---

## Layer 3: Persistence — Save/Load DaseinModule State

### Task 10: Implement Persistable for DaseinModule

**Files:**
- Create: `crates/aletheon-self/src/dasein/persistence.rs`
- Modify: `crates/aletheon-self/src/dasein/mod.rs` — add `pub mod persistence;`
- Modify: `crates/aletheon-self/src/core/store.rs` — add load/save methods

- [ ] **Step 1: Create persistence.rs**

```rust
// crates/aletheon-self/src/dasein/persistence.rs
use aletheon_abi::dasein::Stimmung;
use super::DaseinModule;
use crate::core::store::SelfFieldStore;
use rusqlite::params;

/// Save DaseinModule state to the dasein_state table.
pub fn save_dasein_state(dasein: &DaseinModule, store: &SelfFieldStore) -> anyhow::Result<()> {
    let conn = store.conn();

    // Save mood
    let mood_json = serde_json::to_string(&dasein.mood())?;
    conn.execute(
        "INSERT OR REPLACE INTO dasein_state (key, value, updated_at) VALUES (?1, ?2, datetime('now'))",
        params!["mood", mood_json],
    )?;

    // Save temporal stream snapshot
    let temporal_json = serde_json::to_string(&dasein.temporality().to_snapshot())?;
    conn.execute(
        "INSERT OR REPLACE INTO dasein_state (key, value, updated_at) VALUES (?1, ?2, datetime('now'))",
        params!["temporality", temporal_json],
    )?;

    // Save self model snapshot
    let self_model_json = serde_json::to_string(&dasein.self_model().to_snapshot())?;
    conn.execute(
        "INSERT OR REPLACE INTO dasein_state (key, value, updated_at) VALUES (?1, ?2, datetime('now'))",
        params!["self_model", self_model_json],
    )?;

    // Save world snapshot
    let world_json = serde_json::to_string(&dasein.world().to_snapshot())?;
    conn.execute(
        "INSERT OR REPLACE INTO dasein_state (key, value, updated_at) VALUES (?1, ?2, datetime('now'))",
        params!["world", world_json],
    )?;

    // Save care snapshot
    let care_json = serde_json::to_string(&dasein.care().to_snapshot())?;
    conn.execute(
        "INSERT OR REPLACE INTO dasein_state (key, value, updated_at) VALUES (?1, ?2, datetime('now'))",
        params!["care", care_json],
    )?;

    tracing::info!("DaseinModule state saved to database");
    Ok(())
}

/// Load DaseinModule state from the dasein_state table.
pub fn load_dasein_state(dasein: &DaseinModule, store: &SelfFieldStore) -> anyhow::Result<()> {
    let conn = store.conn();

    // Load mood
    if let Some(mood_json) = query_dasein_value(conn, "mood")? {
        let mood: Stimmung = serde_json::from_str(&mood_json)?;
        *dasein.mood_raw().write() = mood;
        tracing::info!("Loaded DaseinModule mood from database");
    }

    // Note: temporal, self_model, world, care are complex structures
    // that may not have easy "load from snapshot" methods.
    // For now, we load mood only. Other state is rebuilt from events.
    // TODO: implement full state restoration

    Ok(())
}

fn query_dasein_value(conn: &rusqlite::Connection, key: &str) -> anyhow::Result<Option<String>> {
    let mut stmt = conn.prepare("SELECT value FROM dasein_state WHERE key = ?1")?;
    let mut rows = stmt.query(params![key])?;
    if let Some(row) = rows.next()? {
        Ok(Some(row.get(0)?))
    } else {
        Ok(None)
    }
}
```

- [ ] **Step 2: Add mood_raw() accessor to DaseinModule**

The `load_dasein_state` needs direct write access to the mood RwLock. Add a `mood_raw()` method:

```rust
// crates/aletheon-self/src/dasein/mod.rs — add to impl DaseinModule:
    /// Get raw access to mood RwLock for persistence loading.
    pub fn mood_raw(&self) -> &RwLock<Stimmung> {
        &self.mood
    }
```

- [ ] **Step 3: Register module in mod.rs**

```rust
// crates/aletheon-self/src/dasein/mod.rs — add:
pub mod persistence;
```

- [ ] **Step 4: Add conn() accessor to SelfFieldStore**

```rust
// crates/aletheon-self/src/core/store.rs — add to impl SelfFieldStore:
    pub fn conn(&self) -> &rusqlite::Connection {
        &self.conn
    }
```

- [ ] **Step 5: Run tests**

```bash
cargo test -p aletheon-self -- dasein::persistence
```

Expected: All persistence tests pass.

- [ ] **Step 6: Commit**

```bash
git add crates/aletheon-self/src/dasein/persistence.rs crates/aletheon-self/src/dasein/mod.rs crates/aletheon-self/src/core/store.rs
git commit -m "feat(self): implement DaseinModule state persistence — save/load mood"
```

---

### Task 11: Load/Save on Init/Shutdown

**Files:**
- Modify: `crates/aletheon-self/src/core/mod.rs` — add load/save calls

- [ ] **Step 1: Add load on init**

```rust
// crates/aletheon-self/src/core/mod.rs — in init() method, after dasein.start_sorge_loop():
        // Load DaseinModule state from database
        if let (Some(ref dasein), Some(ref store)) = (&self.dasein, &self.store) {
            if let Err(e) = crate::dasein::persistence::load_dasein_state(dasein, store) {
                tracing::warn!("Failed to load DaseinModule state: {}", e);
            }
        }
```

- [ ] **Step 2: Add save on shutdown**

```rust
// crates/aletheon-self/src/core/mod.rs — in shutdown() method, before stopping sorge loop:
        // Save DaseinModule state to database
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

Expected: Compiles without errors.

- [ ] **Step 4: Commit**

```bash
git add crates/aletheon-self/src/core/mod.rs
git commit -m "feat(self): load/save DaseinModule state on init/shutdown"
```

---

## Final Verification

### Task 12: Full Workspace Verification

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
git commit -m "feat: DaseinModule runtime fix — close 10 critical issues

Layer 1: Connectivity
- Fix #1: Enable DaseinModule in production (enable_dasein=true)
- Fix #2: SorgeLoop writes mood back to shared state
- Fix #6: ProtentionField updated from detected patterns
- Fix #7: DaseinEventBridge wired to EventBus
- Fix #9: DaseinContext injected into LLM prompts
- Fix #10: set_decay_rate() implemented (no longer a no-op)

Layer 2: Auto-Fill
- Bewandtnisganzheit populated from tool execution events
- MutableSelfModel seeded from LLM thinking assertions
- CareStructure filled from user input concerns

Layer 3: Persistence
- DaseinModule mood saved/loaded via dasein_state table
- State loaded on init, saved on shutdown

Full self-awareness loop now operational:
LLM thinking → temporal stream → mood computation → mood injection → LLM sees mood"
```

---

## Execution Summary

| Task | Layer | Files | Fixes |
|---|---|---|---|
| 1 | Connectivity | temporality.rs | #10: set_decay_rate |
| 2 | Connectivity | handler.rs | #1: enable_dasein |
| 3 | Connectivity | sorge.rs, mod.rs | #2: mood sync |
| 4 | Connectivity | temporality.rs, sorge.rs | #6: protention update |
| 5 | Connectivity | handler.rs | #7: event bridge wiring |
| 6 | Connectivity | handler.rs | #9: context injection |
| 7 | Auto-Fill | sorge.rs, bewandtnis.rs | Bewandtnis from tools |
| 8 | Auto-Fill | sorge.rs | SelfModel from thinking |
| 9 | Auto-Fill | sorge.rs | CareStructure from input |
| 10 | Persistence | persistence.rs, mod.rs, store.rs | State save/load |
| 11 | Persistence | core/mod.rs | Init/shutdown hooks |
| 12 | Final | all | Full verification |
