"""aletheon_analyze — composite diagnostic: parallel snapshot + perf + journal + anomaly scan."""

import asyncio

from ..anomaly import run_all as run_anomaly_rules
from ..client import AletheonClient


async def analyze(client: AletheonClient) -> dict:
    """Run parallel queries and return merged diagnostic with anomaly scan.

    Fetches session.snapshot + session.perf + session.journal in parallel,
    merges results, runs anomaly detection rules.
    """
    # Parallel fetch of all diagnostic sources
    snap_resp, perf_resp, journal_resp = await asyncio.gather(
        client.rpc("session.snapshot"),
        client.rpc("session.perf"),
        client.rpc("session.journal", {"last_n": 20}),
        return_exceptions=True,
    )

    # Normalize responses
    def unwrap(r):
        if isinstance(r, Exception):
            return {"error": str(r)}
        if isinstance(r, dict):
            return r.get("result", r)
        return {}

    snapshot_data = unwrap(snap_resp)
    perf_data = unwrap(perf_resp)
    journal_data = unwrap(journal_resp)

    # Extract journal events
    journal_events = journal_data.get("events", journal_data.get("journal", []))

    # Run anomaly detection
    anomalies = run_anomaly_rules(perf_data, snapshot_data, journal_events)

    # Determine overall health
    criticals = [a for a in anomalies if a.get("severity") == "CRITICAL"]
    healthy = len(criticals) == 0

    # Also check socket existence as a basic health gate
    import os
    socket_path = client.socket_path
    socket_ok = os.path.exists(socket_path)

    return {
        "healthy": healthy and socket_ok,
        "snapshot": snapshot_data,
        "perf": perf_data,
        "recent_journal": journal_events,
        "anomalies": anomalies,
        "socket": {
            "path": socket_path,
            "exists": socket_ok,
        },
    }
