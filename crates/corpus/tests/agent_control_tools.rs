use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use corpus::tools::tools::agent_control::AgentControlTools;
use fabric::tool::{Tool, ToolContext};
use fabric::{
    AgentControlError, AgentControlMessage, AgentControlPort, AgentHandle, AgentId,
    AgentListRequest, AgentProfileId, AgentResult, AgentRunStatus, AgentSendRequest, AgentSnapshot,
    AgentSpawnRequest, AgentToolContext, AgentWaitRequest, AttemptUsage, OperationId, ProcessId,
    RuntimeId,
};

#[derive(Default)]
struct Calls {
    spawn: Vec<AgentSpawnRequest>,
    wait: Vec<AgentWaitRequest>,
    send: Vec<AgentSendRequest>,
    cancel: Vec<(AgentId, AgentId)>,
    list: Vec<AgentListRequest>,
}

#[derive(Default)]
struct FakeControl(Mutex<Calls>);

fn handle(request: &AgentSpawnRequest) -> AgentHandle {
    AgentHandle {
        agent_id: AgentId::new(),
        root_agent_id: request.root_agent_id,
        parent_agent_id: request.parent_agent_id,
        process_id: ProcessId::new(),
        operation_id: OperationId::new(),
        runtime_id: request.runtime_id.clone(),
        profile_id: request.profile_id.clone(),
    }
}

fn snapshot(root: AgentId, agent: AgentId, status: AgentRunStatus) -> AgentSnapshot {
    AgentSnapshot {
        handle: AgentHandle {
            agent_id: agent,
            root_agent_id: root,
            parent_agent_id: None,
            process_id: ProcessId::new(),
            operation_id: OperationId::new(),
            runtime_id: RuntimeId("native-cognit".into()),
            profile_id: AgentProfileId("worker".into()),
        },
        status,
        result: Some(AgentResult {
            output: "done".into(),
            usage: AttemptUsage::default(),
            evidence: vec![],
            artifacts: vec![],
        }),
        created_at_ms: 1,
        started_at_ms: Some(2),
        ended_at_ms: Some(3),
        last_error: None,
    }
}

#[async_trait]
impl AgentControlPort for FakeControl {
    async fn spawn(&self, request: AgentSpawnRequest) -> Result<AgentHandle, AgentControlError> {
        let value = handle(&request);
        self.0.lock().unwrap().spawn.push(request);
        Ok(value)
    }

    async fn wait(&self, request: AgentWaitRequest) -> Result<AgentSnapshot, AgentControlError> {
        self.0.lock().unwrap().wait.push(request.clone());
        Ok(snapshot(
            request.caller_root_agent_id,
            request.agent_id,
            AgentRunStatus::Succeeded,
        ))
    }

    async fn send(
        &self,
        request: AgentSendRequest,
    ) -> Result<AgentControlMessage, AgentControlError> {
        self.0.lock().unwrap().send.push(request.clone());
        Ok(AgentControlMessage {
            delivery_id: request.delivery_id.unwrap_or_else(uuid::Uuid::new_v4),
            sequence: 7,
            from: request
                .sender_agent_id
                .unwrap_or(request.caller_root_agent_id),
            to: request.agent_id,
            kind: request.kind,
            delivery: fabric::AgentMessageDeliveryState::Delivered,
            content: request.message,
        })
    }

    async fn cancel(
        &self,
        caller_root_agent_id: AgentId,
        agent_id: AgentId,
    ) -> Result<AgentSnapshot, AgentControlError> {
        self.0
            .lock()
            .unwrap()
            .cancel
            .push((caller_root_agent_id, agent_id));
        Ok(snapshot(
            caller_root_agent_id,
            agent_id,
            AgentRunStatus::Cancelled,
        ))
    }

    async fn inspect(
        &self,
        caller_root_agent_id: AgentId,
        agent_id: AgentId,
    ) -> Result<AgentSnapshot, AgentControlError> {
        Ok(snapshot(
            caller_root_agent_id,
            agent_id,
            AgentRunStatus::Running,
        ))
    }

    async fn list(
        &self,
        request: AgentListRequest,
    ) -> Result<Vec<AgentSnapshot>, AgentControlError> {
        self.0.lock().unwrap().list.push(request.clone());
        Ok(vec![])
    }
}

fn context(root: AgentId, parent: AgentId, process: ProcessId) -> ToolContext {
    ToolContext {
        approval_authority: None,
        agent: Some(AgentToolContext {
            caller_root_agent_id: root,
            parent_agent_id: parent,
            parent_process_id: process,
        }),
        working_dir: std::env::temp_dir(),
        session_id: "agent-control-test".into(),
        clock: Arc::new(aletheon_kernel::chronos::TestClock::default()),
        turn_event_sender: None,
    }
}

fn tools(control: Arc<FakeControl>) -> Vec<Arc<dyn Tool>> {
    AgentControlTools::new(control).tools()
}

fn find<'a>(tools: &'a [Arc<dyn Tool>], name: &str) -> &'a Arc<dyn Tool> {
    tools.iter().find(|tool| tool.name() == name).unwrap()
}

