# Aletheon CLI Agent Design

**Date:** 2026-06-19
**Status:** Draft
**Scope:** Complete CLI experience — interactive TUI, multi-provider LLM, permissions, context management, MCP, session persistence, skills

---

## 1. Goal

Make aletheon practically usable as an interactive CLI coding agent, inspired by Claude Code, Codex, and OpenCode. Preserve the existing triune brain architecture (SelfField / BrainCore / BodyRuntime) and core/bridge/impl pattern. Add one new crate (`aletheon-tui`), enhance existing crates.

## 2. Architecture Overview

```
┌─────────────────────────────────────────────────────────┐
│                    aletheon-tui (NEW)                    │
│              ratatui TUI + interactive REPL              │
│                deps: crossterm, ratatui                  │
├─────────────────────────────────────────────────────────┤
│                  aletheon-cli (enhanced)                  │
│          subcommand routing: run / daemon / config       │
├──────────┬──────────┬───────────┬───────────────────────┤
│  body    │  brain   │   self    │      runtime          │
│(enhanced)│(enhanced)│ (unchanged)│   (enhanced)         │
│          │          │           │                       │
│ LLM abs. │ provider │           │ context compaction    │
│ streaming│ registry │           │ session persistence   │
│ perms    │          │           │ MCP integration       │
│ tool out │          │           │ hooks enhancement     │
│ mgmt     │          │           │ skills enhancement    │
├──────────┴──────────┴───────────┴───────────────────────┤
│              aletheon-abi (enhanced: new types)          │
│              aletheon-comm (enhanced: SQ/EQ)             │
│              aletheon-memory (unchanged)                 │
└─────────────────────────────────────────────────────────┘
```

**Data flow:**
```
User input → TUI → comm(SQ) → runtime(orchestrator) → brain(reasoning)
                                      ↓
                              body(tool execution) → comm(EQ) → TUI(render)
```

**New crate:** `aletheon-tui` only
**Enhanced crates:** body, brain, runtime, abi, comm
**Unchanged:** memory, self, meta

## 3. LLM Provider Abstraction

### 3.1 Core Trait (aletheon-abi/src/llm_types.rs)

```rust
pub trait LlmProvider: Send + Sync {
    fn stream_chat(&self, request: ChatRequest) -> BoxStream<'static, LlmEvent>;
    fn chat(&self, request: ChatRequest) -> impl Future<Output = Result<ChatResponse>>;
    fn model_info(&self) -> ModelInfo;
}

pub enum LlmEvent {
    Delta { content: String },
    ToolCallStart { id: String, name: String },
    ToolCallDelta { id: String, args_delta: String },
    ToolCallEnd { id: String },
    Thinking { content: String },
    Usage { input_tokens: u32, output_tokens: u32 },
    Done,
    Error { message: String },
}

pub struct ProviderRegistry {
    providers: HashMap<ProviderId, Box<dyn LlmProvider>>,
    aliases: HashMap<String, ProviderId>,
}
```

### 3.2 Provider Implementations

| Provider | Format | Covers |
|----------|--------|--------|
| `OpenAiCompatProvider` | OpenAI Chat Completions | MiMo, DeepSeek, Ollama, etc. |
| `AnthropicProvider` | Anthropic Messages | Claude native |
| `ProviderRouter` | Auto-routing | Model alias → provider selection |

### 3.3 Configuration (~/.aletheon/config.toml)

```toml
[providers.openai_compat]
base_url = "${OPENAI_BASE_URL:-https://api.openai.com/v1}"
api_key = "${OPENAI_API_KEY}"

[providers.anthropic]
api_key = "${ANTHROPIC_API_KEY}"

[providers.ollama]
base_url = "http://localhost:11434"

[model_aliases]
pro = "openai_compat/mimo-v2.5-pro"
flash = "openai_compat/mimo-v2.5-flash"
claude = "anthropic/claude-sonnet-4"
local = "ollama/qwen3:8b"
```

### 3.4 Streaming Pipeline

```
Provider.stream_chat()
  → LlmEvent stream
  → runtime consumes events
  → comm(EQ) pushes to TUI
  → TUI renders in real-time
```

## 4. TUI Design (aletheon-tui crate)

### 4.1 Crate Structure

