# Aletheon Skill Plugin System, Hook Mechanism & Embedded MCP Server

## Overview

This design integrates three mechanisms into Aletheon's existing runtime/body/brain layers:

1. **Skill Plugin System** — Multi-file skill directories with SKILL.md manifest
2. **Lifecycle Hooks** — Synchronous callbacks at key points in the ReAct loop
3. **Embedded MCP Server** — Body tools auto-exposed via MCP protocol

**Design Approach**: Layer-Integrated (方案 A) — extend existing code directly, no new trait abstractions.

**Reference Sources**: Claude Code Skills/Hooks structure + Hermes lifecycle hooks and MemoryProvider plugin pattern.

---

## 1. Skill Plugin System

### 1.1 Directory Structure

```
~/.aletheon/skills/
├── git-workflow/
│   ├── SKILL.md              # Required: manifest + system prompt injection
│   ├── references/           # Optional: reference documents
│   │   └── git-patterns.md
│   └── scripts/              # Optional: executable scripts
│       └── check-branch.sh
├── code-review/
│   ├── SKILL.md
│   └── references/
│       └── review-checklist.md
└── custom-tools/
    ├── SKILL.md              # Declares tool definitions
    └── scripts/
        └── api-call.sh       # Tool implementation
```

### 1.2 SKILL.md Format

```markdown
---
name: git-workflow
version: 1.0.0
description: Git workflow automation skills
trigger: manual
keywords: [git, branch, merge]
tools:
  - name: check_branch
    description: Check if branch is up to date
    script: scripts/check-branch.sh
    permission: L0
    exposure: Direct
hooks:
  pre_tool:
    - name: validate_git_input
      script: scripts/validate-input.sh
      priority: 10
  post_tool:
    - name: log_git_action
      script: scripts/log-action.sh
      priority: 100
---

## System Prompt Injection

When working with git, always use feature branches.
Never commit directly to main.
```

### 1.3 Frontmatter Schema

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `name` | string | Yes | Unique skill identifier |
| `version` | string | No | Semver version (default: "0.1.0") |
| `description` | string | Yes | Human-readable description |
| `trigger` | enum | No | `manual` \| `auto` \| `keyword` (default: `manual`) |
| `keywords` | string[] | No | Activation keywords (for `keyword` trigger) |
| `tools` | ToolDef[] | No | Tools provided by this skill |
| `hooks` | HookDef[] | No | Hooks registered by this skill |

#### ToolDef

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `name` | string | Yes | Tool name (must be unique across all skills) |
| `description` | string | Yes | Tool description for LLM |
| `script` | path | Yes | Relative path to executable script |
| `permission` | enum | No | `L0` \| `L1` \| `L2` \| `L3` (default: `L1`) |
| `exposure` | enum | No | `Direct` \| `Deferred` \| `Hidden` (default: `Direct`) |
| `input_schema` | object | No | JSON Schema for input (auto-generated if omitted) |

#### HookDef (under `hooks.<point>`)

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `name` | string | Yes | Hook identifier |
| `script` | path | Yes | Relative path to executable script |
| `priority` | int | No | Execution order (lower = first, default: 100) |

### 1.4 Core Types

```rust
// runtime/src/impl/skills/plugin.rs

/// A fully parsed skill plugin with all metadata and content.
#[derive(Debug, Clone)]
pub struct SkillPlugin {
    pub name: String,
    pub version: String,
    pub description: String,
    pub trigger: TriggerType,
    pub keywords: Vec<String>,
    pub tools: Vec<SkillToolDef>,
    pub hooks: Vec<SkillHookDef>,
    pub system_prompt: String,
    pub references: Vec<ReferenceFile>,
    pub scripts_dir: PathBuf,
    pub skill_dir: PathBuf,
}

#[derive(Debug, Clone)]
pub enum TriggerType {
    Manual,
    Auto,
    Keyword,
}

#[derive(Debug, Clone)]
pub struct SkillToolDef {
    pub name: String,
    pub description: String,
    pub script: PathBuf,
    pub permission: PermissionLevel,
    pub exposure: ToolExposure,
    pub input_schema: Option<serde_json::Value>,
}

#[derive(Debug, Clone)]
pub struct SkillHookDef {
    pub name: String,
    pub point: HookPoint,
    pub script: PathBuf,
    pub priority: i32,
}

#[derive(Debug, Clone)]
pub struct ReferenceFile {
    pub name: String,
    pub content: String,
}
```

