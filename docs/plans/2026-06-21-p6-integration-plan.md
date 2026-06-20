# P6: Integration Plan

> **For agentic workers:** Use `workflow-feature` or `writing-plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Wire all layers together end-to-end: TUI ↔ Runtime ↔ Brain ↔ Skills/Hooks. Run full integration test suite.

**Architecture:** This phase connects the P0-P4 components. No new files — only wiring existing components.

**Tech Stack:** Rust, all crates

**Depends on:** P0-P5

---

### Task 1: Wire TUI AppState ↔ Runtime Session

**Files:**
- Modify: `crates/aletheon-body/src/impl/ui/mod.rs`

- [ ] **Step 1: Update App to use AppState for status rendering**

Replace the old StatusBar usage with the new AppState-based rendering:

```rust
// In draw() method
let status_line = self.app_state.format_status_line();
// Render status_line using the new StatusBar
```

- [ ] **Step 2: Update event handling to sync AppState**

Ensure all incoming events update `self.app_state`:
- `usage` event → `self.app_state.total_tokens`
- `tool_call_start` → `self.app_state.turn_tool_count += 1`
- `turn_start` → `self.app_state.streaming = true`, `self.app_state.turn_active = true`
- `turn_done` → `self.app_state.streaming = false`, `self.app_state.turn_active = false`
- `mode_changed` → `self.app_state.mode`
- `awareness_changed` → `self.app_state.awareness`
- `context_update` → `self.app_state.context`
- `model_switch` → `self.app_state.model_name`

- [ ] **Step 3: Verify it compiles**

Run: `cargo check -p aletheon-body`

- [ ] **Step 4: Commit**

```bash
git add crates/aletheon-body/src/impl/ui/mod.rs
git commit -m "feat: wire TUI AppState ↔ Runtime event stream"
```

---

### Task 2: Wire Runtime Session ↔ Daemon handler

**Files:**
- Modify: `crates/aletheon-runtime/src/impl/daemon/handler.rs`

- [ ] **Step 1: Initialize SessionManager in RequestHandler::new()**

```rust
let session_manager = SessionManager::new(config.max_context_tokens);
```

- [ ] **Step 2: Use SessionManager in chat method**

```rust
// At the start of chat method
let session = self.session_manager.active_mut()
    .ok_or("No active session")?;
session.turn_count += 1;
```

- [ ] **Step 3: Wire mode_switch to ModeRouter**

```rust
"mode_switch" => {
    let mode = /* parse from params */;
    self.runtime.mode_router_mut().set_mode(mode);
    // Update session mode too
    if let Some(session) = self.session_manager.active_mut() {
        session.mode = mode;
    }
    // Emit mode_changed event
    // ...
}
```

- [ ] **Step 4: Wire interrupt to InterruptFlag**

```rust
"interrupt" => {
    let reason = /* parse from params */;
    self.runtime.interrupt_flag().request(reason);
    // ...
}
```

- [ ] **Step 5: Verify it compiles**

Run: `cargo check -p aletheon-runtime`

- [ ] **Step 6: Commit**

```bash
git add crates/aletheon-runtime/src/impl/daemon/handler.rs
git commit -m "feat: wire Runtime SessionManager ↔ Daemon handler"
```

---

### Task 3: Wire Brain awareness → Runtime → TUI

**Files:**
- Modify: `crates/aletheon-runtime/src/core/react_loop.rs`
- Modify: `crates/aletheon-runtime/src/impl/daemon/handler.rs`

- [ ] **Step 1: Drain awareness events after ReAct loop**

In the chat method, after `react_loop.run_streaming()`:

```rust
let (response, metrics) = result?;

