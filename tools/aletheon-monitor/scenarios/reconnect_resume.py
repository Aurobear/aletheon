"""TUI long-output, real scrolling, same-session reconnect and persistence."""
from __future__ import annotations

import hashlib
import shutil
import uuid
from pathlib import Path

from src import scenarios as base
from src import tui_session
from src.client import AletheonClient
from src.tools import tui


def _params(event: dict) -> dict:
    value = event.get("params", {})
    return value if isinstance(value, dict) else {}


def _final_text(events: list[dict]) -> str:
    """Reconstruct the final iteration exclusively from recorded text deltas."""
    start = max((i for i, event in enumerate(events) if event.get("type") == "turn_started"), default=-1)
    return "".join(
        _params(event).get("text", "") for event in events[start + 1:]
        if event.get("type") == "text_delta" and isinstance(_params(event).get("text"), str)
    )


def _contains(value: object, needle: str) -> bool:
    if isinstance(value, dict):
        return any(_contains(item, needle) for item in value.values())
    if isinstance(value, list):
        return any(_contains(item, needle) for item in value)
    return isinstance(value, str) and needle in value


async def _current_session(client: AletheonClient) -> str | None:
    response = await client.rpc("status")
    value = response.get("result", {}).get("status", {}).get("session_id")
    return value if isinstance(value, str) and value else None


async def _send_key(key: str) -> bool:
    rc, _, _ = await tui_session._tmux("send-keys", "-t", tui_session.DEFAULT_SESSION, key)
    return rc == 0


async def run(source_root: str, timeout: float = 180.0) -> dict:
    root = Path(source_root).resolve()
    marker = f"ALETHEON_FINAL_{uuid.uuid4().hex}"
    receipt_root = root / ".scenario-runs" / uuid.uuid4().hex
    receipt_root.mkdir(parents=True)
    client = AletheonClient(timeout=15)
    session_id = None
    completed: dict = {}
    before_scroll = after_scroll = returned_bottom = ""

    started = await tui.tui_start(working_dir=str(root), cols=110, rows=35)
    if not started.get("ok"):
        return {"scenario": "reconnect_resume", "status": "FAIL", "failure": started}
    try:
        session_id = await _current_session(client)
        prompt = f"输出至少 60 行 workspace crate 说明；最后一行必须严格为 {marker}"
        sent = await tui.tui_send(prompt, submit=True)
        if sent.get("ok"):
            completed = await tui.tui_wait_turn_done(started.get("turn_done_count", 0), timeout)
            before_scroll = completed.get("frame", "")
            page_up = await _send_key("PageUp")
            capture = await tui.tui_capture(wait_stable=True, require_change=False, timeout=10)
            after_scroll = capture.get("frame", "")
            page_down = await _send_key("PageDown")
            bottom = await tui.tui_capture(wait_stable=True, require_change=False, timeout=10)
            returned_bottom = bottom.get("frame", "")
        else:
            page_up = page_down = False
            completed = {"turn_done": False, "error": sent}
    finally:
        await tui.tui_stop()

    first_events = base._events(completed.get("event_path"))
    durable_path = receipt_root / "initial-events.jsonl"
    if completed.get("event_path") and Path(completed["event_path"]).is_file():
        shutil.copyfile(completed["event_path"], durable_path)
    final_text = _final_text(first_events)
    final_hash = hashlib.sha256(final_text.encode()).hexdigest() if final_text else None

    reconnected = await tui.tui_start(working_dir=str(root), cols=110, rows=35)
    resumed_session = None
    reconnect_frame = ""
    try:
        if reconnected.get("ok") and session_id:
            await tui.tui_send(f"/resume {session_id}", submit=True)
            await tui.tui_capture(wait_stable=True, require_change=False, timeout=10)
            resumed_session = await _current_session(client)
            await tui.tui_send("/status", submit=True)
            reconnect_frame = (await tui.tui_capture(
                wait_stable=True, require_change=False, timeout=10
            )).get("frame", "")
    finally:
        await tui.tui_stop()

    resume_rpc = await client.rpc("resume", {"session_id": session_id}) if session_id else {}
    journal = await client.rpc("session.journal", {"limit": 500}) if session_id else {}
    await client.close()
    persisted = _contains(journal.get("result", {}), marker)
    assertions = [
        {"name": "initial_turn_done", "passed": completed.get("turn_done") is True},
        {"name": "structured_long_output", "passed": len(final_text.encode()) > 1000 and final_text.count("\n") >= 59},
        {"name": "final_marker_recorded", "passed": final_text.rstrip().endswith(marker)},
        {"name": "real_page_scroll", "passed": page_up and page_down and bool(before_scroll) and after_scroll != before_scroll},
        {"name": "returned_to_final_view", "passed": marker in returned_bottom},
        {"name": "tui_reconnected", "passed": reconnected.get("ok") is True},
        {"name": "same_session_id", "passed": bool(session_id) and resumed_session == session_id},
        {"name": "resume_record_count", "passed": resume_rpc.get("result", {}).get("recovered_messages", 0) >= 2},
        {"name": "final_answer_persisted", "passed": persisted},
    ]
    status = "PASS" if all(item["passed"] for item in assertions) else "FAIL"
    return {"scenario": "reconnect_resume", "status": status, "assertions": assertions,
            "evidence": {"event_path": str(durable_path), "session_id": session_id,
                "resumed_session_id": resumed_session, "final_sha256": final_hash,
                "final_bytes": len(final_text.encode()), "final_lines": final_text.count("\n") + 1,
                "reconnect_frame_bytes": len(reconnect_frame.encode()),
                "journal_entries": journal.get("result", {}).get("count")},
            "failure": None if status == "PASS" else {"failed_assertions": [
                item["name"] for item in assertions if not item["passed"]]},}
