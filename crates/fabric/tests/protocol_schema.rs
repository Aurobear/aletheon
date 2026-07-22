use fabric::protocol::client::{
    client_schema, negotiate_protocol_version, ApprovalDecisionRequest, ApprovalRequest,
    CancelRequest, ChatRequest, ClientCapabilities, ClientEvent, ClientMessage, ClientRequest,
    ClientRpcRequest, EventCursor, EventSubscription, InitializeParams, InitializedResult,
    SnapshotRequest, TransientApprovalDecision, TurnCompletionError, TurnCompletionUsage,
    CLIENT_PROTOCOL_VERSION,
};
use fabric::{
    ConnectionId, LocalOsPrincipal, OperationId, PrincipalId, SessionId, ThreadId, TurnId,
    TurnStop, TurnTerminalStatus,
};

#[test]
fn schema_is_versioned_and_exposes_snapshot_reconnect_and_incremental_subscription() {
    let schema = client_schema();
    let encoded = serde_json::to_string(&schema).unwrap();
    for contract in [
        "protocol_version",
        "initialize",
        "initialize_response",
        "initialized",
        "snapshot",
        "reconnected",
        "subscribe",
        "after",
    ] {
        assert!(encoded.contains(contract), "schema missing {contract}");
    }
}

#[test]
fn unknown_versions_fail_and_optional_forward_fields_are_retained() {
    let value = serde_json::json!({
        "protocol_version": 9,
        "payload": {"type":"failed", "data":{"message":"nope"}},
        "future_trace": {"enabled": true}
    });
    let decoded: ClientMessage<ClientEvent> = serde_json::from_value(value).unwrap();
    assert_eq!(decoded.extensions["future_trace"]["enabled"], true);
    assert_eq!(decoded.into_v1().unwrap_err().actual, 9);
}

#[test]
fn typed_requests_round_trip_at_the_supported_version() {
    let session_id = SessionId("session-1".into());
    for request in [
        ClientRequest::Snapshot(SnapshotRequest {
            session_id: session_id.clone(),
        }),
        ClientRequest::Subscribe(EventSubscription {
            session_id,
            after: EventCursor {
                sequence: 41,
                event_id: Some("event-41".into()),
            },
        }),
    ] {
        let wire = ClientMessage::v1(request.clone());
        assert_eq!(wire.protocol_version, CLIENT_PROTOCOL_VERSION);
        let json = serde_json::to_value(&wire).unwrap();
        let decoded: ClientMessage<ClientRequest> = serde_json::from_value(json).unwrap();
        assert_eq!(decoded.into_v1().unwrap(), request);
    }
}

#[test]
fn versioned_mutations_round_trip_explicit_identity_tuples() {
    let thread_id = ThreadId("thread-explicit".into());
    let turn_id = TurnId::new();
    let operation_id = OperationId::new();
    for request in vec![
        ClientRequest::Chat(ChatRequest {
            thread_id: thread_id.clone(),
            message: "hello".into(),
            working_dir: "/tmp".into(),
            additional_writable_roots: vec![],
        }),
        ClientRequest::Approval(ApprovalRequest {
            thread_id: thread_id.clone(),
            turn_id,
            operation_id,
            approval_id: fabric::ApprovalId::new(),
            version: 1,
            decision: ApprovalDecisionRequest::Approve,
            reason: None,
        }),
        ClientRequest::Cancel(CancelRequest {
            thread_id,
            turn_id,
            operation_id,
        }),
    ] {
        let encoded = request.to_json_rpc(7).unwrap();
        let decoded: ClientMessage<ClientRequest> =
            serde_json::from_value(encoded["params"].clone()).unwrap();
        assert_eq!(decoded.into_v1().unwrap(), request);
    }
}

#[test]
fn initialize_has_version_and_capabilities_but_no_uid() {
    let value = serde_json::to_value(ClientRequest::Initialize(InitializeParams {
        client_version: "0.1.0".into(),
        protocol_versions: vec![1],
        capabilities: ClientCapabilities {
            item_events: true,
            cursors: true,
        },
    }))
    .unwrap();
    assert_eq!(value["type"], "initialize");
    assert_eq!(value["data"]["protocol_versions"], serde_json::json!([1]));
    assert!(!value.to_string().contains("uid"));
}

#[test]
fn initialized_is_a_distinct_client_message() {
    assert_eq!(
        serde_json::to_value(ClientRequest::Initialized).unwrap()["type"],
        "initialized"
    );
}

