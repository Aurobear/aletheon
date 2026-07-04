# Aletheon Monitor MCP Server — Design Spec

**Date:** 2026-07-05
**Status:** Draft
**Branch:** `main`

## 1. Motivation

### 1.1 Problem

Aletheon daemon currently has no external-facing API beyond a Unix socket
JSON-RPC interface at `/run/aletheon/aletheon.sock`. There is no HTTP/WebSocket
protocol, and the daemon does **not** expose itself as an MCP server. This
means:

- Claude Code CLI cannot connect to or monitor Aletheon.
- Manual TUI-based inspection is the only way to observe daemon state.
- There is no automated anomaly detection for crashes, tool failures, memory
  corruption, or config drift.
- Diagnostic feedback loops (observe → diagnose → fix → redeploy) are
  completely manual.

### 1.2 Goal

Build `aletheon-monitor` — a Python FastMCP server that bridges Claude Code to
the Aletheon daemon via the existing Session Gateway JSON-RPC protocol, enabling
**automated SRE-style monitoring and proactive issue remediation**.

### 1.3 Non-Goals

- This is NOT a new daemon endpoint. All RPC methods already exist in the
  Session Gateway (`crates/runtime/src/core/session_gateway/`).
- This is NOT a user-facing agent interface (users still use `aletheon` TUI or
  `aletheon` CLI for that).
- This does NOT add HTTP/WebSocket to the daemon itself (future consideration).

## 2. Architecture

```
┌──────────────────────────────────────────────┐
│              Claude Code                      │
│  (invokes aletheon_* MCP tools via stdio)     │
└──────────────┬───────────────────────────────┘
               │ MCP (stdio)
               ▼
┌──────────────────────────────────────────────┐
│         aletheon-monitor                      │
│      tools/aletheon-monitor/                  │
│      Python + FastMCP                          │
│                                              │
│  Tools:                                      │
│  ┌ aletheon_health     健康检查              │
│  ├ aletheon_snapshot   完整状态快照           │
│  ├ aletheon_journal    事件日志查询           │
│  ├ aletheon_ask        向 agent 提问          │
│  ├ aletheon_logs       daemon 日志 tail       │
│  ├ aletheon_watch      实时事件流订阅          │
│  ├ aletheon_memory     记忆系统查询           │
│  ├ aletheon_sessions   会话列表/恢复          │
│  └ aletheon_analyze    诊断分析（复合查询）     │
└──────────────┬───────────────────────────────┘
               │ Unix socket JSON-RPC 2.0
               ▼
┌──────────────────────────────────────────────┐
│         Aletheon Daemon                       │
│  /run/aletheon/aletheon.sock                  │
│  Session Gateway methods (existing)            │
└──────────────────────────────────────────────┘
```

### 2.1 Key Design Decisions

| Decision | Choice | Rationale |
|----------|--------|-----------|
| Transport | MCP over stdio | No port to expose, no auth to configure; Claude Code spawns the process directly |
| Language | Python | FastMCP is mature; zero new Rust dependencies; rapid iteration |
| Location | `tools/aletheon-monitor/` (same repo) | Versioned with the daemon; no drift |
| Daemon changes | **Zero** | All RPC methods already exist in Session Gateway |
| Config | `tools/aletheon-monitor/config.toml` | Socket path, timeouts, optional defaults |

### 2.2 Component: MCP Bridge Client

A thin `AletheonClient` class handles all communication with the daemon:

```
class AletheonClient:
    def __init__(self, socket_path: str, timeout: float = 5.0)
    async def rpc(self, method: str, params: dict = None) -> dict
    async def close(self)
```

- Sends line-delimited JSON-RPC 2.0 over Unix socket
- Single connection, reused across all tool calls
- Auto-reconnects on broken pipe
- Timeout handled per call

## 3. MCP Tools

### 3.1 Tool-to-RPC Mapping

