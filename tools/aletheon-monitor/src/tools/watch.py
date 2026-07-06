"""aletheon_watch — real-time event polling via session.journal + session.perf."""

import asyncio
import time

from ..client import AletheonClient


async def watch(
    client: AletheonClient,
    topic: str = "tool",
    duration_seconds: int = 30,
) -> dict:
    """Poll session events over a configurable duration.

    The daemon's session.watch is a long-lived subscription that doesn't
    match the request/response model of the MCP bridge.  Instead we poll
    session.journal and session.perf at short intervals and return the
    delta (new events since the last poll).

    Args:
        client: Connected AletheonClient.
        topic: "tool", "perf", "session", or "all".
        duration_seconds: How long to collect events (max 60).
    """
    duration_seconds = min(duration_seconds, 60)
    duration_seconds = max(duration_seconds, 1)

    want_tools = topic in ("tool", "all")
    want_perf = topic in ("perf", "all")
    want_session = topic in ("session", "all")

    events: list[dict] = []
    seen_ids: set[str] = set()
    deadline = time.monotonic() + duration_seconds
    poll_interval = 1.0  # seconds between polls

    while time.monotonic() < deadline:
        tasks = []
        if want_perf:
            tasks.append(client.rpc("session.perf"))
        if want_tools or want_session:
            tasks.append(client.rpc("session.journal", {"last_n": 50}))

        results = await asyncio.gather(*tasks, return_exceptions=True)

        for result in results:
            if isinstance(result, Exception):
                continue
            if not isinstance(result, dict):
                continue

            inner = result.get("result", result)

            # session.perf returns { tokens_in, tokens_out, turns, ... }
            if "tokens_in" in inner or "turns" in inner:
                key = str(hash(str(inner)))
                if key not in seen_ids:
                    seen_ids.add(key)
                    events.append({"type": "perf", "data": inner, "ts": time.time()})

            # session.journal returns { events: [...], count: N }
            for ev in inner.get("events", []):
                ts = ev.get("ts") or ev.get("timestamp") or ""
                event_type = ev.get("type") or ev.get("event_type") or ""
                key = f"{ts}:{event_type}"
                if key not in seen_ids:
                    seen_ids.add(key)
                    events.append({"type": "journal", "data": ev, "ts": time.time()})

        if time.monotonic() >= deadline:
            break
        await asyncio.sleep(poll_interval)

    return {
        "topic": topic,
        "duration_seconds": duration_seconds,
        "events": events,
        "count": len(events),
    }
