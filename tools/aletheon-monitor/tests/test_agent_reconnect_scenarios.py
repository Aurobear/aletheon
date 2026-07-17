import json
import hashlib

from scenarios import reconnect_resume, subagent_research


MARKER = "ALETHEON_AGENT_test"
MARKER_HASH = hashlib.sha256(MARKER.encode()).hexdigest()


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


def agent_result(output=f"42 crates {MARKER} {MARKER_HASH}"):
    return {"output": output,
            "usage": {"input_tokens": 10, "output_tokens": 4,
                      "cost_usd": None, "elapsed_ms": 25},
            "evidence": [], "artifacts": []}


def valid_agent_events(*, promoted=True):
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
    if promoted:
        events += [event("text_delta", text=f"parent conclusion {MARKER} {MARKER_HASH}")]
    return events


def test_subagent_requires_exact_structured_lifecycle_and_result_promotion():
    evidence = subagent_research._lifecycle_evidence(
        valid_agent_events(), MARKER, MARKER_HASH
    )
    assert evidence["missing_tools"] == []
    assert evidence["call_ids_unique"] is True
    assert evidence["all_calls_completed"] is True
    assert evidence["spawn_agent_ids"] == ["agent-first", "agent-second"]
    assert evidence["two_distinct_agents"] is True
    assert evidence["first_agent_listed"] is True
    assert evidence["mailbox_delivered_to_first"] is True
    assert evidence["first_agent_succeeded"] is True
    assert evidence["agent_result_contains_marker_hash"] is True
    assert evidence["parent_text_contains_marker_hash"] is True
    assert evidence["result_promoted_to_parent"] is True
    assert evidence["promotion_evidence"] == {
        "wait_call_id": "wait-1", "agent_id": "agent-first", "status": "succeeded",
        "agent_result": agent_result(),
        "result_sha256": subagent_research._canonical_hash(agent_result()),
        "marker_in_agent_result": True,
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
    evidence = subagent_research._lifecycle_evidence(events, MARKER, MARKER_HASH)
    assert evidence["mailbox_delivered_to_first"] is True


def test_subagent_does_not_accept_prompt_or_prose_as_evidence():
    events = [event("text_delta", text="spawn progress mailbox cancel succeeded 42 crates")]
    evidence = subagent_research._lifecycle_evidence(events, MARKER, MARKER_HASH)
    assert evidence["missing_tools"]
    assert evidence["result_promoted_to_parent"] is False


def test_subagent_requires_completed_call_ids():
    events = [event("tool_call_start", call_id="missing", tool="agent_spawn", args={}),
              result("other", "agent_spawn", {"ok": True, "result": {"agent_id": "x"}})]
    assert subagent_research._lifecycle_evidence(
        events, MARKER, MARKER_HASH
    )["all_calls_completed"] is False


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
    evidence = subagent_research._lifecycle_evidence(events, MARKER, MARKER_HASH)
    assert evidence["mailbox_delivered_to_first"] is False
    assert evidence["first_agent_succeeded"] is False
    assert evidence["result_promoted_to_parent"] is False
    assert evidence["second_agent_cancelled"] is False


def test_subagent_wait_result_alone_is_not_parent_promotion():
    evidence = subagent_research._lifecycle_evidence(
        valid_agent_events(promoted=False), MARKER, MARKER_HASH
    )
    assert evidence["first_agent_succeeded"] is True
    assert evidence["agent_result_contains_marker_hash"] is True
    assert evidence["parent_text_contains_marker_hash"] is False
    assert evidence["result_promoted_to_parent"] is False


def test_subagent_requires_assistant_journal_marker_not_tool_result_marker():
    tool_only = {"entries": [{"event_type": "tool_result_block",
                              "event": {"content": f"{MARKER} {MARKER_HASH}"}}]}
    promoted = {"entries": [*tool_only["entries"],
                             {"event_type": "assistant_message",
                              "event": {"content": f"final {MARKER} {MARKER_HASH}"}}]}
    assert subagent_research._assistant_journal_promotes(
        tool_only, MARKER, MARKER_HASH
    ) is False
    assert subagent_research._assistant_journal_promotes(
        promoted, MARKER, MARKER_HASH
    ) is True


def test_subagent_requeries_both_persisted_terminal_states_and_result_hashes():
    first_result = agent_result()
    events = call(
        "list-after", "agent_list", {"limit": 100},
        [snapshot("agent-first", "succeeded", first_result),
         snapshot("agent-second", "cancelled")],
    )
    evidence = subagent_research._post_restart_agent_evidence(
        events,
        ["agent-first", "agent-second"],
        {"agent-first": "succeeded", "agent-second": "cancelled"},
        {"agent-first": subagent_research._canonical_hash(first_result),
         "agent-second": None},
    )
    assert evidence["call_ids_unique"] is True
    assert evidence["both_agents_requeried"] is True
    assert evidence["terminal_states_persisted"] is True
    assert evidence["result_hashes_persisted"] is True


def test_subagent_restart_requires_new_process_and_same_candidate_binary():
    before = {"ok": True, "main_pid": 10, "start_timestamp": "before",
              "binary_sha256": "a" * 64}
    after = {"ok": True, "main_pid": 11, "start_timestamp": "after",
             "binary_sha256": "a" * 64}
    assert subagent_research._restart_provenance_valid(before, after) is True
    assert subagent_research._restart_provenance_valid(before, {**after, "main_pid": 10}) is False
    assert subagent_research._restart_provenance_valid(
        before, {**after, "binary_sha256": "b" * 64}
    ) is False


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
