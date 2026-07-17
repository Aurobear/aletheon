"""SubAgent lifecycle scenario backed only by daemon/session evidence."""
from __future__ import annotations

import asyncio
import hashlib
import json
import uuid
from pathlib import Path

from src import scenarios as base
from src.client import AletheonClient
from src.tools import tui


_REQUIRED_TOOLS = {"agent_spawn", "agent_list", "agent_send", "agent_cancel", "agent_wait"}


def _params(event: dict) -> dict:
    value = event.get("params", {})
    return value if isinstance(value, dict) else {}


def _json_output(event: dict) -> object | None:
    output = _params(event).get("output")
    if not isinstance(output, str):
        return None
    try:
        return json.loads(output)
    except json.JSONDecodeError:
        return None


def _tool_result(value: object | None) -> object | None:
    if not isinstance(value, dict) or value.get("ok") is not True or "result" not in value:
        return None
    return value["result"]


def _agent_id(value: object) -> str | None:
    if not isinstance(value, dict):
        return None
    direct = value.get("agent_id")
    if isinstance(direct, str) and direct:
        return direct
    handle = value.get("handle")
    if isinstance(handle, dict):
        nested = handle.get("agent_id")
        if isinstance(nested, str) and nested:
            return nested
    return None


def _snapshot_for_agent(value: object, agent_id: str) -> dict | None:
    candidates = value if isinstance(value, list) else [value]
    for candidate in candidates:
        if isinstance(candidate, dict) and _agent_id(candidate) == agent_id:
            return candidate
    return None


def _valid_agent_result(value: object) -> bool:
    if not isinstance(value, dict):
        return False
    output = value.get("output")
    usage = value.get("usage")
    if not isinstance(output, str) or not output.strip() or not isinstance(usage, dict):
        return False
    integer_fields = ("input_tokens", "output_tokens", "elapsed_ms")
    return (
        all(isinstance(usage.get(field), int) and usage[field] >= 0 for field in integer_fields)
        and (usage.get("cost_usd") is None or isinstance(usage.get("cost_usd"), (int, float)))
        and isinstance(value.get("evidence"), list)
        and isinstance(value.get("artifacts"), list)
    )


def _canonical_hash(value: object) -> str | None:
    if value is None:
        return None
    encoded = json.dumps(
        value, ensure_ascii=False, sort_keys=True, separators=(",", ":")
    ).encode("utf-8")
    return hashlib.sha256(encoded).hexdigest()


def _final_text(events: list[dict]) -> str:
    start = max(
        (index for index, event in enumerate(events) if event.get("type") == "turn_started"),
        default=-1,
    )
    return "".join(
        _params(event).get("text", "")
        for event in events[start + 1 :]
        if event.get("type") == "text_delta"
        and isinstance(_params(event).get("text"), str)
    )


def _contains(value: object, needle: str) -> bool:
    if isinstance(value, dict):
        return any(_contains(item, needle) for item in value.values())
    if isinstance(value, list):
        return any(_contains(item, needle) for item in value)
    return isinstance(value, str) and needle in value


def _assistant_journal_promotes(journal: object, marker: str, marker_hash: str) -> bool:
    if not isinstance(journal, dict):
        return False
    entries = journal.get("entries")
    if not isinstance(entries, list):
        return False
    return any(
        isinstance(entry, dict)
        and entry.get("event_type") == "assistant_message"
        and _contains(entry.get("event"), marker)
        and _contains(entry.get("event"), marker_hash)
        for entry in entries
    )