| MCP Tool | JSON-RPC Method | Reader | Writer | Notes |
|----------|----------------|--------|--------|-------|
| `aletheon_health` | `health` + `status` | ✅ | ❌ | Merges both RPCs into one response |
| `aletheon_snapshot` | `session.snapshot` | ✅ | ❌ | Full runtime state dump |
| `aletheon_journal` | `session.journal` | ✅ | ❌ | Query last N events, with optional type filter |
| `aletheon_ask` | `session.ask` | ✅ | ❌ | Send a question to the running agent |
| `aletheon_logs` | `session.log` | ✅ | ❌ | Tail recent log lines, with optional level filter |
| `aletheon_watch` | `session.watch` | ✅ | ❌ | Subscribe to real-time event topic(s) for N seconds |
| `aletheon_memory` | `session.memory` | ✅ | ❌ | Search/query the agent's memory store |
| `aletheon_sessions` | `sessions` + `session.list` | ✅ | ✅ | List sessions; `resume` support is read+write |
| `aletheon_analyze` | `snapshot` + `perf` + `journal` | ✅ | ❌ | Composite: parallel fetches, merged response |

### 3.2 Tool Specifications

#### `aletheon_health`

**Purpose:** Quick liveness and readiness check. Always the first call in any
monitoring flow.

**Input:** None

**Output:**
```json
{
  "daemon": {
    "pid": 178410,
    "uptime_seconds": 82340,
    "version": "0.1.0"
  },
  "systemd": {
    "active": true,
    "restart_count": 0,
    "memory_mb": 36.0
  },
  "session": {
    "id": "uuid-...",
    "turn_count": 42,
    "status": "idle"
  },
  "socket": {
    "path": "/run/aletheon/aletheon.sock",
    "exists": true,
    "writable": true
  },
  "healthy": true
}
```

**Error handling:** Returns `healthy: false` with an `error` field if the
daemon is unreachable. Does NOT throw MCP errors; it always returns a
structured result so the caller can check `healthy`.

#### `aletheon_snapshot`

**Purpose:** Deep dive into agent state: what it's doing, what it knows,
what's pending.

**Input:**
| Param | Type | Default | Description |
|-------|------|---------|-------------|
| `include_memory` | bool | `false` | Include memory store contents (can be large) |

**Output:** Full Session Gateway snapshot JSON. Key sections:
- `state`: current agent state (idle, thinking, tool_exec, etc.)
- `turn`: active turn metadata (iteration, goal, tool budget consumed)
- `config`: effective runtime configuration
- `self_field`: SelfField genome/policy state
- `memory` (optional): memory store dump

#### `aletheon_journal`

**Purpose:** Retrieve recent event history for diagnostics.

**Input:**
| Param | Type | Default | Description |
|-------|------|---------|-------------|
| `last_n` | int | 20 | Number of recent events |
| `event_type` | string | null | Filter: "tool_use", "user_message", "error", "compacted", "checkpoint" |

**Output:** Array of journal events, each with `timestamp`, `event_type`, and `payload`.

#### `aletheon_ask`

**Purpose:** Send a question to the running agent and get its internal
perspective. Uses the agent's own LLM for introspection.

**Input:**
| Param | Type | Description |
|-------|------|-------------|
| `question` | string | Question to ask the agent |

**Output:** The agent's response text.

**Rate limit:** At most 1 call per 30 seconds (agent may be busy in a turn).

#### `aletheon_logs`

**Purpose:** Tail daemon log output, optionally filtered by level.

**Input:**
| Param | Type | Default | Description |
|-------|------|---------|-------------|
| `last_n` | int | 50 | Number of recent lines |
| `level` | string | null | Filter: "ERROR", "WARN", "INFO" |

**Output:** Array of log lines with timestamps.

**Fallback:** If `session.log` is not available (daemon crashed), falls back to
`journalctl -u aletheon --no-pager -n {last_n}` via subprocess.

#### `aletheon_watch`

**Purpose:** Subscribe to a real-time event topic for a configurable duration.
Useful for "watch this while I test something."

**Input:**
| Param | Type | Default | Description |
|-------|------|---------|-------------|
| `topic` | string | "perf" | Event topic: "perf", "tool", "session", "all" |
| `duration_seconds` | int | 10 | How long to collect events (max 60) |

**Output:** Array of events received during the window.

**Implementation note:** Subscribes via `session.watch`, collects events into a
buffer, then unsubscribes and returns the buffer. No persistent streaming.

#### `aletheon_memory`

