"""Aletheon Monitor MCP Server.

FastMCP server exposing 15 tools for Claude Code to monitor and diagnose
the Aletheon daemon through the existing Session Gateway JSON-RPC interface.

Prerequisites: Run aletheon's setup.sh first. This server expects:
  - $ALETHEON_SOCKET set (or /run/aletheon/aletheon.sock as default)
  - The aletheon daemon running and listening on the socket

Usage:
    python -m aletheon_monitor.server
"""

import json
import os
import sys

from mcp.server import Server
from mcp.server.stdio import stdio_server
from mcp.types import Tool, TextContent

from .client import AletheonClient
from .tools import (
    analyze as analyze_mod,
    ask as ask_mod,
    health as health_mod,
    journal as journal_mod,
    logs as logs_mod,
    memory as memory_mod,
    sessions as sessions_mod,
    snapshot as snapshot_mod,
    watch as watch_mod,
    tui as tui_mod,
    diagnose as diagnose_mod,
)

# ── Global client (initialized once at startup) ──────────────────────────
_client: AletheonClient | None = None
_install_validated: bool = False


async def validate_installation() -> dict:
    """Pre-flight check that aletheon was deployed via setup.sh.

    Async so it is awaitable in the ``await handler(...)`` dispatch path
    (see ``call_tool``); the body itself performs only fast sync checks.

    Returns a dict with keys: ok (bool), message (str), socket_path (str|null).
    This is the "constraint" that enforces setup.sh deployment.
    """
    global _install_validated
    if _install_validated:
        return {"ok": True, "message": "already validated", "socket_path": None}

    client = AletheonClient()
    socket_path = client.socket_path

    # Check 1: Does the socket file exist?
    if not os.path.exists(socket_path):
        _install_validated = True  # Don't spam — validate once
        return {
            "ok": False,
            "message": (
                f"Aletheon socket not found at {socket_path}. "
                "Run aletheon setup.sh to install and start the daemon, "
                "or set ALETHEON_SOCKET to the correct path."
            ),
            "socket_path": socket_path,
        }

    # Check 2: Can we find the env file?
    env_candidates = [
        "/etc/aletheon/.env",
        os.path.expanduser("~/.config/aletheon/.env"),
    ]
    env_found = any(os.path.isfile(p) for p in env_candidates)
    if not env_found:
        _install_validated = True
        return {
            "ok": False,
            "message": (
                "Aletheon env file not found at /etc/aletheon/.env or "
                "~/.config/aletheon/.env. Run setup.sh to generate it."
            ),
            "socket_path": socket_path,
        }

    _install_validated = True
    return {"ok": True, "message": "installation validated", "socket_path": socket_path}


def get_client() -> AletheonClient:
    """Return the singleton AletheonClient, creating it if needed."""
    global _client
    if _client is None:
        _client = AletheonClient()
    return _client


# ── Tool definitions ─────────────────────────────────────────────────────

