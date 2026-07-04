"""aletheon_logs — daemon log tail with journalctl fallback."""

import asyncio

from ..client import AletheonClient


async def logs(
    client: AletheonClient,
    last_n: int = 50,
    level: str = "",
) -> dict:
    """Tail daemon log output.

    Tries session.log RPC first; falls back to journalctl if daemon is
    unreachable or the method is unavailable.

    Args:
        client: Connected AletheonClient.
        last_n: Number of recent lines to return.
        level: Optional filter: "ERROR", "WARN", "INFO".
    """
    params = {"last_n": last_n}
    if level and level != "all":
        params["level"] = level

    resp = await client.rpc("session.log", params)

    if "error" not in resp:
        result = resp.get("result", resp)
        lines = result.get("lines", result.get("logs", []))
        if lines:
            return {"lines": lines, "source": "daemon"}

    # Fallback to journalctl
    try:
        cmd = ["journalctl", "-u", "aletheon", "--no-pager", "-n", str(last_n)]
        if level and level != "all":
            cmd.extend(["-p", level.lower()])

        proc = await asyncio.create_subprocess_exec(
            *cmd,
            stdout=asyncio.subprocess.PIPE,
            stderr=asyncio.subprocess.PIPE,
        )
        stdout, stderr = await asyncio.wait_for(proc.communicate(), timeout=5.0)

        if proc.returncode == 0:
            raw_lines = stdout.decode("utf-8", errors="replace").strip().split("\n")
            return {
                "lines": [l for l in raw_lines if l],
                "source": "journalctl",
            }
        else:
            return {
                "lines": [],
                "source": "journalctl",
                "error": stderr.decode("utf-8", errors="replace").strip(),
            }
    except Exception as e:
        return {
            "lines": [],
            "source": "none",
            "error": f"Both session.log and journalctl failed: {e}",
        }
