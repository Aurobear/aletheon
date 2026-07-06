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


async def diagnose(client, task: str) -> dict:
    """Drive the TUI with `task`, capture the settled frame, and bundle it
    with daemon-side analysis, logs, audit tail, and a merged timeline."""
    started = await tui_tools.tui_start(task=task)
    if not started.get("ok"):
        return {"error": "tui_start failed", "detail": started}

    cap = await tui_tools.tui_capture(scrollback=True, wait_stable=True)
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
        "tui_checks": cap.get("checks", []),
        "daemon": {"analyze": daemon_analyze, "logs": daemon_logs},
        "audit_tail": audit_tail,
        "timeline": build_timeline(recent_journal, audit_tail),
        "verdict": verdict,
    }
