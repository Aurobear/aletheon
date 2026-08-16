import asyncio

from src.tools.analyze import analyze


class FakeClient:
    socket_path = "/tmp/aletheon-test.sock"

    def __init__(self, sessions, journal=None):
        self.sessions = sessions
        self.journal = journal if journal is not None else {"count": 0, "entries": []}
        self.calls = []

    async def rpc(self, method, params=None):
        self.calls.append((method, params or {}))
        if method == "session.read_sessions/v1":
            return {"result": {"protocol_version": 1, "payload": {
                "schema_version": 1, "sessions": self.sessions,
            }}}
        if method == "session.read_snapshot/v1":
            session_id = params["payload"]["data"]["session_id"]
            return {"result": {"protocol_version": 1, "payload": {
                "schema_version": 1,
                "session": {"id": session_id, "status": "active"},
                "tasks": [{"runtime_facts": {
                    "context_capacity_tokens": 100_000,
                    "active_context_occupancy_tokens": 5_000,
                    "cumulative_usage": {"total_input_tokens": 900_000},
                }}],
                "activities": [], "items": [],
            }}}
        if method == "debug.perf":
            return {"result": {"perf": {"tool_calls": {"by_tool": {}}}}}
        if method == "session.read_events/v1":
            if isinstance(self.journal, dict) and "error" in self.journal:
                return {"error": self.journal["error"]}
            events = self.journal.get("entries", self.journal.get("events", []))
            return {"result": {"protocol_version": 1, "payload": {
                "schema_version": 1,
                "session_id": params["payload"]["data"]["session_id"],
                "events": events,
            }}}
        raise AssertionError(method)


def test_analyze_selects_unique_active_session_and_binds_journal(monkeypatch):
    monkeypatch.setattr("src.tools.analyze.os.path.exists", lambda _path: True)
    client = FakeClient([
        {"id": "idle", "status": "completed"},
        {"id": "active", "status": "active"},
    ])
    result = asyncio.run(analyze(client))

    assert result["healthy"] is True
    assert result["session_id"] == "active"
    journal_call = next(
        call for call in client.calls if call[0] == "session.read_events/v1"
    )
    assert journal_call[1]["payload"]["data"]["session_id"] == "active"
    assert journal_call[1]["payload"]["data"]["after"] == {"sequence": 0}
    assert not any(item["type"] == "active_context_pressure" for item in result["anomalies"])


def test_analyze_fails_closed_when_active_session_is_ambiguous(monkeypatch):
    monkeypatch.setattr("src.tools.analyze.os.path.exists", lambda _path: True)
    client = FakeClient([
        {"id": "one", "status": "active"},
        {"id": "two", "status": "active"},
    ])
    result = asyncio.run(analyze(client))

    assert result["healthy"] is False
    assert result["session_id"] is None
    assert result["anomalies"][0]["type"] == "ambiguous_active_session"


def test_analyze_journal_failure_and_tool_errors_are_not_healthy(monkeypatch):
    monkeypatch.setattr("src.tools.analyze.os.path.exists", lambda _path: True)
    failed = asyncio.run(analyze(
        FakeClient([{"id": "one", "status": "active"}], journal={
            "error": {"message": "session not found"},
        })
    ))
    assert failed["healthy"] is False
    assert any(item["type"] == "journal_query_failed" for item in failed["anomalies"])

    tool_error = asyncio.run(analyze(
        FakeClient([{"id": "one", "status": "active"}], journal={
            "entries": [{
                "event_type": "tool_result",
                "item": {"payload": {"data": {"is_error": True}}},
            }],
        })
    ))
    assert tool_error["healthy"] is False
    assert any(item["type"] == "recent_tool_errors" for item in tool_error["anomalies"])
