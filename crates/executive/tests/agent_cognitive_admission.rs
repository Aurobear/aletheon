mod agent_control_support;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use executive::application::agent_control::{
    AgentEventSink, AgentRuntimeInput, AgentRuntimeLauncher, CognitiveTaskAdmissionPort,
};
use fabric::cognitive_workflow::{
    CognitiveRole, CognitiveRoleProfile, CognitiveTaskNodeId, CognitiveTaskRuntimeBinding,
};
use fabric::{
    AgentControlError, AgentId, AgentResult, AgentWaitRequest, AgoraSpaceId, AttemptUsage,
    ProcessId,
};

struct RecordingAdmission {
    admitted: Arc<AtomicBool>,
}

#[async_trait]
impl CognitiveTaskAdmissionPort for RecordingAdmission {
    async fn bind_before_launch(
        &self,
        _binding: CognitiveTaskRuntimeBinding,
        _allocated_process: ProcessId,
    ) -> Result<(), AgentControlError> {
        self.admitted.store(true, Ordering::SeqCst);
        Ok(())
    }
}

struct OrderingLauncher {
    admitted: Arc<AtomicBool>,
}

#[async_trait]
impl AgentRuntimeLauncher for OrderingLauncher {
    async fn launch(
        &self,
        _input: AgentRuntimeInput,
        _events: Arc<dyn AgentEventSink>,
    ) -> Result<AgentResult, AgentControlError> {
        assert!(
            self.admitted.load(Ordering::SeqCst),
            "runtime launched before cognitive task admission"
        );
        Ok(AgentResult {
            output: "terminal".into(),
            usage: AttemptUsage::default(),
            evidence: Vec::new(),
            artifacts: Vec::new(),
        })
    }
}

#[tokio::test]
async fn allocated_process_is_bound_before_runtime_launch() {
    let admitted = Arc::new(AtomicBool::new(false));
    let fixture = agent_control_support::fixture_with_task_admission(
        1,
        Arc::new(OrderingLauncher {
            admitted: admitted.clone(),
        }),
        Some(Arc::new(RecordingAdmission {
            admitted: admitted.clone(),
        })),
    );
    let root = AgentId::new();
    let mut request = agent_control_support::spawn_request(root, None);
    let profile = CognitiveRoleProfile::canonical(CognitiveRole::Planner);
    request.cognitive_binding = Some(CognitiveTaskRuntimeBinding {
        space: AgoraSpaceId("root-task".into()),
        task_node_id: CognitiveTaskNodeId("plan".into()),
        expected_workspace_version: 4,
        expected_owner: ProcessId::new(),
        role: CognitiveRole::Planner,
        role_profile: profile.reference,
        budget: profile.budget,
        workspace_scope: Vec::new(),
    });
    let handle = fixture.port.spawn(request).await.unwrap();
    let terminal = fixture
        .port
        .wait(AgentWaitRequest {
            caller_root_agent_id: root,
            agent_id: handle.agent_id,
            timeout_ms: 1_000,
        })
        .await
        .unwrap();
    assert_eq!(terminal.status, fabric::AgentRunStatus::Succeeded);
    assert!(admitted.load(Ordering::SeqCst));
}