```
crates/aletheon-tui/
├── Cargo.toml          # deps: ratatui, crossterm, tokio, aletheon-comm
├── src/
│   ├── lib.rs
│   ├── app.rs          # App state machine (core)
│   ├── event.rs        # Event handling (keyboard, mouse, resize)
│   ├── handler.rs      # Event → action mapping
│   ├── tui.rs          # Terminal init/restore
│   ├── ui/
│   │   ├── mod.rs
│   │   ├── chat.rs     # Chat panel (message list, markdown render)
│   │   ├── input.rs    # Input panel (multiline edit, history)
│   │   ├── status.rs   # Status bar (model, tokens, permission mode)
│   │   ├── tools.rs    # Tool execution panel (progress, results)
│   │   └── command.rs  # Command palette (/help, /model, /permission)
│   └── renderer.rs     # Render scheduling
```

### 4.2 App State Machine

```rust
pub struct App {
    state: AppState,
    messages: Vec<Message>,
    input: InputBuffer,
    mode: InputMode,
    status: StatusInfo,
    scroll_offset: usize,
}

pub enum AppState {
    Initializing,
    Ready,
    WaitingForResponse,
    StreamingResponse,
    ToolExecution { tool: String },
    PermissionRequest { tool: String, details: String },
    Error { message: String },
    Quitting,
}

pub enum InputMode {
    Normal,
    Command,
    Permission,  // y/n/a/d
    Multiline,   // Shift+Enter
}
```

### 4.3 Layout (Three Panels)

```
┌─────────────────────────────────────┐
│  Status Bar  [model: pro] [tokens]  │
├─────────────────────────────────────┤
│                                     │
│         Chat / Messages             │
│    (markdown render, tool results)  │
│                                     │
├─────────────────────────────────────┤
│  Tool Execution (collapsible)       │
├─────────────────────────────────────┤
│  > Input Area                       │
│    (multiline, history, Tab comp)   │
└─────────────────────────────────────┘
```

### 4.4 Key Bindings

| Key | Action |
|-----|--------|
| Enter | Send message |
| Shift+Enter | Newline |
| Ctrl+C | Interrupt / exit |
| Ctrl+L | Clear screen |
| Up/Down | History / scroll |
| Tab | Command completion |
| Esc | Cancel current mode |
| / | Enter command mode |

### 4.5 Communication with Runtime

TUI communicates with runtime via `aletheon-comm` SQ/EQ channels (reusing existing IPC mechanisms in `aletheon-comm`):

- **SQ (user → runtime):** `UserMessage`, `Interrupt`, `PermissionResponse`, `SwitchModel`
- **EQ (runtime → user):** `AssistantDelta`, `ToolCallStart`, `ToolResult`, `PermissionRequest`, `Error`

## 5. Permission System

### 5.1 Core Types (aletheon-abi/src/permission.rs)

```rust
pub enum PermissionBehavior { Allow, Deny, Ask }

pub struct PermissionRule {
    pub tool: String,
    pub pattern: Option<String>,
    pub behavior: PermissionBehavior,
}

pub enum PermissionMode {
    Default,       // dangerous ops ask
    AcceptEdits,   // file edits auto-approve
    Plan,          // read-only
    Auto,          // AI classifier decides
    BypassAll,     // all auto-approve (restricted env)
}

pub struct PermissionContext {
    pub mode: PermissionMode,
    pub rules: Vec<PermissionRule>,
    pub session_approvals: HashSet<String>,
}
```

### 5.2 Rule Sources (highest to lowest priority)

1. `policy` — enterprise policy (read-only)
2. `cli` — command-line flags
3. `project` — `.aletheon/settings.toml`
4. `user` — `~/.aletheon/settings.toml`
5. `session` — ephemeral session rules

### 5.3 Permission Check Flow

```
Tool.call(args)
  → body::security::check_permission(tool, args, &perm_ctx)
  → match rules: Allow / Deny / Ask
  → Ask → comm sends PermissionRequest to TUI
  → TUI shows confirmation dialog (y/n/a=always/d=deny)
  → user responds → comm returns PermissionResponse
  → cache in session_approvals
  → continue or reject
```

### 5.4 Configuration (~/.aletheon/settings.toml)

```toml
[permissions]
mode = "default"

[[permissions.rules]]
tool = "bash_exec"
pattern = "git *"
behavior = "allow"

[[permissions.rules]]
tool = "bash_exec"
pattern = "rm -rf *"
behavior = "deny"

[[permissions.rules]]
tool = "file_write"
behavior = "ask"
```

## 6. Context Management & Compaction

### 6.1 Compaction Layers

| Layer | Trigger | Strategy | Reference |
|-------|---------|----------|-----------|
| 1. Tool result budget | Single message exceeds threshold | Truncate + persist to disk, keep preview | Claude Code |
| 2. Microcompact | Every N turns | Clear stale tool results, keep summary | Claude Code |
| 3. Sliding window | Message count exceeds limit | Drop oldest non-system messages | OpenCode |
| 4. Auto-compact | Near context window | Model summarizes conversation | Codex |
| 5. Reactive compact | API returns prompt-too-long | Emergency compact + retry | Claude Code |

