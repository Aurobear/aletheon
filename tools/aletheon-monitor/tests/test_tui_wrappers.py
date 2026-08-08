import asyncio

from src import tui_session as ts
from src.tools import tui as tui_tools


def test_tui_start_derives_user_runtime_environment_from_official_socket(
    monkeypatch, tmp_path
):
    calls = []

    async def fake_tmux(*args, timeout=5.0):
        calls.append(args)
        return 0, "", ""

    monkeypatch.setattr(ts, "_tmux", fake_tmux)
    monkeypatch.setenv(
        "ALETHEON_SOCKET", "/run/user/1234/aletheon/aletheon.sock"
    )
    monkeypatch.delenv("XDG_RUNTIME_DIR", raising=False)
    monkeypatch.delenv("DBUS_SESSION_BUS_ADDRESS", raising=False)

    result = asyncio.run(
        ts.start("aletheon", session="test-tui", working_dir=str(tmp_path))
    )

    assert result["ok"] is True
    launch = calls[-1]
    assert "XDG_RUNTIME_DIR=/run/user/1234" in launch
    assert "DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/1234/bus" in launch


def test_tui_start_preserves_explicit_user_runtime_environment(monkeypatch, tmp_path):
    calls = []

    async def fake_tmux(*args, timeout=5.0):
        calls.append(args)
        return 0, "", ""

    monkeypatch.setattr(ts, "_tmux", fake_tmux)
    monkeypatch.setenv(
        "ALETHEON_SOCKET", "/run/user/1234/aletheon/aletheon.sock"
    )
    monkeypatch.setenv("XDG_RUNTIME_DIR", "/operator/runtime")
    monkeypatch.setenv("DBUS_SESSION_BUS_ADDRESS", "unix:path=/operator/bus")

    result = asyncio.run(
        ts.start("aletheon", session="test-tui", working_dir=str(tmp_path))
    )

    assert result["ok"] is True
    launch = calls[-1]
    assert "XDG_RUNTIME_DIR=/operator/runtime" in launch
    assert "DBUS_SESSION_BUS_ADDRESS=unix:path=/operator/bus" in launch


def test_tui_capture_waits_for_stable(monkeypatch):
    frames = iter(["a", "b", "done", "done", "done", "done"])

    async def fake_capture(session=ts.DEFAULT_SESSION, scrollback=False):
        try:
            return next(frames)
        except StopIteration:
            return "done"

    monkeypatch.setattr(ts, "capture", fake_capture)
    res = asyncio.run(tui_tools.tui_capture(
        wait_stable=True, poll=0.01, stable_secs=0.03, timeout=5.0
    ))
    assert res["stable"] is True
    assert res["frame"] == "done"
    assert isinstance(res["checks"], list)


def test_tui_capture_reports_timeout_when_never_stable(monkeypatch):
    counter = {"n": 0}

    async def fake_capture(session=ts.DEFAULT_SESSION, scrollback=False):
        counter["n"] += 1
        return f"frame-{counter['n']}"  # always changing

    monkeypatch.setattr(ts, "capture", fake_capture)
    res = asyncio.run(tui_tools.tui_capture(
        wait_stable=True, poll=0.01, stable_secs=0.03, timeout=0.1
    ))
    assert res["stable"] is False
    assert res["timeout"] is True


def test_tui_capture_reports_dead_pane(monkeypatch):
    async def empty_capture(session=ts.DEFAULT_SESSION, scrollback=False):
        return ""
    async def no_session(session=ts.DEFAULT_SESSION):
        return False
    monkeypatch.setattr(ts, "capture", empty_capture)
    monkeypatch.setattr(ts, "has_session", no_session)
    res = asyncio.run(tui_tools.tui_capture(wait_stable=True, poll=0.01, stable_secs=0.03, timeout=5.0))
    assert res["stable"] is False
    assert "error" in res


def test_tui_capture_ignores_pre_response_quiet_window(monkeypatch):
    """The screen sits static ('waiting') during LLM time-to-first-token; that
    must NOT be reported as stable. Only after the response streams and settles
    do we accept. Regression for the diagnose false-pass on empty responses."""
    # 4 identical 'waiting' frames (quiet window) then the streamed answer settles.
    frames = iter(["waiting", "waiting", "waiting", "waiting",
                   "answer", "answer", "answer", "answer"])

    async def fake_capture(session=ts.DEFAULT_SESSION, scrollback=False):
        try:
            return next(frames)
        except StopIteration:
            return "answer"

    monkeypatch.setattr(ts, "capture", fake_capture)
    res = asyncio.run(tui_tools.tui_capture(
        wait_stable=True, poll=0.01, stable_secs=0.03, timeout=5.0
    ))
    assert res["stable"] is True
    assert res["frame"] == "answer"  # NOT "waiting"
