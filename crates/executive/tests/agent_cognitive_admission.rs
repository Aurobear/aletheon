mod agent_control_support;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use executive::application::agent_control::{
    AgentEventSink, AgentRuntimeInput, AgentRuntimeLauncher, CognitiveTaskAdmissionPort,
};
use fabric::cognitive_workflow::{
    CognitiveRole, CognitiveRoleProfile, CognitiveTaskNodeId, CognitiveTaskRuntimeBinding,
};
use fabric::{
    AgentControlError, AgentControlErrorKind, AgentId, AgentResult, AgentWaitRequest, AgoraSpaceId,
    AttemptUsage, ProcessId, WorkspacePolicy,
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

struct RecordingWorkspaceLauncher {
    workspace: Arc<Mutex<Option<WorkspacePolicy>>>,
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

#[async_trait]
impl AgentRuntimeLauncher for RecordingWorkspaceLauncher {
    async fn launch(
        &self,
        input: AgentRuntimeInput,
        _events: Arc<dyn AgentEventSink>,
    ) -> Result<AgentResult, AgentControlError> {
        *self.workspace.lock().unwrap() = input.workspace;
        Ok(AgentResult {
            output: "terminal".into(),
            usage: AttemptUsage::default(),
            evidence: Vec::new(),
            artifacts: Vec::new(),
        })
    }
}

fn binding(role: CognitiveRole, workspace_scope: Vec<String>) -> CognitiveTaskRuntimeBinding {
    let profile = CognitiveRoleProfile::canonical(role);
    CognitiveTaskRuntimeBinding {
        space: AgoraSpaceId("root-task".into()),
        task_node_id: CognitiveTaskNodeId(format!("{role:?}")),
        expected_workspace_version: 4,
        expected_owner: ProcessId::new(),
        role,
        role_profile: profile.reference,
        budget: profile.budget,
        workspace_scope,
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
    let temporary = tempfile::tempdir().unwrap();
    request.trusted_workspace = Some(
        WorkspacePolicy::from_resolved_roots(temporary.path().to_path_buf(), Vec::new()).unwrap(),
    );
    request.cognitive_binding = Some(binding(CognitiveRole::Planner, Vec::new()));
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

#[tokio::test]
async fn cognitive_binding_attenuates_effective_runtime_workspace() {
    let temporary = tempfile::tempdir().unwrap();
    std::fs::create_dir(temporary.path().join("src")).unwrap();
    let allowed = temporary.path().join("src/allowed.rs");
    let sibling = temporary.path().join("src/sibling.rs");
    std::fs::write(&allowed, "allowed").unwrap();
    std::fs::write(&sibling, "sibling").unwrap();
    let recorded = Arc::new(Mutex::new(None));
    let admitted = Arc::new(AtomicBool::new(false));
    let fixture = agent_control_support::fixture_with_task_admission(
        1,
        Arc::new(RecordingWorkspaceLauncher {
            workspace: recorded.clone(),
        }),
        Some(Arc::new(RecordingAdmission { admitted })),
    );
    let root = AgentId::new();
    let mut request = agent_control_support::spawn_request(root, None);
    request.trusted_workspace = Some(
        WorkspacePolicy::from_resolved_roots(temporary.path().to_path_buf(), Vec::new()).unwrap(),
    );
    request.cognitive_binding = Some(binding(CognitiveRole::Fixer, vec!["src/allowed.rs".into()]));

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
    let workspace = recorded.lock().unwrap().clone().unwrap();
    assert_eq!(workspace.writable_roots(), &[allowed]);
    assert!(!workspace
        .writable_roots()
        .iter()
        .any(|path| path == &sibling));
}

#[tokio::test]
async fn read_only_cognitive_binding_receives_no_writable_roots() {
    let temporary = tempfile::tempdir().unwrap();
    let recorded = Arc::new(Mutex::new(None));
    let admitted = Arc::new(AtomicBool::new(false));
    let fixture = agent_control_support::fixture_with_task_admission(
        1,
        Arc::new(RecordingWorkspaceLauncher {
            workspace: recorded.clone(),
        }),
        Some(Arc::new(RecordingAdmission { admitted })),
    );
    let root = AgentId::new();
    let mut request = agent_control_support::spawn_request(root, None);
    request.trusted_workspace = Some(
        WorkspacePolicy::from_resolved_roots(temporary.path().to_path_buf(), Vec::new()).unwrap(),
    );
    request.cognitive_binding = Some(binding(CognitiveRole::Reviewer, Vec::new()));

    let handle = fixture.port.spawn(request).await.unwrap();
    fixture
        .port
        .wait(AgentWaitRequest {
            caller_root_agent_id: root,
            agent_id: handle.agent_id,
            timeout_ms: 1_000,
        })
        .await
        .unwrap();

    assert!(recorded
        .lock()
        .unwrap()
        .as_ref()
        .unwrap()
        .writable_roots()
        .is_empty());
}

#[tokio::test]
async fn invalid_cognitive_workspace_bindings_fail_closed() {
    async fn assert_rejected(
        trusted_workspace: Option<WorkspacePolicy>,
        role: CognitiveRole,
        scope: Vec<String>,
        expected_kind: AgentControlErrorKind,
    ) {
        let admitted = Arc::new(AtomicBool::new(false));
        let fixture = agent_control_support::fixture_with_task_admission(
            1,
            Arc::new(RecordingWorkspaceLauncher {
                workspace: Arc::new(Mutex::new(None)),
            }),
            Some(Arc::new(RecordingAdmission { admitted })),
        );
        let mut request = agent_control_support::spawn_request(AgentId::new(), None);
        request.trusted_workspace = trusted_workspace;
        request.cognitive_binding = Some(binding(role, scope));
        assert_eq!(
            fixture.port.spawn(request).await.unwrap_err().kind,
            expected_kind
        );
    }

    let temporary = tempfile::tempdir().unwrap();
    let trusted =
        WorkspacePolicy::from_resolved_roots(temporary.path().to_path_buf(), Vec::new()).unwrap();
    assert_rejected(
        Some(trusted.clone()),
        CognitiveRole::Executor,
        Vec::new(),
        AgentControlErrorKind::Forbidden,
    )
    .await;
    assert_rejected(
        Some(trusted.clone()),
        CognitiveRole::Planner,
        vec!["src/lib.rs".into()],
        AgentControlErrorKind::Forbidden,
    )
    .await;
    assert_rejected(
        Some(trusted),
        CognitiveRole::Fixer,
        vec!["../escape.rs".into()],
        AgentControlErrorKind::InvalidRequest,
    )
    .await;
    assert_rejected(
        None,
        CognitiveRole::Fixer,
        vec!["src/lib.rs".into()],
        AgentControlErrorKind::Forbidden,
    )
    .await;
}
