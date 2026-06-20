# TUI / Runtime / Brain Overhaul Design

**Date:** 2026-06-21
**Status:** Draft
**Scope:** Full stack — TUI, Runtime, Brain, Skills, Hooks, Testing

## 1. Overview

This design comprehensively overhauls Aletheon's user-facing layer, runtime orchestration, and brain integration. Inspired by patterns from Claude Code (React/Ink architecture, permission modes, hooks, skills), Codex (SQ/EQ protocol, collaboration modes, style guide), OpenCode (context management), and DeepSeek Reasonix (config-driven, two-model collaboration).

### 1.1 Goals

- Make TUI production-ready (decompose monolith, add modes, status line, interrupt)
- Surface brain awareness signals and plan mode in TUI
- Add sub-agent orchestration with inline TUI display
- Implement hooks system (core 5 + event bus)
- Implement two-layer skills system (Rust built-in + Markdown user skills)
- tmux-based automated testing with real LLM

### 1.2 Key Decisions

| Decision | Choice | Rationale |
|----------|--------|-----------|
| Approach | Reference-Driven Redesign | Leverages existing code, incorporates best patterns |
| Phasing | All parallel, dependency order | Unified spec, incremental implementation |
| Monolith | Progressive decomposition | Module split first, crate split later |
| Status line | Hybrid (built-in + shell script) | Open-box default + full customization |
| Sub-agents | Inline display | Non-disruptive, like Claude Code |
| Hooks | Core 5 + event bus | Covers 80% use cases, extensible |
| Awareness | Dual-layer (status bar + inline) | Always visible + important transitions |
| Plan mode | Claude Code flow + BrainCore visualization | Best of both worlds |
| Skills | Two-layer (Rust + Markdown) | Performance + user extensibility |
| Testing | Real LLM, tmux, JSONL frame validation | High fidelity, reproducible |

## 2. Architecture

### 2.1 Five-Layer Model

```
┌─────────────────────────────────────────────────┐
│  Layer 1: Interaction (TUI)                      │
│  ratatui/crossterm · modes · status line · hooks │
├─────────────────────────────────────────────────┤
│  Layer 2: Orchestration (Session)                │
│  SessionManager · ModeRouter · ApprovalGate      │
├─────────────────────────────────────────────────┤
│  Layer 3: Cognitive (Runtime + Brain)             │
│  ReActLoop · BrainCore · SelfField · Evolution   │
├─────────────────────────────────────────────────┤
│  Layer 4: Capabilities (Tools + Skills)           │
│  ToolRegistry · SkillRouter · Hooks · MCP        │
├─────────────────────────────────────────────────┤
│  Layer 5: Foundation (Memory + Comm)              │
│  MemoryRouter · EventBus · IPC · LLM Providers   │
└─────────────────────────────────────────────────┘
```

### 2.2 Communication Protocol

Inspired by Codex SQ/EQ protocol, adapted to Aletheon's existing JSON-RPC over Unix socket:

**TUI → Runtime (`Command` enum):**
- `UserMessage { text: String }` — user input
- `Interrupt { reason: InterruptReason }` — cancel streaming
- `ChangeMode { mode: CollaborationMode }` — switch mode
- `ApprovalResponse { approved: bool }` — tool approval
- `ModelSwitch { model: String }` — change LLM model
- `PlanApproval` — approve plan and execute

**Runtime → TUI (`UiEvent` enum):**
- `TextDelta { text: String }` — streaming text
- `ThinkingDelta { text: String }` — thinking content
- `ToolCallStart { id, name, input }` — tool invocation begins
- `ToolCallResult { id, output, success }` — tool result
- `Usage { tokens, cost }` — token/cost update
- `TurnDone { response, interrupted }` — turn complete
- `Error { message }` — error occurred
- `ApprovalRequest { tool, input, risk }` — needs user approval
- `AwarenessSignal { signal, context }` — brain awareness change
- `PlanUpdate { version, plan, critique }` — plan mode update
- `SubAgentStatus { agent_id, status }` — sub-agent state change
- `ModeChanged { old, new }` — mode switched
- `EvolutionProgress { stage, detail }` — evolution status
- `ContextUpdate { used, max }` — context usage change
- `ModelSwitch { from, to }` — model changed
- `Interrupt { reason }` — interrupt acknowledged