#[test]
fn initialize_response_contains_only_the_effective_server_identity() {
    let value = serde_json::to_value(ClientEvent::InitializeResponse(InitializedResult {
        protocol_version: CLIENT_PROTOCOL_VERSION,
        server_capabilities: ClientCapabilities {
            item_events: true,
            cursors: true,
        },
        connection_id: ConnectionId::new(),
        principal_id: PrincipalId::local_uid(1001),
        os_principal: LocalOsPrincipal {
            uid: 1001,
            gid: 1001,
        },
        runtime_version: "0.1.0".into(),
    }))
    .unwrap();
    assert_eq!(value["type"], "initialize_response");
    assert_eq!(value["data"]["protocol_version"], CLIENT_PROTOCOL_VERSION);
    assert_eq!(value["data"]["principal_id"], "local-uid:1001");
    assert_eq!(value["data"]["os_principal"]["uid"], 1001);
}

#[test]
fn protocol_negotiation_selects_the_highest_shared_version() {
    assert_eq!(negotiate_protocol_version(&[0, 1, 2]).unwrap(), 1);
    assert_eq!(
        negotiate_protocol_version(&[]).unwrap_err().expected,
        CLIENT_PROTOCOL_VERSION
    );
    assert_eq!(negotiate_protocol_version(&[2]).unwrap_err().actual, 2);
}

#[test]
fn internal_stops_map_to_one_external_terminal_status() {
    assert_eq!(
        TurnTerminalStatus::from(TurnStop::Completed),
        TurnTerminalStatus::Completed
    );
    assert_eq!(
        TurnTerminalStatus::from(TurnStop::Cancelled),
        TurnTerminalStatus::Interrupted
    );
    assert_eq!(
        TurnTerminalStatus::from(TurnStop::Blocked),
        TurnTerminalStatus::Failed
    );
    assert_eq!(
        TurnTerminalStatus::from(TurnStop::Failed),
        TurnTerminalStatus::Failed
    );
}

#[test]
fn rich_turn_terminal_round_trips_structured_outcome() {
    let event = ClientEvent::TurnCompleted {
        thread_id: ThreadId("thread-1".into()),
        turn_id: TurnId::new(),
        operation_id: OperationId::new(),
        stop: TurnStop::Failed,
        status: Some(TurnTerminalStatus::Failed),
        error: Some(TurnCompletionError {
            code: Some("provider_timeout".into()),
            message: "provider timed out".into(),
        }),
        retryable: true,
        usage: TurnCompletionUsage {
            input_tokens: 10,
            output_tokens: 4,
            tool_calls: 2,
            elapsed_ms: 250,
        },
    };
    let wire = serde_json::to_value(&event).unwrap();
    let decoded: ClientEvent = serde_json::from_value(wire).unwrap();
    assert_eq!(decoded, event);
}

#[test]
fn current_client_decodes_legacy_turn_terminal_without_additive_fields() {
    let event = ClientEvent::TurnCompleted {
        thread_id: ThreadId("thread-legacy".into()),
        turn_id: TurnId::new(),
        operation_id: OperationId::new(),
        stop: TurnStop::Completed,
        status: Some(TurnTerminalStatus::Completed),
        error: None,
        retryable: false,
        usage: TurnCompletionUsage::default(),
    };
    let mut wire = serde_json::to_value(event).unwrap();
    let data = wire["data"].as_object_mut().unwrap();
    data.remove("status");
    data.remove("error");
    data.remove("retryable");
    data.remove("usage");

    let decoded: ClientEvent = serde_json::from_value(wire).unwrap();
    assert!(matches!(
        decoded,
        ClientEvent::TurnCompleted {
            status: None,
            error: None,
            retryable: false,
            usage: TurnCompletionUsage {
                input_tokens: 0,
                output_tokens: 0,
                tool_calls: 0,
                elapsed_ms: 0,
            },
            ..
        }
    ));
}

