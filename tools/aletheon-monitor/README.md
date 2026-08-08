# Aletheon Monitor — MCP Bridge for Claude Code

MCP server that bridges Claude Code to the Aletheon daemon, enabling
automated SRE-style monitoring and proactive issue remediation.

## Quick Start

```bash
cd tools/aletheon-monitor
pip install -e .
```

## MCP Configuration

Add to `~/.claude/mcp.json` (or equivalent Claude Code MCP config):

```json
{
  "mcpServers": {
    "aletheon-monitor": {
      "command": "python",
      "args": ["tools/aletheon-monitor/src/server.py"],
      "env": {
        "ALETHEON_SOCKET": "/run/aletheon/aletheon.sock",
        "ALETHEON_TIMEOUT": "5"
      }
    }
  }
}
```

## Prerequisites

- Aletheon daemon running (`sudo systemctl start aletheon`)
- Python >= 3.10
- `mcp` package (`pip install mcp`)

## Tools

| Tool | Description | Reads | Writes |
|------|-------------|-------|--------|
| `aletheon_health` | Liveness + readiness: daemon, socket, systemd | ✅ | ❌ |
| `aletheon_snapshot` | Full runtime state dump (state, turn, config, self_field) | ✅ | ❌ |
| `aletheon_analyze` | Composite diagnostic: parallel snapshot + perf + journal + anomaly scan | ✅ | ❌ |
| `aletheon_journal` | Event history query with optional type filter | ✅ | ❌ |
| `aletheon_logs` | Daemon log tail (falls back to journalctl) | ✅ | ❌ |
| `aletheon_memory` | Memory system search (CoreMemory, RecallMemory, FactStore) | ✅ | ❌ |
| `aletheon_sessions` | List sessions or resume by ID | ✅ | ✅ |
| `aletheon_ask` | Forward question to agent's LLM for introspection | ✅ | ❌ |
| `aletheon_watch` | Real-time event subscription (time-bounded, max 60s) | ✅ | ❌ |

## TUI observability tools

| Tool | Purpose |
|------|---------|
| `aletheon_tui_start` | Launch the real TUI in tmux (optionally send a task); returns first frame |
| `aletheon_tui_send`  | Type text into the running TUI (submit with Enter) |
| `aletheon_tui_capture` | Capture the settled frame + render checks (dup-render, raw markdown, …) |
| `aletheon_tui_stop`  | Tear down the TUI tmux session |
| `aletheon_diagnose`  | One-stop: TUI frame + checks + daemon analyze/logs + audit tail + timeline + verdict |

Requires `tmux`. The TUI command defaults to `aletheon --socket $ALETHEON_SOCKET`;
override with `ALETHEON_TUI_CMD`. Audit path defaults to the repo
`.aletheon-audit.jsonl`; override with `ALETHEON_AUDIT`.

## Monitoring Schedule

Configure Claude Code cron jobs for automated monitoring:

| Tier | Interval | Tool |
|------|----------|------|
| L1 — Liveness | Every 5 min | `aletheon_health` |
| L2 — Health | Every 30 min | `aletheon_snapshot` |
| L3 — Deep | Every 2 hours | `aletheon_analyze` |

## Nightwatch continuous engineering

Nightwatch is an external supervisor for continuous tests. It tracks a reviewed
Git ref, creates a clean detached worktree for the exact commit, runs configured
argv-only campaigns, captures bounded logs plus complete SHA-256 digests, and
stores sealed bundles under its private state root. Failure occurrences are
clustered in SQLite by a model-independent fingerprint.

DeepSeek is optional and receives only bounded, redacted failure evidence. Its
typed diagnosis cannot change the deterministic verdict, execute a command,
publish an issue, push a branch, or operate Robot hardware. Nightwatch writes a
local issue draft for each failure cluster; GitHub publication stays outside
this first trust boundary.

Install through the normal Aletheon install flow, then review and activate the
example configuration:

```bash
install -m 0600 \
  ~/.config/aletheon/nightwatch.toml.example \
  ~/.config/aletheon/nightwatch.toml
aletheon-lab --config ~/.config/aletheon/nightwatch.toml cycle --force
systemctl --user enable --now aletheon-nightwatch.service
```

The example runs deterministic coding, monitor and architecture contracts, plus
the real 20-task coding benchmark through `/usr/bin/aletheon`. That benchmark is
diagnostic unless the separate installed-runtime gates prove matching source,
release, installed and running daemon generations.

Useful one-shot commands:

```bash
aletheon-lab --config ~/.config/aletheon/nightwatch.toml run \
  --case coding.contracts.v1
aletheon-lab --config ~/.config/aletheon/nightwatch.toml watch --max-cycles 1
```

The API key named by `diagnostics.api_key_env` must come from the existing
credential environment. Do not write it into the TOML file.

## Development

```bash
# Test client connectivity
python -c "
import asyncio
from src.client import AletheonClient
async def test():
    c = AletheonClient()
    print(await c.rpc('health'))
    await c.close()
asyncio.run(test())
"
```

## Design

See `docs/plans/2026-07-05-aletheon-monitor-mcp-design.md` for the full design spec.
