"""aletheon_diagnose — one-stop bundle: rendered TUI + daemon analyze + logs
+ audit tail, correlated into a single timeline.

Timeline alignment uses the raw ISO timestamp strings each source already
emits (daemon-log granularity); ISO-8601 strings sort chronologically, so no
parsing is required.
"""
import json
import os

from . import analyze as analyze_mod
from . import logs as logs_mod
from . import tui as tui_tools


def _audit_path() -> str:
    return os.environ.get(
        "ALETHEON_AUDIT",
        "/home/aurobear/Bear-ws/aletheon/.aletheon-audit.jsonl",
    )


def _audit_tail(n: int = 20) -> list[str]:
    try:
        with open(_audit_path(), encoding="utf-8", errors="replace") as f:
            return [ln.rstrip("\n") for ln in f.readlines()[-n:]]
    except OSError:
        return []


def build_timeline(journal: list[dict], audit_lines: list[str]) -> list[dict]:
    """Merge journal events and audit JSONL lines into one ts-sorted list."""
    events: list[dict] = []
    for ev in journal or []:
        ts = ev.get("timestamp") or ev.get("ts") or ""
        events.append({
            "ts": ts, "source": "journal",
            "summary": ev.get("type", ev.get("event", "event")),
        })
    for line in audit_lines or []:
        try:
            rec = json.loads(line)
        except (json.JSONDecodeError, TypeError):
            continue
        ts = rec.get("timestamp", "")
        tool = rec.get("tool_name", "tool")
        err = " [error]" if rec.get("is_error") else ""
        events.append({"ts": ts, "source": "audit",
                       "summary": f"{tool}{err}"})
    events.sort(key=lambda e: e["ts"])
    return events


async def diagnose(client, task: str, settle_secs: float = 6.0,
                   timeout: float = 120.0, cols: int = 120,
                   rows: int = 50) -> dict:
    """Drive the TUI with `task`, capture the settled frame, and bundle it
    with daemon-side analysis, logs, audit tail, and a merged timeline.

    COMPLETION IS HEURISTIC. There is currently no authoritative turn-complete
    signal available to this tool: the daemon's journal/status RPCs run on a
    *different* session than the TUI client creates (and journals aren't
    persisted — see the I2 bug), and this build's TUI does not render a
    machine-detectable busy/idle indicator. So the response phase waits until
    the frame has changed beyond the submitted-input baseline and then stayed
    unchanged for `settle_secs`. A multi-step turn whose inter-step LLM gap
    exceeds `settle_secs` may be captured mid-turn; raise `settle_secs` (and
    `timeout`) for slow/complex tasks. `settle_secs` defaults to 6s — larger
    than typical observed inter-step gaps but still a heuristic, not a
    guarantee. Robust completion needs upstream work (shared TUI/RPC session or
    a TUI idle marker)."""
    # Phase 0: launch the TUI (ready-gated) WITHOUT sending the task yet.
    # NOTE: ratatui uses the terminal ALTERNATE screen (no tmux scrollback),
    # and the TUI keeps its own internal scroll, auto-scrolling to the input
    # prompt when a turn ends. So a long answer can be scrolled off the pane by
    # completion time and a single capture-pane only sees the tail. A taller
    # pane (`rows`) helps short/medium answers fit but does NOT fully solve
    # long ones — that needs scroll-and-stitch or a TUI export mode (follow-up).
    started = await tui_tools.tui_start(task="", cols=cols, rows=rows)
    if not started.get("ok"):
        return {"error": "tui_start failed", "detail": started}

    try:
        # Phase 1: submit the task and let the input echo settle -> baseline.
        # This baseline includes the user's echoed input but no response yet.
        await tui_tools.tui_send(task, submit=True)
        submitted = await tui_tools.tui_capture(
            scrollback=True, wait_stable=True, stable_secs=0.8, timeout=20.0,
        )
        baseline_frame = submitted.get("frame", "")

        # Phase 2: wait for the assistant response to appear BEYOND the
        # submitted baseline and then stay quiet for settle_secs (heuristic
        # completion — see the docstring caveat).
        cap = await tui_tools.tui_capture(
            scrollback=True, wait_stable=True, baseline=baseline_frame,
            stable_secs=settle_secs, timeout=timeout,
        )
    finally:
        await tui_tools.tui_stop()

    daemon_analyze = await analyze_mod.analyze(client)
    daemon_logs = await logs_mod.logs(client, last_n=50)
    audit_tail = _audit_tail()

    recent_journal = []
    if isinstance(daemon_analyze, dict):
        recent_journal = daemon_analyze.get("recent_journal", []) or []

    verdict = "pass"
    if cap.get("checks"):
        verdict = "fail"
    if isinstance(daemon_analyze, dict) and daemon_analyze.get("healthy") is False:
        verdict = "fail"
    if cap.get("stable") is False:
        verdict = "fail"

    return {
        "task": task,
        "rendered_frame": cap.get("frame", ""),
        "stable": cap.get("stable"),
        "completion": f"heuristic (settled {settle_secs}s beyond input echo; "
                      "may be mid-turn if an inter-step gap exceeds that)",
        "tui_checks": cap.get("checks", []),
        "daemon": {"analyze": daemon_analyze, "logs": daemon_logs},
        "audit_tail": audit_tail,
        "timeline": build_timeline(recent_journal, audit_tail),
        "verdict": verdict,
    }
