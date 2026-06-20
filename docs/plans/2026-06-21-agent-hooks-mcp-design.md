# Aletheon AgentTool + Hooks + MCP + TUI Design

> Date: 2026-06-21
> Context: Integration testing revealed 4 critical gaps after P0-P6 implementation
> Reference: Claude Code multi-agent architecture, aurb project patterns

---

## 1. Problem Statement

Interactive tmux testing of the Aletheon system revealed 4 gaps:

| # | Gap | Severity | Root Cause |
|---|-----|----------|------------|
| 1 | Sub-agents not exposed as LLM tools | 🔴 Critical | No AgentTool registered in ToolRegistry |
| 2 | Mode indicator not visible in TUI | 🟡 Medium | mode_changed event field mismatch |
| 3 | Hooks system incomplete | 🟡 Medium | Only 1 builtin hook, no user hooks |
| 4 | MCP embedded server not running | 🟡 Medium | Server not started in daemon |

---

## 2. Architecture Overview

```
┌─────────────────────────────────────────────────────────────┐
│                      Aletheon Runtime                        │
│                                                              │
│  ┌──────────────┐    ┌──────────────┐    ┌──────────────┐   │
│  │  ReActLoop   │    │  AgentTool   │    │  HookEngine  │   │
│  │  (主循环)     │───→│  (子代理)     │    │  (生命周期)   │   │
│  └──────┬───────┘    └──────┬───────┘    └──────┬───────┘   │
│         │                   │                   │           │
│  ┌──────▼───────┐    ┌──────▼───────┐    ┌──────▼───────┐   │
│  │ ToolRegistry │    │ AgentLoader  │    │ HookRegistry │   │
│  │ (工具注册)    │    │ (代理定义)    │    │ (Hook 注册)  │   │
│  └──────────────┘    └──────────────┘    └──────────────┘   │
│                                                              │
│  ┌──────────────┐    ┌──────────────┐    ┌──────────────┐   │
│  │ SelfField    │    │ ModeRouter   │    │ MCP Server   │   │
│  │ (权限审查)    │    │ (模式路由)    │    │ (外部接口)    │   │
│  └──────────────┘    └──────────────┘    └──────────────┘   │
└─────────────────────────────────────────────────────────────┘
```

---

## 3. AgentTool — Sub-Agent Delegation

### 3.1 Architecture

Claude Code's multi-agent model is NOT simple proxy delegation. Each sub-agent runs a
**full independent conversation loop** with:

- **Own context window** — fresh messages, no shared history
- **Own tool pool** — filtered from parent's tool list
- **Own system prompt** — from agent definition + environment
- **Own SelfField review** — independent permission checks (not shared with parent)
- **Full ReAct loop** — multiple API calls, tool executions, until done

```
父代理 ReActLoop
  │
  ├─ LLM 推理: "I need to delegate to code-agent"
  ├─ tool_use: Agent(agent_type="code-agent", prompt="Create hello.py with...")
  │
  ├─ AgentTool.execute()
  │    ├─ load_agent_definition("code-agent")
  │    ├─ filter_tool_pool(["bash_exec", "file_read", "file_write"])
  │    ├─ build_system_prompt(agent_def)
  │    ├─ create_isolated_context()
  │    └─ ReActLoop::new(config).run(prompt, llm, tools)
  │         ├─ LLM 推理: "I'll use file_write"
  │         ├─ tool_use: file_write(path="hello.py", content="print('hello')")
  │         ├─ 工具执行
  │         └─ LLM: "Created hello.py successfully"
  │
  └─ 返回结果到父代理
```

### 3.2 Agent Definition Format

Compatible with Claude Code's `.claude/agents/` format (markdown + YAML frontmatter):

```markdown
# ~/.aletheon/agents/code-agent.md
---
name: code-agent
description: "Handles code execution and file operations"
tools: [bash_exec, file_read, file_write, apply_patch]
model: deepseek-v4-flash
max_iterations: 20
permission_mode: default
---

You are a code execution agent. Your job is to:
1. Understand the task from the prompt
2. Execute code changes using available tools
3. Verify the changes work
4. Return a summary of what you did

## Rules
- Always verify file changes with a read-back
- Use bash_exec for running commands
- Report errors clearly
```

### 3.3 AgentTool Implementation

