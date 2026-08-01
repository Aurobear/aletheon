use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::{TimeZone, Utc};
use executive::application::memory_gateway::MemoryGatewayService;
use fabric::protocol::memory::{
    MemoryFeedbackRequestV1, MemoryFeedbackSignalV1, MemoryIntakeStatusV1, MemoryObservationKindV1,
    MemoryObservationRequestV1, MemoryRecallRequestV1, MemorySensitivityV1,
    MAX_MEMORY_RECALL_CONTENT_BYTES, MAX_MEMORY_RECALL_ITEMS,
};
use fabric::PrincipalId;
use kernel::chronos::TestClock;
use mnemosyne::{
    ExperienceEvent, ForgetPolicy, ForgetReceipt, MemoryAuthority, MemoryIntakeLedger, MemoryKind,
    MemoryMetadata, MemoryProvenance, MemoryScope, MemorySensitivity, MemoryService, RecallItem,
    RecallRequest, RecallSet, TemporalState, WorkspaceMemoryKey,
};
use tempfile::TempDir;

#[derive(Default)]
struct CapturingMemory {
    requests: Mutex<Vec<RecallRequest>>,
    result: Mutex<RecallSet>,
}

impl CapturingMemory {
    fn returning(items: Vec<RecallItem>) -> Self {
        Self {
            requests: Mutex::new(Vec::new()),
            result: Mutex::new(RecallSet {
                items,
                degraded_sources: Vec::new(),
            }),
        }
    }
}

#[async_trait]
impl MemoryService for CapturingMemory {
    async fn record(&self, _event: ExperienceEvent) -> anyhow::Result<()> {
        Ok(())
    }

    async fn recall(&self, request: RecallRequest) -> anyhow::Result<RecallSet> {
        self.requests.lock().unwrap().push(request);
        Ok(self.result.lock().unwrap().clone())
    }

    async fn consolidate(&self, _scope: mnemosyne::service::MemoryScope) -> anyhow::Result<()> {
        Ok(())
    }

    async fn forget(&self, _policy: ForgetPolicy) -> anyhow::Result<ForgetReceipt> {
        Ok(ForgetReceipt::default())
    }
}

fn project(root: &Path) -> PathBuf {
    let project = root.join("project");
    std::fs::create_dir_all(project.join("nested")).unwrap();
    project
}

fn observation(working_dir: PathBuf, id: &str) -> MemoryObservationRequestV1 {
    MemoryObservationRequestV1 {
        observation_id: id.into(),
        client_session_id: "client-session".into(),
        client_turn_id: Some("turn-1".into()),
        working_dir,
        kind: MemoryObservationKindV1::ExplicitNote,
        content: "bounded durable observation".into(),
        occurred_at: None,
        source_refs: Vec::new(),
        sensitivity_hint: MemorySensitivityV1::Internal,
        explicit_user_action: true,
    }
}

fn item(record_id: &str, scope: MemoryScope, content: &str) -> RecallItem {
    let observed = Utc.timestamp_opt(1_700_000_000, 0).single().unwrap();
    RecallItem {
        content: content.into(),
        kind: MemoryKind::SemanticFact,
        metadata: MemoryMetadata {
            record_id: record_id.into(),
            provenance: MemoryProvenance {
                source: "mnemosyne.local".into(),
                source_id: record_id.into(),
                principal: Some("principal-a".into()),
                source_commit: None,
            },
            source_time: Some(observed),
            observed_time: observed,
            valid_from: Some(observed),
            valid_until: None,
            supersedes: None,
            superseded_by: None,
            confidence: 0.9,
            sensitivity: MemorySensitivity::Internal,
        },
        temporal_state: TemporalState::Current,
        authority: MemoryAuthority::VerifiedLocalSemantic,
        scope,
        score: 0.8,
        evidence: None,
    }
}

fn service(
    state: &TempDir,
    memory: Arc<dyn MemoryService>,
    installation_id: &str,
) -> MemoryGatewayService {
    let ledger = Arc::new(MemoryIntakeLedger::open(state.path().join("intake.db")).unwrap());
    MemoryGatewayService::from_parts(
        ledger,
        memory,
        Arc::new(TestClock::new(1_700_000_000_000, 10)),
        installation_id,
    )
    .unwrap()
}

#[tokio::test]
async fn observe_canonicalizes_aliases_and_scopes_idempotency_to_host_authority() {
    let state = TempDir::new().unwrap();
    let project = project(state.path());
    let gateway = service(
        &state,
        Arc::new(CapturingMemory::default()),
        "50e0337d-9c16-4d02-8b57-233ab7248759",
    );
    let principal_a = PrincipalId("principal-a".into());
    let first = gateway
        .observe(
            &principal_a,
            "test-client",
            observation(project.join("nested/.."), "observation-1"),
        )
        .await
        .unwrap();
    let duplicate = gateway
        .observe(
            &principal_a,
            "test-client",
            observation(project.clone(), "observation-1"),
        )
        .await
        .unwrap();
    let other_principal = gateway
        .observe(
            &PrincipalId("principal-b".into()),
            "test-client",
            observation(project.clone(), "observation-1"),
        )
        .await
        .unwrap();

    assert_eq!(first.intake_status, MemoryIntakeStatusV1::Observed);
    assert_eq!(duplicate.intake_status, MemoryIntakeStatusV1::Duplicate);
    assert_eq!(duplicate.durable_intake_id, first.durable_intake_id);
    assert_ne!(other_principal.durable_intake_id, first.durable_intake_id);
    assert!(gateway
        .observe(
            &principal_a,
            "test-client",
            observation(state.path().join("missing"), "observation-2"),
        )
        .await
        .is_err());
}