### 6.2 Token Budget Tracking

```rust
pub struct TokenBudget {
    pub context_window: u32,
    pub max_output_tokens: u32,
    pub used_tokens: u32,
    pub reserve_tokens: u32,
}

impl TokenBudget {
    pub fn should_compact(&self) -> bool {
        self.used_tokens > (self.context_window - self.reserve_tokens) * 80 / 100
    }

    pub fn available(&self) -> u32 {
        self.context_window.saturating_sub(self.used_tokens + self.reserve_tokens)
    }
}
```

### 6.3 Auto-Compact Implementation

```rust
pub async fn auto_compact(
    messages: &[Message],
    budget: &TokenBudget,
    provider: &dyn LlmProvider,
) -> Result<CompactedHistory> {
    // 1. Extract system messages (preserve)
    // 2. Select message range to compress
    // 3. Call LLM to generate summary
    // 4. Replace original messages with summary
    // 5. Return new message list
}
```

## 7. Tool System Enhancement

### 7.1 Tool Output Management

```rust
pub struct ToolOutputConfig {
    pub max_preview_lines: usize,    // default 50
    pub max_preview_bytes: usize,    // default 10_000
    pub persist_threshold: usize,    // default 50_000
    pub temp_dir: PathBuf,
}

pub enum ToolOutputResult {
    Inline(String),
    Truncated { preview: String, full_path: PathBuf },
}
```

### 7.2 Streaming Tool Execution (integrate into existing `body/impl/tools/executor.rs`)

```rust
pub struct StreamingToolExecutor {
    pending: VecDeque<ToolCall>,
    running: JoinSet<ToolResult>,
    concurrency_limit: usize,
}

impl StreamingToolExecutor {
    pub fn submit(&mut self, call: ToolCall) { ... }
    pub async fn next_result(&mut self) -> Option<ToolResult> { ... }
}
```

### 7.3 Built-in Tools

| Tool | Status | Description |
|------|--------|-------------|
| `bash_exec` | existing | Shell command execution |
| `file_read` | existing | File read |
| `file_write` | existing | File write |
| `file_search` | existing | File search |
| `apply_patch` | existing | Patch apply |
| `process_list` | existing | Process list |
| `system_status` | existing | System status |
| `code_graph` | existing | Code graph analysis |
| `script_tool` | existing | Script execution |
| `tool_search` | existing | Tool discovery |
| `web_fetch` | **new** | HTTP request |
| `web_search` | **new** | Web search |
| `glob` | **new** | Glob file finder |
| `grep` | **new** | Content search |
| `task_*` | **new** | Task management (create/update/list/get) |
| `agent` | **new** | Sub-agent dispatch (reuses existing `runtime/impl/orchestration/`) |

### 7.4 Sandbox Enhancement

```rust
pub struct SandboxProfile {
    pub read_roots: Vec<PathBuf>,
    pub write_roots: Vec<PathBuf>,
    pub deny_paths: Vec<PathBuf>,
    pub network_enabled: bool,
    pub env_vars: HashMap<String, String>,
}

pub enum SandboxStrategy {
    None,
    Bubblewrap(SandboxProfile),
    Auto,
}
```

## 8. MCP Integration

### 8.1 Enhanced MCP Manager

```rust
pub struct McpManager {
    connections: HashMap<String, McpConnection>,
    tool_registry: McpToolRegistry,
}

impl McpManager {
    pub async fn connect(&mut self, config: &McpServerConfig) -> Result<()>;
    pub async fn discover_tools(&mut self) -> Result<Vec<McpToolDef>>;
    pub async fn call_tool(&self, server: &str, tool: &str, args: Value) -> Result<ToolResult>;
    pub async fn read_resource(&self, server: &str, uri: &str) -> Result<String>;
}
```

### 8.2 Configuration (~/.aletheon/config.toml)

```toml
[mcp_servers.filesystem]
command = "npx"
args = ["-y", "@modelcontextprotocol/server-filesystem", "/home/user"]

[mcp_servers.github]
command = "npx"
args = ["-y", "@modelcontextprotocol/server-github"]
env = { GITHUB_TOKEN = "${GITHUB_TOKEN}" }
```

MCP-discovered tools are injected into `ToolRegistry` alongside built-in tools.

## 9. Session Management

### 9.1 Session Store