### 1.5 SkillLoader Enhancement

The existing `SkillLoader` (`runtime/src/impl/skills/loader.rs`) is enhanced to support multi-file skill directories:

```rust
impl SkillLoader {
    /// Scan skills directory for subdirectories containing SKILL.md
    pub fn load_all(&mut self) -> usize {
        // 1. Scan for subdirectories (not just .md files)
        // 2. For each subdir with SKILL.md:
        //    a. Parse YAML frontmatter
        //    b. Read references/ directory
        //    c. Validate scripts/ directory
        //    d. Create SkillPlugin
        // 3. Also support legacy single .md files (backward compat)
    }

    /// Parse SKILL.md into SkillPlugin
    fn parse_skill_dir(dir: &Path) -> Result<SkillPlugin> {
        let manifest_path = dir.join("SKILL.md");
        let raw = std::fs::read_to_string(&manifest_path)?;

        // Split frontmatter (between --- markers) and body
        let (frontmatter, body) = parse_frontmatter(&raw)?;

        // Parse YAML frontmatter
        let manifest: SkillManifest = serde_yaml::from_str(&frontmatter)?;

        // Read references
        let references = read_references_dir(&dir.join("references"))?;

        // Resolve scripts directory
        let scripts_dir = dir.join("scripts");

        Ok(SkillPlugin {
            name: manifest.name,
            version: manifest.version.unwrap_or("0.1.0".into()),
            description: manifest.description,
            trigger: manifest.trigger.unwrap_or(TriggerType::Manual),
            keywords: manifest.keywords.unwrap_or_default(),
            tools: manifest.tools.unwrap_or_default(),
            hooks: manifest.hooks.unwrap_or_default(),
            system_prompt: body,
            references,
            scripts_dir,
            skill_dir: dir.to_path_buf(),
        })
    }
}
```

### 1.6 ScriptTool

Tools declared in SKILL.md are wrapped as `ScriptTool` instances that implement the `Tool` trait:

```rust
// body/src/impl/tools/script_tool.rs

/// A tool backed by an external script.
pub struct ScriptTool {
    name: String,
    description: String,
    script_path: PathBuf,
    permission: PermissionLevel,
    exposure: ToolExposure,
    input_schema: serde_json::Value,
}

impl ScriptTool {
    pub fn new(
        name: String,
        description: String,
        script_path: PathBuf,
        permission: PermissionLevel,
    ) -> Self { ... }
}

#[async_trait]
impl Tool for ScriptTool {
    fn name(&self) -> &str { &self.name }
    fn description(&self) -> &str { &self.description }
    fn input_schema(&self) -> serde_json::Value { self.input_schema.clone() }
    fn permission_level(&self) -> PermissionLevel { self.permission }
    fn exposure(&self) -> ToolExposure { self.exposure }

    async fn execute(&self, input: Value, ctx: &ToolContext) -> ToolResult {
        // 1. Serialize input to JSON
        // 2. Spawn script process with input on stdin
        // 3. Read stdout/stderr
        // 4. Parse output as ToolResult
        // 5. Handle timeout, errors
    }

    fn boxed_clone(&self) -> Box<dyn Tool> {
        Box::new(ScriptTool {
            name: self.name.clone(),
            description: self.description.clone(),
            script_path: self.script_path.clone(),
            permission: self.permission,
            exposure: self.exposure,
            input_schema: self.input_schema.clone(),
        })
    }
}
```

