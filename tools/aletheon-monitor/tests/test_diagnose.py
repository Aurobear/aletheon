import asyncio
from src.tools import tui as tui_tools
from src.tools import diagnose as diag
from src.tools.diagnose import build_timeline


def test_diagnose_stops_tui_on_capture_error(monkeypatch):
    calls = {"stopped": False}
    async def ok_start(task=""):
        return {"ok": True, "session": "s", "frame": ""}
    async def boom_capture(scrollback=True, wait_stable=True):
        raise RuntimeError("capture failed")
    async def rec_stop():
        calls["stopped"] = True
        return {"ok": True}
    monkeypatch.setattr(tui_tools, "tui_start", ok_start)
    monkeypatch.setattr(tui_tools, "tui_capture", boom_capture)
    monkeypatch.setattr(tui_tools, "tui_stop", rec_stop)
    try:
        asyncio.run(diag.diagnose(client=None, task="x"))
    except RuntimeError:
        pass
    assert calls["stopped"] is True


def test_build_timeline_sorts_sources_by_timestamp():
    journal = [
        {"timestamp": "2026-07-06T11:15:01Z", "type": "user_message"},
        {"timestamp": "2026-07-06T11:15:03Z", "type": "reflection"},
    ]
    audit = [
        '{"timestamp":"2026-07-06T11:15:02Z","tool_name":"glob"}',
        'not json — ignored',
    ]
    tl = build_timeline(journal, audit)
    assert [e["source"] for e in tl] == ["journal", "audit", "journal"]
    assert tl[0]["ts"] == "2026-07-06T11:15:01Z"
    assert tl[1]["summary"].startswith("glob") or "glob" in tl[1]["summary"]
