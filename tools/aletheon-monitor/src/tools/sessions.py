"""aletheon_sessions — list and resume sessions."""

from ..client import AletheonClient


async def sessions(
    client: AletheonClient,
    action: str = "list",
    session_id: str = "",
) -> dict:
    """List sessions or resume a specific session.

    Args:
        client: Connected AletheonClient.
        action: "list" (default) or "resume".
        session_id: Required if action is "resume".
    """
    if action == "resume":
        if not session_id:
            return {"error": "session_id is required for resume action"}
        resp = await client.rpc("resume", {"session_id": session_id})
        if "error" in resp:
            return {"error": resp["error"], "action": "resume", "session_id": session_id}
        return {
            "action": "resume",
            "session_id": session_id,
            "result": resp.get("result", {}),
        }

    # List: try session.list first, then fall back to sessions
    resp = await client.rpc("session.list")
    if "error" in resp:
        resp = await client.rpc("sessions")

    if "error" in resp:
        return {"error": resp["error"], "sessions": []}

    result = resp.get("result", resp)
    session_list = result.get("sessions", result.get("list", []))

    return {
        "action": "list",
        "sessions": session_list,
        "count": len(session_list),
    }