### 1.7 Skill Registration

```rust
// runtime/src/impl/skills/plugin.rs

/// Register a skill's tools and hooks into the runtime.
pub fn register_skill(
    skill: &SkillPlugin,
    tool_registry: &mut ToolRegistry,
    hook_registry: &mut HookRegistry,
) -> Result<()> {
    // Register tools
    for tool_def in &skill.tools {
        let script_path = skill.scripts_dir.join(&tool_def.script);
        if !script_path.exists() {
            warn!(skill = %skill.name, tool = %tool_def.name,
                  "Tool script not found: {}", script_path.display());
            continue;
        }

        let tool = ScriptTool::new(
            tool_def.name.clone(),
            tool_def.description.clone(),
            script_path,
            tool_def.permission,
        );
        tool_registry.register(Arc::new(tool));
        info!(skill = %skill.name, tool = %tool_def.name, "Registered skill tool");
    }

    // Register hooks
    for hook_def in &skill.hooks {
        let script_path = skill.scripts_dir.join(&hook_def.script);
        if !script_path.exists() {
            warn!(skill = %skill.name, hook = %hook_def.name,
                  "Hook script not found: {}", script_path.display());
            continue;
        }

        hook_registry.register(RegisteredHook {
            name: format!("{}:{}", skill.name, hook_def.name),
            source: format!("skill:{}", skill.name),
            script_path: Some(script_path),
            point: hook_def.point,
            priority: hook_def.priority,
        });
        info!(skill = %skill.name, hook = %hook_def.name, "Registered skill hook");
    }

    Ok(())
}
```

---

## 2. Lifecycle Hooks

### 2.1 Hook Points

```rust
// abi/src/hook.rs

/// Points in the execution lifecycle where hooks can intervene.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HookPoint {
    // Session lifecycle
    OnSessionStart,
    OnSessionEnd,

    // Turn lifecycle
    PreTurn,
    PostTurn,

    // Tool lifecycle
    PreTool,
    PostTool,

    // Memory lifecycle
    OnMemoryStore,
    OnMemoryRecall,
}
```

### 2.2 Hook Context and Result

```rust
// abi/src/hook.rs

/// Context passed to hook execution.
#[derive(Debug, Clone)]
pub struct HookContext {
    pub point: HookPoint,
    pub session_id: String,
    pub turn_count: usize,
    pub tool_name: Option<String>,
    pub tool_input: Option<serde_json::Value>,
    pub tool_result: Option<ToolResult>,
    pub message: Option<String>,
    pub metadata: HashMap<String, String>,
}

/// Result of hook execution.
#[derive(Debug, Clone)]
pub enum HookResult {
    /// Continue normal execution.
    Continue,
    /// Modify the tool input (only valid for PreTool).
    ModifyInput(serde_json::Value),
    /// Block execution with a reason.
    Block { reason: String },
    /// Inject additional content into the user message.
    Inject(String),
}
```

### 2.3 HookRegistry