#[tokio::test]
async fn recall_binds_workspace_and_clamps_client_budgets_before_retrieval() {
    let state = TempDir::new().unwrap();
    let project = project(state.path());
    std::fs::create_dir(project.join(".git")).unwrap();
    std::fs::write(
        project.join(".git/config"),
        "[remote \"origin\"]\nurl = https://example.invalid/team/project.git\n",
    )
    .unwrap();
    let identity = executive::application::workspace_trust::workspace_identity(&project);
    let expected_workspace =
        WorkspaceMemoryKey::derive(&identity, "50e0337d-9c16-4d02-8b57-233ab7248759").unwrap();
    let memory = Arc::new(CapturingMemory::returning(vec![
        item(
            "visible-workspace",
            MemoryScope::Workspace(expected_workspace.as_str().into()),
            "visible workspace fact",
        ),
        item(
            "other-workspace",
            MemoryScope::Workspace("ws:repo:other".into()),
            "must not escape authority filter",
        ),
    ]));
    let gateway = service(
        &state,
        memory.clone(),
        "50e0337d-9c16-4d02-8b57-233ab7248759",
    );
    let result = gateway
        .recall(
            &PrincipalId("principal-a".into()),
            "test-client",
            MemoryRecallRequestV1 {
                request_id: "recall-1".into(),
                client_session_id: "client-session".into(),
                working_dir: project,
                query: "workspace fact".into(),
                max_items: usize::MAX,
                max_content_bytes: usize::MAX,
                include_historical: false,
                requested_kinds: None,
            },
        )
        .await
        .unwrap();

    assert_eq!(
        result
            .items
            .iter()
            .map(|item| item.record_id.as_str())
            .collect::<Vec<_>>(),
        vec!["visible-workspace"]
    );
    let requests = memory.requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].max_items, MAX_MEMORY_RECALL_ITEMS);
    assert_eq!(
        requests[0].max_content_bytes,
        MAX_MEMORY_RECALL_CONTENT_BYTES
    );
    drop(requests);

    let feedback = gateway
        .feedback(
            &PrincipalId("principal-a".into()),
            "test-client",
            MemoryFeedbackRequestV1 {
                observation_id: "feedback-1".into(),
                client_session_id: "client-session".into(),
                target_record_id: "visible-workspace".into(),
                signal: MemoryFeedbackSignalV1::Useful,
                correction_text: None,
                working_dir: state.path().join("project"),
            },
        )
        .await
        .unwrap();
    assert_eq!(
        feedback.observation.intake_status,
        MemoryIntakeStatusV1::Observed
    );
    let other_project = state.path().join("other-project");
    std::fs::create_dir(&other_project).unwrap();
    assert!(gateway
        .feedback(
            &PrincipalId("principal-a".into()),
            "test-client",
            MemoryFeedbackRequestV1 {
                observation_id: "feedback-2".into(),
                client_session_id: "client-session".into(),
                target_record_id: "visible-workspace".into(),
                signal: MemoryFeedbackSignalV1::Useful,
                correction_text: None,
                working_dir: other_project,
            },
        )
        .await
        .is_err());
}

#[tokio::test]
async fn installation_identity_is_private_durable_and_reused_after_reopen() {
    let state = TempDir::new().unwrap();
    let project = project(state.path());
    let principal = PrincipalId("principal-a".into());
    let memory: Arc<dyn MemoryService> = Arc::new(CapturingMemory::default());
    let first =
        MemoryGatewayService::open(state.path(), memory.clone(), Arc::new(TestClock::default()))
            .unwrap();
    let receipt = first
        .observe(
            &principal,
            "test-client",
            observation(project.clone(), "observation-reopen"),
        )
        .await
        .unwrap();
    drop(first);
    let second =
        MemoryGatewayService::open(state.path(), memory, Arc::new(TestClock::default())).unwrap();
    let duplicate = second
        .observe(
            &principal,
            "test-client",
            observation(project, "observation-reopen"),
        )
        .await
        .unwrap();

    assert_eq!(duplicate.intake_status, MemoryIntakeStatusV1::Duplicate);
    assert_eq!(duplicate.durable_intake_id, receipt.durable_intake_id);
    let gateway_root = state.path().join("memory-gateway");
    assert_eq!(
        std::fs::metadata(&gateway_root)
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
    assert_eq!(
        std::fs::metadata(gateway_root.join("installation-id"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
}