#[test]
fn exposes_five_exact_bounded_schemas() {
    let tools = tools(Arc::new(FakeControl::default()));
    let mut names = tools
        .iter()
        .map(|tool| tool.name().to_string())
        .collect::<Vec<_>>();
    names.sort();
    assert_eq!(
        names,
        [
            "agent_cancel",
            "agent_list",
            "agent_send",
            "agent_spawn",
            "agent_wait"
        ]
    );
    for tool in tools {
        let schema = tool.input_schema();
        assert_eq!(schema["additionalProperties"], false);
    }
}

#[tokio::test]
async fn spawn_injects_trusted_parent_and_rejects_forged_identity() {
    let control = Arc::new(FakeControl::default());
    let tools = tools(control.clone());
    let root = AgentId::new();
    let parent = AgentId::new();
    let process = ProcessId::new();
    let ctx = context(root, parent, process);
    let input = serde_json::json!({
        "profile":"worker","runtime":"native-cognit","task":"work",
        "tools":["file_read"],
        "budget":{"max_input_tokens":100,"max_output_tokens":100,"max_tool_calls":2,"max_elapsed_ms":1000,"max_depth":2}
    });
    let result = find(&tools, "agent_spawn").execute(input, &ctx).await;
    assert!(!result.is_error);
    let request = control.0.lock().unwrap().spawn[0].clone();
    assert_eq!(request.root_agent_id, root);
    assert_eq!(request.parent_agent_id, Some(parent));
    assert_eq!(request.parent_process_id, Some(process));
    assert_eq!(request.runtime_id.0, "native-cognit");
    assert_eq!(
        request
            .trusted_workspace
            .as_ref()
            .map(|policy| policy.cwd()),
        Some(ctx.working_dir.as_path())
    );

    let forged = serde_json::json!({
        "caller_root_agent_id":AgentId::new(),
        "profile":"worker","runtime":"native-cognit","task":"work",
        "budget":{"max_input_tokens":100,"max_output_tokens":100,"max_tool_calls":2,"max_elapsed_ms":1000,"max_depth":2}
    });
    assert!(
        find(&tools, "agent_spawn")
            .execute(forged, &ctx)
            .await
            .is_error
    );
    assert_eq!(control.0.lock().unwrap().spawn.len(), 1);
}

#[tokio::test]
async fn wait_list_send_and_cancel_are_root_scoped_and_structured() {
    let control = Arc::new(FakeControl::default());
    let tools = tools(control.clone());
    let root = AgentId::new();
    let ctx = context(root, AgentId::new(), ProcessId::new());
    let agent = AgentId::new();

    let wait = find(&tools, "agent_wait")
        .execute(serde_json::json!({"agent_id":agent,"timeout_ms":500}), &ctx)
        .await;
    assert!(!wait.is_error);
    let parsed: serde_json::Value = serde_json::from_str(&wait.content).unwrap();
    assert_eq!(parsed["result"]["status"], "succeeded");

    let send = find(&tools, "agent_send")
        .execute(
            serde_json::json!({"agent_id":agent,"message":"continue","start_turn":true}),
            &ctx,
        )
        .await;
    assert!(!send.is_error);
    let parsed: serde_json::Value = serde_json::from_str(&send.content).unwrap();
    assert_eq!(parsed["result"]["sequence"], 7);

    let cancel = find(&tools, "agent_cancel")
        .execute(serde_json::json!({"agent_id":agent}), &ctx)
        .await;
    assert!(!cancel.is_error);
    let list = find(&tools, "agent_list")
        .execute(serde_json::json!({"limit":10}), &ctx)
        .await;
    assert!(!list.is_error);

    let calls = control.0.lock().unwrap();
    assert_eq!(calls.wait[0].caller_root_agent_id, root);
    assert_eq!(calls.send[0].caller_root_agent_id, root);
    assert_eq!(calls.cancel[0].0, root);
    assert_eq!(calls.list[0].caller_root_agent_id, root);
}

#[tokio::test]
async fn missing_trusted_context_and_zero_wait_fail_before_control() {
    let control = Arc::new(FakeControl::default());
    let tools = tools(control.clone());
    let untrusted_context = ToolContext {
        approval_authority: None,
        agent: None,
        working_dir: std::env::temp_dir(),
        session_id: "untrusted".into(),
        clock: Arc::new(aletheon_kernel::chronos::TestClock::default()),
        turn_event_sender: None,
    };
    assert!(
        find(&tools, "agent_list")
            .execute(serde_json::json!({"limit":1}), &untrusted_context)
            .await
            .is_error
    );

    let root = AgentId::new();
    let context = context(root, root, ProcessId::new());
    assert!(
        find(&tools, "agent_wait")
            .execute(
                serde_json::json!({"agent_id":AgentId::new(),"timeout_ms":0}),
                &context,
            )
            .await
            .is_error
    );
    assert!(control.0.lock().unwrap().wait.is_empty());
}
