import asyncio

from src import tui_session as ts
from src.tools import tui as tui_tools


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
