import asyncio

from src.tools.sessions import sessions


class FakeClient:
    def __init__(self, response):
        self.response = response

    async def rpc(self, method, params=None):
        assert method == "session.list"
        return self.response


def test_session_list_accepts_the_installed_daemon_list_shape():
    result = asyncio.run(
        sessions(FakeClient({"result": [{"session_id": "one"}, {"session_id": "two"}]}))
    )

    assert result == {
        "action": "list",
        "sessions": [{"session_id": "one"}, {"session_id": "two"}],
        "count": 2,
    }


def test_session_list_preserves_legacy_wrapped_shape():
    result = asyncio.run(
        sessions(FakeClient({"result": {"sessions": [{"session_id": "one"}]}}))
    )

    assert result["sessions"] == [{"session_id": "one"}]
    assert result["count"] == 1
