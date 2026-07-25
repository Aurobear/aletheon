import asyncio
from src.tools import tui as tui_tools
from src.tools import diagnose as diag
from src.tools.diagnose import build_timeline, frame_assertions, provider_metrics


def test_diagnose_stops_tui_on_capture_error(monkeypatch):
    calls = {"stopped": False}
    async def ok_start(task="", cols=120, rows=50, working_dir=None):
        return {"ok": True, "session": "s", "frame": ""}
    async def ok_send(text, submit=True):
        return {"ok": True}
    async def boom_capture(*args, **kwargs):
        raise RuntimeError("capture failed")
    async def rec_stop():
        calls["stopped"] = True
        return {"ok": True}
    monkeypatch.setattr(tui_tools, "tui_start", ok_start)
    monkeypatch.setattr(tui_tools, "tui_send", ok_send)
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


def test_provider_error_frame_cannot_pass_as_final_answer():
    assertions = frame_assertions(
        "│ hello\nerror: cognitive session: provider_unavailable\n❯",
        prompt_visible=True,
    )
    assert next(a for a in assertions if a["name"] == "final_answer")["passed"] is False
    assert next(
        a for a in assertions if a["name"] == "forbidden:provider_unavailable"
    )["passed"] is False


def test_provider_metrics_count_retries_and_errors():
    metrics = provider_metrics({
        "lines": [
            "Streaming inference unavailable; retrying attempt=1 provider_unavailable",
            "inference provider failed: provider_unavailable",
        ]
    })
    assert metrics["retry_attempts"] >= 1
    assert metrics["provider_error_markers"] >= 2