## 3. TUI Layer

### 3.1 Monolith Decomposition

Current `ui/mod.rs` (2055 lines) → progressive split:

```
ui/
├── mod.rs           (~100 lines) — re-exports, run_with_config()
├── app.rs           (~400 lines) — App struct, state machine, event loop
├── render.rs        (~300 lines) — layout composition, widget orchestration
├── dispatch.rs      (~200 lines) — event → action mapping
├── state.rs         (~150 lines) — AppState enum, mode, context
├── chat.rs          (existing) — ChatWidget
├── input.rs         (existing) — CommandHistory
├── command.rs       (existing + extend) — /command parser
├── streaming.rs     (existing) — StreamController
├── status.rs        (rewrite) — StatusBar with awareness + context
├── pager.rs         (existing) — PagerOverlay
├── toolcard.rs      (existing) — ToolCard
├── completion.rs    (existing) — CompletionPopup
├── approval_dialog.rs (existing) — ApprovalDialog
├── markdown.rs      (existing) — markdown rendering
├── term_compat.rs   (existing) — terminal capabilities
├── help_overlay.rs  (existing) — help overlay
├── skill.rs         (existing) — SkillLoader
├── computer.rs      (existing) — computer use UI
├── plan_view.rs     (NEW) — plan mode visualization
├── subagent_view.rs (NEW) — sub-agent inline display
└── awareness.rs     (NEW) — awareness signal indicators
```

### 3.2 Collaboration Modes

Inspired by Claude Code's permission modes, mapped to Aletheon's SelfField verdicts:

| Mode | Icon | SelfField Default | Behavior |
|------|------|-------------------|----------|
| `default` | `💬` | Allow (with Ask for destructive) | Normal operation, ask for risky tools |
| `plan` | `📋` | Deny (all mutations) | Read-only explore + generate plan, user approves before execution |
| `auto` | `⚡` | Allow (all) | No approval prompts, full autonomy |
| `sandbox` | `🔒` | SandboxFirst | All side-effect tools run in sandbox first |

**Mode switching:** `/mode <name>` command or `Ctrl+M` cycle.

**Mode affects:**
- `ApprovalGate` policy (what requires user approval)
- System prompt injection (mode-specific instructions, like Codex collaboration-mode-templates)
- TUI status bar indicator

**Mode-specific system prompts:**

```
# default mode
You are Aletheon, a persistent self-evolving AI agent. Operate normally.
Ask for user approval before destructive operations.

# plan mode
You are in PLAN MODE. You may only use read-only tools (glob, grep, read, web_fetch).
Generate a detailed plan. Do NOT execute any mutations.
Wait for user approval before proceeding.

# auto mode
You are in AUTO MODE. Execute without asking for approval.
Be thorough and autonomous. Persist until the task is fully handled.

# sandbox mode
You are in SANDBOX MODE. All side-effect operations run in a sandbox first.
Review sandbox results before applying to the real environment.
```

### 3.3 Status Line (Hybrid)

**Default built-in status bar:**
```
💬 default | claude-sonnet-4-6 | ctx: 45k/200k (22%) | tokens: 12.5k | 💚 confident | 3 tools used
```

Fields: mode icon + name, model, context usage (bar), token count, awareness state, tool count.

**Shell script override** (like Claude Code):
- User provides script path in config
- Main process sends JSON via stdin: `{"session_id","model","context_used","context_max","tokens","awareness","mode","tools_used","cost"}`
- Script outputs ANSI-formatted text
- Refresh: event-driven + configurable interval (default 5s)

