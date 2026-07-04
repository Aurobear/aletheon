"""aletheon_journal — event history query."""

from ..client import AletheonClient


async def journal(
    client: AletheonClient,
    last_n: int = 20,
    event_type: str = "",
) -> dict:
    """Retrieve recent session journal events.

    Args:
        client: Connected AletheonClient.
        last_n: Number of recent events to return.
        event_type: Optional filter: "tool_use", "user_message", "error",
            "compacted", "checkpoint".
    """
    params = {"last_n": last_n}
    if event_type and event_type != "all":
        params["event_type"] = event_type

    resp = await client.rpc("session.journal", params)

    if "error" in resp:
        return {"error": resp["error"]}

    result = resp.get("result", resp)
    events = result.get("events", result.get("journal", []))

    return {
        "events": events,
        "count": len(events),
    }
