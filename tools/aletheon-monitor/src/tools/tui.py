"""MCP wrappers for driving and capturing the real aletheon TUI via tmux."""
import asyncio
import os
import time

from .. import frame as frame_mod
from .. import tui_checks
from .. import tui_session as ts


def _tui_cmd() -> str:
    """Command that launches the TUI client bound to the daemon socket."""
    override = os.environ.get("ALETHEON_TUI_CMD")
    if override:
        return override
    sock = os.environ.get("ALETHEON_SOCKET", "/run/aletheon/aletheon.sock")
    return f"aletheon --socket {sock}"


async def tui_start(task: str = "", cols: int = 100, rows: int = 40) -> dict:
    """Launch the TUI in tmux; optionally send an initial task. Returns first frame."""
    started = await ts.start(_tui_cmd(), cols=cols, rows=rows)
    if not started.get("ok"):
        return started
    await asyncio.sleep(0.5)  # let the TUI paint its welcome screen
    if task:
        await ts.send(task, submit=True)
    raw = await ts.capture()
    return {"ok": True, "session": started["session"],
            "frame": frame_mod.normalize(raw)}


async def tui_send(text: str, submit: bool = True) -> dict:
    """Type text into the running TUI (optionally submit with Enter)."""
    return await ts.send(text, submit=submit)


async def tui_capture(scrollback: bool = True, wait_stable: bool = True,
                      poll: float = 0.5, stable_secs: float = 1.5,
                      timeout: float = 90.0) -> dict:
    """Capture the TUI frame. With wait_stable, poll until the screen settles
    (no change for `stable_secs`) or `timeout` elapses."""
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
        if frame_mod.is_stable(frames, window=window):
            norm = frames[-1]
            return {"stable": True, "frame": norm,
                    "checks": tui_checks.run_checks(norm)}
        await asyncio.sleep(poll)

    norm = frames[-1] if frames else ""
    return {"stable": False, "timeout": True, "frame": norm,
            "checks": tui_checks.run_checks(norm), "last_frames": frames[-3:]}


async def tui_stop() -> dict:
    """Tear down the TUI tmux session."""
    return await ts.kill()