```rust
// crates/aletheon-body/src/impl/tools/agent_tool.rs

pub struct AgentTool {
    agents: HashMap<String, AgentDefinition>,
    llm: Arc<dyn LlmProvider>,
    tools: Arc<Mutex<ToolRegistry>>,
    self_field: Arc<Mutex<SelfField>>,
}

pub struct AgentDefinition {
    pub name: String,
    pub description: String,
    pub tools: Vec<String>,
    pub model: Option<String>,
    pub max_iterations: usize,
    pub permission_mode: String,
    pub system_prompt: String,
}

impl Tool for AgentTool {
    fn name(&self) -> &str { "agent" }

    fn description(&self) -> &str {
        "Spawn a sub-agent to handle a task independently. \
         Available agents: code-agent, fs-agent, net-agent."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "agent_type": {
                    "type": "string",
                    "description": "Agent to spawn",
                    "enum": self.agents.keys().collect::<Vec<_>>()
                },
                "prompt": {
                    "type": "string",
                    "description": "Self-contained task description with ALL context"
                },
                "description": {
                    "type": "string",
                    "description": "Short description of what this agent does"
                }
            },
            "required": ["agent_type", "prompt"]
        })
    }

    async fn execute(&self, params: Value, ctx: &ToolContext) -> ToolResult {
        let agent_type = params["agent_type"].as_str().unwrap();
        let prompt = params["prompt"].as_str().unwrap();

        // 1. Load agent definition
        let agent = self.agents.get(agent_type)
            .ok_or_else(|| format!("Unknown agent: {}", agent_type))?;

        // 2. Filter tool pool
        let agent_tools = self.filter_tools(&agent.tools).await;

        // 3. Build system prompt
        let system_prompt = format!(
            "{}\n\nYou are a sub-agent of type '{}'. \
             Your parent agent delegated this task to you. \
             Execute it and return a clear summary.",
            agent.system_prompt, agent.name
        );

        // 4. Create isolated ReActLoop config
        let config = ReActLoopConfig {
            max_iterations: agent.max_iterations,
            system_prompt,
            ..Default::default()
        };

        // 5. Run independent ReActLoop
        let mut react_loop = ReActLoop::new(config);
        let tools_clone = self.tools.clone();
        let result = react_loop.run(
            prompt,
            &*self.llm,
            &agent_tools,
            move |id: &str, name: &str, input: &Value| {
                let tools = tools_clone.clone();
                let id = id.to_string();
                let name = name.to_string();
                let input = input.clone();
                async move {
                    let reg = tools.lock().await;
                    if let Some(tool) = reg.get(&name) {
                        let result = tool.execute(input, &ToolContext::default()).await;
                        (result.content, result.is_error)
                    } else {
                        (format!("Unknown tool: {}", name), true)
                    }
                }
            },
        ).await;

        // 6. Return result
        match result {
            Ok(text) => ToolResult {
                content: text,
                is_error: false,
                ..Default::default()
            },
            Err(e) => ToolResult {
                content: format!("Agent error: {}", e),
                is_error: true,
                ..Default::default()
            },
        }
    }
}
```

### 3.4 Agent Definition Loader

```rust
// crates/aletheon-runtime/src/impl/agents/loader.rs

pub struct AgentLoader {
    agents_dir: PathBuf,  // ~/.aletheon/agents/
}

impl AgentLoader {
    pub fn new(agents_dir: PathBuf) -> Self {
        Self { agents_dir }
    }

    pub fn load_all(&self) -> HashMap<String, AgentDefinition> {
        let mut agents = HashMap::new();

        if let Ok(entries) = std::fs::read_dir(&self.agents_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().map_or(false, |e| e == "md") {
                    if let Ok(agent) = self.load_one(&path) {
                        agents.insert(agent.name.clone(), agent);
                    }
                }
            }
        }

        agents
    }

    fn load_one(&self, path: &Path) -> Result<AgentDefinition> {
        let content = std::fs::read_to_string(path)?;

        // Parse YAML frontmatter between --- markers
        let (frontmatter, body) = parse_frontmatter(&content)?;

        Ok(AgentDefinition {
            name: frontmatter.get("name")?.as_str()?.to_string(),
            description: frontmatter.get("description")?.as_str()?.to_string(),
            tools: frontmatter.get("tools")?.as_array()?
                .iter().filter_map(|v| v.as_str().map(String::from)).collect(),
            model: frontmatter.get("model").and_then(|v| v.as_str().map(String::from)),
            max_iterations: frontmatter.get("max_iterations")
                .and_then(|v| v.as_u64()).unwrap_or(20) as usize,
            permission_mode: frontmatter.get("permission_mode")
                .and_then(|v| v.as_str()).unwrap_or("default").to_string(),
            system_prompt: body.to_string(),
        })
    }
}
```