```rust
// runtime/src/impl/hooks/registry.rs

pub struct HookRegistry {
    hooks: HashMap<HookPoint, Vec<RegisteredHook>>,
}

#[derive(Debug, Clone)]
pub struct RegisteredHook {
    pub name: String,
    pub source: String,           // "skill:<name>" | "builtin" | "config"
    pub script_path: Option<PathBuf>,
    pub priority: i32,
}

impl HookRegistry {
    pub fn new() -> Self {
        Self { hooks: HashMap::new() }
    }

    /// Register a hook for a specific point.
    pub fn register(&mut self, hook: RegisteredHook) {
        let entry = self.hooks.entry(hook.point).or_default();
        entry.push(hook);
        // Keep sorted by priority
        entry.sort_by_key(|h| h.priority);
    }

    /// Execute all hooks for a given point.
    /// Returns the aggregate result (first Block wins, all Injects merged).
    pub async fn execute(&self, ctx: &HookContext) -> HookResult {
        let hooks = match self.hooks.get(&ctx.point) {
            Some(h) => h,
            None => return HookResult::Continue,
        };

        let mut injections = Vec::new();

        for hook in hooks {
            let result = self.execute_single(hook, ctx).await;
            match result {
                HookResult::Continue => {},
                HookResult::ModifyInput(v) => return HookResult::ModifyInput(v),
                HookResult::Block { reason } => return HookResult::Block { reason },
                HookResult::Inject(s) => injections.push(s),
            }
        }

        if injections.is_empty() {
            HookResult::Continue
        } else {
            HookResult::Inject(injections.join("\n"))
        }
    }

    /// Execute a single hook (script-based or builtin).
    async fn execute_single(
        &self,
        hook: &RegisteredHook,
        ctx: &HookContext,
    ) -> HookResult {
        if let Some(ref script) = hook.script_path {
            // Execute script with context as JSON on stdin
            let ctx_json = serde_json::to_string(ctx).unwrap_or_default();
            match tokio::process::Command::new(script)
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .spawn()
            {
                Ok(mut child) => {
                    if let Some(stdin) = child.stdin.take() {
                        let _ = tokio::io::AsyncWriteExt::write_all(
                            &mut tokio::io::BufWriter::new(stdin),
                            ctx_json.as_bytes(),
                        ).await;
                    }
                    match child.wait_with_output().await {
                        Ok(output) => {
                            parse_hook_output(&output.stdout)
                        }
                        Err(e) => {
                            warn!(hook = %hook.name, error = %e, "Hook execution failed");
                            HookResult::Continue
                        }
                    }
                }
                Err(e) => {
                    warn!(hook = %hook.name, error = %e, "Hook spawn failed");
                    HookResult::Continue
                }
            }
        } else {
            HookResult::Continue
        }
    }
}

/// Parse hook script stdout into HookResult.
fn parse_hook_output(stdout: &[u8]) -> HookResult {
    let text = String::from_utf8_lossy(stdout).trim().to_string();
    if text.is_empty() {
        return HookResult::Continue;
    }

    // Try to parse as JSON for structured responses
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) {
        match value.get("action").and_then(|v| v.as_str()) {
            Some("block") => {
                let reason = value.get("reason")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Blocked by hook")
                    .to_string();
                return HookResult::Block { reason };
            }
            Some("inject") => {
                let content = value.get("content")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                return HookResult::Inject(content);
            }
            Some("modify_input") => {
                if let Some(input) = value.get("input") {
                    return HookResult::ModifyInput(input.clone());
                }
            }
            _ => {}
        }
    }

    // Plain text → inject
    HookResult::Inject(text)
}
```

### 2.4 Integration Points in handler.rs

```rust
// In RequestHandler::handle("chat"):

// 1. OnSessionStart (first turn only)
if turn_count == 0 {
    let ctx = HookContext { point: HookPoint::OnSessionStart, ... };
    hook_registry.execute(&ctx).await;
}

// 2. PreTurn
let ctx = HookContext { point: HookPoint::PreTurn, message: Some(message), ... };
match hook_registry.execute(&ctx).await {
    HookResult::Block { reason } => return error_response(reason),
    HookResult::Inject(text) => effective_message.push_str(&text),
    _ => {}
}

// 3. PreTool (inside tool call loop)
let ctx = HookContext { point: HookPoint::PreTool, tool_name: Some(name), ... };
match hook_registry.execute(&ctx).await {
    HookResult::Block { reason } => { /* skip tool, return error */ },
    HookResult::ModifyInput(v) => input = v,
    _ => {}
}

// 4. PostTool
let ctx = HookContext { point: HookPoint::PostTool, tool_result: Some(result), ... };
hook_registry.execute(&ctx).await;

// 5. PostTurn
let ctx = HookContext { point: HookPoint::PostTurn, ... };
hook_registry.execute(&ctx).await;
```

### 2.5 Builtin Hooks

