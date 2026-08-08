"""tui_session.py — drive and capture a real TUI running inside a tmux pane.

Thin async wrappers over tmux primitives. State (the tmux session) lives in
the OS, so these functions are stateless apart from the session name.
"""
import asyncio
import os
import re

DEFAULT_SESSION = "aletheon-tui-debug"


def _user_runtime_environment(base: dict[str, str] | None = None) -> dict[str, str]:
    """Fill login-session variables omitted by headless MCP hosts.

    The monitored per-user socket identifies the authoritative user runtime.
    Preserve explicit operator values, but derive the standard runtime and D-Bus
    paths when the host process did not inherit them from a login session.
    """
    environment = dict(os.environ if base is None else base)
    socket_path = environment.get("ALETHEON_SOCKET", "")
    match = re.match(r"^/run/user/([0-9]+)(?:/|$)", socket_path)
    if match:
        runtime_dir = f"/run/user/{match.group(1)}"
        environment.setdefault("XDG_RUNTIME_DIR", runtime_dir)
        environment.setdefault(
            "DBUS_SESSION_BUS_ADDRESS", f"unix:path={runtime_dir}/bus"
        )
    return environment


async def _tmux(*args: str, timeout: float = 5.0) -> tuple[int, str, str]:
    """Run a tmux command, return (returncode, stdout, stderr)."""
    proc = await asyncio.create_subprocess_exec(
        "tmux", *args,
        stdout=asyncio.subprocess.PIPE,
        stderr=asyncio.subprocess.PIPE,
    )
    out, err = await asyncio.wait_for(proc.communicate(), timeout=timeout)
    return (
        proc.returncode,
        out.decode("utf-8", "replace"),
        err.decode("utf-8", "replace"),
    )


async def start(cmd: str, session: str = DEFAULT_SESSION,
                cols: int = 100, rows: int = 40,
                working_dir: str | None = None) -> dict:
    """Start `cmd` in a fresh detached tmux session (idempotent)."""
    await kill(session)  # clean slate
    cwd = os.path.realpath(working_dir or os.getcwd())
    args = ["new-session", "-d", "-s", session,
            "-x", str(cols), "-y", str(rows), "-c", cwd]
    environment = _user_runtime_environment()
    for name in ("XDG_RUNTIME_DIR", "DBUS_SESSION_BUS_ADDRESS"):
        if value := environment.get(name):
            # Set the new session environment explicitly.  Merely passing env
            # to the tmux client is insufficient when an existing tmux server
            # was itself started without login-session variables.
            args.extend(["-e", f"{name}={value}"])
    args.append(cmd)
    rc, _, err = await _tmux(*args)
    if rc != 0:
        return {"ok": False, "error": f"tmux new-session failed: {err.strip()}"}
    return {"ok": True, "session": session, "cols": cols, "rows": rows,
            "working_dir": cwd}


async def send(text: str, session: str = DEFAULT_SESSION,
               submit: bool = True) -> dict:
    """Type literal `text` into the pane; optionally press Enter to submit."""
    rc, _, err = await _tmux("send-keys", "-t", session, "-l", text)
    if rc != 0:
        return {"ok": False, "error": f"send-keys failed: {err.strip()}"}
    if submit:
        rc, _, err = await _tmux("send-keys", "-t", session, "Enter")
        if rc != 0:
            return {"ok": False, "error": f"send Enter failed: {err.strip()}"}
    return {"ok": True}


async def capture(session: str = DEFAULT_SESSION,
                  scrollback: bool = False) -> str:
    """Capture the pane's rendered text. With scrollback, include history."""
    args = ["capture-pane", "-t", session, "-p"]
    if scrollback:
        args += ["-S", "-"]
    rc, out, _ = await _tmux(*args)
    return out if rc == 0 else ""


async def kill(session: str = DEFAULT_SESSION) -> dict:
    """Kill the tmux session if it exists (idempotent)."""
    rc, _, _ = await _tmux("kill-session", "-t", session)
    return {"ok": rc == 0}


async def has_session(session: str = DEFAULT_SESSION) -> bool:
    rc, _, _ = await _tmux("has-session", "-t", session)
    return rc == 0
