"""MCP wrappers for driving and capturing the real aletheon TUI via tmux."""
import asyncio
import os
import time
import re
import json
import tempfile
from pathlib import Path

from .. import frame as frame_mod
from .. import tui_checks
from .. import tui_session as ts

_EVENT_PATH = Path(tempfile.gettempdir()) / "aletheon-tui-debug-events.jsonl"


def _tui_cmd() -> str:
    """Command that launches the TUI client bound to the daemon socket."""
    override = os.environ.get("ALETHEON_TUI_CMD")
    if override:
        return override
    sock = os.environ.get("ALETHEON_SOCKET", "/run/aletheon/aletheon.sock")
    return f"aletheon --socket {sock} --record-events {_EVENT_PATH}"


def _turn_done_count() -> int:
    try:
        count = 0
        for line in _EVENT_PATH.read_text(encoding="utf-8").splitlines():
            record = json.loads(line)
            if record.get("type") == "turn_done":
                count += 1
        return count
    except (OSError, json.JSONDecodeError):
        return 0


async def _wait_ready(ready_timeout: float = 10.0, poll: float = 0.3) -> str:
    """Poll until the TUI has painted its shell (header contains 'aletheon').

    Returns the ready frame. The TUI must connect to the daemon before it can
    accept input; sending keys too early silently drops them, so callers MUST
    wait for readiness before send()."""
    deadline = time.monotonic() + ready_timeout
    frame = ""
    while time.monotonic() < deadline:
        await asyncio.sleep(poll)
        frame = frame_mod.normalize(await ts.capture())
        if "aletheon" in frame.lower():
            return frame
    return frame


async def tui_start(task: str = "", cols: int = 100, rows: int = 40,
                    working_dir: str | None = None) -> dict:
    """Launch the TUI in tmux, wait until it is ready, optionally send a task.
    Returns the frame after the (optional) task echo."""
    _EVENT_PATH.unlink(missing_ok=True)
    started = await ts.start(_tui_cmd(), cols=cols, rows=rows,
                             working_dir=working_dir)
    if not started.get("ok"):
        return started
    frame = await _wait_ready()
    if task:
        await ts.send(task, submit=True)
        frame = frame_mod.normalize(await ts.capture())
    return {"ok": True, "session": started["session"], "frame": frame,
            "working_dir": started.get("working_dir"),
            "turn_done_count": _turn_done_count(),
            "event_path": str(_EVENT_PATH)}


async def tui_send(text: str, submit: bool = True) -> dict:
    """Type text into the running TUI (optionally submit with Enter)."""
    return await ts.send(text, submit=submit)


async def tui_capture(scrollback: bool = True, wait_stable: bool = True,
                      poll: float = 0.5, stable_secs: float = 1.5,
                      timeout: float = 90.0, require_change: bool = True,
                      baseline: str | None = None,
                      require_prompt: bool = False) -> dict:
    """Capture the TUI frame. With wait_stable, poll until the screen settles
    (no change for `stable_secs`) or `timeout` elapses.

    With `require_change` (default), a settled screen is only accepted once it
    has *changed* from a reference frame — otherwise a static period (the LLM's
    time-to-first-token quiet window, or the just-submitted input echo) would
    be mistaken for "done", yielding a false pass on an empty response. The
    reference is `baseline` if given (pass the post-submit frame so only a real
    assistant response counts), else the first captured frame. Set
    `require_change=False` to accept the current settled frame regardless."""
    if not wait_stable:
        norm = frame_mod.normalize(await ts.capture(scrollback=scrollback))
        return {"stable": None, "frame": norm, "checks": tui_checks.run_checks(norm)}

    window = max(2, int(stable_secs / poll))
    frames: list[str] = []
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        frames.append(frame_mod.normalize(await ts.capture(scrollback=scrollback)))
        if not frames[-1] and not await ts.has_session():
            return {"stable": False, "error": "tui session ended (pane died)",
                    "frame": "", "checks": []}
        # Ignore stability until the screen has changed from the reference
        # (baseline if given, else first frame): the pre-response quiet window
        # and the bare input echo are both static and must not count as "done".
        ref = baseline if baseline is not None else frames[0]
        activity_seen = (not require_change) or (frames[-1] != ref)
        prompt_visible = bool(re.search(r"(?m)^\s*❯", frames[-1]))
        if (activity_seen and frame_mod.is_stable(frames, window=window)
                and (prompt_visible or not require_prompt)):
            norm = frames[-1]
            return {"stable": True, "frame": norm,
                    "prompt_visible": prompt_visible,
                    "checks": tui_checks.run_checks(norm)}
        await asyncio.sleep(poll)

    norm = frames[-1] if frames else ""
    return {"stable": False, "timeout": True, "frame": norm,
            "checks": tui_checks.run_checks(norm), "last_frames": frames[-3:]}


async def tui_wait_turn_done(baseline: int = 0, timeout: float = 120.0) -> dict:
    """Wait for an authoritative daemon TurnDone event recorded by the TUI."""
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        count = _turn_done_count()
        if count > baseline:
            captured = await tui_capture(wait_stable=True, require_change=False,
                                         require_prompt=False, timeout=10.0)
            return {**captured, "turn_done": True, "turn_done_count": count,
                    "completion_source": "client_event:turn_done",
                    "event_path": str(_EVENT_PATH)}
        if not await ts.has_session():
            return {"turn_done": False, "error": "tui session ended"}
        await asyncio.sleep(0.25)
    return {"turn_done": False, "timeout": True,
            "turn_done_count": _turn_done_count(),
            "event_path": str(_EVENT_PATH)}


async def tui_stop() -> dict:
    """Tear down the TUI tmux session."""
    return await ts.kill()