```rust
// runtime/src/impl/hooks/builtin/audit_hook.rs

/// Logs all tool calls to the audit log.
pub struct AuditHook {
    logger: AuditLogger,
}

impl AuditHook {
    pub fn new(logger: AuditLogger) -> Self { Self { logger } }

    pub fn register(registry: &mut HookRegistry) {
        registry.register(RegisteredHook {
            name: "builtin:audit".into(),
            source: "builtin".into(),
            script_path: None,
            point: HookPoint::PostTool,
            priority: 1000,  // Run last
        });
    }
}
```

---

## 3. Embedded MCP Server

### 3.1 Architecture

```
daemon/handler.rs
├── RequestHandler (existing)
│   ├── handle("chat")    → internal tool calls
│   ├── handle("status")  → status query
│   └── ...
│
└── McpEmbedded (new)
    ├── Gets tool list from ToolRegistry at startup
    ├── Listens on Unix socket (independent from daemon socket)
    ├── tools/list → returns all Direct-exposure tools from ToolRegistry
    └── tools/call → delegates to ToolRegistry.execute()
```

### 3.2 Core Implementation

```rust
// runtime/src/impl/daemon/mcp_embedded.rs

use std::path::PathBuf;
use std::sync::Arc;
use serde_json::{json, Value};
use tokio::net::UnixListener;
use tracing::{info, warn, error};

use aletheon_body::impl::tools::ToolRegistry;

/// Embedded MCP server that exposes body tools via MCP protocol.
pub struct McpEmbedded {
    tool_registry: Arc<ToolRegistry>,
    socket_path: PathBuf,
}

impl McpEmbedded {
    pub fn new(tool_registry: Arc<ToolRegistry>, socket_path: PathBuf) -> Self {
        Self { tool_registry, socket_path }
    }

    /// Start the MCP server, listening on a Unix socket.
    pub async fn serve(&self) -> anyhow::Result<()> {
        // Remove stale socket
        if self.socket_path.exists() {
            std::fs::remove_file(&self.socket_path)?;
        }

        let listener = UnixListener::bind(&self.socket_path)?;
        info!(path = %self.socket_path.display(), "MCP server listening");

        loop {
            match listener.accept().await {
                Ok((stream, _)) => {
                    let registry = self.tool_registry.clone();
                    tokio::spawn(async move {
                        if let Err(e) = Self::handle_connection(stream, registry).await {
                            warn!(error = %e, "MCP connection error");
                        }
                    });
                }
                Err(e) => {
                    error!(error = %e, "MCP accept error");
                }
            }
        }
    }

    async fn handle_connection(
        stream: tokio::net::UnixStream,
        registry: Arc<ToolRegistry>,
    ) -> anyhow::Result<()> {
        use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
        let (reader, mut writer) = stream.into_split();
        let mut reader = BufReader::new(reader);
        let mut line = String::new();

        loop {
            line.clear();
            if reader.read_line(&mut line).await? == 0 {
                break;  // Connection closed
            }

            let request: Value = serde_json::from_str(line.trim())?;
            let response = Self::handle_request(&request, &registry).await;
            let response_str = serde_json::to_string(&response)?;
            writer.write_all(response_str.as_bytes()).await?;
            writer.write_all(b"\n").await?;
            writer.flush().await?;
        }

        Ok(())
    }

    async fn handle_request(request: &Value, registry: &Arc<ToolRegistry>) -> Value {
        let method = request.get("method").and_then(|v| v.as_str()).unwrap_or("");
        let id = request.get("id").cloned().unwrap_or(Value::Null);

        match method {
            "initialize" => json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": {
                    "protocolVersion": "2024-11-05",
                    "capabilities": { "tools": {} },
                    "serverInfo": {
                        "name": "aletheon-embedded-mcp",
                        "version": env!("CARGO_PKG_VERSION")
                    }
                }
            }),
            "tools/list" => {
                let tools = Self::list_tools(registry);
                json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": { "tools": tools }
                })
            }
            "tools/call" => {
                let params = request.get("params").cloned().unwrap_or(json!({}));
                Self::call_tool(id, &params, registry).await
            }
            "ping" => json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": {}
            }),
            _ => json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": {
                    "code": -32601,
                    "message": format!("Method not found: {}", method)
                }
            }),
        }
    }

    fn list_tools(registry: &Arc<ToolRegistry>) -> Vec<Value> {
        registry.definitions()
            .into_iter()
            .map(|def| json!({
                "name": def.name,
                "description": def.description,
                "inputSchema": def.input_schema,
            }))
            .collect()
    }

    async fn call_tool(id: Value, params: &Value, registry: &Arc<ToolRegistry>) -> Value {
        let tool_name = params.get("name").and_then(|v| v.as_str()).unwrap_or("");
        let arguments = params.get("arguments").cloned().unwrap_or(json!({}));

        let tool = match registry.get(tool_name) {
            Some(t) => t,
            None => {
                return json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "error": {
                        "code": -32602,
                        "message": format!("Unknown tool: {}", tool_name)
                    }
                });
            }
        };

        let ctx = aletheon_abi::tool::ToolContext {
            working_dir: std::env::current_dir().unwrap_or_default(),
            session_id: "mcp".into(),
        };

        let result = tool.execute(arguments, &ctx).await;
        let content_text = if result.is_error {
            format!("Error: {}", result.content)
        } else {
            result.content
        };

        json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "content": [{ "type": "text", "text": content_text }],
                "isError": result.is_error
            }
        })
    }
}
```

