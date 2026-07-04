"""aletheon_snapshot — full runtime state dump."""

from ..client import AletheonClient


async def snapshot(client: AletheonClient, include_memory: bool = False) -> dict:
    """Get full runtime snapshot from Session Gateway.

    Args:
        client: Connected AletheonClient.
        include_memory: If True, include memory store contents (can be large).
    """
    params = {}
    if include_memory:
        params["include_memory"] = True

    resp = await client.rpc("session.snapshot", params)

    if "error" in resp:
        return {"error": resp["error"]}

    result = resp.get("result", resp)
    # Normalize top-level keys for consistent consumption
    output = {
        "state": result.get("state", {}),
        "turn": result.get("turn", {}),
        "config": result.get("config", {}),
        "self_field": result.get("self_field", {}),
    }
    if include_memory:
        output["memory"] = result.get("memory", {})

    return output