TOOLS = [
    Tool(
        name="aletheon_check_install",
        description="Pre-flight check: verifies Aletheon was deployed via setup.sh. Run this first before any other aletheon_* tools. Returns OK + socket path, or an error telling you to run setup.sh.",
        inputSchema={
            "type": "object",
            "properties": {},
            "required": [],
        },
    ),
    Tool(
        name="aletheon_health",
        description="Quick liveness check: daemon status, socket health, systemd state. Always the first call in any monitoring flow.",
        inputSchema={
            "type": "object",
            "properties": {},
            "required": [],
        },
    ),
    Tool(
        name="aletheon_snapshot",
        description="Full runtime state dump: agent state, active turn, config, SelfField policy, optional memory store.",
        inputSchema={
            "type": "object",
            "properties": {
                "include_memory": {
                    "type": "boolean",
                    "description": "Include memory store contents (can be large)",
                    "default": False,
                },
            },
        },
    ),
    Tool(
        name="aletheon_analyze",
        description="COMPOSITE diagnostic: parallel snapshot + perf + journal + anomaly scan. The primary diagnostic tool for proactive monitoring.",
        inputSchema={
            "type": "object",
            "properties": {},
            "required": [],
        },
    ),
    Tool(
        name="aletheon_journal",
        description="Retrieve recent session event history. Filter by event type.",
        inputSchema={
            "type": "object",
            "properties": {
                "last_n": {
                    "type": "integer",
                    "description": "Number of recent events (default: 20)",
                    "default": 20,
                },
                "event_type": {
                    "type": "string",
                    "description": "Filter: tool_use, user_message, error, compacted, checkpoint",
                },
            },
        },
    ),
    Tool(
        name="aletheon_logs",
        description="Tail daemon log output. Falls back to journalctl if daemon is unreachable.",
        inputSchema={
            "type": "object",
            "properties": {
                "last_n": {
                    "type": "integer",
                    "description": "Number of recent lines (default: 50)",
                    "default": 50,
                },
                "level": {
                    "type": "string",
                    "description": "Filter: ERROR, WARN, INFO",
                },
            },
        },
    ),
    Tool(
        name="aletheon_memory",
        description="Query the agent's memory system (CoreMemory, RecallMemory, FactStore).",
        inputSchema={
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Search query",
                },
                "memory_type": {
                    "type": "string",
                    "description": "core, recall, facts, or all (default: all)",
                    "default": "all",
                },
                "limit": {
                    "type": "integer",
                    "description": "Max results (default: 10)",
                    "default": 10,
                },
            },
            "required": ["query"],
        },
    ),
    Tool(
        name="aletheon_sessions",
        description="List all sessions or resume a specific session by ID.",
        inputSchema={
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "description": "list or resume (default: list)",
                    "default": "list",
                },
                "session_id": {
                    "type": "string",
                    "description": "Session ID (required if action is resume)",
                },
            },
        },
    ),
    Tool(
        name="aletheon_ask",
        description="Send a question to the running agent for introspection. Uses the agent's own LLM.",
        inputSchema={
            "type": "object",
            "properties": {
                "question": {
                    "type": "string",
                    "description": "Question to ask the agent",
                },
            },
            "required": ["question"],
        },
    ),
    Tool(
        name="aletheon_watch",
        description="Subscribe to real-time events for a fixed duration (max 60s). Collects events into a buffer.",
        inputSchema={
            "type": "object",
            "properties": {
                "topic": {
                    "type": "string",
                    "description": "Event topic: perf, tool, session, all (default: perf)",
                    "default": "perf",
                },
                "duration_seconds": {
                    "type": "integer",
                    "description": "How long to collect (default: 10, max: 60)",
                    "default": 10,
                },
            },
        },
    ),
    Tool(
        name="aletheon_tui_start",
        description="Launch the real aletheon TUI in a tmux pane (optionally send an initial task). Returns the first rendered frame. Use this to observe what the USER actually sees, not just the RPC response.",
        inputSchema={
            "type": "object",
            "properties": {
                "task": {"type": "string", "description": "Optional first message to send", "default": ""},
                "cols": {"type": "integer", "description": "Pane width (default 100)", "default": 100},
                "rows": {"type": "integer", "description": "Pane height (default 40)", "default": 40},
            },
        },
    ),
    Tool(
        name="aletheon_tui_send",
        description="Type text into the running TUI pane; submit with Enter by default. Use for multi-turn or slash-command input.",
        inputSchema={
            "type": "object",
            "properties": {
                "text": {"type": "string", "description": "Literal text to type"},
                "submit": {"type": "boolean", "description": "Press Enter after typing (default true)", "default": True},
            },
            "required": ["text"],
        },
    ),
    Tool(
        name="aletheon_tui_capture",
        description="Capture the current rendered TUI frame. With wait_stable, polls until the screen stops changing (1.5s) or times out (90s). Returns the frame text plus render checks (dup-render, raw markdown, permission-denied, etc).",
        inputSchema={
            "type": "object",
            "properties": {
                "scrollback": {"type": "boolean", "description": "Include scrollback history (default true)", "default": True},
                "wait_stable": {"type": "boolean", "description": "Wait until the frame settles (default true)", "default": True},
            },
        },
    ),
    Tool(
        name="aletheon_tui_stop",
        description="Tear down the TUI tmux session started by aletheon_tui_start.",
        inputSchema={"type": "object", "properties": {}},
    ),
    Tool(
        name="aletheon_diagnose",
        description="One-stop diagnosis: drives the real TUI with a task, captures the settled frame + render checks, and bundles daemon analyze + logs + audit tail into a single ts-sorted timeline with a pass/fail verdict. Prefer this over aletheon_ask when the bug might be in the TUI layer.",
        inputSchema={
            "type": "object",
            "properties": {
                "task": {"type": "string", "description": "Task to send to the TUI"},
            },
            "required": ["task"],
        },
    ),
]

