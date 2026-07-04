"""aletheon_health — daemon liveness + readiness check."""

from ..client import AletheonClient


async def health(client: AletheonClient) -> dict:
    """Check daemon health by calling the health and status RPCs in parallel.

    Always returns a structured dict with ``healthy`` boolean — never throws.
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

    # Fetch health + status in parallel
    import asyncio
    health_resp = None
    status_resp = None
    systemd_info = None

    try:
        health_resp, status_resp = await asyncio.gather(
            client.rpc("health"),
            client.rpc("status"),
            return_exceptions=True,
        )

        # Best-effort systemd check via journalctl
        try:
            proc = await asyncio.create_subprocess_exec(
                "systemctl", "is-active", "aletheon",
                stdout=asyncio.subprocess.PIPE,
                stderr=asyncio.subprocess.PIPE,
            )
            stdout, _ = await asyncio.wait_for(proc.communicate(), timeout=3.0)
            systemd_info = {
                "active": stdout.decode().strip() == "active",
                "restart_count": None,  # filled below if available
            }
        except Exception:
            systemd_info = {"active": None, "error": "systemctl unavailable"}

        result["daemon"]["reachable"] = True
    except Exception as e:
        result["error"] = f"Daemon unreachable: {e}"
        return result

    # Merge health response
    if isinstance(health_resp, dict) and "result" in health_resp:
        hr = health_resp["result"]
        result["daemon"].update({
            "pid": hr.get("pid"),
            "uptime_seconds": hr.get("uptime_seconds"),
            "version": hr.get("version"),
        })

    # Merge status response
    if isinstance(status_resp, dict) and "result" in status_resp:
        sr = status_resp["result"]
        result["session"] = {
            "id": sr.get("session_id", ""),
            "turn_count": sr.get("turn_count", 0),
            "status": sr.get("state", "unknown"),
        }
        result["daemon"]["version"] = (
            result["daemon"].get("version") or sr.get("version")
        )

    result["systemd"] = systemd_info or {"active": None}

    # Healthy if daemon is reachable + no critical errors
    result["healthy"] = (
        result["daemon"]["reachable"]
        and not result.get("error")
        and (systemd_info is None or systemd_info.get("active", True) is not False)
    )

    return result
