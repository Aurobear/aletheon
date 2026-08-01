use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::{TimeZone, Utc};
use executive::application::memory_gateway::{
    MemoryGatewayService, SupplementalBindingNegotiator, SupplementalBindingRecallPort,
};
use fabric::protocol::memory::{
    MemoryFeedbackRequestV1, MemoryFeedbackSignalV1, MemoryIntakeStatusV1, MemoryObservationKindV1,
    MemoryObservationRequestV1, MemoryRecallRequestV1, MemorySensitivityV1,
    MemoryWorkspaceBindRequestV1, MemoryWorkspaceBindingSpecV1, MemoryWorkspaceBindingStateV1,
    MemoryWorkspacePreviewBindRequestV1, MemoryWorkspaceStateV1, MemoryWorkspaceUnbindRequestV1,
    MAX_MEMORY_RECALL_CONTENT_BYTES, MAX_MEMORY_RECALL_ITEMS,
};
use fabric::PrincipalId;
use kernel::chronos::TestClock;
use mnemosyne::{
    ExperienceEvent, ForgetPolicy, ForgetReceipt, MemoryAuthority, MemoryIntakeLedger, MemoryKind,
    MemoryMetadata, MemoryProvenance, MemoryScope, MemorySensitivity, MemoryService, RecallItem,
    RecallRequest, RecallSet, SupplementalCapabilityGrant, TemporalState,
    WorkspaceMemoryBindingRegistry, WorkspaceMemoryKey,
};
use tempfile::TempDir;

#[derive(Default)]
struct CapturingMemory {
    requests: Mutex<Vec<RecallRequest>>,
    result: Mutex<RecallSet>,
}

struct FixedNegotiator {
    grant: Mutex<SupplementalCapabilityGrant>,
}

#[derive(Default)]
struct CountingSupplementalRecall {
    calls: AtomicUsize,
}

#[async_trait]
impl SupplementalBindingRecallPort for CountingSupplementalRecall {
    async fn recall(
        &self,
        _binding: &mnemosyne::WorkspaceMemoryBinding,
        _request: &RecallRequest,
    ) -> anyhow::Result<RecallSet> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(RecallSet::default())
    }
}