**Purpose:** Query the agent's memory system (CoreMemory, RecallMemory,
FactStore).

**Input:**
| Param | Type | Default | Description |
|-------|------|---------|-------------|
| `query` | string | (required) | Search query |
| `memory_type` | string | "all" | "core", "recall", "facts", "all" |
| `limit` | int | 10 | Max results |

**Output:** Memory entries matching the query, with timestamps and relevance
scores.

#### `aletheon_sessions`

**Purpose:** List and switch between sessions.

**Input:**
| Param | Type | Default | Description |
|-------|------|---------|-------------|
| `action` | string | "list" | "list" or "resume" |
| `session_id` | string | null | Required if action is "resume" |

**Output:** For `list`: array of session summaries (id, created_at, status,
turn_count). For `resume`: confirmation with session snapshot.

#### `aletheon_analyze`

**Purpose:** **The primary diagnostic tool.** Runs parallel queries against
snapshot + perf + journal, merges results, and returns a structured analysis
for Claude to interpret.

**Input:** None (pulls everything at once)

**Output:**
```json
{
  "healthy": true,
  "snapshot": { ... },
  "perf": {
    "llm_calls": { "total": 523, "errors": 2, "avg_latency_ms": 1200 },
    "tool_calls": { "total": 1840, "errors": 15, "by_tool": {...} },
    "token_usage": { "input": 450000, "output": 120000, "cache_hit_pct": 0.34 }
  },
  "recent_journal": [ ... ],
  "anomalies": [
    { "type": "tool_error_rate", "severity": "warn", "detail": "grep tool: 12% error rate (threshold: 5%)" }
  ]
}
```

**Anomaly detection rules (client-side, in Python):**

| Rule | Condition | Severity |
|------|-----------|----------|
| tool_error_rate | >10% for any tool | WARN |
| tool_error_rate | >25% for any tool | CRITICAL |
| llm_error_rate | >5% over last hour | WARN |
| restart_loop | restart_count > 5 in 10 min | CRITICAL |
| memory_growth | >2x baseline in 24h | WARN |
| context_overflow | >3 compactions in 10 turns | WARN |
| socket_missing | socket file not found | CRITICAL |
| provider_unreachable | health probe fails | CRITICAL |

## 4. Monitoring Strategy

### 4.1 Three-Tier Cron Schedule

All cron jobs are configured via Claude Code's `CronCreate`.

| Tier | Interval | Tool | Token Cost | Action on Anomaly |
|------|----------|------|-----------|-------------------|
| L1 — Liveness | Every 5 min | `aletheon_health` | ~1 token | Push notify + auto-restart |
| L2 — Health | Every 30 min | `aletheon_snapshot` | ~200 tokens | Push notify + analyze + propose fix |
| L3 — Deep | Every 2 hours | `aletheon_analyze` | ~500 tokens | Push notify + full diagnosis + auto-fix |

**Silence = healthy.** No message to user unless an anomaly is detected.

### 4.2 Anomaly Response Matrix

| Anomaly | Detect | Claude Action | Auto-Fix? |
|---------|--------|---------------|-----------|
| Daemon crash | L1 health | Read journalctl, inspect exit code | ✅ Locate root cause → edit code → suggest commit |
| Socket missing | L1 health | Check systemd status, permissions, check /run partition | ✅ systemctl restart aletheon |
| Provider unreachable | L2 health | Ping base_url, validate API key in .env | ❌ Notify user (creds issue) |
| Tool call failure spike | L2 perf | Read tool-specific errors from journal, check tool impl | ✅ Locate bug → edit code |
| LLM error rate high | L2 perf | Check provider status, try failover, inspect request payloads | ✅ Switch provider or fix code |
| Memory system corrupt | L3 analyze | Read recall_memory.db integrity, check fact_store.db wal files | ✅ Repair DB or suggest rebuild |
| Disk space low | L2 health | df -h, du /var/lib/aletheon, prune old session journals | ✅ Auto-clean if safe |
| Restart loop (rapid crash) | L1 health | Read full crash log, bisect recent code changes | ✅ Git diff → locate → fix |
| Config drift | L3 analyze | Compare running config vs config.toml vs defaults | ✅ Report drift → suggest correction |
| SelfField policy deny | L3 analyze | Read journal for deny reasons, analyze intent | ❌ Notify user (policy decision) |

