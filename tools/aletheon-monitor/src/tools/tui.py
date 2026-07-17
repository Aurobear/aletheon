"""MCP wrappers for driving and capturing the real aletheon TUI via tmux."""
import asyncio
import hashlib
import os
import json
import re
import shlex
import stat
import time
import uuid
from pathlib import Path

from .. import frame as frame_mod
from .. import tui_checks
from .. import tui_session as ts
from ..client import default_socket_path

_ACTIVE_EVENT_PATH: Path | None = None


def _tui_cmd(event_path: Path) -> str:
    """Command that launches the TUI client bound to the daemon socket."""
    override = os.environ.get("ALETHEON_TUI_CMD")
    if override:
        quoted_path = shlex.quote(str(event_path))
        if "{event_path}" in override:
            return override.replace("{event_path}", quoted_path)
        return f"{override} --record-events {quoted_path}"
    sock = os.environ.get("ALETHEON_SOCKET", default_socket_path())
    return (f"aletheon --socket {shlex.quote(sock)} "
            f"--record-events {shlex.quote(str(event_path))}")


def _automatic_event_path(working_dir: str | None) -> Path:
    root = Path(working_dir or os.getcwd()).resolve()
    evidence_dir = root / ".scenario-runs" / "tui-events"
    evidence_dir.mkdir(mode=0o700, parents=True, exist_ok=True)
    if evidence_dir.is_symlink() or not evidence_dir.is_dir():
        raise RuntimeError("TUI evidence directory is unsafe")
    metadata = evidence_dir.stat()
    if metadata.st_uid != os.geteuid() or stat.S_IMODE(metadata.st_mode) & 0o077:
        raise RuntimeError("TUI evidence directory ownership or mode is unsafe")
    return evidence_dir / f"{uuid.uuid4().hex}.jsonl"


def _prepare_event_path(value: str | None, working_dir: str | None) -> Path:
    path = Path(value).resolve(strict=False) if value else _automatic_event_path(working_dir)
    parent = path.parent
    if parent.is_symlink() or not parent.is_dir():
        raise RuntimeError("TUI evidence parent is missing or unsafe")
    parent_metadata = parent.stat()
    if parent_metadata.st_uid != os.geteuid() or stat.S_IMODE(parent_metadata.st_mode) & 0o077:
        raise RuntimeError("TUI evidence parent ownership or mode is unsafe")
    flags = os.O_WRONLY | os.O_CREAT | os.O_EXCL
    if hasattr(os, "O_NOFOLLOW"):
        flags |= os.O_NOFOLLOW
    descriptor = os.open(path, flags, 0o600)
    try:
        os.write(descriptor, b"aletheon-event-recorder-pending")
    finally:
        os.close(descriptor)
    return path


def event_file_evidence(path_value: str | Path) -> dict:
    """Validate and summarize one durable, private TUI event recording."""
    path = Path(path_value)
    metadata = path.lstat()
    if path.is_symlink() or not stat.S_ISREG(metadata.st_mode):
        raise RuntimeError("TUI event evidence is not a regular file")
    mode = stat.S_IMODE(metadata.st_mode)
    if metadata.st_uid != os.geteuid() or mode != 0o600:
        raise RuntimeError("TUI event evidence ownership or mode is unsafe")
    content = path.read_bytes()
    if not content:
        raise RuntimeError("TUI event evidence is empty")
    try:
        lines = content.decode("utf-8").splitlines()
        records = [json.loads(line) for line in lines]
    except (UnicodeDecodeError, json.JSONDecodeError) as error:
        raise RuntimeError(f"TUI event evidence is invalid JSONL: {error}") from error
    if not records or any(not isinstance(record, dict) for record in records):
        raise RuntimeError("TUI event evidence has no structured events")
    return {
        "path": str(path.resolve()),
        "event_count": len(records),
        "sha256": hashlib.sha256(content).hexdigest(),
        "size_bytes": len(content),
        "uid": metadata.st_uid,
        "mode": "0600",
    }


