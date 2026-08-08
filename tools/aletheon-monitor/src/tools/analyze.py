"""Composite diagnostic bound to one authoritative canonical Session."""

import asyncio
import os

from ..anomaly import run_all as run_anomaly_rules
from ..client import AletheonClient


def _unwrap(response):
    if isinstance(response, Exception):
        return {"error": str(response)}
    if not isinstance(response, dict):
        return {"error": "invalid daemon response"}
    if "error" in response:
        return {"error": response["error"]}
    value = response.get("result", response)
    if isinstance(value, dict) and "payload" in value:
        value = value["payload"]
    return value if isinstance(value, dict) else {"error": "invalid daemon result"}


async def canonical_sessions(client: AletheonClient) -> tuple[list[dict], dict | None]:
    response = await client.rpc(
        "session.read_sessions/v1",
        {"protocol_version": 1, "payload": {"type": "read_sessions"}},
    )
    value = _unwrap(response)
    if "error" in value:
        return [], value["error"]
    sessions = value.get("sessions")
    if not isinstance(sessions, list):
        return [], {"message": "session list has no canonical sessions array"}
    return [item for item in sessions if isinstance(item, dict)], None


def _session_identity(session: dict) -> str | None:
    value = session.get("id", session.get("session_id"))
    return value if isinstance(value, str) and value else None


async def resolve_session_id(
    client: AletheonClient, requested: str = ""
) -> tuple[str | None, list[dict]]:
    sessions, error = await canonical_sessions(client)
    if error is not None:
        return None, [{
            "type": "session_list_unavailable",
            "severity": "CRITICAL",
            "detail": f"canonical session list failed: {error}",
        }]
    identities = {_session_identity(item): item for item in sessions}
    identities.pop(None, None)
    if requested:
        if requested not in identities:
            return None, [{
                "type": "session_not_found",
                "severity": "CRITICAL",
                "detail": f"requested canonical session `{requested}` was not found",
            }]
        return requested, []
    active = [
        session_id
        for session_id, item in identities.items()
        if str(item.get("status", "")).lower() == "active"
    ]
    if len(active) == 1:
        return active[0], []
    if len(identities) == 1:
        return next(iter(identities)), []
    detail = (
        f"{len(active)} active sessions among {len(identities)} canonical sessions; "
        "supply session_id or preserve the active turn/thread identity"
    )
    return None, [{
        "type": "ambiguous_active_session",
        "severity": "CRITICAL",
        "detail": detail,
    }]


async def analyze(client: AletheonClient, session_id: str = "") -> dict:
    """Analyze one canonical Session and fail closed on identity/source errors."""
    selected, identity_anomalies = await resolve_session_id(client, session_id)
    if selected is None:
        return {
            "healthy": False,
            "session_id": None,
            "snapshot": {},
            "perf": {},
            "recent_journal": [],
            "anomalies": identity_anomalies,
            "socket": {"path": client.socket_path, "exists": os.path.exists(client.socket_path)},
        }

    snapshot_params = {
        "protocol_version": 1,
        "payload": {"type": "read_snapshot", "data": {"session_id": selected}},
    }
    snap_resp, perf_resp, journal_resp = await asyncio.gather(
        client.rpc("session.read_snapshot/v1", snapshot_params),
        client.rpc("session.perf"),
        client.rpc("session.journal", {"session_id": selected, "limit": 20}),
        return_exceptions=True,
    )
    snapshot_data = _unwrap(snap_resp)
    perf_data = _unwrap(perf_resp)
    journal_data = _unwrap(journal_resp)
    if isinstance(perf_data.get("perf"), dict):
        perf_data = perf_data["perf"]
    journal_events = journal_data.get(
        "entries", journal_data.get("events", journal_data.get("journal", []))
    )
    if not isinstance(journal_events, list):
        journal_events = []

    source_anomalies = []
    for source, value in (
        ("snapshot", snapshot_data),
        ("perf", perf_data),
        ("journal", journal_data),
    ):
        if "error" in value:
            source_anomalies.append({
                "type": f"{source}_query_failed",
                "severity": "CRITICAL",
                "detail": f"{source} query for `{selected}` failed: {value['error']}",
            })

    anomalies = identity_anomalies + source_anomalies
    anomalies.extend(run_anomaly_rules(perf_data, snapshot_data, journal_events))
    socket_ok = os.path.exists(client.socket_path)
    if not socket_ok:
        anomalies.append({
            "type": "socket_missing",
            "severity": "CRITICAL",
            "detail": f"socket not found at {client.socket_path}",
        })

    return {
        "healthy": not anomalies,
        "session_id": selected,
        "snapshot": snapshot_data,
        "perf": perf_data,
        "recent_journal": journal_events,
        "anomalies": anomalies,
        "socket": {"path": client.socket_path, "exists": socket_ok},
    }