**Configuration:**
```toml
[ui.status_line]
# "builtin" (default) or "script"
mode = "builtin"
# Only used when mode = "script"
script = "~/.aletheon/status.sh"
refresh_interval_secs = 5
```

### 3.4 Keyboard Shortcuts

| Key | Action |
|-----|--------|
| `Ctrl+C` | Cancel streaming / clear input / double-press quit |
| `Ctrl+D` | Quit (when input empty) |
| `Ctrl+L` | Clear screen |
| `Ctrl+O` | Toggle thinking display |
| `Ctrl+B` | Toggle last tool card |
| `Ctrl+T` | Open pager overlay |
| `Ctrl+M` | Cycle collaboration mode |
| `Ctrl+I` | Toggle awareness indicators |
| `Ctrl+P` | Enter/exit plan mode |
| `Ctrl+A/E` | Cursor to beginning/end |
| `Ctrl+W` | Delete word backward |
| `Ctrl+K/U` | Delete to end/beginning |
| `Tab` | Trigger completion |
| `Esc` | Hide completion / clear input |
| `PageUp/Down` | Scroll chat |

### 3.5 Commands

| Command | Alias | Description |
|---------|-------|-------------|
| `/help` | `/h` | Show help overlay |
| `/clear` | — | Clear screen |
| `/status` | `/st` | Show detailed status |
| `/quit` | `/q` | Quit |
| `/copy` | `/cp` | Copy last response |
| `/reflect` | `/r` | Trigger reflection |
| `/reflect_now` | `/rn` | Immediate reflection |
| `/evolution` | `/evo` | Evolution status |
| `/genome` | `/gene` | Genome inspection |
| `/computer` | — | Computer use mode |
| `/sessions` | `/sess` | List sessions |
| `/resume <id>` | — | Resume session |
| `/compact` | `/cmp` | Manual context compaction |
| `/model <name>` | `/m` | Switch model |
| `/mode <name>` | — | Switch collaboration mode |
| `/plan` | `/p` | Enter plan mode |
| `/approve` | `/a` | Approve current plan |
| `/agents` | `/ag` | List active sub-agents |
| `/agent <id>` | — | Show sub-agent details |
| `/hooks` | `/hk` | List registered hooks |
| `/skills` | `/sk` | List available skills |
| `/skill <name>` | — | Run a skill |
| `/interrupt` | `/int` | Send interrupt signal |
| `/context` | `/ctx` | Show context breakdown |

### 3.6 New Widgets

#### Plan View (`plan_view.rs`)

```
┌─ Plan v1 ─────────────────────────────────────┐
│ 1. Add auth middleware to src/middleware.rs     │
│ 2. Update routes to use auth                   │
│ 3. Add tests                                   │
├─ Critique ─────────────────────────────────────┤
│ ⚠ Risk: No error handling for invalid tokens   │
│ ⚠ Completeness: Missing rate limiting          │
├─ Plan v2 (revised) ────────────────────────────┤
│ 1. Add auth middleware with error handling      │
│ 2. Add rate limiting middleware                 │
│ 3. Update routes to use both                   │
│ 4. Add tests for edge cases                    │
├─ Critique ─────────────────────────────────────┤
│ ✅ No critical issues remaining                │
└────────────────────────────────────────────────┘
```

#### Sub-Agent View (`subagent_view.rs`)

```
┌─ SubAgent: fix-auth-bug ──────────────────── executing ─┐
│  Step 3/5: Modifying src/auth.rs                         │
│  ████████████░░░░░ 60%                                   │
└──────────────────────────────────────────────────────────┘
```

#### Awareness Indicator (`awareness.rs`)

Status bar section (always visible):
```
💚 confident
```

Inline transitions (only on state change):
```
⚡ 检测到impasse，切换策略 → 从ChainOfThought切换到Direct
```

## 4. Runtime Layer

### 4.1 Session Management

New `SessionManager` extracted from daemon handler:

