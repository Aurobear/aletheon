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

## Monitoring Schedule

Configure Claude Code cron jobs for automated monitoring:

| Tier | Interval | Tool |
|------|----------|------|
| L1 — Liveness | Every 5 min | `aletheon_health` |
| L2 — Health | Every 30 min | `aletheon_snapshot` |
| L3 — Deep | Every 2 hours | `aletheon_analyze` |

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