### 3.3 Configuration

```toml
# ~/.aletheon/config.toml

[mcp]
enabled = true
socket_path = "/run/aletheon/mcp.sock"
```

### 3.4 Integration in handler.rs

```rust
// In RequestHandler::new():
let mcp_socket = config.mcp_socket_path
    .unwrap_or_else(|| PathBuf::from("/run/aletheon/mcp.sock"));
let mcp = McpEmbedded::new(tool_registry.clone(), mcp_socket);

// Spawn MCP server in background
let mcp_clone = mcp;
tokio::spawn(async move {
    if let Err(e) = mcp_clone.serve().await {
        error!(error = %e, "MCP server failed");
    }
});
```

---

## 4. Data Flow

### 4.1 Complete Request Flow

```
User message (via MCP or daemon JSON-RPC)
    │
    ▼
┌─────────────────────────────────────────────────┐
│ runtime: daemon/handler.rs                      │
│                                                 │
│  1. OnSessionStart hooks (first turn only)      │
│  2. PreTurn hooks                               │
│     → may inject context, block, or modify      │
│  3. SelfField review                            │
│  4. Memory queue drain → inject into message    │
│  5. Build messages from session history         │
│  6. LLM complete                                │
│  7. Tool call loop:                             │
│     ┌──────────────────────────────┐            │
│     │ PreTool hooks                │            │
│     │ → may block or modify input  │            │
│     │ Tool.execute() ← body layer  │            │
│     │ PostTool hooks               │            │
│     │ → may inject observations    │            │
│     └──────────────────────────────┘            │
│  8. PostTurn hooks                              │
│  9. Memory store + reflection                   │
│ 10. OnSessionEnd hooks (on session close)       │
└─────────────────────────────────────────────────┘
```

### 4.2 Layer Dependencies

```
abi (trait definitions)
  ├── Tool trait, EventBus trait, Subsystem trait
  ├── HookPoint, HookContext, HookResult (NEW)
  └── No new crate dependencies

body (tool implementations)
  ├── ToolRegistry: register/find/execute tools
  ├── SandboxExecutor: secure execution
  ├── ScriptTool: create tools from skill scripts (NEW)
  └── Depends on: abi

brain (LLM inference)
  ├── LlmProvider: call LLM APIs
  ├── Reflector: reflection analysis
  └── Depends on: abi

runtime (orchestration)
  ├── SkillLoader: scan skills dir, parse SKILL.md (ENHANCED)
  ├── HookRegistry: register/trigger lifecycle hooks (NEW)
  ├── McpEmbedded: embedded MCP server (NEW)
  ├── RequestHandler: integrate all components (MODIFIED)
  ├── AgentRegistry: multi-agent dispatch
  └── Depends on: abi, body, brain
```

