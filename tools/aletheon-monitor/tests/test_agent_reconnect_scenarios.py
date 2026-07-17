import json

from scenarios import reconnect_resume, subagent_research


def event(kind, **params):
    return {"type": kind, "params": {"type": kind, **params}}


def result(call_id, tool, value, *, error=False):
    return event("tool_call_result", call_id=call_id, tool=tool, output=json.dumps(value),
                 is_error=error, elapsed_ms=1)


def call(call_id, tool, args, value):
    return [event("tool_call_start", call_id=call_id, tool=tool, args=args),
            result(call_id, tool, {"ok": True, "result": value})]


def snapshot(agent_id, status, agent_result=None):
    return {"handle": {"agent_id": agent_id}, "status": status, "result": agent_result}


def agent_result(output="42 crates"):
    return {"output": output,
            "usage": {"input_tokens": 10, "output_tokens": 4,
                      "cost_usd": None, "elapsed_ms": 25},
            "evidence": [], "artifacts": []}


def valid_agent_events():
    first, second = "agent-first", "agent-second"
    events = []
    events += call("spawn-1", "agent_spawn", {"task": "research"}, {"agent_id": first})
    events += call("list-1", "agent_list", {"limit": 10}, [snapshot(first, "running")])
    events += call("send-1", "agent_send", {"agent_id": first, "message": "progress?"},
                   {"to": first, "delivery": "delivered"})
    events += call("wait-1", "agent_wait", {"agent_id": first, "timeout_ms": 1000},
                   snapshot(first, "succeeded", agent_result()))
    events += call("spawn-2", "agent_spawn", {"task": "cancel me"}, {"agent_id": second})
    events += call("cancel-2", "agent_cancel", {"agent_id": second},
                   snapshot(second, "cancelled"))
    return events


def test_subagent_requires_exact_structured_lifecycle_and_result_promotion():
    evidence = subagent_research._lifecycle_evidence(valid_agent_events())
    assert evidence["missing_tools"] == []
    assert evidence["call_ids_unique"] is True
    assert evidence["all_calls_completed"] is True
    assert evidence["spawn_agent_ids"] == ["agent-first", "agent-second"]
    assert evidence["two_distinct_agents"] is True
    assert evidence["first_agent_listed"] is True
    assert evidence["mailbox_delivered_to_first"] is True
    assert evidence["first_agent_succeeded"] is True
    assert evidence["result_promoted_to_parent"] is True
    assert evidence["promotion_evidence"] == {
        "wait_call_id": "wait-1", "agent_id": "agent-first", "status": "succeeded",
        "agent_result": agent_result(),
    }
    assert evidence["second_agent_cancelled"] is True


def test_subagent_uses_completed_args_and_exact_call_id_pairing():
    events = valid_agent_events()
    start = next(item for item in events if item["params"].get("call_id") == "send-1"
                 and item["type"] == "tool_call_start")
    start["params"]["args"] = {}
    events.insert(events.index(start) + 1, event(
        "tool_call_complete", call_id="send-1", tool="agent_send",
        args={"agent_id": "agent-first", "message": "progress?"},
    ))
    evidence = subagent_research._lifecycle_evidence(events)
    assert evidence["mailbox_delivered_to_first"] is True


def test_subagent_does_not_accept_prompt_or_prose_as_evidence():
    events = [event("text_delta", text="spawn progress mailbox cancel succeeded 42 crates")]
    evidence = subagent_research._lifecycle_evidence(events)
    assert evidence["missing_tools"]
    assert evidence["result_promoted_to_parent"] is False


def test_subagent_requires_completed_call_ids():
    events = [event("tool_call_start", call_id="missing", tool="agent_spawn", args={}),
              result("other", "agent_spawn", {"ok": True, "result": {"agent_id": "x"}})]
    assert subagent_research._lifecycle_evidence(events)["all_calls_completed"] is False


def test_subagent_rejects_wrong_agent_relationships_and_terminal_shapes():
    events = valid_agent_events()
    for item in events:
        params = item["params"]
        if item["type"] == "tool_call_start" and params.get("call_id") == "send-1":
            params["args"]["agent_id"] = "agent-second"
        if item["type"] == "tool_call_result" and params.get("call_id") == "wait-1":
            params["output"] = json.dumps({"ok": True, "result": snapshot(
                "agent-first", "succeeded", agent_result(""))})
        if item["type"] == "tool_call_result" and params.get("call_id") == "cancel-2":
            params["output"] = json.dumps({"ok": True, "result": snapshot(
                "agent-second", "running")})
    evidence = subagent_research._lifecycle_evidence(events)
    assert evidence["mailbox_delivered_to_first"] is False
    assert evidence["first_agent_succeeded"] is False
    assert evidence["result_promoted_to_parent"] is False
    assert evidence["second_agent_cancelled"] is False


def test_reconnect_reconstructs_only_last_turn_text():
    events = [event("turn_started", iteration=1), event("text_delta", text="old"),
              event("turn_done"), event("turn_started", iteration=2),
              event("text_delta", text="new\n"), event("text_delta", text="FINAL")]
    assert reconnect_resume._final_text(events) == "new\nFINAL"


def test_reconnect_persistence_search_is_structural():
    journal = {"entries": [{"event_type": "assistant_message",
                            "event": {"content": "answer MARKER"}}]}
    assert reconnect_resume._contains(journal, "MARKER") is True
    assert reconnect_resume._contains(journal, "missing") is False
