import asyncio

from src.tools.health import health, systemd_user_environment


class FakeClient:
    socket_path = "/run/user/1000/aletheon/aletheon.sock"

    async def rpc(self, method):
        if method == "health":
            return {
                "result": {
                    "uptime_seconds": 10,
                    "daemon_version": "0.1.0",
                    "liveness": "alive",
                    "readiness": "ready",
                    "operator_slo": {
                        "readiness_required": "ready",
                        "alerts": [],
                    },
                }
            }
        return {"result": {"state": "idle", "turn_count": 1}}


class HealthErrorClient(FakeClient):
    async def rpc(self, method):
        if method == "health":
            return {"error": {"code": -32000, "message": "health unavailable"}}
        return await super().rpc(method)


class FakeProcess:
    def __init__(self, output):
        self.output = output

    async def communicate(self):
        return self.output, b""


def test_health_queries_user_manager_for_user_socket(monkeypatch):
    calls = []
    environments = []

    async def spawn(*args, **kwargs):
        calls.append(args)
        environments.append(kwargs.get("env", {}))
        if "is-active" in args:
            return FakeProcess(b"active\n")
        return FakeProcess(b"2\n")

    monkeypatch.setattr(asyncio, "create_subprocess_exec", spawn)
    monkeypatch.setattr("os.path.exists", lambda _path: True)

    result = asyncio.run(health(FakeClient()))

    assert result["healthy"] is True
    assert result["systemd"] == {
        "active": True,
        "scope": "user",
        "unit": "aletheon.service",
        "restart_count": 2,
    }
    assert calls[0][:3] == ("systemctl", "--user", "is-active")
    assert calls[1][:3] == ("systemctl", "--user", "show")
    for environment in environments:
        assert environment["XDG_RUNTIME_DIR"] == "/run/user/1000"
        assert environment["DBUS_SESSION_BUS_ADDRESS"] == "unix:path=/run/user/1000/bus"


def test_user_manager_environment_is_derived_without_overriding_operator_values():
    derived = systemd_user_environment(
        "/run/user/1234/aletheon/aletheon.sock", {"HOME": "/home/test"}
    )
    assert derived["XDG_RUNTIME_DIR"] == "/run/user/1234"
    assert derived["DBUS_SESSION_BUS_ADDRESS"] == "unix:path=/run/user/1234/bus"

    explicit = systemd_user_environment(
        "/run/user/1234/aletheon/aletheon.sock",
        {
            "XDG_RUNTIME_DIR": "/operator/runtime",
            "DBUS_SESSION_BUS_ADDRESS": "unix:path=/operator/bus",
        },
    )
    assert explicit["XDG_RUNTIME_DIR"] == "/operator/runtime"
    assert explicit["DBUS_SESSION_BUS_ADDRESS"] == "unix:path=/operator/bus"


def test_health_fails_closed_when_a_mandatory_rpc_errors(monkeypatch):
    async def spawn(*args, **kwargs):
        if "is-active" in args:
            return FakeProcess(b"active\n")
        return FakeProcess(b"0\n")

    monkeypatch.setattr(asyncio, "create_subprocess_exec", spawn)
    monkeypatch.setattr("os.path.exists", lambda _path: True)

    result = asyncio.run(health(HealthErrorClient()))

    assert result["daemon"]["reachable"] is True
    assert result["healthy"] is False
    assert "health unavailable" in result["error"]


def test_health_fails_when_daemon_readiness_or_slo_disagrees(monkeypatch):
    class DegradedClient(FakeClient):
        async def rpc(self, method):
            response = await super().rpc(method)
            response["result"]["readiness"] = "degraded"
            return response

    class AlertingClient(FakeClient):
        async def rpc(self, method):
            response = await super().rpc(method)
            response["result"]["operator_slo"]["alerts"] = [
                "provider_metrics_unavailable"
            ]
            return response

    async def spawn(*args, **kwargs):
        if "is-active" in args:
            return FakeProcess(b"active\n")
        return FakeProcess(b"0\n")

    monkeypatch.setattr(asyncio, "create_subprocess_exec", spawn)
    monkeypatch.setattr("os.path.exists", lambda _path: True)

    degraded = asyncio.run(health(DegradedClient()))
    assert degraded["healthy"] is False
    assert "readiness" in degraded["error"]

    alerting = asyncio.run(health(AlertingClient()))
    assert alerting["healthy"] is False
    assert "provider_metrics_unavailable" in alerting["error"]


def test_health_never_throws_on_invalid_result_shape(monkeypatch):
    class InvalidClient(FakeClient):
        async def rpc(self, method):
            return {"result": ["not", "a", "health", "snapshot"]}

    async def spawn(*args, **kwargs):
        return FakeProcess(b"active\n")

    monkeypatch.setattr(asyncio, "create_subprocess_exec", spawn)
    monkeypatch.setattr("os.path.exists", lambda _path: True)

    result = asyncio.run(health(InvalidClient()))
    assert result["healthy"] is False
    assert "invalid result" in result["error"]