```rust
// crates/aletheon-runtime/src/core/session.rs
pub struct SessionManager {
    sessions: HashMap<String, Session>,
    active_session: Option<String>,
}

pub struct Session {
    pub id: String,
    pub mode: CollaborationMode,
    pub messages: Vec<Message>,
    pub context_state: ContextState,
    pub approval_policy: ApprovalPolicy,
    pub sub_agents: Vec<SubAgentHandle>,
    pub created_at: Instant,
    pub model_override: Option<String>,
}

pub struct ContextState {
    pub used_tokens: usize,
    pub max_tokens: usize,
    pub compaction_count: usize,
    pub last_compaction: Option<Instant>,
}
```

**Session persistence:** JSONL transcript to `~/.aletheon/sessions/<id>.jsonl` (like Claude Code). `/resume <id>` restores session state.

**Model hot-switching:** `/model <name>` changes LLM provider mid-session without losing history. Runtime re-initializes provider, sends updated tool definitions.

### 4.2 Mode Router

Routes user intents through different paths based on collaboration mode:

```rust
// crates/aletheon-runtime/src/core/mode_router.rs
pub enum CollaborationMode {
    Default,   // Normal: SelfField reviews, approval for destructive
    Plan,      // Read-only: BrainCore generates plan, no execution until approve
    Auto,      // Full autonomy: no approval prompts
    Sandbox,   // All side-effects in sandbox first
}

pub struct ModeRouter {
    current_mode: CollaborationMode,
    mode_configs: HashMap<CollaborationMode, ModeConfig>,
}

pub struct ModeConfig {
    pub system_prompt_suffix: String,
    pub approval_policy: ApprovalPolicy,
    pub allowed_tools: Option<HashSet<String>>,  // None = all
    pub max_iterations: Option<usize>,
}
```

**Mode → SelfField mapping:**
- `Default` → standard verdict handling (current behavior)
- `Plan` → all tool intents get `Deny` verdict except read-only tools; BrainCore generates plan; user approves → switch to Default for execution
- `Auto` → all verdicts treated as `Allow`; no approval gate
- `Sandbox` → all side-effect verdicts become `SandboxFirst`

### 4.3 Interrupt Mechanism

```rust
pub enum InterruptReason {
    UserCancelled,      // Ctrl+C during streaming
    Timeout,            // Turn took too long
    BudgetExceeded,     // Token/cost limit hit
}
```

**IPC:** `{"method":"interrupt","params":{"reason":"user_cancelled"}}`

**ReActLoop implementation:**
1. Check `AtomicBool` cancel flag between iterations
2. On interrupt: abort HTTP request, emit partial response
3. Clean up in-flight tool executions (wait for serial, abort read-only)
4. Emit `TurnDone { interrupted: true, partial_response }`

### 4.4 Sub-Agent Orchestration

```rust
// crates/aletheon-runtime/src/core/sub_agent.rs
pub struct SubAgentHandle {
    pub id: String,
    pub task: String,
    pub status: SubAgentStatus,
    pub parent_turn_id: String,
    pub spawned_at: Instant,
}

pub enum SubAgentStatus {
    Planning,
    Executing { current_step: String },
    WaitingApproval,
    Completed { summary: String },
    Failed { error: String },
}
```

**Orchestration patterns** (exposed as tools the LLM can call):
- `pipeline(items, stage1, stage2, ...)` — sequential per-item
- `parallel(thunks)` — concurrent with barrier
- `agent(prompt, opts)` — spawn sub-agent

### 4.5 Hooks System

**Core 5 hooks:**

| Hook | Trigger | Payload |
|------|---------|---------|
| `session_start` | Session created | session_id, mode, model |
| `pre_tool` | Before tool execution | tool_name, input, verdict |
| `post_tool` | After tool execution | tool_name, input, output, duration, success |
| `pre_response` | Before final response | response_text, token_usage |
| `session_end` | Session closed | session_id, duration, total_tokens |

**Hook types:**
- `command` — spawn child process
- `prompt` — inject as system message
- `event` — emit to event bus