### 3.5 Integration with Handler

In `handler.rs`, during initialization:

```rust
// Load agent definitions
let agent_loader = AgentLoader::new(aletheon_abi::paths::agents_dir());
let agent_defs = agent_loader.load_all();
info!(agents = agent_defs.len(), "Loaded agent definitions");

// Register AgentTool
let agent_tool = AgentTool::new(agent_defs, llm.clone(), tools.clone(), self_field.clone());
tools.lock().await.register(Arc::new(agent_tool))?;
```

---

## 4. HookEngine — Lifecycle Hooks

### 4.1 Five Hook Points

```
SessionStart ──→ PreTool ──→ [Tool 执行] ──→ PostTool ──→ PreResponse ──→ [LLM 响应] ──→ SessionEnd
```

### 4.2 Hook Definition Format

```toml
# ~/.aletheon/hooks/format-on-save.toml
[hook]
name = "format-on-save"
point = "PostTool"
hook_type = "Command"
command = "shfmt -w $TOOL_INPUT_PATH"
timeout_ms = 5000

[hook.match]
tool_names = ["file_write", "apply_patch"]
file_patterns = ["*.sh", "*.bash"]
```

### 4.3 Built-in Hooks

| Hook | Point | Type | Implementation |
|------|-------|------|----------------|
| `block-dangerous` | PreTool | Command | Block `rm -rf /`, `dd if=/dev/zero`, etc. |
| `auto-format` | PostTool | Command | shfmt for .sh, rustfmt for .rs, prettier for .js/.ts |
| `inject-context` | PreResponse | Prompt | Inject relevant memories into context |
| `init-session` | SessionStart | Event | Initialize session state, load memories |
| `summarize-session` | SessionEnd | Prompt | LLM summarizes session, stores to episodic memory |

### 4.4 HookEngine Implementation

```rust
// crates/aletheon-runtime/src/impl/hooks/engine.rs

pub struct HookEngine {
    hooks: HashMap<HookPoint, Vec<HookEntry>>,
}

pub struct HookEntry {
    pub config: HookConfig,
    pub matcher: HookMatcher,
}

pub struct HookMatcher {
    pub tool_names: Option<Vec<String>>,
    pub file_patterns: Option<Vec<String>>,
}

impl HookEngine {
    pub async fn execute(&self, point: HookPoint, ctx: &HookContext) -> HookResult {
        let hooks = match self.hooks.get(&point) {
            Some(h) => h,
            None => return HookResult::Continue,
        };

        for hook in hooks {
            if !hook.matcher.matches(ctx) {
                continue;
            }

            let result = self.execute_single(hook, ctx).await;
            match result {
                HookResult::Block { reason } => return HookResult::Block { reason },
                HookResult::ModifyInput { new_input } => {
                    ctx.modify_input(new_input);
                }
                HookResult::Continue => {}
            }
        }

        HookResult::Continue
    }

    async fn execute_single(&self, hook: &HookEntry, ctx: &HookContext) -> HookResult {
        match &hook.config.hook_type {
            HookType::Command => {
                let output = tokio::time::timeout(
                    Duration::from_millis(hook.config.timeout_ms),
                    Command::new("sh")
                        .arg("-c")
                        .arg(&hook.config.command)
                        .env("TOOL_NAME", &ctx.tool_name)
                        .env("TOOL_INPUT", &ctx.tool_input.to_string())
                        .output()
                ).await;

                match output {
                    Ok(Ok(out)) if out.status.success() => HookResult::Continue,
                    Ok(Ok(out)) => HookResult::Block {
                        reason: String::from_utf8_lossy(&out.stderr).to_string()
                    },
                    _ => HookResult::Continue,
                }
            }
            HookType::Prompt => {
                HookResult::ModifyInput {
                    new_input: hook.config.prompt.clone()
                }
            }
            HookType::Event => {
                HookResult::Continue
            }
        }
    }
}
```

### 4.5 Hook Integration with ReActLoop

Hooks are called at specific points in `react_loop.rs`:

