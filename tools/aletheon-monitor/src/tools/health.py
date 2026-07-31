"""aletheon_health — daemon liveness + readiness check."""

from ..client import AletheonClient


async def health(client: AletheonClient) -> dict:
    """Check daemon health through the process-global health RPC.

    Always returns a structured dict with ``healthy`` boolean — never throws.
    The daemon's ``status`` RPC is session-scoped and therefore must not be
    called here without an authoritative session ID.
    """
    result = {
        "healthy": False,
        "daemon": {"reachable": False},
        "socket": {"path": client.socket_path, "exists": False},
    }

    import os
    result["socket"]["exists"] = os.path.exists(client.socket_path)

    try:
        import stat
        if result["socket"]["exists"]:
            result["socket"]["writable"] = bool(
                stat.S_IMODE(os.stat(client.socket_path).st_mode) & stat.S_IWUSR
            )
    except Exception:
        result["socket"]["writable"] = False

    # Fetch the process-global health snapshot.
    import asyncio
    health_resp = None
    systemd_info = None

    try:
        health_resp = await client.rpc("health")

        # The monitored client uses the per-user daemon when its socket lives
        # below /run/user/<uid>. Query the matching systemd manager instead of
        # the inactive system compatibility unit.
        try:
            user_scope = client.socket_path.startswith("/run/user/")
            systemctl_scope = ["--user"] if user_scope else []
            systemd_unit = (
                "aletheon.service" if user_scope else "aletheon-core.service"
            )
            proc = await asyncio.create_subprocess_exec(
                "systemctl", *systemctl_scope, "is-active", systemd_unit,
                stdout=asyncio.subprocess.PIPE,
                stderr=asyncio.subprocess.PIPE,
            )
            stdout, _ = await asyncio.wait_for(proc.communicate(), timeout=3.0)
            show = await asyncio.create_subprocess_exec(
                "systemctl", *systemctl_scope, "show", systemd_unit,
                "--property=NRestarts", "--value",
                stdout=asyncio.subprocess.PIPE,
                stderr=asyncio.subprocess.PIPE,
            )
            restart_stdout, _ = await asyncio.wait_for(
                show.communicate(), timeout=3.0
            )
            try:
                restart_count = int(restart_stdout.decode().strip())
            except ValueError:
                restart_count = None
            systemd_info = {
                "active": stdout.decode().strip() == "active",
                "scope": "user" if user_scope else "system",
                "unit": systemd_unit,
                "restart_count": restart_count,
            }
        except Exception:
            systemd_info = {"active": None, "error": "systemctl unavailable"}

        rpc_errors = []
        if not isinstance(health_resp, dict):
            rpc_errors.append("health: invalid response")
        elif "error" in health_resp:
            message = health_resp["error"].get("message", "RPC error")
            rpc_errors.append(f"health: {message}")
        elif "result" not in health_resp:
            rpc_errors.append("health: missing result")
        elif not isinstance(health_resp["result"], dict):
            rpc_errors.append("health: invalid result")
        # Any JSON-RPC response proves the daemon socket is reachable, but the
        # mandatory health probe must succeed before this monitor may report PASS.
        result["daemon"]["reachable"] = isinstance(health_resp, dict)
        if rpc_errors:
            result["error"] = "Mandatory daemon probe failed: " + "; ".join(rpc_errors)
    except Exception as e:
        result["error"] = f"Daemon unreachable: {e}"
        return result

    # Merge health response
    if (
        isinstance(health_resp, dict)
        and isinstance(health_resp.get("result"), dict)
    ):
        hr = health_resp["result"]
        readiness = hr.get("readiness", hr.get("status"))
        liveness = hr.get("liveness")
        operator_slo = hr.get("operator_slo") or {}
        if not isinstance(operator_slo, dict):
            operator_slo = {}
            result["error"] = "Daemon returned an invalid operator SLO snapshot"
        alerts = operator_slo.get("alerts") or []
        if not isinstance(alerts, list):
            alerts = ["invalid_operator_slo_alerts"]
        required_readiness = operator_slo.get("readiness_required", "ready")
        result["daemon"].update({
            "pid": hr.get("pid"),
            "uptime_seconds": hr.get("uptime_seconds"),
            "version": hr.get("version") or hr.get("daemon_version"),
            "liveness": liveness,
            "readiness": readiness,
        })
        result["operator_slo"] = operator_slo
        if liveness not in (None, "alive"):
            result["error"] = f"Daemon liveness is {liveness!r}"
        elif readiness != required_readiness:
            result["error"] = (
                f"Daemon readiness is {readiness!r}; "
                f"required {required_readiness!r}"
            )
        elif alerts:
            result["error"] = (
                "Daemon operator SLO alerts are active: " + ", ".join(alerts)
            )

    result["systemd"] = systemd_info or {"active": None}

    # Healthy if daemon is reachable + no critical errors
    result["healthy"] = (
        result["daemon"]["reachable"]
        and not result.get("error")
        and (systemd_info is None or systemd_info.get("active", True) is not False)
    )

    return result