**Hook configuration** (`~/.aletheon/config.toml`):
```toml
[hooks]
session_start = { type = "command", cmd = "echo 'Session started' >> ~/.aletheon/log.txt" }
pre_tool = { type = "event", emit = "tool_call" }
post_tool = { type = "event", emit = "tool_result" }
pre_response = { type = "command", cmd = "~/.aletheon/hooks/format-response.sh" }
session_end = { type = "command", cmd = "~/.aletheon/hooks/session-summary.sh" }

[hooks.custom]
on_evolution = { type = "command", cmd = "~/.aletheon/hooks/evolution-notify.sh" }
on_awareness_shift = { type = "event", emit = "awareness_change" }
```

**Environment variables for command hooks:**
```
ALETHEON_SESSION_ID=abc123
ALETHEON_TOOL_NAME=glob
ALETHEON_TOOL_INPUT={"pattern":"*.rs"}
ALETHEON_MODE=default
ALETHEON_MODEL=claude-sonnet-4-6
ALETHEON_AWARENESS=confident
```

### 4.6 Event Bus

```
┌─────────────┐     ┌─────────────┐     ┌─────────────┐
│  BrainCore  │────▶│             │────▶│    TUI      │
│  (signals)  │     │             │     │ (display)   │
├─────────────┤     │   EventBus  │     ├─────────────┤
│  Runtime    │────▶│             │────▶│   Hooks     │
│  (actions)  │     │  (pub/sub)  │     │ (command)   │
├─────────────┤     │             │     ├─────────────┤
│  SelfField  │────▶│             │────▶│  Sub-agents │
│  (verdicts) │     │             │     │ (status)    │
└─────────────┘     └─────────────┘     └─────────────┘
```

Uses `tokio::broadcast` channels. Fire-and-forget — slow subscribers don't block publishers.

## 5. Brain Layer

### 5.1 Awareness Signal Surfacing

Extended awareness signals:

| Signal | Detector | TUI Display |
|--------|----------|-------------|
| `Confident` | No critical issues | 💚 status bar |
| `Hesitant` | Hedging language detected | 🟡 status bar + inline |
| `Confused` | 3+ consecutive errors | 🔴 status bar + inline |
| `Curious` | Domain shift detected | 🔵 status bar + inline |
| `Planning` | BrainCore generating plan | 📋 status bar + thinking indicator |
| `Reflecting` | Post-turn reflection running | 🔄 status bar |
| `Evolving` | Morphogenesis triggered | ⚡ status bar + inline |

**Pipeline:** `BrainCore.detectors → AwarenessSignal → EventBus → TUI`

### 5.2 Plan Mode Integration

**Phase 1: Exploration (read-only)**
- BrainCore uses only read-only tools (glob, grep, read, web_fetch)
- SelfField verdict forced to `Deny` for all mutation tools
- TUI shows exploration progress

**Phase 2: Plan Generation**
- `BrainCore.think_with_refinement()` generates plan
- Critique loop runs (up to 3 rounds)
- TUI shows each iteration via PlanView widget

**Phase 3: Approval & Execution**
- User types `/approve` or `Ctrl+P`
- TUI sends `PlanApproval` command to Runtime
- Runtime switches mode from `Plan` to `Default`
- Approved plan steps are injected as context into the conversation
- ReActLoop executes each plan step sequentially
- TUI shows progress per step (current step highlighted in plan view)

### 5.3 Dual-Model Visibility

When DualModelBridge activates:
```
🧠 planner: claude-opus-4-8 → generating plan...
⚡ executor: claude-sonnet-4-6 → executing step 1/4...
```

Status bar shows current model. `/model` shows both models.

### 5.4 Evolution Status Display

Post-turn evolution progress:
```
🔄 Reflecting on execution... (2/5 reflections accumulated)
📊 Patterns detected: repeated auth-related failures
🧬 Morphogenesis triggered: proposing new auth-check skill
```

`/evolution` shows: reflection window, detected patterns, pending mutations, lineage history.