// Drain awareness signals and emit as UI events
let awareness_events = react_loop.drain_awareness_events();
for (level, context) in awareness_events {
    notify_tx.send(serde_json::json!({
        "method": "event",
        "type": "awareness_changed",
        "level": level,
        "context": context,
    })).ok();
}
```

- [ ] **Step 2: Verify it compiles**

Run: `cargo check -p aletheon-runtime`

- [ ] **Step 3: Commit**

```bash
git add crates/aletheon-runtime/src/core/react_loop.rs crates/aletheon-runtime/src/impl/daemon/handler.rs
git commit -m "feat: wire Brain awareness signals → Runtime → TUI"
```

---

### Task 4: Wire Skills → TUI commands

**Files:**
- Modify: `crates/aletheon-body/src/impl/ui/mod.rs`

- [ ] **Step 1: Load skills at startup**

```rust
let mut skill_loader = SkillLoader::new(
    dirs::home_dir().unwrap_or_default().join(".aletheon/skills")
);
skill_loader.load_all().ok();
```

- [ ] **Step 2: Handle /skills and /skill commands**

```rust
BuiltinCommand::Skills => {
    let skills = self.skill_loader.list();
    let msg = skills.iter().map(|s| {
        format!("{}: {}", s.trigger, s.description)
    }).collect::<Vec<_>>().join("\n");
    self.chat.add_message(Role::System, &msg);
}
BuiltinCommand::SkillRun { name, args } => {
    if let Some(skill) = self.skill_loader.get(&name) {
        let prompt = skill.system_prompt();
        // Send as a chat message with skill context
        let msg = serde_json::json!({
            "method": "chat",
            "params": { "message": format!("[Skill: {}]\n{}\n\n{}", name, prompt, args) }
        });
        self.stream.write_all(format!("{}\n", msg).as_bytes())?;
    }
}
```

- [ ] **Step 3: Add skill commands to completion**

```rust
// In completion list
let mut candidates = vec![/* existing */];
candidates.extend(self.skill_loader.completion_candidates());
```

- [ ] **Step 4: Verify it compiles**

Run: `cargo check -p aletheon-body`

- [ ] **Step 5: Commit**

```bash
git add crates/aletheon-body/src/impl/ui/mod.rs
git commit -m "feat: wire Skills system → TUI commands and completion"
```

---

### Task 5: Full build and test

- [ ] **Step 1: Full workspace build**

Run: `cargo build --workspace`
Expected: Clean build with no errors.

- [ ] **Step 2: Full test suite**

Run: `cargo test --workspace`
Expected: All tests pass.

- [ ] **Step 3: Clippy**

Run: `cargo clippy --workspace -- -D warnings`
Expected: No warnings.

- [ ] **Step 4: Integration tests with real LLM**

Run: `ANTHROPIC_API_KEY=sk-... cargo test --test integration -- --test-threads=1`
Expected: All integration tests pass.

- [ ] **Step 5: Final commit**

```bash
git add -A
git commit -m "feat: P6 Integration complete — all layers wired, tests passing"
```

---

## Summary

P6 is a wiring phase — no new files, only connections:

| What | From | To |
|------|------|----|
| AppState | TUI | Runtime event stream |
| SessionManager | Runtime | Daemon handler |
| Awareness signals | Brain → ReActLoop | TUI status bar + inline |
| Mode switching | TUI commands | Runtime ModeRouter |
| Interrupt | TUI Ctrl+C | Runtime InterruptFlag → ReActLoop |
| Skills | SkillLoader | TUI commands + completion |
| Hooks | HookRegistry | Chat turn flow |

## Overall Project Summary

| Phase | Files Added | Files Modified | Estimate |
|-------|------------|----------------|----------|
| P0 ABI | 2 | 2 | 1 day |
| P1 Runtime | 4 | 3 | 2 days |
| P2 TUI | 5 | 4 | 3 days |
| P3 Brain | 0 | 3 | 1 day |
| P4 Skills/Hooks | 3 | 2 | 1 day |
| P5 Testing | 5 | 0 | 2 days |
| P6 Integration | 0 | 3 | 1 day |
| **Total** | **19** | **17** | **~11 days** |
