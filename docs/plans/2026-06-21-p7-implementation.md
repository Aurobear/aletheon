# P7: AgentTool + HookEngine + MCP Server + TUI Fix

> **For agentic workers:** Use `workflow-feature` or `writing-plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Wire up all 4 components so the LLM can delegate to sub-agents, hooks run at lifecycle points, MCP clients connect, and the TUI shows the current mode.

**Architecture:** Most code already exists in the codebase — the primary work is wiring existing components into the daemon handler and creating definition files. The 4 components are independent and can be implemented in parallel.

**Tech Stack:** Rust (tokio, serde_json), Unix sockets, YAML frontmatter parsing, TOML config files.

---

## Current State Audit

| Component | Code Exists | Wired in Handler | Missing |
|-----------|------------|-----------------|---------|
| AgentTool | `aletheon-body/src/impl/tools/agent_tool.rs` | ❌ Not registered in ToolRegistry | Wiring in handler.rs |
| AgentLoader (runtime) | `aletheon-runtime/src/impl/agent_loader/mod.rs` | ✅ Loaded | Needs to feed AgentTool |
| HookRegistry | `aletheon-runtime/src/impl/hooks/registry.rs` | ✅ Registered | User hook TOML loader missing |
| McpEmbedded | `aletheon-runtime/src/impl/daemon/mcp_embedded.rs` | ❌ Not started | `tokio::spawn` in daemon/mod.rs |
| TUI Mode Indicator | `aletheon-body/src/impl/ui/status.rs` | ✅ Renders icon/mode | Verify mode_changed event arrives |
| Agent definitions | ❌ | N/A | `~/.aletheon/agents/*.md` files |
| User hook configs | ❌ | N/A | `~/.aletheon/hooks/*.toml` loader |

---

## Task 1: Wire AgentTool into Handler

**Files:**
- Modify: `crates/aletheon-runtime/src/impl/daemon/handler.rs:280-316`

The handler already loads `AgentLoader` (runtime version) at line 502. We need to:
1. Convert `AgentRole` → `AgentDefinition` (body version)
2. Create `ExecuteSubAgentFn` that runs a sub-agent ReActLoop
3. Register `AgentTool` in the `ToolRegistry`

- [ ] **Step 1: Add AgentTool registration after MCP tool registration**

In `handler.rs`, after line 316 (after the MCP tool registration block), add:

```rust
// ── AgentTool — sub-agent delegation ───────────────────────────────────────
{
    let agents_dir = aletheon_dir.join("agents");
    let mut rt_agent_loader = crate::r#impl::agent_loader::AgentLoader::new();
    if agents_dir.exists() {
        let _ = rt_agent_loader.load_from_dir(&agents_dir);
    }

    let mut agent_defs: std::collections::HashMap<String, aletheon_body::r#impl::tools::agent_tool::AgentDefinition> = std::collections::HashMap::new();
    for role in rt_agent_loader.list() {
        agent_defs.insert(
            role.name.clone(),
            aletheon_body::r#impl::tools::agent_tool::AgentDefinition {
                name: role.name.clone(),
                description: role.description.clone(),
                tools: role.tools.clone(),
                model: role.model.clone(),
                max_iterations: 20,
                system_prompt: role.body.clone(),
            },
        );
    }

    if !agent_defs.is_empty() {
        let llm_for_agents: Arc<dyn aletheon_brain::r#impl::llm::LlmProvider> = llm.clone();
        let tools_for_agents = tools.clone();
        let execute_fn: aletheon_body::r#impl::tools::agent_tool::ExecuteSubAgentFn = Arc::new(
            move |system_prompt: String, user_prompt: String, allowed_tools: Vec<String>| {
                let llm = llm_for_agents.clone();
                let tools = tools_for_agents.clone();
                Box::pin(async move {
                    // Filter tool registry to only allowed tools
                    let reg = tools.lock().await;
                    let agent_tool_defs: Vec<aletheon_abi::ToolDefinition> = reg
                        .definitions()
                        .into_iter()
                        .filter(|d| allowed_tools.contains(&d.name))
                        .collect();
                    drop(reg);

                    // Build messages for the LLM
                    let mut current_messages = vec![
                        aletheon_abi::message::Message::system(&system_prompt),
                        aletheon_abi::message::Message::user(&user_prompt),
                    ];

                    // ReAct loop: up to 20 iterations
                    let mut response_text = String::new();
                    for _ in 0..20 {
                        let response = llm.complete(&current_messages, &agent_tool_defs).await?;

                        // Extract text and tool calls from response
                        let mut text_parts = Vec::new();
                        let mut tool_calls = Vec::new();
                        for block in &response.content {
                            match block {
                                aletheon_abi::message::ContentBlock::Text { text } => {
                                    text_parts.push(text.clone());
                                }
                                aletheon_abi::message::ContentBlock::ToolUse { id, name, input } => {
                                    tool_calls.push((id.clone(), name.clone(), input.clone()));
                                }
                                _ => {}
                            }
                        }

                        if tool_calls.is_empty() {
                            response_text = text_parts.join("\n");
                            break;
                        }

                        // Add assistant message verbatim (text + tool_use blocks)
                        current_messages.push(aletheon_abi::message::Message {
                            role: aletheon_abi::message::Role::Assistant,
                            content: response.content.clone(),
                        });

                        // Execute each tool call
                        for (id, name, input) in tool_calls {
                            let reg = tools.lock().await;
                            let result = if let Some(tool) = reg.get(&name) {
                                let ctx = aletheon_abi::tool::ToolContext {
                                    working_dir: std::env::current_dir().unwrap_or_default(),
                                    session_id: "sub-agent".into(),
                                };
                                tool.execute(input, &ctx).await
                            } else {
                                aletheon_abi::tool::ToolResult {
                                    content: format!("Unknown tool: {}", name),
                                    is_error: true,
                                    ..Default::default()
                                }
                            };
                            drop(reg);

                            // Add tool result as user message
                            current_messages.push(aletheon_abi::message::Message::tool_result(
                                &id,
                                &result.content,
                                result.is_error,
                            ));
                        }
                    }

                    Ok(response_text)
                })
            },
        );

        let agent_tool = aletheon_body::r#impl::tools::agent_tool::AgentTool::new(
            agent_defs.clone(),
            execute_fn,
        );
        use aletheon_abi::Registry;
        if let Err(e) = tools.lock().await.register(Arc::new(agent_tool)) {
            tracing::warn!(error = %e, "Failed to register AgentTool");
        } else {
            info!(agents = agent_defs.len(), "Registered AgentTool with sub-agents");
        }
    }
}
```

**API Notes (verified):**
- `LlmProvider::complete(&self, messages: &[Message], tools: &[ToolDefinition]) -> Result<LlmResponse>`
- `LlmResponse { content: Vec<ContentBlock>, stop_reason: StopReason, usage: Usage }`
- `ContentBlock::Text { text }`, `ContentBlock::ToolUse { id, name, input }`, `ContentBlock::ToolResult { tool_use_id, content, is_error }`
- `Message::system(text)`, `Message::user(text)`, `Message::assistant(text)`, `Message::tool_result(id, content, is_error)`
- `ToolRegistry::definitions()` returns `Vec<ToolDefinition>`
- `ToolRegistry::get(name)` returns `Option<&Arc<dyn Tool>>`

- [ ] **Step 2: Compile and fix errors**

Run: `cargo check -p aletheon-runtime 2>&1 | head -50`

Fix any import issues or type mismatches.

- [ ] **Step 3: Commit**

```bash
git add crates/aletheon-runtime/src/impl/daemon/handler.rs
git commit -m "feat: wire AgentTool into handler with sub-agent ReAct loop (P7.1)"
```

---

## Task 2: Start MCP Embedded Server in Daemon

**Files:**
- Modify: `crates/aletheon-runtime/src/impl/daemon/mod.rs:260-280`

The `McpEmbedded` server is fully implemented but never started. We need to spawn it in the daemon startup.

- [ ] **Step 1: Add MCP server spawn after handler creation**

In `daemon/mod.rs`, after line 263 (`let request_handler = handler::RequestHandler::new(...)`), add:

```rust
// Start MCP embedded server on a separate socket
let mcp_socket = socket.parent().unwrap_or(&PathBuf::from("/tmp/aletheon")).join("aletheon-mcp.sock");
let mcp_server = mcp_embedded::McpEmbedded::new(
    request_handler.tools(),  // Need to expose tools from handler
    mcp_socket.clone(),
);
tokio::spawn(async move {
    if let Err(e) = mcp_server.serve().await {
        tracing::error!("MCP embedded server error: {}", e);
    }
});
info!(path = %mcp_socket.display(), "MCP embedded server started");
```

- [ ] **Step 2: Expose tools from RequestHandler**

Check if `RequestHandler` exposes its `tools: Arc<Mutex<ToolRegistry>>` field. If not, add a getter:

In `handler.rs`, add:
```rust
/// Get a reference to the tool registry (for MCP server).
pub fn tools(&self) -> Arc<Mutex<ToolRegistry>> {
    self.tools.clone()
}
```

- [ ] **Step 3: Add import for McpEmbedded**

In `daemon/mod.rs`, add at the top:
```rust
use mcp_embedded::McpEmbedded;
```

- [ ] **Step 4: Compile and fix errors**

Run: `cargo check -p aletheon-runtime 2>&1 | head -30`

- [ ] **Step 5: Commit**

```bash
git add crates/aletheon-runtime/src/impl/daemon/mod.rs crates/aletheon-runtime/src/impl/daemon/handler.rs
git commit -m "feat: start MCP embedded server in daemon startup (P7.3)"
```

---

## Task 3: Create Agent Definition Files

**Files:**
- Create: `~/.aletheon/agents/code-agent.md`
- Create: `~/.aletheon/agents/fs-agent.md`
- Create: `~/.aletheon/agents/net-agent.md`

- [ ] **Step 1: Create agents directory**

```bash
mkdir -p ~/.aletheon/agents
```

- [ ] **Step 2: Create code-agent.md**

```bash
cat > ~/.aletheon/agents/code-agent.md << 'AGENT_EOF'
---
name: code-agent
description: "Handles code execution, debugging, and code analysis tasks"
tools: [bash_exec, file_read, file_write, apply_patch, grep, glob]
max_iterations: 20
---

You are a code execution sub-agent. Your job is to:
1. Understand the task from the prompt
2. Execute code changes using available tools
3. Verify the changes work
4. Return a clear summary of what you did

## Rules
- Always verify file changes with a read-back after writing
- Use bash_exec for running commands and tests
- Report errors clearly with file paths and line numbers
- Keep changes minimal and focused
AGENT_EOF
```

- [ ] **Step 3: Create fs-agent.md**

```bash
cat > ~/.aletheon/agents/fs-agent.md << 'AGENT_EOF'
---
name: fs-agent
description: "Handles file system operations: read, write, search, and organize files"
tools: [file_read, file_write, file_search, glob, grep, apply_patch]
max_iterations: 15
---

You are a file system sub-agent. Your job is to:
1. Understand the file operation requested
2. Execute the operation safely
3. Verify the result
4. Return a summary

## Rules
- Always check if a file exists before modifying it
- Create parent directories when needed
- Report full file paths in your response
AGENT_EOF
```

- [ ] **Step 4: Create net-agent.md**

```bash
cat > ~/.aletheon/agents/net-agent.md << 'AGENT_EOF'
---
name: net-agent
description: "Handles network operations: web search, fetch, and API calls"
tools: [web_search, web_fetch, bash_exec]
max_iterations: 10
---

You are a network sub-agent. Your job is to:
1. Understand the network request
2. Execute web searches or fetches
3. Extract relevant information
4. Return a concise summary

## Rules
- Always include source URLs when reporting findings
- Handle network errors gracefully
- Prefer web_search for finding information, web_fetch for specific URLs
AGENT_EOF
```

- [ ] **Step 5: Commit the agent definitions**

```bash
git add -A  # if tracked, otherwise just document the creation
echo "Agent definitions created at ~/.aletheon/agents/"
```

---

## Task 4: Add User Hook TOML Loader

**Files:**
- Create: `crates/aletheon-runtime/src/impl/hooks/loader.rs`
- Modify: `crates/aletheon-runtime/src/impl/hooks/mod.rs`

Currently hooks only come from builtin code and skill plugins. We need a loader for user-defined hooks from `~/.aletheon/hooks/*.toml`.

- [ ] **Step 1: Create hooks/loader.rs**

```rust
//! User Hook Loader — scans `~/.aletheon/hooks/*.toml` for hook definitions.
//!
//! TOML format:
//! ```toml
//! [hook]
//! name = "block-dangerous"
//! point = "PreTool"
//! priority = 1
//! script = "/path/to/script.sh"
//! ```

use anyhow::{Context, Result};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use super::registry::RegisteredHook;
use aletheon_abi::hook::HookPoint;

/// A hook definition parsed from a TOML file.
#[derive(Debug, Clone)]
pub struct HookConfig {
    pub name: String,
    pub point: HookPoint,
    pub priority: i32,
    pub script: PathBuf,
}

/// Loads user hook definitions from TOML files.
pub struct HookLoader {
    dir: PathBuf,
}

impl HookLoader {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    /// Load all hook definitions from the directory.
    pub fn load_all(&self) -> Vec<HookConfig> {
        let mut hooks = Vec::new();

        if !self.dir.is_dir() {
            return hooks;
        }

        let entries = match fs::read_dir(&self.dir) {
            Ok(e) => e,
            Err(_) => return hooks,
        };

        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file()
                && path.extension().map_or(false, |e| e.eq_ignore_ascii_case("toml"))
            {
                match load_hook_file(&path) {
                    Ok(hook) => hooks.push(hook),
                    Err(e) => {
                        tracing::warn!(path = %path.display(), error = %e, "Failed to load hook");
                    }
                }
            }
        }

        hooks
    }

    /// Load hooks and register them in the HookRegistry.
    pub fn register_all(&self, registry: &mut super::registry::HookRegistry) -> usize {
        let configs = self.load_all();
        let count = configs.len();
        for config in configs {
            registry.register(RegisteredHook {
                name: config.name,
                source: "user".into(),
                script_path: Some(config.script),
                point: config.point,
                priority: config.priority,
            });
        }
        count
    }
}

fn load_hook_file(path: &Path) -> Result<HookConfig> {
    let content = fs::read_to_string(path)
        .with_context(|| format!("reading {}", path.display()))?;

    // Simple TOML parser for [hook] section
    let mut name = String::new();
    let mut point_str = String::new();
    let mut priority: i32 = 100;
    let mut script = PathBuf::new();

    let mut in_hook_section = false;
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed == "[hook]" {
            in_hook_section = true;
            continue;
        }
        if trimmed.starts_with('[') {
            in_hook_section = false;
            continue;
        }
        if !in_hook_section {
            continue;
        }

        if let Some((key, value)) = parse_toml_kv(trimmed) {
            match key.as_str() {
                "name" => name = unquote(&value),
                "point" => point_str = unquote(&value),
                "priority" => {
                    if let Ok(n) = value.parse::<i32>() {
                        priority = n;
                    }
                }
                "script" => script = PathBuf::from(unquote(&value)),
                _ => {}
            }
        }
    }

    if name.is_empty() || script.as_os_str().is_empty() {
        return Err(anyhow::anyhow!("Missing required fields (name, script)"));
    }

    let point = match point_str.as_str() {
        "SessionStart" | "session_start" => HookPoint::OnSessionStart,
        "PreTool" | "pre_tool" => HookPoint::PreTool,
        "PostTool" | "post_tool" => HookPoint::PostTool,
        "PreResponse" | "pre_response" | "PostTurn" | "post_turn" => HookPoint::PostTurn,
        "SessionEnd" | "session_end" => HookPoint::OnSessionEnd,
        _ => return Err(anyhow::anyhow!("Unknown hook point: {}", point_str)),
    };

    Ok(HookConfig {
        name,
        point,
        priority,
        script,
    })
}

fn parse_toml_kv(line: &str) -> Option<(String, String)> {
    let eq = line.find('=')?;
    let key = line[..eq].trim().to_string();
    let value = line[eq + 1..].trim().to_string();
    if key.is_empty() {
        return None;
    }
    Some((key, value))
}

fn unquote(s: &str) -> String {
    let s = s.trim();
    if (s.starts_with('"') && s.ends_with('"')) || (s.starts_with('\'') && s.ends_with('\'')) {
        s[1..s.len() - 1].to_string()
    } else {
        s.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_load_hook_file() {
        let dir = TempDir::new().unwrap();
        let hook_toml = r#"
[hook]
name = "block-dangerous"
point = "PreTool"
priority = 1
script = "/usr/local/bin/block-dangerous.sh"
"#;
        std::fs::write(dir.path().join("block.toml"), hook_toml).unwrap();

        let loader = HookLoader::new(dir.path().to_path_buf());
        let hooks = loader.load_all();
        assert_eq!(hooks.len(), 1);
        assert_eq!(hooks[0].name, "block-dangerous");
        assert_eq!(hooks[0].priority, 1);
    }

    #[test]
    fn test_load_empty_dir() {
        let dir = TempDir::new().unwrap();
        let loader = HookLoader::new(dir.path().to_path_buf());
        let hooks = loader.load_all();
        assert!(hooks.is_empty());
    }
}
```

- [ ] **Step 2: Add loader module to hooks/mod.rs**

In `crates/aletheon-runtime/src/impl/hooks/mod.rs`, add after line 5:

```rust
pub mod loader;
```

- [ ] **Step 3: Wire hook loader in handler.rs**

In `handler.rs`, after the hook registry initialization (around line 431), add:

```rust
// Load user hooks from ~/.aletheon/hooks/
let hooks_dir = aletheon_dir.join("hooks");
let hook_loader = crate::r#impl::hooks::loader::HookLoader::new(hooks_dir);
let user_hook_count = hook_loader.register_all(&mut hook_registry);
if user_hook_count > 0 {
    info!(count = user_hook_count, "Loaded user hooks");
}
```

Note: This must be BEFORE `let hook_registry = Arc::new(Mutex::new(hook_registry));` (line 431).

- [ ] **Step 4: Compile**

Run: `cargo check -p aletheon-runtime 2>&1 | head -30`

- [ ] **Step 5: Commit**

```bash
git add crates/aletheon-runtime/src/impl/hooks/loader.rs crates/aletheon-runtime/src/impl/hooks/mod.rs crates/aletheon-runtime/src/impl/daemon/handler.rs
git commit -m "feat: add user hook TOML loader (P7.2)"
```

---

## Task 5: Create Hook Definition Files

**Files:**
- Create: `~/.aletheon/hooks/block-dangerous.toml`
- Create: `~/.aletheon/hooks/auto-format.toml`

- [ ] **Step 1: Create hooks directory and scripts**

```bash
mkdir -p ~/.aletheon/hooks
```

- [ ] **Step 2: Create block-dangerous script**

```bash
cat > ~/.aletheon/hooks/block-dangerous.sh << 'SCRIPT_EOF'
#!/bin/bash
# Block dangerous commands in PreTool hooks
# Reads HookContext JSON from stdin
INPUT=$(cat)
TOOL_NAME=$(echo "$INPUT" | jq -r '.tool_name // empty')
TOOL_INPUT=$(echo "$INPUT" | jq -r '.tool_input.command // empty')

# Block dangerous patterns
if echo "$TOOL_INPUT" | grep -qE 'rm -rf /|dd if=/dev/zero|mkfs\.|> /dev/sd'; then
    echo '{"action":"block","reason":"Dangerous command blocked by safety hook"}'
    exit 0
fi

# Allow everything else
exit 0
SCRIPT_EOF
chmod +x ~/.aletheon/hooks/block-dangerous.sh
```

- [ ] **Step 3: Create block-dangerous.toml**

```bash
cat > ~/.aletheon/hooks/block-dangerous.toml << 'TOML_EOF'
[hook]
name = "block-dangerous"
point = "PreTool"
priority = 1
script = "/home/aurobear/.aletheon/hooks/block-dangerous.sh"
TOML_EOF
```

- [ ] **Step 4: Create auto-format script**

```bash
cat > ~/.aletheon/hooks/auto-format.sh << 'SCRIPT_EOF'
#!/bin/bash
# Auto-format files after write operations
INPUT=$(cat)
TOOL_NAME=$(echo "$INPUT" | jq -r '.tool_name // empty')
TOOL_INPUT=$(echo "$INPUT" | jq -r '.tool_input // {}')

if [ "$TOOL_NAME" = "file_write" ] || [ "$TOOL_NAME" = "apply_patch" ]; then
    FILE_PATH=$(echo "$TOOL_INPUT" | jq -r '.path // empty')
    if [ -n "$FILE_PATH" ] && [ -f "$FILE_PATH" ]; then
        case "$FILE_PATH" in
            *.rs) rustfmt "$FILE_PATH" 2>/dev/null ;;
            *.sh|*.bash) shfmt -w "$FILE_PATH" 2>/dev/null ;;
            *.js|*.ts|*.json) npx prettier --write "$FILE_PATH" 2>/dev/null ;;
        esac
    fi
fi

exit 0
SCRIPT_EOF
chmod +x ~/.aletheon/hooks/auto-format.sh
```

- [ ] **Step 5: Create auto-format.toml**

```bash
cat > ~/.aletheon/hooks/auto-format.toml << 'TOML_EOF'
[hook]
name = "auto-format"
point = "PostTool"
priority = 100
script = "/home/aurobear/.aletheon/hooks/auto-format.sh"
TOML_EOF
```

---

## Task 6: Verify TUI Mode Indicator

**Files:**
- Read: `crates/aletheon-body/src/impl/ui/status.rs`
- Read: `crates/aletheon-body/src/impl/ui/mod.rs:1555` (mode_changed handler)

The mode indicator code exists. Verify it works end-to-end.

- [ ] **Step 1: Check mode_changed event parsing**

Run: `grep -A 20 '"mode_changed"' crates/aletheon-body/src/impl/ui/mod.rs`

Verify the handler parses `params.mode` (string) correctly.

- [ ] **Step 2: Check Mode enum has icon() and display_name()**

Run: `grep -n "fn icon\|fn display_name" crates/aletheon-body/src/impl/ui/state.rs`

- [ ] **Step 3: Verify status bar renders mode**

The status.rs code at line 104 already renders:
```rust
format!("{} {}", self.state.mode.icon(), self.state.mode.display_name())
```

This should show `💬 Default`, `📋 Plan`, `⚡ Auto`, or `🔒 Sandbox` in the status bar.

- [ ] **Step 4: Test manually**

```bash
# Start daemon
systemctl --user start aletheond-test

# Send mode change via CLI
cargo run --bin aletheon -- -m "test" --socket /tmp/aletheon/aletheond.sock

# In the TUI, type /mode plan
# Verify status bar shows 📋 Plan
```

---

## Task 7: Integration Testing

- [ ] **Step 1: Full compile check**

```bash
cargo check --workspace 2>&1
```

Expected: Clean compile.

- [ ] **Step 2: Run existing tests**

```bash
cargo test --workspace 2>&1 | tail -30
```

Expected: All tests pass.

- [ ] **Step 3: Test AgentTool manually**

```bash
# Start daemon
systemctl --user restart aletheond-test

# Connect via CLI and ask the LLM to use a sub-agent
cargo run --bin aletheon -- --socket /tmp/aletheon/aletheond.sock -m "Use the code-agent to create a file called /tmp/test-agent.txt with content 'hello from sub-agent'"
```

Expected: LLM calls `agent` tool with `agent_type="code-agent"`, sub-agent creates the file.

- [ ] **Step 4: Test MCP server**

```bash
# Connect to MCP socket
echo '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}' | socat - UNIX-CONNECT:/tmp/aletheon/aletheon-mcp.sock

# List tools
echo '{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}' | socat - UNIX-CONNECT:/tmp/aletheon/aletheon-mcp.sock
```

Expected: Returns server info and tool list.

- [ ] **Step 5: Test hooks**

```bash
# Verify hooks are loaded
# Check daemon logs for "Loaded user hooks"
journalctl --user -u aletheond-test --no-pager -n 20 | grep -i hook
```

---

## Summary

| Task | Component | Effort | Dependency |
|------|-----------|--------|------------|
| 1 | Wire AgentTool | 1h | None |
| 2 | Start MCP Server | 30m | None |
| 3 | Agent definitions | 15m | None |
| 4 | Hook TOML Loader | 1h | None |
| 5 | Hook definitions | 15m | Task 4 |
| 6 | Verify TUI Mode | 15m | None |
| 7 | Integration test | 30m | All |

Tasks 1-4 are independent and can be done in parallel. Task 5 depends on Task 4. Task 7 depends on all others.
