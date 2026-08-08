"""aletheon_watch — real-time event polling via session.journal + session.perf."""

import asyncio
import time

from ..client import AletheonClient
from .analyze import resolve_session_id

_monotonic = time.monotonic


async def watch(
    client: AletheonClient,
    topic: str = "tool",
    duration_seconds: int = 30,
    session_id: str = "",
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

    selected_session = session_id or None
    anomalies: list[dict] = []
    if want_tools or want_session:
        selected_session, anomalies = await resolve_session_id(client, session_id)
        if selected_session is None:
            return {
                "topic": topic,
                "duration_seconds": duration_seconds,
                "session_id": None,
                "events": [],
                "count": 0,
                "anomalies": anomalies,
            }

    events: list[dict] = []
    seen_ids: set[str] = set()
    deadline = _monotonic() + duration_seconds
    poll_interval = 1.0  # seconds between polls

    while _monotonic() < deadline:
        tasks = []
        if want_perf:
            tasks.append(client.rpc("session.perf"))
        if want_tools or want_session:
            tasks.append(client.rpc(
                "session.journal", {"session_id": selected_session, "limit": 50}
            ))

        results = await asyncio.gather(*tasks, return_exceptions=True)

        for result in results:
            if isinstance(result, Exception):
                anomalies.append({
                    "type": "watch_query_failed",
                    "severity": "CRITICAL",
                    "detail": str(result),
                })
                continue
            if not isinstance(result, dict):
                anomalies.append({
                    "type": "watch_query_failed",
                    "severity": "CRITICAL",
                    "detail": "invalid daemon response",
                })
                continue
            if "error" in result:
                anomalies.append({
                    "type": "watch_query_failed",
                    "severity": "CRITICAL",
                    "detail": str(result["error"]),
                })
                continue

            inner = result.get("result", result)

            # session.perf returns { tokens_in, tokens_out, turns, ... }
            perf = inner.get("perf", inner)
            if "tokens_in" in perf or "turns" in perf or "tool_calls" in perf:
                key = f"perf:{hash(str(perf))}"
                if key not in seen_ids:
                    seen_ids.add(key)
                    events.append({"type": "perf", "data": perf, "ts": time.time()})

            # session.journal returns { events: [...], count: N }
            for ev in inner.get("entries", inner.get("events", [])):
                ts = ev.get("ts") or ev.get("timestamp") or ""
                event_type = ev.get("type") or ev.get("event_type") or ""
                if want_tools and not want_session and event_type not in (
                    "tool_call", "tool_result"
                ):
                    continue
                sequence = ev.get("sequence", "")
                key = f"journal:{sequence}:{ts}:{event_type}:{hash(str(ev))}"
                if key not in seen_ids:
                    seen_ids.add(key)
                    events.append({"type": "journal", "data": ev, "ts": time.time()})

        if _monotonic() >= deadline:
            break
        await asyncio.sleep(poll_interval)

    return {
        "topic": topic,
        "duration_seconds": duration_seconds,
        "session_id": selected_session,
        "events": events,
        "count": len(events),
        "anomalies": anomalies,
    }