#[async_trait]
impl SupplementalBindingNegotiator for FixedNegotiator {
    async fn negotiate(
        &self,
        _destination_handle: &str,
        _backend_id: &str,
        _expected_source: &str,
    ) -> anyhow::Result<SupplementalCapabilityGrant> {
        Ok(self.grant.lock().unwrap().clone())
    }
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

fn binding_spec() -> MemoryWorkspaceBindingSpecV1 {
    MemoryWorkspaceBindingSpecV1 {
        backend_id: "supplemental/gbrain".into(),
        write_destination_handle: "gbrain-workspace".into(),
        read_destination_handles: vec!["gbrain-workspace".into(), "gbrain-personal".into()],
        expected_write_source: "workspace-a".into(),
        expected_read_sources: vec!["workspace-a".into(), "personal".into()],
        credential_ref: "mcp-server:gbrain-workspace".into(),
    }
}

fn binding_grant() -> SupplementalCapabilityGrant {
    SupplementalCapabilityGrant {
        backend_id: "supplemental/gbrain".into(),
        write_source: Some("workspace-a".into()),
        read_sources: vec!["personal".into(), "workspace-a".into()],
        can_read: true,
        can_write: true,
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
        Arc::new(WorkspaceMemoryBindingRegistry::open(state.path().join("bindings.db")).unwrap()),
        None,
        None,
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
async fn workspace_binding_requires_previewed_grants_and_controls_receipt_state() {
    let state = TempDir::new().unwrap();
    let project = project(state.path());
    let negotiator = Arc::new(FixedNegotiator {
        grant: Mutex::new(binding_grant()),
    });
    let supplemental_recall = Arc::new(CountingSupplementalRecall::default());
    let ledger = Arc::new(MemoryIntakeLedger::open(state.path().join("intake.db")).unwrap());
    let gateway = MemoryGatewayService::from_parts(
        ledger,
        Arc::new(CapturingMemory::default()),
        Arc::new(TestClock::new(1_700_000_000_000, 10)),
        "50e0337d-9c16-4d02-8b57-233ab7248759",
        Arc::new(WorkspaceMemoryBindingRegistry::open(state.path().join("bindings.db")).unwrap()),
        Some(negotiator.clone()),
        Some(supplemental_recall.clone()),
    )
    .unwrap();
    let principal = PrincipalId("principal-a".into());
    let mut unsafe_spec = binding_spec();
    unsafe_spec.credential_ref = "sk-not-a-reference".into();
    assert!(gateway
        .preview_workspace_bind(
            &principal,
            MemoryWorkspacePreviewBindRequestV1 {
                working_dir: project.clone(),
                binding: unsafe_spec,
            },
        )
        .await
        .is_err());
    let preview = gateway
        .preview_workspace_bind(
            &principal,
            MemoryWorkspacePreviewBindRequestV1 {
                working_dir: project.clone(),
                binding: binding_spec(),
            },
        )
        .await
        .unwrap();
    assert!(preview.compatible);
    assert_eq!(preview.binding.state, MemoryWorkspaceBindingStateV1::Active);
    let digest = preview.binding.verified_capability_digest.unwrap();

    let bound = gateway
        .bind_workspace(
            &principal,
            MemoryWorkspaceBindRequestV1 {
                working_dir: project.clone(),
                binding: binding_spec(),
                expected_capability_digest: digest,
            },
        )
        .await
        .unwrap();
    assert_eq!(bound.state, MemoryWorkspaceBindingStateV1::Active);
    let receipt = gateway
        .observe(
            &principal,
            "test-client",
            observation(project.clone(), "bound-observation"),
        )
        .await
        .unwrap();
    assert_eq!(receipt.workspace_state, MemoryWorkspaceStateV1::Bound);
    gateway
        .recall(
            &principal,
            "test-client",
            MemoryRecallRequestV1 {
                request_id: "bound-recall".into(),
                client_session_id: "client-session".into(),
                working_dir: project.clone(),
                query: "memory".into(),
                max_items: 4,
                max_content_bytes: 4096,
                include_historical: false,
                requested_kinds: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(supplemental_recall.calls.load(Ordering::SeqCst), 1);

    negotiator.grant.lock().unwrap().write_source = Some("drifted".into());
    assert!(gateway
        .bind_workspace(
            &principal,
            MemoryWorkspaceBindRequestV1 {
                working_dir: project.clone(),
                binding: binding_spec(),
                expected_capability_digest: bound.verified_capability_digest.unwrap(),
            },
        )
        .await
        .is_err());

    let revoked = gateway
        .unbind_workspace(
            &principal,
            MemoryWorkspaceUnbindRequestV1 {
                working_dir: project.clone(),
            },
        )
        .await
        .unwrap();
    assert_eq!(revoked.state, MemoryWorkspaceBindingStateV1::Revoked);
    let receipt = gateway
        .observe(
            &principal,
            "test-client",
            observation(project.clone(), "local-observation"),
        )
        .await
        .unwrap();
    assert_eq!(receipt.workspace_state, MemoryWorkspaceStateV1::LocalOnly);
    gateway
        .recall(
            &principal,
            "test-client",
            MemoryRecallRequestV1 {
                request_id: "local-recall".into(),
                client_session_id: "client-session".into(),
                working_dir: project,
                query: "memory".into(),
                max_items: 4,
                max_content_bytes: 4096,
                include_historical: false,
                requested_kinds: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(supplemental_recall.calls.load(Ordering::SeqCst), 1);
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

#[tokio::test]
async fn observe_scrubs_secret_and_pii_before_any_durable_intake_write() {
    let state = TempDir::new().unwrap();
    let project = project(state.path());
    let memory: Arc<dyn MemoryService> = Arc::new(CapturingMemory::default());
    let gateway = MemoryGatewayService::open(
        state.path(),
        memory,
        Arc::new(TestClock::new(1_700_000_000_000, 10)),
    )
    .unwrap();
    let mut request = observation(project, "secret-observation");
    request.content = "api_key=do-not-persist user=person@example.com".into();
    request.sensitivity_hint = MemorySensitivityV1::Public;
    gateway
        .observe(&PrincipalId("principal-a".into()), "test-client", request)
        .await
        .unwrap();
    drop(gateway);

    let database = state.path().join("memory-gateway/intake-v1.db");
    let connection = rusqlite::Connection::open(&database).unwrap();
    connection
        .execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")
        .unwrap();
    let json: String = connection
        .query_row(
            "SELECT observation_json FROM memory_intakes LIMIT 1",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let stored: mnemosyne::GovernedMemoryObservation = serde_json::from_str(&json).unwrap();
    assert_eq!(stored.content, "[REDACTED] user=[REDACTED]");
    assert_eq!(stored.scrub_redactions, 2);
    assert_eq!(stored.sensitivity, MemorySensitivityV1::Restricted);
    assert!(stored.content_fingerprint.starts_with("keyed-sha256:"));
    drop(connection);
    let bytes = std::fs::read(database).unwrap();
    let durable = String::from_utf8_lossy(&bytes);
    assert!(!durable.contains("do-not-persist"));
    assert!(!durable.contains("person@example.com"));
}

#[tokio::test]
async fn observe_rejects_secret_bearing_identifier_fields_before_persistence() {
    let state = TempDir::new().unwrap();
    let project = project(state.path());
    let gateway = MemoryGatewayService::open(
        state.path(),
        Arc::new(CapturingMemory::default()),
        Arc::new(TestClock::default()),
    )
    .unwrap();
    let mut request = observation(project, "identifier-secret");
    request.source_refs = vec!["api_key=must-not-persist".into()];
    assert!(gateway
        .observe(&PrincipalId("principal-a".into()), "test-client", request)
        .await
        .is_err());
    drop(gateway);

    let connection =
        rusqlite::Connection::open(state.path().join("memory-gateway/intake-v1.db")).unwrap();
    let count: i64 = connection
        .query_row("SELECT COUNT(*) FROM memory_intakes", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count, 0);
}