```rust
// In run_streaming() method:

// 1. SessionStart — called once at the beginning of run()
hook_engine.execute(HookPoint::SessionStart, &ctx).await;

// 2. PreTool — called before each tool execution (inside the tool loop)
let hook_ctx = HookContext {
    point: HookPoint::PreTool,
    tool_name: Some(name.to_string()),
    tool_input: Some(input.clone()),
    ..Default::default()
};
let hook_result = hook_engine.execute(HookPoint::PreTool, &hook_ctx).await;
if let HookResult::Block { reason } = hook_result {
    // Skip this tool call, return error to LLM
    event_sink.emit(Event::ToolResult { result: Err(format!("Blocked: {}", reason)) });
    continue;
}

// 3. Tool execution
let result = execute_tool(id, name, input).await;

// 4. PostTool — called after each tool execution
let hook_result = hook_engine.execute(HookPoint::PostTool, &hook_ctx).await;

// 5. PreResponse — called before sending final response to user
hook_engine.execute(HookPoint::PreResponse, &ctx).await;

// 6. SessionEnd — called once at the end of run()
hook_engine.execute(HookPoint::SessionEnd, &ctx).await;
```

---

## 5. MCP Embedded Server

### 5.1 Architecture

```
MCP 客户端 (Cursor, VS Code, etc.)
  │
  ├─ 连接 /tmp/aletheon/mcp.sock
  │
  └─ MCP 协议
       ├─ initialize → 返回 server info
       ├─ tools/list → 返回工具定义
       └─ tools/call → 执行工具 → 返回结果
```

### 5.2 Implementation

```rust
// crates/aletheon-runtime/src/impl/daemon/mcp_server.rs

pub struct McpEmbeddedServer {
    socket_path: PathBuf,
    tools: Arc<Mutex<ToolRegistry>>,
}

impl McpEmbeddedServer {
    pub async fn start(&self) -> Result<()> {
        let listener = UnixListener::bind(&self.socket_path)?;
        info!(path = %self.socket_path.display(), "MCP server listening");

        loop {
            let (stream, _) = listener.accept().await?;
            let tools = self.tools.clone();

            tokio::spawn(async move {
                Self::handle_connection(stream, tools).await;
            });
        }
    }

    async fn handle_connection(stream: UnixStream, tools: Arc<Mutex<ToolRegistry>>) {
        let (reader, mut writer) = stream.into_split();
        let mut reader = BufReader::new(reader);
        let mut line = String::new();

        while reader.read_line(&mut line).await.unwrap_or(0) > 0 {
            let request: Value = serde_json::from_str(&line).unwrap_or_default();
            line.clear();

            let response = match request["method"].as_str() {
                Some("initialize") => {
                    json!({
                        "jsonrpc": "2.0",
                        "id": request["id"],
                        "result": {
                            "protocolVersion": "2024-11-05",
                            "serverInfo": {"name": "aletheon", "version": "0.1.0"},
                            "capabilities": {"tools": {"listChanged": false}}
                        }
                    })
                }
                Some("tools/list") => {
                    let reg = tools.lock().await;
                    let tools: Vec<Value> = reg.definitions().iter().map(|d| {
                        json!({
                            "name": d.name,
                            "description": d.description,
                            "inputSchema": d.input_schema
                        })
                    }).collect();

                    json!({
                        "jsonrpc": "2.0",
                        "id": request["id"],
                        "result": {"tools": tools}
                    })
                }
                Some("tools/call") => {
                    let tool_name = request["params"]["name"].as_str().unwrap_or("");
                    let tool_args = &request["params"]["arguments"];

                    let reg = tools.lock().await;
                    if let Some(tool) = reg.get(tool_name) {
                        let result = tool.execute(tool_args.clone(), &ToolContext::default()).await;
                        json!({
                            "jsonrpc": "2.0",
                            "id": request["id"],
                            "result": {
                                "content": [{"type": "text", "text": result.content}],
                                "isError": result.is_error
                            }
                        })
                    } else {
                        json!({
                            "jsonrpc": "2.0",
                            "id": request["id"],
                            "error": {"code": -32601, "message": format!("Unknown tool: {}", tool_name)}
                        })
                    }
                }
                _ => {
                    json!({
                        "jsonrpc": "2.0",
                        "id": request["id"],
                        "error": {"code": -32601, "message": "Unknown method"}
                    })
                }
            };

            let resp_str = serde_json::to_string(&response).unwrap_or_default();
            let _ = writer.write_all(format!("{}\n", resp_str).as_bytes()).await;
            let _ = writer.flush().await;
        }
    }
}
```

