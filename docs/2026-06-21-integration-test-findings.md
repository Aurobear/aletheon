# Aletheon Integration Test Findings & Fix Plan

> Date: 2026-06-21
> Context: P0-P6 implementation complete, interactive tmux testing against live daemon
> Reference: aurb project patterns (`/home/aurobear/Bear-ws/work/aurb`)

---

## Executive Summary

After implementing P0-P6 (ABI types, Runtime, TUI, Brain, Skills/Hooks, Testing, Integration),
interactive testing reveals **4 critical gaps** that prevent the system from being fully functional:

| # | Gap | Severity | aurb Reference |
|---|-----|----------|----------------|
| 1 | Sub-agents not exposed as LLM tools | 🔴 Critical | coordinator.md Agent() dispatch |
| 2 | Mode indicator not visible in TUI | 🟡 Medium | model.sh interactive switcher |
| 3 | Hooks system incomplete (only 1 builtin) | 🟡 Medium | 5 lifecycle hooks in aurb |
| 4 | MCP embedded server not wired to LLM | 🟡 Medium | skill-as-tool pattern |

---

## Issue 1: Sub-Agent Delegation Gap (🔴 Critical)

### Problem

Three agents are registered in the runtime:
- `code-agent` — tools: file_read, file_write, bash_exec
- `fs-agent` — tools: file_read, file_write
- `net-agent` — tools: system_status, process_list

The `SubAgentSpawner` is initialized, the `sub_agents` RPC works, but **no tool is exposed to the LLM**
that allows it to delegate tasks. The agent responds:

> "I haven't used a 'code-agent sub-agent' because no such tool is available to me in my current toolset."

### Architecture Comparison

**aurb pattern** (Claude Code path):
```
coordinator.md → Agent(prompt, {subagent_type: "developer"}) → spawns sub-agent
```
- Coordinator decomposes tasks into DAG with dependencies
- Up to 4 concurrent sub-agents per batch
- Context compression: retain only STATUS/SUMMARY/CHANGED_FILES/FAILURES
- Fix loop: max 2 retries, then BLOCKED

**aurb pattern** (Codex path):
```
board.sh create → worker claims → codex --full-auto → classify → update board
```
- File-system task board (`.aurb/tasks/board/*.json`)
- Atomic claim via `mkdir .lock-<task_id>`
- Heartbeat + stale reaping (300s timeout)

### Aletheon Current State

```
Agent Registry (config)     SubAgentSpawner (runtime)     LLM Tools
┌─────────────────┐        ┌─────────────────┐          ┌─────────┐
│ code-agent.toml │──✓──→  │ spawn()         │    ✗     │ (none)  │
│ fs-agent.toml   │──✓──→  │ update_status() │────────→ │         │
│ net-agent.toml  │──✓──→  │ list()          │          │         │
└─────────────────┘        └─────────────────┘          └─────────┘
     Registered                Runtime wired              NOT EXPOSED
```

### Fix Plan

**Option A: Register a `delegate_task` tool** (recommended)

Add a new tool to the ToolRegistry that wraps SubAgentSpawner:

```rust
// crates/aletheon-body/src/impl/tools/delegate_task.rs
pub struct DelegateTaskTool {
    spawner: Arc<Mutex<SubAgentSpawner>>,
    runtime: Arc<Mutex<AletheonRuntime>>,
}

impl Tool for DelegateTaskTool {
    fn name(&self) -> &str { "delegate_task" }
    fn description(&self) -> &str {
        "Delegate a task to a sub-agent. Available agents: code-agent (code execution), \
         fs-agent (file ops), net-agent (network diagnostics)."
    }
    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "agent_id": {"type": "string", "enum": ["code-agent", "fs-agent", "net-agent"]},
                "task": {"type": "string", "description": "Task description for the sub-agent"},
            },
            "required": ["agent_id", "task"]
        })
    }
    async fn execute(&self, params: Value, ctx: &ToolContext) -> ToolResult {
        let agent_id = params["agent_id"].as_str().unwrap();
        let task = params["task"].as_str().unwrap();
        // Spawn sub-agent, wait for completion, return result
        // ...
    }
}
```

**Option B: Expose agents via system prompt** (simpler)