# Tool handler dispatch table
_HANDLERS = {
    "aletheon_check_install": lambda client, args: validate_installation(),
    "aletheon_health": lambda client, args: health_mod.health(client),
    "aletheon_snapshot": lambda client, args: snapshot_mod.snapshot(
        client, include_memory=args.get("include_memory", False)
    ),
    "aletheon_analyze": lambda client, args: analyze_mod.analyze(client),
    "aletheon_journal": lambda client, args: journal_mod.journal(
        client,
        last_n=args.get("last_n", 20),
        event_type=args.get("event_type", ""),
    ),
    "aletheon_logs": lambda client, args: logs_mod.logs(
        client,
        last_n=args.get("last_n", 50),
        level=args.get("level", ""),
    ),
    "aletheon_memory": lambda client, args: memory_mod.memory(
        client,
        query=args.get("query", ""),
        memory_type=args.get("memory_type", "all"),
        limit=args.get("limit", 10),
    ),
    "aletheon_sessions": lambda client, args: sessions_mod.sessions(
        client,
        action=args.get("action", "list"),
        session_id=args.get("session_id", ""),
    ),
    "aletheon_ask": lambda client, args: ask_mod.ask(
        client, question=args.get("question", "")
    ),
    "aletheon_watch": lambda client, args: watch_mod.watch(
        client,
        topic=args.get("topic", "perf"),
        duration_seconds=args.get("duration_seconds", 10),
    ),
    "aletheon_tui_start": lambda client, args: tui_mod.tui_start(
        task=args.get("task", ""),
        cols=args.get("cols", 100),
        rows=args.get("rows", 40),
    ),
    "aletheon_tui_send": lambda client, args: tui_mod.tui_send(
        text=args.get("text", ""),
        submit=args.get("submit", True),
    ),
    "aletheon_tui_capture": lambda client, args: tui_mod.tui_capture(
        scrollback=args.get("scrollback", True),
        wait_stable=args.get("wait_stable", True),
    ),
    "aletheon_tui_stop": lambda client, args: tui_mod.tui_stop(),
    "aletheon_diagnose": lambda client, args: diagnose_mod.diagnose(
        client, task=args.get("task", ""),
    ),
}


# ── Server ────────────────────────────────────────────────────────────────

server = Server("aletheon-monitor")


@server.list_tools()
async def list_tools():
    return TOOLS


@server.call_tool()
async def call_tool(name: str, arguments: dict) -> list[TextContent]:
    handler = _HANDLERS.get(name)
    if handler is None:
        return [TextContent(type="text", text=f"Unknown tool: {name}")]

    try:
        result = await handler(get_client(), arguments)
        return [TextContent(
            type="text",
            text=json.dumps(result, ensure_ascii=False, indent=2, default=str),
        )]
    except Exception as e:
        return [TextContent(
            type="text",
            text=json.dumps(
                {"error": str(e), "tool": name},
                ensure_ascii=False,
                indent=2,
            ),
        )]


async def main():
    """Run the MCP server over stdio."""
    async with stdio_server() as (read_stream, write_stream):
        await server.run(
            read_stream,
            write_stream,
            server.create_initialization_options(),
        )


if __name__ == "__main__":
    import asyncio
    asyncio.run(main())