### 5.3 Startup Integration

In `daemon/mod.rs`:

```rust
// Start MCP embedded server on separate socket
let mcp_socket = socket.parent().unwrap().join("aletheon-mcp.sock");
let mcp_server = McpEmbeddedServer::new(mcp_socket.clone(), tools.clone());
tokio::spawn(async move {
    if let Err(e) = mcp_server.start().await {
        tracing::error!("MCP server error: {}", e);
    }
});
info!(path = %mcp_socket.display(), "MCP server started");
```

---

## 6. TUI Mode Indicator

### 6.1 Problem

`/mode plan` sends correct RPC, daemon processes it and sends `mode_changed` notification,
but TUI status bar doesn't show the mode.

### 6.2 Root Cause

1. Daemon sends `params.mode` (string)
2. TUI expected `params.new` (already fixed in commit `2c12852b`)
3. Status bar rendering code exists but mode not visible in current layout

### 6.3 Fix

Ensure status bar renders mode icon:

```rust
// crates/aletheon-body/src/impl/ui/status.rs

let mode_display = format!(
    "{} {}",
    app.app_state.mode.icon(),      // 💬 / 📋 / ⚡ / 🔒
    app.app_state.mode.display_name() // Default / Plan / Auto / Sandbox
);
```

---

## 7. File Changes Summary

| File | Action | Description |
|------|--------|-------------|
| `aletheon-body/src/impl/tools/agent_tool.rs` | NEW | AgentTool implementation |
| `aletheon-body/src/impl/tools/mod.rs` | MODIFY | Register AgentTool |
| `aletheon-runtime/src/impl/agents/loader.rs` | NEW | Agent definition loader |
| `aletheon-runtime/src/impl/agents/mod.rs` | NEW | Module declaration |
| `aletheon-runtime/src/impl/hooks/engine.rs` | NEW | HookEngine implementation |
| `aletheon-runtime/src/impl/hooks/loader.rs` | NEW | Hook config loader |
| `aletheon-runtime/src/impl/hooks/mod.rs` | MODIFY | Add engine + loader modules |
| `aletheon-runtime/src/impl/daemon/mcp_server.rs` | NEW | MCP embedded server |
| `aletheon-runtime/src/impl/daemon/mod.rs` | MODIFY | Start MCP server |
| `aletheon-runtime/src/impl/daemon/handler.rs` | MODIFY | Register AgentTool, init HookEngine |
| `aletheon-body/src/impl/ui/status.rs` | MODIFY | Ensure mode icon visible |
| `~/.aletheon/agents/code-agent.md` | NEW | Code agent definition |
| `~/.aletheon/agents/fs-agent.md` | NEW | File system agent definition |
| `~/.aletheon/agents/net-agent.md` | NEW | Network agent definition |
| `~/.aletheon/hooks/block-dangerous.toml` | NEW | PreTool safety hook |
| `~/.aletheon/hooks/auto-format.toml` | NEW | PostTool format hook |

---

## 8. Implementation Priority

| Phase | Task | Effort | Dependency |
|-------|------|--------|------------|
| P7.1 | AgentTool + AgentLoader | 4h | None |
| P7.2 | HookEngine + HookLoader | 3h | None |
| P7.3 | MCP Embedded Server | 2h | None |
| P7.4 | TUI Mode Indicator | 30m | None |
| P7.5 | Agent/Hook definitions | 1h | P7.1, P7.2 |
| P7.6 | Integration + Testing | 2h | All |

Total: ~13h

---

## 9. Test Plan

| Test | Type | What |
|------|------|------|
| AgentTool basic | Unit | Create agent, run simple task |
| AgentTool isolation | Unit | Verify independent context |
| AgentTool tool filtering | Unit | Verify only allowed tools |
| HookEngine PreTool | Unit | Block dangerous command |
| HookEngine PostTool | Unit | Auto-format after write |
| MCP tools/list | Integration | List 23+ tools via MCP |
| MCP tools/call | Integration | Execute tool via MCP |
| TUI mode indicator | Manual | /mode plan → see 📋 |
| Full workflow | Manual | Delegate task → sub-agent executes → result returned |
