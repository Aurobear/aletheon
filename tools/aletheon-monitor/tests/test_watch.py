import asyncio

from src.tools.watch import watch


class FakeClient:
    socket_path = "/tmp/aletheon-test.sock"

    def __init__(self):
        self.calls = []

    async def rpc(self, method, params=None):
        self.calls.append((method, params or {}))
        if method == "session.read_sessions/v1":
            return {"result": {"payload": {"sessions": [
                {"id": "active", "status": "active"},
            ]}}}
        if method == "session.read_events/v1":
            return {"result": {"payload": {"events": [
                {"sequence": 1, "event_type": "user_message"},
                {"sequence": 2, "event_type": "tool_result"},
            ]}}}
        raise AssertionError(method)


def test_watch_binds_unique_active_session_and_reads_canonical_entries(monkeypatch):
    ticks = iter([0.0, 0.0, 2.0])
    monkeypatch.setattr("src.tools.watch._monotonic", lambda: next(ticks))
    client = FakeClient()

    result = asyncio.run(watch(client, topic="tool", duration_seconds=1))

    assert result["session_id"] == "active"
    assert result["anomalies"] == []
    assert [event["data"]["event_type"] for event in result["events"]] == [
        "tool_result"
    ]
    journal = next(
        call for call in client.calls if call[0] == "session.read_events/v1"
    )
    assert journal[1]["payload"]["data"] == {
        "session_id": "active",
        "after": {"sequence": 0},
    }


def test_watch_fails_closed_for_ambiguous_active_sessions():
    client = FakeClient()

    async def ambiguous(method, params=None):
        if method == "session.read_sessions/v1":
            return {"result": {"payload": {"sessions": [
                {"id": "one", "status": "active"},
                {"id": "two", "status": "active"},
            ]}}}
        raise AssertionError(method)

    client.rpc = ambiguous
    result = asyncio.run(watch(client, topic="tool", duration_seconds=1))

    assert result["session_id"] is None
    assert result["events"] == []
    assert result["anomalies"][0]["type"] == "ambiguous_active_session"
