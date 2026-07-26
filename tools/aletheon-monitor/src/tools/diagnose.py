"""aletheon_diagnose — one-stop bundle: rendered TUI + daemon analyze + logs
+ audit tail, correlated into a single timeline.

Timeline alignment uses the raw ISO timestamp strings each source already
emits (daemon-log granularity); ISO-8601 strings sort chronologically, so no
parsing is required.
"""
import json
import os
import asyncio
import re

from . import analyze as analyze_mod
from . import logs as logs_mod
from . import tui as tui_tools


def _audit_path() -> str:
    return os.environ.get(
        "ALETHEON_AUDIT",
        os.path.expanduser("~/.local/state/aletheon/audit.jsonl"),
    )


INFRASTRUCTURE_ERRORS = (
    "provider_unavailable",
    "provider_rejected_request",
    "provider_timeout",
    "inference provider failed",
    "google_unauthorized_account",
    "Can't mount proc",
    "Permission denied",
    "Aletheon authorization failed",
)


def frame_assertions(frame: str, prompt_visible: bool,
                     forbidden_strings: list[str] | None = None) -> list[dict]:
    """Return semantic TUI assertions; prompt/input text alone is not success."""
    forbidden = list(INFRASTRUCTURE_ERRORS)
    forbidden.extend(forbidden_strings or [])
    assertions = [
        {
            "name": f"forbidden:{text}",
            "passed": text.lower() not in frame.lower(),
        }
        for text in dict.fromkeys(forbidden)
    ]
    infrastructure_clean = all(item["passed"] for item in assertions)
    assertions.append({
        "name": "final_answer",
        "passed": infrastructure_clean and len(frame.strip()) > 20,
    })
    assertions.append({"name": "prompt_returned", "passed": prompt_visible})
    return assertions


def provider_metrics(daemon_logs: dict) -> dict:
    lines = daemon_logs.get("lines", []) if isinstance(daemon_logs, dict) else []
    text = "\n".join(str(line) for line in lines)
    retry_attempts = len(re.findall(r"(?:retrying|attempt)", text, re.IGNORECASE))
    provider_errors = sum(
        text.lower().count(marker.lower()) for marker in INFRASTRUCTURE_ERRORS[:4]
    )
    return {
        "retry_attempts": retry_attempts,
        "provider_error_markers": provider_errors,
    }


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
                   rows: int = 50, working_dir: str | None = None,
                   expected_cwd: str | None = None,
                   forbidden_strings: list[str] | None = None) -> dict:
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
    cwd = os.path.realpath(working_dir or os.getcwd())
    started = await tui_tools.tui_start(task="", cols=cols, rows=rows,
                                        working_dir=cwd)
    if not started.get("ok"):
        return {"error": "tui_start failed", "detail": started}

    try:
        # Test runs must not inherit a previously failed or structurally
        # incomplete conversation. The production TUI resumes the most recent
        # workspace session by default, so explicitly create a clean session
        # before establishing the task baseline.
        await tui_tools.tui_send("/new", submit=True)
        await tui_tools.tui_capture(
            scrollback=True, wait_stable=True, stable_secs=0.8, timeout=20.0,
        )

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
        cap = await tui_tools.tui_wait_turn_done(
            baseline=started.get("turn_done_count", 0), timeout=timeout
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
    if cap.get("turn_done") is not True or cap.get("stable") is False:
        verdict = "fail"

    assertions = []
    frame = cap.get("frame", "")
    expected = os.path.realpath(expected_cwd) if expected_cwd else None
    if expected:
        actual_cwd = os.path.realpath(started.get("working_dir", ""))
        assertions.append({"name": "expected_cwd", "passed": actual_cwd == expected,
                           "actual": actual_cwd,
                           "expected": expected})
    assertions.extend(frame_assertions(
        frame,
        cap.get("prompt_visible") is True,
        forbidden_strings,
    ))
    if any(not item["passed"] for item in assertions):
        verdict = "fail"

    async def command(*args: str) -> str:
        try:
            proc = await asyncio.create_subprocess_exec(
                *args, cwd=cwd, stdout=asyncio.subprocess.PIPE,
                stderr=asyncio.subprocess.STDOUT)
            out, _ = await asyncio.wait_for(proc.communicate(), timeout=5)
            return out.decode("utf-8", "replace").strip()
        except (OSError, asyncio.TimeoutError):
            return ""

    preflight = {
        "working_dir": cwd,
        "git_commit": await command("git", "rev-parse", "HEAD"),
        "git_toplevel": await command("git", "rev-parse", "--show-toplevel"),
        "binary": await command("aletheon", "version"),
        "service_active": await command("systemctl", "is-active", "aletheon"),
        "service_started": await command(
            "systemctl", "show", "aletheon", "-p", "ActiveEnterTimestamp"),
    }

    return {
        "task": task,
        "rendered_frame": cap.get("frame", ""),
        "stable": cap.get("stable"),
        "prompt_visible": cap.get("prompt_visible"),
        "completion": f"heuristic (settled {settle_secs}s beyond input echo; "
                      "may be mid-turn if an inter-step gap exceeds that)",
        "tui_checks": cap.get("checks", []),
        "daemon": {"analyze": daemon_analyze, "logs": daemon_logs},
        "audit_tail": audit_tail,
        "timeline": build_timeline(recent_journal, audit_tail),
        "provider_metrics": provider_metrics(daemon_logs),
        "preflight": preflight,
        "assertions": assertions,
        "verdict": verdict,
    }
