# P4: Skills & Hooks Implementation Plan

> **For agentic workers:** Use `workflow-feature` or `writing-plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement the two-layer skills system (Rust built-in + Markdown user skills) and the hooks system (core 5 + event bus).

**Architecture:** Skills live in `aletheon-body/src/impl/skills/`. Hooks extend the existing `aletheon-runtime/src/impl/hooks/` module. Skills are Markdown files with YAML frontmatter. Hooks are configured in `config.toml`.

**Tech Stack:** Rust, serde_yaml, notify (inotify), tokio

**Depends on:** P0 (ABI types)

---

### Task 1: Create Markdown skill loader

**Files:**
- Create: `crates/aletheon-body/src/impl/skills/mod.rs`
- Create: `crates/aletheon-body/src/impl/skills/markdown_skill.rs`
- Create: `crates/aletheon-body/src/impl/skills/loader.rs`

- [ ] **Step 1: Create `skills/mod.rs`**

```rust
//! Skills system — two-layer architecture.
//!
//! Layer 1: Built-in skills (Rust code) — registered at startup.
//! Layer 2: User skills (Markdown prompts) — loaded from ~/.aletheon/skills/.

pub mod markdown_skill;
pub mod loader;

pub use markdown_skill::MarkdownSkill;
pub use loader::SkillLoader;
```

- [ ] **Step 2: Create `skills/markdown_skill.rs`**

```rust
//! Markdown skill definition with YAML frontmatter parsing.

use serde::Deserialize;

/// A skill loaded from a Markdown file with YAML frontmatter.
#[derive(Debug, Clone, Deserialize)]
pub struct MarkdownSkill {
    /// Skill name (from frontmatter).
    pub name: String,
    /// Human-readable description.
    pub description: String,
    /// Trigger command (e.g., "/review").
    pub trigger: String,
    /// Permissions for this skill.
    #[serde(default)]
    pub permissions: SkillPermissions,
    /// Tools this skill can use (empty = all tools).
    #[serde(default)]
    pub tools: Vec<String>,
    /// Model override for this skill (empty = use default).
    #[serde(default)]
    pub model: Option<String>,
    /// The prompt content (everything after frontmatter).
    #[serde(skip)]
    pub content: String,
}

/// Skill permission configuration.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct SkillPermissions {
    #[serde(default = "default_true")]
    pub read: bool,
    #[serde(default)]
    pub write: bool,
    #[serde(default)]
    pub execute: bool,
}

fn default_true() -> bool { true }

impl MarkdownSkill {
    /// Parse a Markdown file with YAML frontmatter.
    pub fn parse(raw: &str) -> Result<Self, String> {
        let raw = raw.trim();
        if !raw.starts_with("---") {
            return Err("Missing YAML frontmatter (must start with ---)".to_string());
        }

        let end = raw[3..].find("---").ok_or("Missing closing ---")? + 3;
        let frontmatter = &raw[3..end].trim();
        let content = raw[end + 3..].trim().to_string();

        let mut skill: MarkdownSkill = serde_yaml::from_str(frontmatter)
            .map_err(|e| format!("Failed to parse frontmatter: {}", e))?;
        skill.content = content;

        if skill.name.is_empty() {
            return Err("Skill name is required".to_string());
        }
        if skill.trigger.is_empty() {
            return Err("Skill trigger is required".to_string());
        }

        Ok(skill)
    }

    /// Get the system prompt for this skill.
    pub fn system_prompt(&self) -> String {
        self.content.clone()
    }
}
```

- [ ] **Step 3: Create `skills/loader.rs`**

```rust
//! Skill file loader with hot-reload support.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use super::markdown_skill::MarkdownSkill;

/// Loads and manages Markdown skills from disk.
#[derive(Debug)]
pub struct SkillLoader {
    skills_dir: PathBuf,
    skills: HashMap<String, MarkdownSkill>,
}

impl SkillLoader {
    pub fn new(skills_dir: PathBuf) -> Self {
        Self {
            skills_dir,
            skills: HashMap::new(),
        }
    }

