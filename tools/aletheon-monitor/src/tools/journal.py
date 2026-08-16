"""aletheon_journal — event history query."""

from ..client import AletheonClient
from .analyze import resolve_session_id


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
    selected, anomalies = await resolve_session_id(client, session_id)
    if selected is None:
        return {"error": "canonical session is ambiguous", "anomalies": anomalies}

    resp = await client.rpc("session.read_events/v1", {
        "protocol_version": 1,
        "payload": {
            "type": "read_events",
            "data": {
                "session_id": selected,
                "after": {"sequence": 0},
            },
        },
    })

    if "error" in resp:
        return {"error": resp["error"]}

    result = resp.get("result", resp)
    if isinstance(result, dict) and isinstance(result.get("payload"), dict):
        result = result["payload"]
    events = result.get("entries", result.get("events", result.get("journal", [])))
    if event_type and event_type != "all":
        events = [
            event for event in events
            if event.get("type", event.get("event_type")) == event_type
        ]
    events = events[-max(0, last_n):]

    return {
        "session_id": selected,
        "events": events,
        "count": len(events),
    }