Inject available agents into the system prompt so the LLM knows to use existing tools
(bash_exec, file_read, etc.) directly, and mention that sub-agents exist for complex tasks.

**Recommended**: Option A — it gives the LLM explicit delegation capability and matches
the aurb coordinator pattern.

---

## Issue 2: TUI Mode Indicator Not Visible (🟡 Medium)

### Problem

The `/mode plan` command sends the correct RPC, the daemon processes it and sends a
`mode_changed` notification, but the TUI status bar doesn't show the current mode.

**Root cause**: The status bar renders `mode.icon() + mode.display_name()` but the
`mode_changed` event parsing was using `params.new` while the daemon sends `params.mode`.

**Already fixed** in this session (commit `2c12852b`), but the status bar still doesn't
show the mode because the `AppState.mode` field starts as `CollaborationMode::Default`
and the mode indicator rendering may be hidden or truncated.

### aurb Reference

aurb's `model.sh` has an interactive switcher that shows:
```
┌─────────────────────────────────┐
│  Current: deepseek/deepseek-v4  │
│                                 │
│  1. mimo/mimo-v2.5-pro         │
│  2. deepseek/deepseek-v4  ✓    │
│  3. ollama/qwen3:8b            │
└─────────────────────────────────┘
```

### Fix Plan

1. Verify `AppState.mode` is updated on `mode_changed` event (already fixed)
2. Ensure status bar renders mode icon: `📋 Plan` / `💬 Default` / `⚡ Auto` / `🔒 Sandbox`
3. Add mode indicator to the TUI title bar or input area for visibility

---

## Issue 3: Hooks System Incomplete (🟡 Medium)

### Problem

Only 1 hook registered: `builtin:audit` at `PostTool` point. The hooks config loader
reads from `~/.aletheon/hooks/` but the directory is empty.

The `hooks_list` RPC handler works (added in this session), but there are no user-configured hooks.

### aurb Reference

aurb has **5 production hooks**:

| Hook | Point | What |
|------|-------|------|
| `block-risky-bash.sh` | PreToolUse | YAML-driven permission engine, blocks destructive commands |
| `run-checks.sh` | PostToolUse | Auto-format (shfmt/black) + validate (bash -n, py_compile) |
| `recall-inject.sh` | UserPromptSubmit | RAG recall: FTS5 + vector + entity graph + skill routing |
| `session-to-shortterm.sh` | Stop | LLM-based session distillation → durable facts |
| `memory-agent.sh` | Stop | Memory consolidation + GC |

The `recall-inject.sh` hook is particularly sophisticated:
1. FTS5 keyword search
2. Entity graph traversal
3. Vector similarity fallback
4. Knowledge/episode table search
5. Cognitive advice injection
6. Skill routing suggestion
7. Trust feedback loop (+0.005 boost, -0.002 decay)

### Fix Plan

**Phase 1: Core hooks** (implement in aletheon-runtime)

| Hook | Point | Implementation |
|------|-------|----------------|
| `block-risky-commands` | PreTool | Reuse existing SelfField verdict system |
| `auto-format` | PostTool | Shell script: shfmt for .sh, rustfmt for .rs |
| `memory-recall` | PreResponse | Inject relevant memories into context |

**Phase 2: Hook config format**

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

**Phase 3: Hook deployment**

Create `aletheon hooks install` command that copies hook scripts to `~/.aletheon/hooks/`
and registers them in the config.

---

## Issue 4: MCP Embedded Server Not Wired to LLM (🟡 Medium)

### Problem

The `tools/list` RPC works on the daemon socket (returns 23 tools), but the MCP embedded
server is not started as a separate listener. The LLM accesses tools through the ToolRegistry
directly, not through MCP protocol.

### aurb Reference

aurb's MCP policy: "Keep MCP only for capabilities that genuinely need a structured tool
beyond normal shell and file access." Skills provide tool-like capabilities through
SKILL.md + scripts/ rather than MCP servers.

The `skill_router.py` suggests skills to the user via the `recall-inject.sh` hook,
implementing a **skill-as-tool pattern** where skills are discovered and routed
dynamically rather than registered as static MCP tools.

### Fix Plan

**Option A: Keep current architecture** (recommended)