## 6. Skills System

### 6.1 Two-Layer Architecture

**Layer 1: Built-in Skills (Rust code)**

```rust
pub trait BuiltinSkill: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn trigger_pattern(&self) -> Option<&str>;
    fn execute(&self, ctx: SkillContext) -> Pin<Box<dyn Future<Output = SkillResult>>>;
}
```

Built-in: `/compact`, `/reflect`, `/evolution`, `/genome`, `/sessions`, `/model`

**Layer 2: User Skills (Markdown prompts)**

`~/.aletheon/skills/<name>.md`:
```markdown
---
name: code-review
description: Review code for bugs and improvements
trigger: /review
permissions:
  read: true
  write: false
  execute: false
tools:
  - glob
  - grep
  - read
model: claude-sonnet-4-6
---

You are a code reviewer. Analyze the provided code for:
1. Bugs and potential issues
2. Performance concerns
3. Style and readability
4. Security vulnerabilities
```

### 6.2 Skill Loading Pipeline

```
~/.aletheon/skills/*.md
  ↓ SkillLoader.parse_frontmatter()
  ↓ SkillLoader.validate()
  ↓ SkillRegistry.register()
  ↓ CommandRouter.add_command(trigger)
  ↓ When triggered: inject prompt as UserMessage
  ↓ Apply permission config to ApprovalGate
  ↓ Execute with specified tools/model
```

**Discovery:**
- Scan `~/.aletheon/skills/` on startup
- Hot-reload on file change (inotify watch)
- Tab-completion includes skill commands
- `/skills` lists all available

## 7. Testing

### 7.1 Test Architecture

```
tests/
├── tui/
│   ├── scenarios/          — JSONL scenario files
│   ├── frames/             — expected frame snapshots
│   ├── harness.rs          — TmuxTestHarness
│   ├── validator.rs        — frame comparison + assertion
│   └── tmux_runner.rs      — tmux session management
├── runtime/
│   ├── session_tests.rs
│   ├── mode_tests.rs
│   ├── interrupt_tests.rs
│   └── subagent_tests.rs
├── brain/
│   ├── awareness_tests.rs
│   ├── plan_mode_tests.rs
│   └── evolution_tests.rs
└── integration/
    ├── full_turn.rs
    ├── hooks_test.rs
    └── skills_test.rs
```

### 7.2 tmux Test Harness

```rust
pub struct TmuxTestHarness {
    session_name: String,
    socket_path: PathBuf,
    llm_config: LlmConfig,  // Real API key + model
    frame_recorder: FrameRecorder,
}

impl TmuxTestHarness {
    pub async fn setup(&mut self) -> Result<()>;           // Start daemon
    pub async fn start_tui(&mut self) -> Result<()>;       // Start TUI in tmux
    pub fn send_keys(&self, keys: &str) -> Result<()>;     // tmux send-keys
    pub fn send_text(&self, text: &str) -> Result<()>;     // Simulate typing
    pub async fn wait_for(&self, condition: WaitCondition, timeout: Duration) -> Result<()>;
    pub fn capture_frame(&self) -> Result<String>;          // tmux capture-pane
    pub fn assert_frame(&self, expected: &str) -> Result<()>;
    pub async fn teardown(&self) -> Result<()>;
}

pub enum WaitCondition {
    StreamingComplete,
    ToolCardVisible,
    ApprovalDialogShown,
    AwarenessSignal(AwarenessSignal),
    PlanDisplayed { version: usize },
    SubAgentVisible { agent_id: String },
    ModeChanged(CollaborationMode),
    Custom(Box<dyn Fn(&str) -> bool>),
}
```

### 7.3 Test Scenarios