def event_evidence_matches(value: object) -> bool:
    """Revalidate that a receipt still describes the same safe event file."""
    if not isinstance(value, dict) or not isinstance(value.get("path"), str):
        return False
    try:
        current = event_file_evidence(value["path"])
    except (OSError, RuntimeError):
        return False
    return current == value


def _turn_done_count(path: Path | None = None) -> int:
    event_path = path or _ACTIVE_EVENT_PATH
    if event_path is None:
        return 0
    try:
        count = 0
        for line in event_path.read_text(encoding="utf-8").splitlines():
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
                    working_dir: str | None = None,
                    event_path: str | None = None) -> dict:
    """Launch the TUI in tmux, wait until it is ready, optionally send a task.
    Returns the frame after the (optional) task echo."""
    global _ACTIVE_EVENT_PATH
    _ACTIVE_EVENT_PATH = None
    try:
        selected_event_path = _prepare_event_path(event_path, working_dir)
    except (OSError, RuntimeError) as error:
        return {"ok": False, "error": f"TUI event setup failed: {error}"}
    started = await ts.start(_tui_cmd(selected_event_path), cols=cols, rows=rows,
                             working_dir=working_dir)
    if not started.get("ok"):
        return started
    frame = await _wait_ready()
    if "aletheon" not in frame.lower():
        await ts.kill()
        return {"ok": False, "error": "TUI readiness marker was not observed",
                "frame": frame, "event_path": str(selected_event_path)}
    try:
        metadata = selected_event_path.lstat()
        if (selected_event_path.is_symlink() or not stat.S_ISREG(metadata.st_mode)
                or metadata.st_uid != os.geteuid()
                or stat.S_IMODE(metadata.st_mode) != 0o600
                or metadata.st_size != 0):
            raise RuntimeError("event file ownership, type, or mode changed")
    except (OSError, RuntimeError) as error:
        await ts.kill()
        return {"ok": False, "error": f"TUI event recorder failed: {error}",
                "event_path": str(selected_event_path)}
    _ACTIVE_EVENT_PATH = selected_event_path
    if task:
        sent = await ts.send(task, submit=True)
        if not sent.get("ok"):
            await ts.kill()
            return {"ok": False, "error": sent.get("error", "TUI task send failed"),
                    "event_path": str(selected_event_path)}
        frame = frame_mod.normalize(await ts.capture())
    return {"ok": True, "session": started["session"], "frame": frame,
            "working_dir": started.get("working_dir"),
            "turn_done_count": _turn_done_count(selected_event_path),
            "event_path": str(selected_event_path)}


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
    event_path = _ACTIVE_EVENT_PATH
    if event_path is None:
        return {"turn_done": False, "ok": False,
                "error": "no active TUI event recording"}
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        count = _turn_done_count(event_path)
        if count > baseline:
            try:
                evidence = event_file_evidence(event_path)
            except (OSError, RuntimeError) as error:
                return {"turn_done": False, "ok": False,
                        "error": f"TUI event validation failed: {error}",
                        "event_path": str(event_path)}
            captured = await tui_capture(wait_stable=True, require_change=False,
                                         require_prompt=False, timeout=10.0)
            return {**captured, "turn_done": True, "turn_done_count": count,
                    "completion_source": "client_event:turn_done",
                    "event_path": str(event_path), "event_evidence": evidence}
        if not await ts.has_session():
            return {"turn_done": False, "error": "tui session ended"}
        await asyncio.sleep(0.25)
    return {"turn_done": False, "timeout": True,
            "turn_done_count": _turn_done_count(event_path),
            "event_path": str(event_path)}


async def tui_stop() -> dict:
    """Tear down the TUI tmux session."""
    global _ACTIVE_EVENT_PATH
    result = await ts.kill()
    _ACTIVE_EVENT_PATH = None
    return result