def _lifecycle_evidence(events: list[dict], marker: str, marker_hash: str) -> dict:
    calls: dict[str, dict] = {}
    duplicate_call_ids: set[str] = set()
    for index, event in enumerate(events):
        params = _params(event)
        call_id = params.get("call_id")
        if not isinstance(call_id, str) or not call_id:
            continue
        if event.get("type") == "tool_call_start":
            if call_id in calls:
                duplicate_call_ids.add(call_id)
                continue
            calls[call_id] = {"call_id": call_id, "tool": params.get("tool"),
                              "args": params.get("args"), "order": index, "result": None}
        elif event.get("type") == "tool_call_complete" and call_id in calls:
            if params.get("tool") == calls[call_id]["tool"]:
                calls[call_id]["args"] = params.get("args")
        elif event.get("type") == "tool_call_result" and call_id in calls:
            if params.get("tool") == calls[call_id]["tool"]:
                if calls[call_id].get("result_event") is not None:
                    duplicate_call_ids.add(call_id)
                calls[call_id]["result"] = (
                    None if params.get("is_error") is True else _tool_result(_json_output(event))
                )
                calls[call_id]["result_event"] = event

    ordered = sorted(calls.values(), key=lambda call: call["order"])
    by_tool = {
        tool: [call for call in ordered if call["tool"] == tool]
        for tool in _REQUIRED_TOOLS
    }
    spawn_calls = by_tool["agent_spawn"]
    spawn_ids = [_agent_id(call["result"]) for call in spawn_calls]
    two_distinct_agents = (
        len(spawn_calls) == 2 and all(spawn_ids) and len(set(spawn_ids)) == 2
    )
    first_id, second_id = (spawn_ids + [None, None])[:2]

    def args_target(call: dict, agent_id: str | None) -> bool:
        args = call.get("args")
        return bool(agent_id) and isinstance(args, dict) and args.get("agent_id") == agent_id

    send_calls = [call for call in by_tool["agent_send"] if args_target(call, first_id)]
    send_delivered = any(
        isinstance(call["result"], dict)
        and call["result"].get("to") == first_id
        and call["result"].get("delivery") == "delivered"
        for call in send_calls
    )
    list_snapshots = [
        snapshot for call in by_tool["agent_list"]
        if call["result"] is not None and first_id
        for snapshot in [_snapshot_for_agent(call["result"], first_id)] if snapshot is not None
    ]
    wait_calls = [call for call in by_tool["agent_wait"] if args_target(call, first_id)]
    terminal_pair = next((
        (call, snapshot) for call in wait_calls
        for snapshot in [_snapshot_for_agent(call["result"], first_id)]
        if snapshot is not None
        and snapshot.get("status") == "succeeded"
        and _valid_agent_result(snapshot.get("result"))
    ), None)
    promotion_evidence = None
    if terminal_pair is not None:
        wait_call, terminal_result = terminal_pair
        agent_result = terminal_result["result"]
        output = agent_result["output"]
        promotion_evidence = {
            "wait_call_id": wait_call["call_id"],
            "agent_id": first_id,
            "status": terminal_result["status"],
            "agent_result": agent_result,
            "result_sha256": _canonical_hash(agent_result),
            "marker_in_agent_result": marker in output and marker_hash in output,
        }
    cancel_calls = [call for call in by_tool["agent_cancel"] if args_target(call, second_id)]
    cancelled = any(
        (snapshot := _snapshot_for_agent(call["result"], second_id)) is not None
        and snapshot.get("status") == "cancelled"
        for call in cancel_calls if second_id
    )
    cancelled_snapshot = next(
        (
            snapshot
            for call in cancel_calls
            for snapshot in [_snapshot_for_agent(call["result"], second_id)]
            if snapshot is not None and snapshot.get("status") == "cancelled"
        ),
        None,
    )
    parent_text = _final_text(events)
    marker_in_parent_text = marker in parent_text and marker_hash in parent_text
    marker_in_agent_result = bool(
        promotion_evidence and promotion_evidence["marker_in_agent_result"]
    )
    tools = {call["tool"] for call in ordered if isinstance(call["tool"], str)}
    return {
        "tools": sorted(tools),
        "missing_tools": sorted(_REQUIRED_TOOLS - tools),
        "call_ids_unique": not duplicate_call_ids,
        "all_calls_completed": bool(calls) and all(call.get("result_event") is not None for call in ordered),
        "two_distinct_agents": two_distinct_agents,
        "spawn_agent_ids": spawn_ids,
        "first_agent_listed": bool(list_snapshots),
        "mailbox_delivered_to_first": send_delivered,
        "first_agent_succeeded": promotion_evidence is not None,
        "agent_result_contains_marker_hash": marker_in_agent_result,
        "parent_text_contains_marker_hash": marker_in_parent_text,
        "result_promoted_to_parent": marker_in_agent_result and marker_in_parent_text,
        "promotion_evidence": promotion_evidence,
        "second_agent_cancelled": cancelled,
        "terminal_statuses": {
            first_id: "succeeded" if promotion_evidence else None,
            second_id: "cancelled" if cancelled else None,
        },
        "terminal_result_hashes": {
            first_id: promotion_evidence["result_sha256"] if promotion_evidence else None,
            second_id: _canonical_hash(cancelled_snapshot.get("result"))
            if cancelled_snapshot
            else None,
        },
    }