```rust
pub struct SessionStore {
    db: rusqlite::Connection,
}

pub struct Session {
    pub id: String,
    pub created_at: DateTime<Utc>,
    pub model: String,
    pub messages: Vec<Message>,
    pub metadata: SessionMetadata,
}

impl SessionStore {
    pub fn create(&self, model: &str) -> Result<Session>;
    pub fn load(&self, id: &str) -> Result<Session>;
    pub fn save_message(&self, session_id: &str, msg: &Message) -> Result<()>;
    pub fn list_recent(&self, limit: usize) -> Result<Vec<Session>>;
    pub fn archive(&self, id: &str) -> Result<()>;
}
```

### 9.2 Session Recovery

- `aletheon run` (no args) → load most recent session
- `aletheon run --session <id>` → restore specific session
- `aletheon run --new` → force new session
- Auto-save every message (SQLite)

### 9.3 Hooks Enhancement

```rust
pub enum HookEvent {
    SessionStart { session_id: String },
    PreToolUse { tool: String, args: Value },
    PostToolUse { tool: String, result: &ToolResult },
    PreCompact { message_count: usize },
    PostCompact { new_count: usize },
    UserPromptSubmit { prompt: String },
    Stop { reason: StopReason },
}
```

### 9.4 Skills Enhancement

- Load skill definitions from `~/.aletheon/skills/`
- Each skill = `.toml` (metadata) + `.md` (instructions)
- Keyword-matched injection into conversation context

## 10. Phased Implementation Plan

### Phase 1: LLM Working + Basic Interaction (1-2 weeks)

**Goal:** `aletheon run "hello"` calls LLM and returns result

- [ ] Enhance `aletheon-abi`: `LlmProvider` trait, `LlmEvent`, `ChatRequest/Response` types
- [ ] Enhance `aletheon-brain`: `ProviderRegistry`, `OpenAiCompatProvider` implementation
- [ ] Enhance `aletheon-body/src/impl/cli/`: simple REPL (readline style)
- [ ] Enhance `aletheon-runtime`: ReAct loop connects to LLM streaming
- [ ] Configuration: provider config, model aliases
- [ ] Verify: `aletheon run "what is 1+1"` returns correct answer

### Phase 2: TUI + Permissions (2-3 weeks)

**Goal:** ratatui TUI interactive, permission system working

- [ ] Create `aletheon-tui` crate: App state machine, three-panel layout, keybindings
- [ ] Enhance `aletheon-comm`: SQ/EQ message types (`LlmEvent`, `PermissionRequest/Response`)
- [ ] Enhance `aletheon-body`: permission check logic, rule matching engine
- [ ] Enhance `aletheon-abi`: permission-related types
- [ ] Configuration: `~/.aletheon/settings.toml` permission rules
- [ ] Verify: TUI conversation, bash execution, permission prompts work

### Phase 3: Context Management + Tool Enhancement (2-3 weeks)

**Goal:** Long conversations don't crash, tool output management complete

- [ ] Enhance `aletheon-runtime`: multi-layer compaction (tool budget, microcompact, auto-compact)
- [ ] Enhance `aletheon-body`: tool output truncation, disk persistence, streaming execution
- [ ] New tools: `web_fetch`, `glob`, `grep`, `task_*`
- [ ] Enhance sandbox: `SandboxProfile`, strategy selection
- [ ] Verify: 100+ turn conversation doesn't OOM, large output correctly truncated

### Phase 4: MCP + Session + Skills (2-3 weeks)

**Goal:** MCP integration, session persistence, skills system

- [ ] Enhance `aletheon-body/mcp`: MCP tool discovery, invocation, resource reading
- [ ] Enhance `aletheon-runtime`: Session persistence (SQLite), recovery
- [ ] Enhance `aletheon-runtime`: Hooks enhancement (7 event types)
- [ ] Enhance `aletheon-runtime`: Skills loading, injection
- [ ] Verify: MCP server connection, session recovery, skill injection

### Later Phases

- Phase 5: Background daemon + event-driven
- Phase 6: Multi-agent orchestration (coordinator mode)
- Phase 7: Self-evolution (meta runtime enhancement)

## 11. Reference Sources

| Source | What was borrowed |
|--------|-------------------|
| Claude Code | Permission rules, context compaction layers, streaming tool execution, tool result budget |
| Codex | SQ/EQ protocol pattern, ratatui TUI approach, crate structure, sandbox architecture |
| OpenCode | Provider abstraction pattern, session management, system context reconciliation |
| Existing aletheon | Triune brain, core/bridge/impl pattern, ReAct loop, tool registry, hooks framework |