---

## 5. File Changes

### 5.1 New Files

| File | Crate | Description |
|------|-------|-------------|
| `abi/src/hook.rs` | aletheon-abi | HookPoint, HookContext, HookResult types |
| `body/src/impl/tools/script_tool.rs` | aletheon-body | ScriptTool implementing Tool trait |
| `runtime/src/impl/hooks/mod.rs` | aletheon-runtime | Hooks module declaration |
| `runtime/src/impl/hooks/types.rs` | aletheon-runtime | Re-export abi::hook types |
| `runtime/src/impl/hooks/registry.rs` | aletheon-runtime | HookRegistry implementation |
| `runtime/src/impl/hooks/builtin/mod.rs` | aletheon-runtime | Builtin hooks module |
| `runtime/src/impl/hooks/builtin/audit_hook.rs` | aletheon-runtime | Audit logging hook |
| `runtime/src/impl/skills/plugin.rs` | aletheon-runtime | SkillPlugin type + register_skill() |
| `runtime/src/impl/skills/manifest.rs` | aletheon-runtime | SKILL.md frontmatter parsing |
| `runtime/src/impl/daemon/mcp_embedded.rs` | aletheon-runtime | Embedded MCP server |

### 5.2 Modified Files

| File | Crate | Changes |
|------|-------|---------|
| `abi/src/lib.rs` | aletheon-abi | Add `pub mod hook;` |
| `body/src/impl/tools/mod.rs` | aletheon-body | Add `pub mod script_tool;` |
| `runtime/src/impl/skills/loader.rs` | aletheon-runtime | Enhance for multi-file skill dirs |
| `runtime/src/impl/skills/mod.rs` | aletheon-runtime | Add `pub mod plugin; pub mod manifest;` |
| `runtime/src/impl/mod.rs` | aletheon-runtime | Add `pub mod hooks;` |
| `runtime/src/impl/daemon/mod.rs` | aletheon-runtime | Add `pub mod mcp_embedded;` |
| `runtime/src/impl/daemon/handler.rs` | aletheon-runtime | Integrate hooks + MCP startup |

---

## 6. Testing Strategy

### 6.1 Unit Tests

- **SkillPlugin parsing**: Test SKILL.md frontmatter parsing with various formats
- **SkillLoader multi-file**: Test loading skills from directory structure
- **ScriptTool execution**: Test tool execution with mock scripts
- **HookRegistry**: Test hook registration, priority ordering, execution
- **HookResult parsing**: Test JSON and plain text output parsing
- **McpEmbedded**: Test tools/list, tools/call, initialize, ping

### 6.2 Integration Tests

- **Skill → Tool → MCP**: Register skill with tool, verify it appears in MCP tools/list
- **Hook chain**: Register multiple hooks, verify execution order
- **Hook block**: Test PreTool hook blocking tool execution
- **Hook inject**: Test PostTurn hook injecting content

---

## 7. Migration Notes

### 7.1 Backward Compatibility

- Existing single `.md` skill files continue to work (SkillLoader detects file vs directory)
- Existing ToolRegistry usage unchanged (ScriptTool is additive)
- MCP server is optional (config-driven, disabled by default)

### 7.2 Migration Path

1. Add `hook.rs` to ABI (no breaking changes)
2. Add `script_tool.rs` to body (additive)
3. Add hooks module to runtime (additive)
4. Enhance SkillLoader (backward compatible)
5. Add McpEmbedded (new feature)
6. Integrate in handler.rs (wiring only)
