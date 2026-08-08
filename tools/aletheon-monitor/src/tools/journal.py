"""aletheon_journal — event history query."""

from ..client import AletheonClient


async def journal(
    client: AletheonClient,
    last_n: int = 20,
    event_type: str = "",
    session_id: str = "",
) -> dict:
    """Retrieve recent session journal events.

    Args:
        client: Connected AletheonClient.
        last_n: Number of recent events to return.
        event_type: Optional filter: "tool_use", "user_message", "error",
            "compacted", "checkpoint".
    """
    params = {"limit": last_n}
    if session_id:
        params["session_id"] = session_id
    if event_type and event_type != "all":
        params["event_type"] = event_type

    resp = await client.rpc("session.journal", params)

    if "error" in resp:
        return {"error": resp["error"]}

    result = resp.get("result", resp)
    events = result.get("entries", result.get("events", result.get("journal", [])))

    return {
        "events": events,
        "count": len(events),
    }