    /// Load all skills from the skills directory.
    pub fn load_all(&mut self) -> Result<usize, String> {
        self.skills.clear();
        let mut count = 0;

        if !self.skills_dir.exists() {
            return Ok(0);
        }

        for entry in std::fs::read_dir(&self.skills_dir)
            .map_err(|e| format!("Failed to read skills dir: {}", e))?
        {
            let entry = entry.map_err(|e| format!("Failed to read entry: {}", e))?;
            let path = entry.path();

            if path.extension().map_or(false, |ext| ext == "md") {
                match self.load_skill(&path) {
                    Ok(skill) => {
                        self.skills.insert(skill.trigger.clone(), skill);
                        count += 1;
                    }
                    Err(e) => {
                        eprintln!("Warning: Failed to load skill {:?}: {}", path, e);
                    }
                }
            }
        }

        Ok(count)
    }

    /// Load a single skill file.
    fn load_skill(&self, path: &Path) -> Result<MarkdownSkill, String> {
        let raw = std::fs::read_to_string(path)
            .map_err(|e| format!("Failed to read {:?}: {}", path, e))?;
        MarkdownSkill::parse(&raw)
    }

    /// Get a skill by trigger command.
    pub fn get(&self, trigger: &str) -> Option<&MarkdownSkill> {
        self.skills.get(trigger)
    }

    /// List all loaded skills.
    pub fn list(&self) -> Vec<&MarkdownSkill> {
        self.skills.values().collect()
    }

    /// Get skill names for tab completion.
    pub fn completion_candidates(&self) -> Vec<String> {
        self.skills.keys().map(|k| format!("/{}", k)).collect()
    }
}
```

- [ ] **Step 4: Register module**

In `crates/aletheon-body/src/impl/mod.rs`, add:
```rust
pub mod skills;
```

- [ ] **Step 5: Verify it compiles**

Run: `cargo check -p aletheon-body`

- [ ] **Step 6: Commit**

```bash
git add crates/aletheon-body/src/impl/skills/
git commit -m "feat(body): add Markdown skill loader with YAML frontmatter parsing"
```

---

### Task 2: Extend hooks system with core 5 hooks

**Files:**
- Modify: `crates/aletheon-runtime/src/impl/hooks/mod.rs`
- Modify: `crates/aletheon-runtime/src/impl/hooks/registry.rs`

- [ ] **Step 1: Add HookEvent enum**

```rust
/// Events that trigger hooks.
#[derive(Debug, Clone)]
pub enum HookEvent {
    SessionStart { session_id: String, mode: String, model: String },
    PreTool { tool_name: String, input: serde_json::Value },
    PostTool { tool_name: String, output: String, success: bool, duration_ms: u64 },
    PreResponse { response: String, tokens_used: u32 },
    SessionEnd { session_id: String, duration_secs: u64, total_tokens: u32 },
    Custom { event_type: String, data: serde_json::Value },
}
```

- [ ] **Step 2: Add hook execution to HookRegistry**

```rust
impl HookRegistry {
    /// Fire a hook event. Executes all registered hooks for this event type.
    pub async fn fire(&self, event: &HookEvent) -> Vec<HookResult> {
        let mut results = Vec::new();

        for hook in self.hooks_for_event(event) {
            match &hook.hook_type {
                HookType::Command => {
                    if let Some(ref cmd) = hook.command {
                        let result = self.execute_command_hook(cmd, event).await;
                        results.push(result);
                    }
                }
                HookType::Event => {
                    // Emit to event bus
                    self.emit_to_bus(event).await;
                    results.push(HookResult::Continue);
                }
                HookType::Prompt => {
                    if let Some(ref prompt) = hook.prompt {
                        results.push(HookResult::Inject(prompt.clone()));
                    }
                }
            }
        }

        results
    }