### 4.3 Session Debug Flow (Manual)

When the user explicitly asks Claude to investigate Aletheon:

```
User: "看看 aletheon 现在什么状态"
Claude:
  1. aletheon_health → quick status
  2. aletheon_snapshot → deep state
  3. aletheon_journal(last_n=10) → recent activity
  4. Summary to user

User: "aletheon 刚才那个 grep 工具调用失败了，为什么？"
Claude:
  1. aletheon_journal(last_n=50, event_type="tool_use")
  2. Filter for grep errors
  3. Read relevant Rust source files
  4. Propose fix

User: "aletheon 的代码有什么可以改进的？"
Claude:
  1. aletheon_analyze → full diagnostic
  2. Cross-reference anomalies with source code
  3. Propose prioritized fix list
```

## 5. Implementation Plan

### 5.1 File Structure

```
tools/aletheon-monitor/
├── README.md                  # Usage and setup
├── config.toml                # Default config (socket path, timeouts)
├── pyproject.toml             # Python deps (mcp, anyhow-like)
├── src/
│   ├── __init__.py
│   ├── server.py              # FastMCP server + tool registration
│   ├── client.py              # AletheonClient (Unix socket JSON-RPC)
│   ├── tools/                  # One file per MCP tool
│   │   ├── __init__.py
│   │   ├── health.py
│   │   ├── snapshot.py
│   │   ├── journal.py
│   │   ├── ask.py
│   │   ├── logs.py
│   │   ├── watch.py
│   │   ├── memory.py
│   │   ├── sessions.py
│   │   └── analyze.py
│   └── anomaly.py             # Anomaly detection rules
└── tests/
    ├── test_client.py
    └── test_tools.py
```

### 5.2 Dependencies

```toml
[project]
name = "aletheon-monitor"
version = "0.1.0"
requires-python = ">=3.10"
dependencies = [
    "mcp>=1.0.0",
]
```

No other runtime dependencies. `socket`, `json`, `asyncio` are stdlib.

### 5.3 Phase Ordering

| Phase | Scope | Est. Effort |
|-------|-------|-------------|
| **P1** | `client.py` + `health` + `snapshot` + `analyze` | Core plumbing + basic monitoring |
| **P2** | `journal` + `logs` + `memory` + `sessions` | Full read access |
| **P3** | `ask` + `watch` + `anomaly.py` | Interactive + anomaly detection rules |
| **P4** | Cron setup + integration testing | Automated monitoring live |

### 5.4 MCP Configuration (Claude Code)

```json
{
  "mcpServers": {
    "aletheon-monitor": {
      "command": "python",
      "args": ["-m", "aletheon_monitor.server"],
      "cwd": "/home/aurobear/Bear-ws/aletheon/tools/aletheon-monitor",
      "env": {
        "ALETHEON_SOCKET": "/run/aletheon/aletheon.sock",
        "ALETHEON_TIMEOUT": "5"
      }
    }
  }
}
```

## 6. Edge Cases & Error Handling

| Scenario | Behavior |
|----------|----------|
| Daemon not running | All tools return `{"error": "daemon unreachable", "healthy": false}`. `aletheon_logs` falls back to journalctl. |
| Socket permission denied | Return error with `chmod`/`chown` instructions |
| RPC timeout | Return partial results with `timeout: true` flag |
| JSON-RPC parse error | Return raw response + `parse_error: true` |
| `session.ask` triggers tool calls | Return the assistant message as-is (agent is autonomous) |
| Too many concurrent `watch` subscribers | Return error; suggest retry |
| Journal is empty (fresh daemon) | Return `{"events": [], "message": "No journal entries yet"}` |

## 7. Open Questions

None. All design decisions confirmed in brainstorming session.

## 8. References

- Session Gateway design: `docs/plans/2026-07-03-session-gateway-design.md`
- Session Gateway code: `crates/runtime/src/core/session_gateway/gateway.rs`
- RPC dispatch: `crates/runtime/src/impl/daemon/handler/rpc.rs`
- Codex AGENTS.md: `/home/aurobear/Bear-ws/codex/AGENTS.md`