#[test]
fn daemon_compatibility_requests_own_method_and_parameter_names() {
    let workspace = fabric::WorkspacePolicy::from_resolved_roots(
        "/tmp/project".into(),
        vec!["/tmp/shared".into()],
    )
    .unwrap();
    let chat = ClientRpcRequest::chat("hello", &workspace)
        .to_json_rpc(Some(7))
        .unwrap();
    assert_eq!(chat["jsonrpc"], "2.0");
    assert_eq!(chat["id"], 7);
    assert_eq!(chat["method"], "chat");
    assert_eq!(chat["params"]["message"], "hello");
    assert_eq!(chat["params"]["working_dir"], "/tmp/project");

    let resume = ClientRpcRequest::resume("session-1")
        .to_json_rpc(Some(8))
        .unwrap();
    assert_eq!(resume["method"], "resume");
    assert_eq!(resume["params"]["session_id"], "session-1");

    let approval = ClientRpcRequest::approval_response(
        "approval-1",
        TransientApprovalDecision::ApproveForSession,
    )
    .to_json_rpc(None)
    .unwrap();
    assert!(approval["id"].is_null());
    assert_eq!(approval["method"], "approval_response");
    assert_eq!(approval["params"]["decision"], "approve_for_session");

    for (request, expected_method) in [
        (ClientRpcRequest::Clear, "clear"),
        (ClientRpcRequest::Status, "status"),
        (ClientRpcRequest::Reflect, "reflect"),
        (ClientRpcRequest::ReflectNow, "reflect_now"),
        (ClientRpcRequest::Evolution, "evolution"),
        (ClientRpcRequest::Genome, "genome"),
        (ClientRpcRequest::Sessions, "sessions"),
        (ClientRpcRequest::Compact, "compact"),
        (ClientRpcRequest::ModelList, "model_list"),
        (ClientRpcRequest::PlanApprove, "plan_approve"),
        (ClientRpcRequest::Cancel, "cancel"),
        (ClientRpcRequest::HooksList, "hooks_list"),
    ] {
        let request = request.to_json_rpc(Some(1)).unwrap();
        assert_eq!(request["method"], expected_method);
        assert!(request.get("params").is_none());
    }

    let profile_list = ClientRpcRequest::AgentProfileList
        .to_json_rpc(Some(1))
        .unwrap();
    assert_eq!(profile_list["method"], "agent.profile.list");
    assert!(profile_list.get("params").is_none());

    let profile_set = ClientRpcRequest::agent_profile_set("reviewer")
        .to_json_rpc(Some(2))
        .unwrap();
    assert_eq!(profile_set["method"], "agent.profile.set");
    assert_eq!(profile_set["params"]["profile"], "reviewer");

    let mode = ClientRpcRequest::mode_switch(fabric::ui_event::CollaborationMode::Plan)
        .to_json_rpc(Some(1))
        .unwrap();
    assert_eq!(mode["params"]["mode"], "plan");

    let interrupt = ClientRpcRequest::interrupt(fabric::ui_event::InterruptReason::UserCancelled)
        .to_json_rpc(Some(1))
        .unwrap();
    assert_eq!(interrupt["params"]["reason"], "user_cancelled");
}

#[test]
fn memory_goal_and_workflow_requests_preserve_legacy_wire_fields() {
    let add = ClientRpcRequest::memory_add("remember", "project", "architecture")
        .to_json_rpc(Some(1))
        .unwrap();
    assert_eq!(add["method"], "memory.add");
    assert_eq!(add["params"]["content"], "remember");
    assert_eq!(add["params"]["scope"], "project");
    assert_eq!(add["params"]["subject"], "architecture");

    let list = ClientRpcRequest::memory_list(None, true)
        .to_json_rpc(Some(1))
        .unwrap();
    assert_eq!(list["method"], "memory.list");
    assert!(list["params"]["scope"].is_null());
    assert_eq!(list["params"]["all"], true);

    let search = ClientRpcRequest::memory_search("typed", Some("session".into()))
        .to_json_rpc(Some(1))
        .unwrap();
    assert_eq!(search["method"], "memory.search");
    assert_eq!(search["params"]["query"], "typed");
    assert_eq!(search["params"]["scope"], "session");

    for (request, method) in [
        (ClientRpcRequest::memory_show(7), "memory.show"),
        (ClientRpcRequest::memory_pin(7), "memory.pin"),
        (ClientRpcRequest::memory_unpin(7), "memory.unpin"),
        (ClientRpcRequest::goal_show(7), "goal.show"),
    ] {
        let wire = request.to_json_rpc(Some(1)).unwrap();
        assert_eq!(wire["method"], method);
        assert_eq!(wire["params"]["id"], 7);
    }

    let forget = ClientRpcRequest::memory_forget(8, true)
        .to_json_rpc(Some(1))
        .unwrap();
    assert_eq!(forget["method"], "memory.forget");
    assert_eq!(forget["params"]["id"], 8);
    assert_eq!(forget["params"]["hard"], true);

    let goal_set = ClientRpcRequest::goal_set("ship", "session")
        .to_json_rpc(Some(1))
        .unwrap();
    assert_eq!(goal_set["method"], "goal.set");
    assert_eq!(goal_set["params"]["description"], "ship");
    assert_eq!(goal_set["params"]["scope"], "session");

    let goal_status =
        ClientRpcRequest::goal_status(Some(9), Some("completed".into()), Some("active".into()))
            .to_json_rpc(Some(1))
            .unwrap();
    assert_eq!(goal_status["method"], "goal.status");
    assert_eq!(goal_status["params"]["id"], 9);
    assert_eq!(goal_status["params"]["status"], "completed");
    assert_eq!(goal_status["params"]["filter"], "active");

    let workflow_save =
        ClientRpcRequest::workflow_save("release", serde_json::json!({"steps": ["check", "ship"]}))
            .to_json_rpc(Some(1))
            .unwrap();
    assert_eq!(workflow_save["method"], "workflow.save");
    assert_eq!(workflow_save["params"]["name"], "release");
    assert_eq!(workflow_save["params"]["def"]["steps"][0], "check");

    for (request, method) in [
        (ClientRpcRequest::workflow_load("release"), "workflow.load"),
        (
            ClientRpcRequest::workflow_delete("release"),
            "workflow.delete",
        ),
        (ClientRpcRequest::workflow_run("release"), "workflow.run"),
    ] {
        let wire = request.to_json_rpc(Some(1)).unwrap();
        assert_eq!(wire["method"], method);
        assert_eq!(wire["params"]["name"], "release");
    }

    for request in [
        ClientRpcRequest::GoalResume,
        ClientRpcRequest::WorkflowList,
        ClientRpcRequest::DaemonShutdown,
    ] {
        let wire = request.to_json_rpc(Some(1)).unwrap();
        assert_eq!(wire["params"], serde_json::json!({}));
    }
}