| Scenario | Tests | LLM Required |
|----------|-------|--------------|
| `basic_chat` | Streaming, tool cards, completion | Yes |
| `mode_switching` | /mode cycle, verdict changes | Yes |
| `plan_mode` | Explore → plan → approve → execute | Yes |
| `interrupt` | Ctrl+C during streaming, partial response | Yes |
| `sub_agent_spawn` | Agent tool call, inline status display | Yes |
| `awareness_display` | Impasse detection, status bar change | Yes |
| `hooks_firing` | pre_tool/post_tool hook execution | Yes |
| `skill_execution` | /skill command, prompt injection | Yes |
| `model_switch` | /model change mid-session | Yes |
| `context_compaction` | Auto-compaction, /compact command | Yes |
| `evolution_display` | Post-turn reflection, morphogenesis | Yes |
| `approval_flow` | L2+ tool approval, deny → retry | Yes |

### 7.4 Frame Validation

```rust
pub fn assert_frame_match(actual: &str, expected: &str, opts: MatchOpts) -> Result<()> {
    // 1. Normalize whitespace
    // 2. Replace dynamic fields with placeholders: {TOKENS}, {TIME}, {MODEL}
    // 3. Compare structure (layout regions match)
    // 4. Compare static text content
    // 5. Report diff if mismatch
}
```

### 7.5 CI Integration

```yaml
# .github/workflows/test.yml
- name: TUI smoke tests (mock LLM)
  run: cargo test --test tui_mock -- --test-threads=1

- name: Integration tests (real LLM)
  if: github.event_name == 'push' && contains(github.event.head_commit.message, '[integration]')
  env:
    ANTHROPIC_API_KEY: ${{ secrets.ANTHROPIC_API_KEY }}
  run: cargo test --test tui_integration -- --test-threads=1
```

## 8. Implementation Phases

| Phase | Layer | Focus | Files Changed | Estimate |
|-------|-------|-------|---------------|----------|
| **P0** | ABI | New types & traits | `aletheon-abi/src/` (~5 files) | 1 day |
| **P1** | Runtime | Session, modes, interrupt | `aletheon-runtime/src/` (~8 files) | 2 days |
| **P2** | TUI | Decompose monolith, new widgets | `aletheon-body/src/impl/ui/` (~15 files) | 3 days |
| **P3** | Brain | Awareness surfacing, plan mode | `aletheon-brain/src/` (~4 files) | 1 day |
| **P4** | Skills | Skill system, hooks | `aletheon-body/src/impl/skills/` (~3 files) | 1 day |
| **P5** | Testing | tmux harness, scenarios | `tests/` (~10 files) | 2 days |
| **P6** | Integration | Wire everything, E2E tests | All layers | 1 day |

**Total estimate: ~11 days**

### 8.1 Dependency Order

```
P0 (ABI types) ─────────────────────────────────────────┐
 │                                                       │
 ├── P1 (Runtime) — needs new types                      │
 │    └── P2a (TUI decompose) — needs basic commands     │
 │                                                       │
 ├── P3 (Brain) — needs EventBus (from P0)               │ (parallel after P0)
 │    └── P2b (TUI widgets) — needs awareness signals    │
 │                                                       │
 └── P4 (Skills) — needs hook types (from P0)            │
      └── P2c (TUI commands) — needs skill commands      │
                                                           │
P5 (Testing) — needs all layers wired ◄───────────────────┘
 └── P6 (Integration) — E2E validation
```

P1, P3, P4 can run in parallel after P0 (ABI types) is stable. P2 (TUI) is split into sub-phases that depend on the corresponding runtime/brain/skills work. P5 (Testing) requires all layers to be wired. P2 (TUI) can be split into:
- P2a: Decompose monolith + basic mode switching
- P2b: New widgets (plan_view, subagent_view, awareness)
- P2c: Status line + commands + completion

## 9. File Map

### P0: ABI Types (aletheon-abi)