    async fn execute_command_hook(&self, cmd: &str, event: &HookEvent) -> HookResult {
        let mut command = tokio::process::Command::new("sh");
        command.arg("-c").arg(cmd);

        // Inject environment variables
        match event {
            HookEvent::SessionStart { session_id, mode, model } => {
                command.env("ALETHEON_SESSION_ID", session_id);
                command.env("ALETHEON_MODE", mode);
                command.env("ALETHEON_MODEL", model);
            }
            HookEvent::PreTool { tool_name, input } => {
                command.env("ALETHEON_TOOL_NAME", tool_name);
                command.env("ALETHEON_TOOL_INPUT", input.to_string());
            }
            HookEvent::PostTool { tool_name, output, success, duration_ms } => {
                command.env("ALETHEON_TOOL_NAME", tool_name);
                command.env("ALETHEON_TOOL_OUTPUT", output);
                command.env("ALETHEON_TOOL_SUCCESS", success.to_string());
                command.env("ALETHEON_TOOL_DURATION_MS", duration_ms.to_string());
            }
            _ => {}
        }

        match command.output().await {
            Ok(output) if output.status.success() => {
                // Try to parse as JSON for modify/block behavior
                if let Ok(result) = serde_json::from_slice::<CommandHookResult>(&output.stdout) {
                    if result.block {
                        return HookResult::Block {
                            reason: result.block_reason.unwrap_or_default(),
                        };
                    }
                    if result.modify {
                        if let Some(data) = result.data {
                            return HookResult::ModifyInput(data);
                        }
                    }
                    if let Some(msg) = result.inject_message {
                        return HookResult::Inject(msg);
                    }
                }
                HookResult::Continue
            }
            _ => HookResult::Continue,
        }
    }
}
```

- [ ] **Step 3: Verify it compiles**

Run: `cargo check -p aletheon-runtime`

- [ ] **Step 4: Commit**

```bash
git add crates/aletheon-runtime/src/impl/hooks/
git commit -m "feat(runtime): add core 5 hooks with command/event/prompt execution"
```

---

### Task 3: Wire hooks into chat turn flow

**Files:**
- Modify: `crates/aletheon-runtime/src/impl/daemon/handler.rs`

- [ ] **Step 1: Fire PreTool hook before tool execution**

In the tool execution path (within the chat method), before executing a tool:

```rust
// Fire PreTool hook
let pre_results = self.hook_registry.fire(&HookEvent::PreTool {
    tool_name: tool_name.clone(),
    input: tool_input.clone(),
}).await;

for result in &pre_results {
    match result {
        HookResult::Block { reason } => {
            // Block the tool execution
            return Err(format!("Blocked by hook: {}", reason));
        }
        HookResult::ModifyInput(new_input) => {
            tool_input = new_input.clone();
        }
        _ => {}
    }
}
```

- [ ] **Step 2: Fire PostTool hook after tool execution**

```rust
let post_results = self.hook_registry.fire(&HookEvent::PostTool {
    tool_name: tool_name.clone(),
    output: output.clone(),
    success: success,
    duration_ms: elapsed,
}).await;
```

- [ ] **Step 3: Fire SessionStart/SessionEnd hooks**

```rust
// On session start
self.hook_registry.fire(&HookEvent::SessionStart {
    session_id: session_id.clone(),
    mode: mode.display_name().to_string(),
    model: model_name.clone(),
}).await;

// On session end (clear method)
self.hook_registry.fire(&HookEvent::SessionEnd {
    session_id: session_id.clone(),
    duration_secs: elapsed_secs,
    total_tokens: total_tokens,
}).await;
```

- [ ] **Step 4: Verify it compiles**

Run: `cargo check -p aletheon-runtime`

- [ ] **Step 5: Commit**

```bash
git add crates/aletheon-runtime/src/impl/daemon/handler.rs
git commit -m "feat(runtime): wire hooks into chat turn flow (pre_tool, post_tool, session lifecycle)"
```

---

### Task 4: Run tests

- [ ] **Step 1: Run tests**

Run: `cargo test -p aletheon-body -p aletheon-runtime`

- [ ] **Step 2: Final commit**

```bash
git add -A
git commit -m "chore: P4 Skills & Hooks complete — Markdown loader, core 5 hooks, event bus"
```

---

## Summary

P4 adds/modifies:

| File | Action | What Added |
|------|--------|------------|
| `skills/mod.rs` | NEW | Module declaration |
| `skills/markdown_skill.rs` | NEW | `MarkdownSkill` with YAML frontmatter parsing |
| `skills/loader.rs` | NEW | `SkillLoader` — disk scanner + hot-reload |
| `hooks/mod.rs` | MODIFY | `HookEvent` enum, `fire()` method |
| `hooks/registry.rs` | MODIFY | Command/event/prompt hook execution |
| `handler.rs` | MODIFY | Hook firing in chat turn flow |