The current architecture is actually correct — tools are registered in ToolRegistry and
accessed directly by the ReAct loop. MCP is an external protocol for third-party
integrations, not an internal tool dispatch mechanism.

What's needed:
1. Start MCP embedded server on a separate socket for external MCP clients
2. Keep internal tool access through ToolRegistry (current path)
3. Add MCP server config to `~/.aletheon/config.toml`

**Option B: Expose tools via MCP for external clients**

```toml
# ~/.aletheon/config.toml
[mcp.embedded]
enabled = true
socket = "/tmp/aletheon/mcp.sock"
tools = ["bash_exec", "file_read", "file_write", "system_status"]
```

---

## Additional Findings

### A. Daemon Background Process Issue

The daemon cannot be started as a background process from the Bash tool — the sandbox
kills child processes when the command exits. **Workaround**: Use systemd user service.

```bash
# ~/.config/systemd/user/aletheond.service
[Unit]
Description=Aletheon daemon

[Service]
ExecStart=/path/to/aletheond --config ~/.aletheon/config.toml --socket /tmp/aletheon/aletheon.sock
Restart=on-failure

[Install]
WantedBy=default.target
```

### B. CLI Streaming Event Handling

The CLI's `single_message()` function was not handling streaming events (text_delta,
tool_call_start, etc.) before the final response. **Fixed** in commit `e435132f`.

### C. Pre-Existing Test Compilation Errors

4 test compilation errors in aletheon-runtime were caused by missing trait imports:
- `chrono::TimeZone` for `with_ymd_and_hms` method
- `aletheon_abi::Registry` for `register` method

**Fixed** in commit `523281dd`.

### D. Event Variant Gap

7 new Event variants were added to `event_sink.rs` (AwarenessChanged, ModeChanged,
SubAgentStatusChanged, PlanUpdate, Interrupted, ContextUpdate, ModelSwitch) but
only `AwarenessChanged` and `ModeChanged` are currently emitted. The others are
defined and serialized but waiting for producers.

---

## Implementation Priority

| Priority | Task | Effort | Impact |
|----------|------|--------|--------|
| P0 | Add `delegate_task` tool for sub-agent dispatch | 2h | 🔴 Unblocks multi-agent |
| P1 | Verify mode indicator in TUI status bar | 30m | 🟡 UX polish |
| P2 | Implement core hooks (format, recall, block) | 4h | 🟡 Lifecycle automation |
| P3 | Start MCP embedded server on separate socket | 2h | 🟡 External integrations |
| P4 | Wire remaining Event variant producers | 3h | 🟡 Feature completeness |

---

## Commit History (This Session)

| Commit | What |
|--------|------|
| `4a9f502d` | P0-P4: TUI/Runtime/Brain overhaul (56 files, 11496+/2536-) |
| `42ed8967` | P5 Testing + P6 Integration wiring (10 files, 219+) |
| `523281dd` | Bug fix: pre-existing test compilation errors |
| `e435132f` | Bug fix: CLI streaming event handling |
| `c7648620` | Bug fix: notification skip ordering in CLI |
| `2c12852b` | feat: hooks_list/tools_list RPC + TUI formatters |

**Total**: 6 commits, 70+ files changed, 12000+ insertions

---

## Test Matrix

| Feature | Unit | Integration | Manual |
|---------|------|-------------|--------|
| Basic chat | ✅ | ✅ | ✅ |
| Streaming events | ✅ | ✅ | ✅ |
| Mode switch RPC | ✅ | ✅ | ✅ (Python direct) |
| Mode indicator TUI | — | — | ⚠️ Not visible |
| /hooks command | — | ✅ | ✅ |
| /skills command | — | ✅ | ✅ |
| /status command | — | ✅ | ✅ |
| tools/list RPC | — | ✅ | ✅ (23 tools) |
| Sub-agent spawn | ✅ | — | ❌ No LLM tool |
| Sub-agent delegate | — | — | ❌ Not implemented |
| Hooks execution | ✅ | — | ⚠️ Only 1 builtin |
| MCP embedded | ✅ | — | ⚠️ Not started |
| Context management | ✅ | — | — |
| Awareness signals | ✅ | ✅ | ✅ (events flow) |
| Interrupt | ✅ | — | — |
| Plan mode | ✅ | — | ⚠️ Mode not visible |