def _post_restart_agent_evidence(
    events: list[dict], agent_ids: list[str], expected_statuses: dict, expected_hashes: dict
) -> dict:
    snapshots: dict[str, dict] = {}
    starts = {
        _params(event).get("call_id")
        for event in events
        if event.get("type") == "tool_call_start"
        and _params(event).get("tool") == "agent_list"
        and isinstance(_params(event).get("call_id"), str)
    }
    result_receipts: set[str] = set()
    duplicate_receipts: set[str] = set()
    for event in events:
        if event.get("type") != "tool_call_result" or _params(event).get("tool") != "agent_list":
            continue
        call_id = _params(event).get("call_id")
        if call_id not in starts:
            continue
        if call_id in result_receipts:
            duplicate_receipts.add(call_id)
            continue
        result_receipts.add(call_id)
        result = _tool_result(_json_output(event))
        candidates = result if isinstance(result, list) else [result]
        for candidate in candidates:
            agent_id = _agent_id(candidate)
            if agent_id in agent_ids and isinstance(candidate, dict):
                snapshots[agent_id] = candidate

    statuses = {agent_id: snapshots.get(agent_id, {}).get("status") for agent_id in agent_ids}
    hashes = {
        agent_id: _canonical_hash(snapshots.get(agent_id, {}).get("result"))
        for agent_id in agent_ids
    }
    return {
        "queried_agent_ids": sorted(snapshots),
        "statuses": statuses,
        "result_hashes": hashes,
        "call_ids_unique": not duplicate_receipts,
        "both_agents_requeried": set(snapshots) == set(agent_ids),
        "terminal_states_persisted": not duplicate_receipts
        and statuses == expected_statuses,
        "result_hashes_persisted": not duplicate_receipts
        and hashes == expected_hashes,
    }


async def _daemon_identity(timeout: float = 10.0) -> dict:
    proc = await asyncio.create_subprocess_exec(
        "systemctl",
        "--user",
        "show",
        "aletheon.service",
        "-p",
        "MainPID",
        "-p",
        "ExecMainStartTimestamp",
        stdout=asyncio.subprocess.PIPE,
        stderr=asyncio.subprocess.PIPE,
    )
    out, err = await asyncio.wait_for(proc.communicate(), timeout=timeout)
    if proc.returncode != 0:
        return {"ok": False, "stderr": err.decode()}
    fields = {}
    for line in out.decode().splitlines():
        key, separator, value = line.partition("=")
        if separator:
            fields[key] = value
    try:
        pid = int(fields.get("MainPID", "0"))
        executable = Path(f"/proc/{pid}/exe").resolve(strict=True)
        binary_sha256 = hashlib.sha256(executable.read_bytes()).hexdigest()
    except (OSError, ValueError) as error:
        return {"ok": False, "error": str(error), "fields": fields}
    return {
        "ok": pid > 0 and bool(fields.get("ExecMainStartTimestamp")),
        "main_pid": pid,
        "start_timestamp": fields.get("ExecMainStartTimestamp"),
        "binary": str(executable),
        "binary_sha256": binary_sha256,
    }


def _restart_provenance_valid(before: object, after: object) -> bool:
    return (
        isinstance(before, dict)
        and isinstance(after, dict)
        and before.get("ok") is True
        and after.get("ok") is True
        and before.get("main_pid") != after.get("main_pid")
        and before.get("start_timestamp") != after.get("start_timestamp")
        and before.get("binary_sha256") == after.get("binary_sha256")
        and isinstance(before.get("binary_sha256"), str)
        and len(before["binary_sha256"]) == 64
    )


