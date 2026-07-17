"""SubAgent lifecycle scenario backed only by daemon/session evidence."""
from __future__ import annotations

import asyncio
import json
import shutil
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


def _lifecycle_evidence(events: list[dict]) -> dict:
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
        promotion_evidence = {
            "wait_call_id": wait_call["call_id"],
            "agent_id": first_id,
            "status": terminal_result["status"],
            "agent_result": terminal_result["result"],
        }
    cancel_calls = [call for call in by_tool["agent_cancel"] if args_target(call, second_id)]
    cancelled = any(
        (snapshot := _snapshot_for_agent(call["result"], second_id)) is not None
        and snapshot.get("status") == "cancelled"
        for call in cancel_calls if second_id
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
        "result_promoted_to_parent": promotion_evidence is not None,
        "promotion_evidence": promotion_evidence,
        "second_agent_cancelled": cancelled,
    }


async def _restart_daemon(timeout: float = 30.0) -> dict:
    proc = await asyncio.create_subprocess_exec(
        "systemctl", "--user", "restart", "aletheon.service",
        stdout=asyncio.subprocess.PIPE, stderr=asyncio.subprocess.PIPE,
    )
    out, err = await asyncio.wait_for(proc.communicate(), timeout=timeout)
    return {"ok": proc.returncode == 0, "stdout": out.decode(), "stderr": err.decode()}


async def _current_session(client: AletheonClient) -> str | None:
    response = await client.rpc("status")
    value = response.get("result", {}).get("status", {}).get("session_id")
    return value if isinstance(value, str) and value else None


async def run(source_root: str, timeout: float = 180.0) -> dict:
    root = Path(source_root).resolve()
    receipt_root = root / ".scenario-runs" / uuid.uuid4().hex
    receipt_root.mkdir(parents=True)
    started = await tui.tui_start(working_dir=str(root), cols=110, rows=45)
    if not started.get("ok"):
        return {"scenario": "subagent_research", "status": "FAIL", "failure": started}

    client = AletheonClient(timeout=15)
    session_id = None
    completed: dict = {}
    try:
        session_id = await _current_session(client)
        prompt = (
            "使用精确的 agent_spawn/agent_list/agent_send/agent_cancel/agent_wait 工具完成验证："
            "先启动一个有界研究 Agent；用 agent_list 查询并确认该 agent_id 的进度，向同一 agent_id 投递 mailbox 消息，"
            "再 agent_wait 到 succeeded 且取得非空 AgentResult。然后启动不同 agent_id 的第二个 Agent 并取消到 cancelled。"
            "最后必须使用第一个 agent_wait 返回的结构化 AgentResult 形成父上下文的最终结论。"
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
    durable_path = receipt_root / "agent-events.jsonl"
    if completed.get("event_path") and Path(completed["event_path"]).is_file():
        shutil.copyfile(completed["event_path"], durable_path)
    lifecycle = _lifecycle_evidence(events)

    restarted = await _restart_daemon()
    recovered: dict = {}
    if restarted["ok"] and session_id:
        recovery_client = AletheonClient(timeout=20)
        try:
            recovered = await recovery_client.rpc("resume", {"session_id": session_id})
        finally:
            await recovery_client.close()

    assertions = [
        {"name": "authoritative_turn_done", "passed": completed.get("turn_done") is True},
        {"name": "session_id_recorded", "passed": bool(session_id)},
        {"name": "exact_agent_lifecycle_tools", "passed": not lifecycle["missing_tools"]},
        {"name": "unique_call_ids", "passed": lifecycle["call_ids_unique"]},
        {"name": "tool_results_accounted", "passed": lifecycle["all_calls_completed"]},
        {"name": "two_distinct_spawned_agents", "passed": lifecycle["two_distinct_agents"]},
        {"name": "first_agent_progress_listed", "passed": lifecycle["first_agent_listed"]},
        {"name": "mailbox_delivered_to_first_agent", "passed": lifecycle["mailbox_delivered_to_first"]},
        {"name": "first_agent_terminal_result", "passed": lifecycle["first_agent_succeeded"]},
        {"name": "result_promoted_to_parent", "passed": lifecycle["result_promoted_to_parent"]},
        {"name": "second_agent_cancelled", "passed": lifecycle["second_agent_cancelled"]},
        {"name": "daemon_restarted", "passed": restarted["ok"]},
        {"name": "same_session_recovered", "passed": recovered.get("result", {}).get("session_id") == session_id},
    ]
    failed = [item["name"] for item in assertions if not item["passed"]]
    status = "PASS" if not failed else "FAIL"
    return {
        "scenario": "subagent_research", "status": status, "assertions": assertions,
        "evidence": {"event_path": str(durable_path), "event_count": len(events),
                     "session_id": session_id, "lifecycle": lifecycle,
                     "restart": restarted, "recovery": recovered.get("result")},
        "failure": None if status == "PASS" else {"failed_assertions": failed},
    }