#[test]
fn debug_requests_preserve_methods_and_typed_parameter_shapes() {
    for (request, method) in [
        (ClientRpcRequest::DebugTopics, "debug.topics"),
        (ClientRpcRequest::DebugNodeInfo, "debug.node_info"),
        (ClientRpcRequest::DebugPerf, "debug.perf"),
        (ClientRpcRequest::DebugTraceStop, "debug.trace_stop"),
        (ClientRpcRequest::DebugTraceStatus, "debug.trace_status"),
        (ClientRpcRequest::DebugHealth, "debug.health"),
        (ClientRpcRequest::DebugNodes, "debug.nodes"),
        (ClientRpcRequest::DebugParamList, "debug.param_list"),
        (ClientRpcRequest::DebugTopology, "debug.topology"),
        (ClientRpcRequest::DebugGraph, "debug.graph"),
    ] {
        let wire = request.to_json_rpc(Some(1)).unwrap();
        assert_eq!(wire["jsonrpc"], "2.0");
        assert_eq!(wire["method"], method);
        assert_eq!(wire["params"], serde_json::json!({}));
    }

    let subscribe =
        ClientRpcRequest::debug_subscribe(Some("info".into()), Some("runtime".into()), None)
            .to_json_rpc(Some(1))
            .unwrap();
    assert_eq!(subscribe["method"], "debug.subscribe");
    assert_eq!(subscribe["params"]["level"], "info");
    assert_eq!(subscribe["params"]["module"], "runtime");
    assert!(subscribe["params"].get("tracepoint").is_none());

    let hz = ClientRpcRequest::debug_subscribe(None, None, Some("turn.done".into()))
        .to_json_rpc(Some(1))
        .unwrap();
    assert_eq!(hz["params"], serde_json::json!({"tracepoint":"turn.done"}));

    let start = ClientRpcRequest::debug_bag_start(
        "debug",
        Some("/tmp/trace.bag".into()),
        Some("executive".into()),
    )
    .to_json_rpc(Some(1))
    .unwrap();
    assert_eq!(start["method"], "debug.bag_start");
    assert_eq!(start["params"]["level"], "debug");
    assert_eq!(start["params"]["path"], "/tmp/trace.bag");
    assert_eq!(start["params"]["module"], "executive");

    let stop = ClientRpcRequest::debug_bag_stop("recording-1")
        .to_json_rpc(Some(2))
        .unwrap();
    assert_eq!(stop["id"], 2);
    assert_eq!(stop["method"], "debug.bag_stop");
    assert_eq!(stop["params"]["recording_id"], "recording-1");

    let replay = ClientRpcRequest::debug_bag_replay("/tmp/trace.bag", 2.5)
        .to_json_rpc(Some(1))
        .unwrap();
    assert_eq!(replay["method"], "debug.bag_replay");
    assert_eq!(replay["params"]["path"], "/tmp/trace.bag");
    assert_eq!(replay["params"]["speed"], 2.5);

    let trace = ClientRpcRequest::debug_trace_start("trace", None)
        .to_json_rpc(Some(1))
        .unwrap();
    assert_eq!(trace["method"], "debug.trace_start");
    assert_eq!(trace["params"], serde_json::json!({"level":"trace"}));

    let param = ClientRpcRequest::debug_param_get("agent.max_iterations")
        .to_json_rpc(Some(1))
        .unwrap();
    assert_eq!(param["method"], "debug.param_get");
    assert_eq!(param["params"]["key"], "agent.max_iterations");

    let logs = ClientRpcRequest::debug_log_subscribe("warn", Some("kernel".into()))
        .to_json_rpc(Some(1))
        .unwrap();
    assert_eq!(logs["method"], "debug.log_subscribe");
    assert_eq!(logs["params"]["level"], "warn");
    assert_eq!(logs["params"]["module"], "kernel");
}
