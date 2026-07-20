use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use corpus::tools::tools::agent_tool::AgentTool;
use fabric::{
    AgentApprovalPolicy, AgentControlError, AgentControlPort, AgentHandle, AgentId,
    AgentListRequest, AgentProfile, AgentProfileId, AgentResult, AgentRunStatus, AgentSendRequest,
    AgentSnapshot, AgentSpawnRequest, AgentToolContext, AgentWaitRequest, AttemptUsage, Clock,
    OperationId, ParentRestriction, ProcessId, RiskTier, RuntimeId, Tool, ToolContext,
};

#[derive(Default)]
struct Control {
    spawned: Mutex<Vec<AgentSpawnRequest>>,
}

#[async_trait]
impl AgentControlPort for Control {
    async fn spawn(&self, request: AgentSpawnRequest) -> Result<AgentHandle, AgentControlError> {
        let handle = AgentHandle {
            agent_id: AgentId::new(),
            root_agent_id: request.root_agent_id,
            parent_agent_id: request.parent_agent_id,
            process_id: ProcessId::new(),
            operation_id: OperationId::new(),
            runtime_id: request.runtime_id.clone(),
            profile_id: request.profile_id.clone(),
        };
        self.spawned.lock().unwrap().push(request);
        Ok(handle)
    }

    async fn wait(&self, request: AgentWaitRequest) -> Result<AgentSnapshot, AgentControlError> {
        Ok(AgentSnapshot {
            handle: AgentHandle {
                agent_id: request.agent_id,
                root_agent_id: request.caller_root_agent_id,
                parent_agent_id: None,
                process_id: ProcessId::new(),
                operation_id: OperationId::new(),
                runtime_id: RuntimeId("native-cognit".into()),
                profile_id: AgentProfileId("reviewer".into()),
            },
            status: AgentRunStatus::Succeeded,
            result: Some(AgentResult {
                output: "review complete".into(),
                usage: AttemptUsage::default(),
                evidence: vec![],
                artifacts: vec![],
            }),
            created_at_ms: 1,
            started_at_ms: Some(2),
            ended_at_ms: Some(3),
            last_error: None,
        })
    }

    async fn send(
        &self,
        _request: AgentSendRequest,
    ) -> Result<fabric::AgentControlMessage, AgentControlError> {
        unreachable!()
    }
    async fn cancel(
        &self,
        _caller_root_agent_id: AgentId,
        _agent_id: AgentId,
    ) -> Result<AgentSnapshot, AgentControlError> {
        unreachable!()
    }
    async fn inspect(
        &self,
        _caller_root_agent_id: AgentId,
        _agent_id: AgentId,
    ) -> Result<AgentSnapshot, AgentControlError> {
        unreachable!()
    }
    async fn list(
        &self,
        _request: AgentListRequest,
    ) -> Result<Vec<AgentSnapshot>, AgentControlError> {
        unreachable!()
    }
}

fn profile() -> AgentProfile {
    AgentProfile {
        id: AgentProfileId("reviewer".into()),
        system_prompt: "Review evidence only.".into(),
        model: "profile-model".into(),
        allowed_tools: vec!["file_read".into(), "grep".into()],
        max_iterations: 7,
        max_input_tokens: 8_000,
        max_output_tokens: 1_000,
        max_tool_calls: 7,
        max_elapsed_ms: 30_000,
        profile_name: "reviewer".into(),
        risk_tier: RiskTier::ReadOnly,
        approval_policy: AgentApprovalPolicy::PromptUser,
        tool_timeout_ms: 30_000,
        inheritable: true,
        parent_restriction: ParentRestriction::SameOrSafer,
    }
}

fn context() -> ToolContext {
    let root = AgentId::new();
    ToolContext {
        approval_authority: None,
        agent: Some(AgentToolContext {
            caller_root_agent_id: root,
            parent_agent_id: root,
            parent_process_id: ProcessId::new(),
        }),
        working_dir: std::env::temp_dir(),
        session_id: "root-session".into(),
        clock: Arc::new(kernel::chronos::TestClock::default()) as Arc<dyn Clock>,
        turn_event_sender: None,
    }
}

#[tokio::test]
async fn compatibility_agent_tool_is_bounded_spawn_plus_wait() {
    let control = Arc::new(Control::default());
    let mut profiles = HashMap::new();
    profiles.insert("reviewer".into(), profile());
    let tool = AgentTool::new(profiles, control.clone(), RuntimeId("native-cognit".into()));

    let result = tool
        .execute(
            serde_json::json!({"agent_type":"reviewer","prompt":"inspect the plan"}),
            &context(),
        )
        .await;

    assert!(!result.is_error);
    assert_eq!(result.content, "review complete");
    let spawned = control.spawned.lock().unwrap();
    assert_eq!(spawned.len(), 1);
    assert_eq!(spawned[0].profile_id.0, "reviewer");
    assert_eq!(spawned[0].runtime_id.0, "native-cognit");
    assert_eq!(spawned[0].allowed_tools, vec!["file_read", "grep"]);
}

#[tokio::test]
async fn unknown_profile_does_not_spawn() {
    let control = Arc::new(Control::default());
    let tool = AgentTool::new(
        HashMap::new(),
        control.clone(),
        RuntimeId("native-cognit".into()),
    );
    let result = tool
        .execute(
            serde_json::json!({"agent_type":"missing","prompt":"inspect"}),
            &context(),
        )
        .await;
    assert!(result.is_error);
    assert!(control.spawned.lock().unwrap().is_empty());
}