async def _restart_daemon(timeout: float = 30.0) -> dict:
    before = await _daemon_identity()
    proc = await asyncio.create_subprocess_exec(
        "systemctl", "--user", "restart", "aletheon.service",
        stdout=asyncio.subprocess.PIPE, stderr=asyncio.subprocess.PIPE,
    )
    out, err = await asyncio.wait_for(proc.communicate(), timeout=timeout)
    after = await _daemon_identity() if proc.returncode == 0 else {}
    return {
        "ok": proc.returncode == 0 and _restart_provenance_valid(before, after),
        "command_ok": proc.returncode == 0,
        "stdout": out.decode(),
        "stderr": err.decode(),
        "before": before,
        "after": after,
        "process_changed": before.get("main_pid") != after.get("main_pid"),
        "start_timestamp_changed": before.get("start_timestamp")
        != after.get("start_timestamp"),
        "same_candidate_binary": before.get("binary_sha256")
        == after.get("binary_sha256"),
    }


async def _current_session(client: AletheonClient) -> str | None:
    response = await client.rpc("status")
    value = response.get("result", {}).get("status", {}).get("session_id")
    return value if isinstance(value, str) and value else None


async def run(source_root: str, timeout: float = 180.0) -> dict:
    root = Path(source_root).resolve()
    marker = f"ALETHEON_AGENT_{uuid.uuid4().hex}"
    marker_hash = hashlib.sha256(marker.encode("utf-8")).hexdigest()
    receipt_root = root / ".scenario-runs" / uuid.uuid4().hex
    receipt_root.mkdir(mode=0o700, parents=True)
    started = await tui.tui_start(
        working_dir=str(root),
        cols=110,
        rows=45,
        event_path=str(receipt_root / "initial-events.jsonl"),
    )
    if not started.get("ok"):
        return {"scenario": "subagent_research", "status": "FAIL", "failure": started}

    client = AletheonClient(timeout=15)
    session_id = None
    completed: dict = {}
    try:
        session_id = await _current_session(client)
        prompt = (
            "使用精确的 agent_spawn/agent_list/agent_send/agent_cancel/agent_wait 工具完成验证："
            f"先启动一个有界研究 Agent，任务要求结果原样包含 marker={marker} 和 marker_sha256={marker_hash}；"
            "用 agent_list 查询并确认该 agent_id 的进度，向同一 agent_id 投递 mailbox 消息，"
            "再 agent_wait 到 succeeded 且取得非空 AgentResult。然后启动不同 agent_id 的第二个 Agent 并取消到 cancelled。"
            f"最后必须使用第一个 agent_wait 返回的结构化 AgentResult 形成父上下文的最终结论，并原样写出 {marker} 和 {marker_hash}。"
        )
        sent = await tui.tui_send(prompt, submit=True)
        if sent.get("ok"):
            completed = await tui.tui_wait_turn_done(started.get("turn_done_count", 0), timeout)
        else:
            completed = {"turn_done": False, "error": sent}
    finally:
        await tui.tui_stop()
        await client.close()

    events = base._events(completed.get("event_path"))
    initial_receipt = completed.get("event_evidence")
    lifecycle = _lifecycle_evidence(events, marker, marker_hash)

    restarted = await _restart_daemon()
    recovered: dict = {}
    journal: dict = {}
    recovery_completed: dict = {}
    post_restart = {
        "call_ids_unique": False,
        "both_agents_requeried": False,
        "terminal_states_persisted": False,
        "result_hashes_persisted": False,
    }
    if restarted["ok"] and session_id:
        agent_ids = [
            agent_id
            for agent_id in lifecycle["spawn_agent_ids"]
            if isinstance(agent_id, str)
        ]
        recovery_tui = await tui.tui_start(
            working_dir=str(root),
            cols=110,
            rows=45,
            event_path=str(receipt_root / "post-restart-events.jsonl"),
        )
        try:
            if recovery_tui.get("ok") and len(agent_ids) == 2:
                await tui.tui_send(f"/resume {session_id}", submit=True)
                await tui.tui_capture(
                    wait_stable=True, require_change=False, timeout=10
                )
                query = (
                    "只使用 agent_list（limit=100）重新读取持久化子 Agent；"
                    f"必须在工具结果中核对这两个 agent_id：{agent_ids[0]} 和 {agent_ids[1]}，"
                    "报告各自终态和结构化 result，不要 spawn、wait 或修改它们。"
                )
                sent = await tui.tui_send(query, submit=True)
                if sent.get("ok"):
                    recovery_completed = await tui.tui_wait_turn_done(
                        recovery_tui.get("turn_done_count", 0), timeout
                    )
        finally:
            await tui.tui_stop()

        recovery_events = base._events(recovery_completed.get("event_path"))
        if len(agent_ids) == 2:
            post_restart = _post_restart_agent_evidence(
                recovery_events,
                agent_ids,
                lifecycle["terminal_statuses"],
                lifecycle["terminal_result_hashes"],
            )
        recovery_client = AletheonClient(timeout=20)
        try:
            recovered = await recovery_client.rpc("resume", {"session_id": session_id})
            journal = await recovery_client.rpc("session.journal", {"limit": 500})
        finally:
            await recovery_client.close()

    journal_promoted = _assistant_journal_promotes(
        journal.get("result", {}), marker, marker_hash
    )

    assertions = [
        {"name": "authoritative_turn_done", "passed": completed.get("turn_done") is True},
        {"name": "unique_initial_event_receipt",
         "passed": tui.event_evidence_matches(initial_receipt)},
        {"name": "session_id_recorded", "passed": bool(session_id)},
        {"name": "exact_agent_lifecycle_tools", "passed": not lifecycle["missing_tools"]},
        {"name": "unique_call_ids", "passed": lifecycle["call_ids_unique"]},
        {"name": "tool_results_accounted", "passed": lifecycle["all_calls_completed"]},
        {"name": "two_distinct_spawned_agents", "passed": lifecycle["two_distinct_agents"]},
        {"name": "first_agent_progress_listed", "passed": lifecycle["first_agent_listed"]},
        {"name": "mailbox_delivered_to_first_agent", "passed": lifecycle["mailbox_delivered_to_first"]},
        {"name": "first_agent_terminal_result", "passed": lifecycle["first_agent_succeeded"]},
        {"name": "agent_result_marker_hash", "passed": lifecycle["agent_result_contains_marker_hash"]},
        {"name": "parent_text_promoted_result", "passed": lifecycle["result_promoted_to_parent"]},
        {"name": "parent_journal_promoted_result", "passed": journal_promoted},
        {"name": "second_agent_cancelled", "passed": lifecycle["second_agent_cancelled"]},
        {"name": "daemon_restart_command", "passed": restarted["command_ok"]},
        {"name": "daemon_process_changed", "passed": restarted["process_changed"]},
        {"name": "daemon_start_timestamp_changed", "passed": restarted["start_timestamp_changed"]},
        {"name": "same_candidate_binary", "passed": restarted["same_candidate_binary"]},
        {"name": "unique_post_restart_event_receipt",
         "passed": tui.event_evidence_matches(recovery_completed.get("event_evidence"))},
        {"name": "post_restart_call_ids_unique", "passed": post_restart["call_ids_unique"]},
        {"name": "both_agents_requeried", "passed": post_restart["both_agents_requeried"]},
        {"name": "terminal_states_persisted", "passed": post_restart["terminal_states_persisted"]},
        {"name": "result_hashes_persisted", "passed": post_restart["result_hashes_persisted"]},
        {"name": "same_session_recovered", "passed": recovered.get("result", {}).get("session_id") == session_id},
    ]
    failed = [item["name"] for item in assertions if not item["passed"]]
    status = "PASS" if not failed else "FAIL"
    return {
        "scenario": "subagent_research", "status": status, "assertions": assertions,
        "evidence": {"initial_event": initial_receipt, "event_count": len(events),
                     "post_restart_event": recovery_completed.get("event_evidence"),
                     "session_id": session_id, "marker": marker,
                     "marker_sha256": marker_hash, "lifecycle": lifecycle,
                     "restart": restarted, "post_restart_agents": post_restart,
                     "journal_promoted": journal_promoted,
                     "recovery": recovered.get("result")},
        "failure": None if status == "PASS" else {"failed_assertions": failed},
    }