```
aletheon-abi/src/
├── types/
│   ├── mod.rs                    — add new type re-exports
│   ├── ui_event.rs               (NEW) — UiEvent enum
│   ├── command.rs                (NEW) — Command enum
│   ├── collaboration_mode.rs     (NEW) — CollaborationMode + ModeConfig
│   ├── awareness.rs              (NEW) — AwarenessSignal + AwarenessLevel
│   ├── sub_agent.rs              (NEW) — SubAgentHandle, SubAgentStatus
│   └── hook.rs                   (NEW) — HookConfig, HookResult, HookEvent
├── traits/
│   ├── mod.rs                    — add new trait re-exports
│   ├── hook.rs                   (NEW) — HookExecutor trait
│   ├── skill.rs                  (NEW) — SkillProvider trait
│   └── session.rs                (NEW) — SessionManager trait
```

### P1: Runtime (aletheon-runtime)

```
aletheon-runtime/src/
├── core/
│   ├── mod.rs                    — add module declarations
│   ├── session.rs                (NEW) — SessionManager impl
│   ├── mode_router.rs            (NEW) — ModeRouter
│   ├── sub_agent.rs              (NEW) — SubAgentSpawner
│   ├── interrupt.rs              (NEW) — InterruptHandler
│   ├── orchestrator.rs           (MODIFY) — integrate ModeRouter, InterruptHandler
│   ├── react_loop.rs             (MODIFY) — add cancel check
│   └── evolution_coordinator.rs  (MODIFY) — emit UiEvent
├── impl/
│   ├── daemon/
│   │   ├── handler.rs            (MODIFY) — new JSON-RPC methods
│   │   └── mod.rs
│   └── hooks/
│       ├── mod.rs                (NEW) — HookRegistry
│       ├── command_hook.rs       (NEW) — process spawning
│       └── event_hook.rs         (NEW) — event bus hook
```

### P2: TUI (aletheon-body)

```
aletheon-body/src/impl/ui/
├── mod.rs                        (REWRITE) — slim re-export
├── app.rs                        (NEW) — App struct, state machine
├── render.rs                     (NEW) — layout composition
├── dispatch.rs                   (NEW) — event → action mapping
├── state.rs                      (NEW) — AppState, mode tracking
├── plan_view.rs                  (NEW) — plan mode visualization
├── subagent_view.rs              (NEW) — sub-agent inline display
├── awareness.rs                  (NEW) — awareness indicator
├── status.rs                     (REWRITE) — hybrid status line
├── command.rs                    (MODIFY) — add new commands
├── completion.rs                 (MODIFY) — include new commands
├── help_overlay.rs               (MODIFY) — update shortcuts
```

### P3: Brain (aletheon-brain)

```
aletheon-brain/src/
├── core/
│   ├── awareness_signal.rs       (MODIFY) — emit UiEvent
│   ├── mod.rs                    (MODIFY) — wire awareness → EventBus
│   └── plan.rs                   (NEW) — PlanVersion, Critique serialization
├── impl/
│   └── llm/
│       └── provider.rs           (MODIFY) — expose model info
```

### P4: Skills (aletheon-body)

```
aletheon-body/src/impl/
├── skills/
│   ├── mod.rs                    (NEW) — SkillRegistry
│   ├── builtin.rs                (NEW) — BuiltinSkill implementations
│   ├── markdown_skill.rs         (NEW) — Markdown parser + executor
│   └── loader.rs                 (NEW) — File scanner, hot-reload
```

### P5: Testing

```
tests/
├── tui/
│   ├── harness.rs                (NEW)
│   ├── validator.rs              (NEW)
│   ├── tmux_runner.rs            (NEW)
│   ├── scenarios/*.jsonl         (NEW)
│   └── frames/*                  (NEW)
├── runtime/
│   ├── session_tests.rs          (NEW)
│   ├── mode_tests.rs             (NEW)
│   ├── interrupt_tests.rs        (NEW)
│   └── subagent_tests.rs         (NEW)
├── brain/
│   ├── awareness_tests.rs        (NEW)
│   ├── plan_mode_tests.rs        (NEW)
│   └── evolution_tests.rs        (NEW)
└── integration/
    ├── full_turn.rs              (NEW)
    ├── hooks_test.rs             (NEW)
    └── skills_test.rs            (NEW)
```
