"""aletheon_watch — real-time event subscription (time-bounded)."""

import asyncio

from ..client import AletheonClient


async def watch(
    client: AletheonClient,
    topic: str = "perf",
    duration_seconds: int = 10,
) -> dict:
    """Subscribe to a real-time event topic for a configurable duration.

    Subscribes, collects events into a buffer, then unsubscribes and returns.

    Args:
        client: Connected AletheonClient.
        topic: Event topic: "perf", "tool", "session", "all".
        duration_seconds: How long to collect events (max 60).
    """
    duration_seconds = min(duration_seconds, 60)
    duration_seconds = max(duration_seconds, 1)

    resp = await client.rpc("session.watch", {"topic": topic})

    if "error" in resp:
        return {"error": resp["error"], "topic": topic, "events": []}

    events = []
    deadline = asyncio.get_event_loop().time() + duration_seconds

    while asyncio.get_event_loop().time() < deadline:
        try:
            line = await asyncio.wait_for(
                client.rpc("session.watch", {"topic": topic, "poll": True}),
                timeout=min(2.0, deadline - asyncio.get_event_loop().time()),
            )
            if isinstance(line, dict):
                evt = line.get("result", line)
                if evt and "error" not in evt:
                    events.append(evt)
        except asyncio.TimeoutError:
            break
        except Exception:
            break

    return {
        "topic": topic,
        "duration_seconds": duration_seconds,
        "events": events,
        "count": len(events),
    }
