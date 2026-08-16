import asyncio
import json

from src.client import AletheonClient, default_socket_path


def test_default_socket_is_private_to_effective_user(monkeypatch):
    monkeypatch.delenv("ALETHEON_SOCKET", raising=False)
    monkeypatch.delenv("XDG_RUNTIME_DIR", raising=False)
    monkeypatch.setattr("src.client.os.geteuid", lambda: 1234)
    assert default_socket_path() == "/run/user/1234/aletheon/aletheon.sock"
    assert AletheonClient().socket_path == "/run/user/1234/aletheon/aletheon.sock"


def test_xdg_runtime_directory_selects_private_socket(monkeypatch):
    monkeypatch.delenv("ALETHEON_SOCKET", raising=False)
    monkeypatch.setenv("XDG_RUNTIME_DIR", "/run/user/42")
    assert AletheonClient().socket_path == "/run/user/42/aletheon/aletheon.sock"


def test_fresh_connection_binds_legacy_monitor_handshake_before_request(tmp_path):
    socket_path = tmp_path / "aletheon.sock"
    received = []

    async def scenario():
        async def handle(reader, writer):
            for result in ({"readiness": "ready"}, {"sessions": []}):
                request = json.loads((await reader.readline()).decode())
                received.append(request)
                writer.write((json.dumps({
                    "jsonrpc": "2.0",
                    "id": request["id"],
                    "result": result,
                }) + "\n").encode())
                await writer.drain()
            writer.close()
            await writer.wait_closed()

        server = await asyncio.start_unix_server(handle, path=socket_path)
        client = AletheonClient(str(socket_path))
        try:
            response = await client.rpc(
                "session.read_sessions/v1",
                {"protocol_version": 1, "payload": {"type": "read_sessions"}},
            )
            assert response["result"] == {"sessions": []}
        finally:
            await client.close()
            server.close()
            await server.wait_closed()

    asyncio.run(scenario())

    assert [request["method"] for request in received] == [
        "health",
        "session.read_sessions/v1",
    ]
