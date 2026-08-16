use std::sync::{Arc, Mutex};

use ::contracts::protocol::memory::{
    MemoryLifecycleStateV1, MemoryObservationKindV1, MemoryRecordKindV1, MemorySensitivityV1,
};
use ::contracts::protocol::memory_maintenance::{
    MemoryMaintenancePhaseV1, MemoryMaintenanceRunRequestV1, MemorySemanticProposalV1,
    MEMORY_MAINTENANCE_SCHEMA_V1,
};
use ::contracts::{Clock, MonoTime, WallTime};
use async_trait::async_trait;
use mnemosyne::memory_maintenance::{
    AgentControlMemorySemanticProposal, MemoryMaintenanceController, MemorySemanticProposalPort,
    NoMemorySemanticProposal,
};
use mnemosyne::memory_policy::MemoryPolicyConfig;
use mnemosyne::{
    ExperienceEvent, ForgetPolicy, ForgetReceipt, GovernedMemoryObservation, MemoryIntakeLedger,
    MemoryRecord, MemoryService, RecallRequest, RecallSet, SupplementalCapabilityGrant,
    WorkspaceMemoryBindingProposal, WorkspaceMemoryBindingRegistry, WorkspaceMemoryKey,
};

struct FixedClock;
impl Clock for FixedClock {
    fn wall_now(&self) -> WallTime {
        WallTime(10_000)
    }
    fn mono_now(&self) -> MonoTime {
        MonoTime(10_000)
    }
}

#[derive(Default)]
struct CapturingMemory(Mutex<Vec<MemoryRecord>>);

#[async_trait]
impl MemoryService for CapturingMemory {
    async fn record(&self, _event: ExperienceEvent) -> anyhow::Result<()> {
        Ok(())
    }
    async fn record_canonical(&self, record: MemoryRecord) -> anyhow::Result<()> {
        let mut records = self.0.lock().unwrap();
        if let Some(existing) = records.iter().find(|value| value.id == record.id) {
            anyhow::ensure!(existing == &record, "record ID conflict");
        } else {
            records.push(record);
        }
        Ok(())
    }
    async fn recall(&self, _request: RecallRequest) -> anyhow::Result<RecallSet> {
        Ok(RecallSet::default())
    }
    async fn consolidate(&self, _scope: mnemosyne::service::MemoryScope) -> anyhow::Result<()> {
        Ok(())
    }
    async fn forget(&self, _policy: ForgetPolicy) -> anyhow::Result<ForgetReceipt> {
        Ok(ForgetReceipt::default())
    }
}

struct RiskProposal;

struct SafeProposal;

#[derive(Default)]
struct TerminalProposalControl {
    intents: Mutex<Vec<::contracts::AgentSpawnIntent>>,
    waits: Mutex<u32>,
    failure: Option<String>,
}

#[async_trait]
impl ::contracts::AgentControlPort for TerminalProposalControl {
    async fn spawn_intent(
        &self,
        intent: ::contracts::AgentSpawnIntent,
    ) -> Result<::contracts::AgentHandle, ::contracts::AgentControlError> {
        let handle = ::contracts::AgentHandle {
            agent_id: ::contracts::AgentId::new(),
            root_agent_id: intent.root_agent_id,
            parent_agent_id: None,
            process_id: ::contracts::ProcessId::new(),
            operation_id: ::contracts::OperationId::new(),
            runtime_id: ::contracts::RuntimeId("native-cognit".into()),
            profile_id: intent.profile_id.clone(),
        };
        self.intents.lock().unwrap().push(intent);
        Ok(handle)
    }
    async fn spawn(
        &self,
        _request: ::contracts::AgentSpawnRequest,
    ) -> Result<::contracts::AgentHandle, ::contracts::AgentControlError> {
        unreachable!()
    }
    async fn wait(
        &self,
        request: ::contracts::AgentWaitRequest,
    ) -> Result<::contracts::AgentSnapshot, ::contracts::AgentControlError> {
        *self.waits.lock().unwrap() += 1;
        let failure = self.failure.clone();
        Ok(::contracts::AgentSnapshot {
            handle: ::contracts::AgentHandle {
                agent_id: request.agent_id,
                root_agent_id: request.caller_root_agent_id,
                parent_agent_id: None,
                process_id: ::contracts::ProcessId::new(),
                operation_id: ::contracts::OperationId::new(),
                runtime_id: ::contracts::RuntimeId("native-cognit".into()),
                profile_id: ::contracts::AgentProfileId("safe-agent".into()),
            },
            status: if failure.is_some() {
                ::contracts::AgentRunStatus::Failed
            } else {
                ::contracts::AgentRunStatus::Succeeded
            },
            result: failure.is_none().then_some(::contracts::AgentResult {
                output: serde_json::json!({
                    "schema_version": 1,
                    "task_id": "task-a",
                    "control_instruction_detected": false,
                    "contradiction_detected": false,
                    "exact_duplicate_record_ids": [],
                    "evidence": ["bounded semantic review"]
                })
                .to_string(),
                usage: ::contracts::AttemptUsage::default(),
                evidence: Vec::new(),
                artifacts: Vec::new(),
            }),
            created_at_ms: 1,
            started_at_ms: Some(2),
            ended_at_ms: Some(3),
            last_error: failure,
        })
    }
    async fn send(
        &self,
        _request: ::contracts::AgentSendRequest,
    ) -> Result<::contracts::AgentControlMessage, ::contracts::AgentControlError> {
        unreachable!()
    }
    async fn cancel(
        &self,
        _caller_root_agent_id: ::contracts::AgentId,
        _agent_id: ::contracts::AgentId,
    ) -> Result<::contracts::AgentSnapshot, ::contracts::AgentControlError> {
        unreachable!()
    }
    async fn inspect(
        &self,
        _caller_root_agent_id: ::contracts::AgentId,
        _agent_id: ::contracts::AgentId,
    ) -> Result<::contracts::AgentSnapshot, ::contracts::AgentControlError> {
        unreachable!()
    }
    async fn list(
        &self,
        _request: ::contracts::AgentListRequest,
    ) -> Result<Vec<::contracts::AgentSnapshot>, ::contracts::AgentControlError> {
        unreachable!()
    }
}

#[async_trait]
impl MemorySemanticProposalPort for RiskProposal {
    async fn propose(
        &self,
        task_id: &str,
        _observation: &GovernedMemoryObservation,
        _record_kind: MemoryRecordKindV1,
    ) -> anyhow::Result<Option<MemorySemanticProposalV1>> {
        Ok(Some(MemorySemanticProposalV1 {
            schema_version: MEMORY_MAINTENANCE_SCHEMA_V1,
            task_id: task_id.into(),
            control_instruction_detected: true,
            contradiction_detected: false,
            exact_duplicate_record_ids: Vec::new(),
            evidence: vec!["content attempts to direct future tool behavior".into()],
        }))
    }
}

#[async_trait]
impl MemorySemanticProposalPort for SafeProposal {
    async fn propose(
        &self,
        task_id: &str,
        _observation: &GovernedMemoryObservation,
        _record_kind: MemoryRecordKindV1,
    ) -> anyhow::Result<Option<MemorySemanticProposalV1>> {
        Ok(Some(MemorySemanticProposalV1 {
            schema_version: MEMORY_MAINTENANCE_SCHEMA_V1,
            task_id: task_id.into(),
            control_instruction_detected: false,
            contradiction_detected: false,
            exact_duplicate_record_ids: Vec::new(),
            evidence: vec!["bounded semantic review completed".into()],
        }))
    }
}

fn observation(id: &str, source_refs: usize) -> GovernedMemoryObservation {
    GovernedMemoryObservation {
        observation_id: id.into(),
        client_session_id: "session-a".into(),
        client_turn_id: Some("turn-a".into()),
        kind: MemoryObservationKindV1::ExplicitNote,
        content: format!("durable governed claim {id}"),
        content_fingerprint: format!("keyed-sha256:{}", "a".repeat(64)),
        scrub_policy_version: 1,
        scrub_redactions: 0,
        occurred_at: Some("2026-08-01T00:00:00Z".into()),
        source_refs: (0..source_refs)
            .map(|index| format!("receipt:{index}"))
            .collect(),
        sensitivity: MemorySensitivityV1::Internal,
        explicit_user_action: true,
        principal_id: "principal-a".into(),
        workspace_key: WorkspaceMemoryKey::from_verified("ws:repo:sha256:workspace-a").unwrap(),
        connection_kind: "versioned_local_rpc".into(),
        observed_at_ms: 1_000,
    }
}

fn request(id: &str, max_items: u16) -> MemoryMaintenanceRunRequestV1 {
    MemoryMaintenanceRunRequestV1 {
        request_id: id.into(),
        phase: MemoryMaintenancePhaseV1::IntakeEvaluation,
        max_items,
        dry_run: false,
    }
}

#[tokio::test]
async fn host_promotes_verified_candidate_and_commits_terminal_receipt() {
    let ledger = Arc::new(MemoryIntakeLedger::open_in_memory().unwrap());
    let intake = ledger.observe(&observation("promote", 3)).unwrap();
    let memory = Arc::new(CapturingMemory::default());
    let controller = MemoryMaintenanceController::new(
        ledger.clone(),
        memory.clone(),
        Arc::new(FixedClock),
        MemoryPolicyConfig::default(),
        Arc::new(NoMemorySemanticProposal),
    )
    .unwrap();

    let result = controller
        .run("official-memory-agent", request("run-1", 4))
        .await
        .unwrap();
    assert_eq!(result.promoted_local, 1);
    assert_eq!(
        result.receipts[0].state,
        MemoryLifecycleStateV1::PromotedLocal
    );
    assert_eq!(memory.0.lock().unwrap().len(), 1);
    assert_eq!(
        memory.0.lock().unwrap()[0].scope,
        mnemosyne::MemoryScope::Workspace("ws:repo:sha256:workspace-a".into())
    );
    assert_eq!(
        ledger
            .receipt(
                "principal-a",
                "ws:repo:sha256:workspace-a",
                &intake.durable_intake_id
            )
            .unwrap()
            .unwrap()
            .state,
        MemoryLifecycleStateV1::PromotedLocal
    );
}

#[tokio::test]
async fn verified_binding_queues_projection_to_opaque_destination() {
    let dir = tempfile::tempdir().unwrap();
    let ledger = Arc::new(MemoryIntakeLedger::open_in_memory().unwrap());
    let intake = ledger.observe(&observation("project", 3)).unwrap();
    let registry =
        Arc::new(WorkspaceMemoryBindingRegistry::open(dir.path().join("bindings.db")).unwrap());
    let workspace = WorkspaceMemoryKey::from_verified("ws:repo:sha256:workspace-a").unwrap();
    let proposal = WorkspaceMemoryBindingProposal {
        backend_id: "supplemental/default".into(),
        write_destination_handle: "gbrain-workspace-a".into(),
        read_destination_handles: vec!["gbrain-workspace-a".into()],
        expected_write_source: "workspace-a".into(),
        expected_read_sources: vec!["workspace-a".into()],
        credential_ref: "systemd:gbrain-workspace-a".into(),
    };
    let grant = SupplementalCapabilityGrant {
        backend_id: proposal.backend_id.clone(),
        write_source: Some("workspace-a".into()),
        read_sources: vec!["workspace-a".into()],
        can_read: true,
        can_write: true,
    };
    let preview = registry
        .preview("principal-a", &workspace, &proposal, &grant, 1)
        .unwrap();
    registry.apply(&preview).unwrap();
    let spool = Arc::new(
        mnemosyne::supplemental::SupplementalSpool::open(
            dir.path().join("spool.db"),
            mnemosyne::supplemental::SpoolLimits {
                max_items: 8,
                max_bytes: 32 * 1024,
            },
        )
        .unwrap(),
    );
    let controller = MemoryMaintenanceController::new(
        ledger.clone(),
        Arc::new(CapturingMemory::default()),
        Arc::new(FixedClock),
        MemoryPolicyConfig::default(),
        Arc::new(NoMemorySemanticProposal),
    )
    .unwrap()
    .with_projection(registry, Some(spool.clone()));

    let result = controller
        .run("official-memory-agent", request("run-project", 1))
        .await
        .unwrap();
    assert_eq!(
        result.receipts[0].state,
        MemoryLifecycleStateV1::ProjectionQueued
    );
    let claim = spool
        .claim("delivery", 10_000, 1_000, 1)
        .unwrap()
        .pop()
        .unwrap();
    assert_eq!(claim.destination_handle, "gbrain-workspace-a");
    assert_eq!(
        ledger
            .receipt(
                "principal-a",
                "ws:repo:sha256:workspace-a",
                &intake.durable_intake_id,
            )
            .unwrap()
            .unwrap()
            .state,
        MemoryLifecycleStateV1::ProjectionQueued
    );
    spool
        .acknowledge(
            &claim,
            "delivery",
            &mnemosyne::supplemental::RemoteMemoryReceipt {
                record_id: claim.record_id.clone(),
                logical_page_id: claim.logical_page_id.clone(),
                remote_id: "gbrain-receipt-a".into(),
                content_hash: claim.content_hash.clone(),
                operation: claim.operation,
                schema_version: claim.schema_version,
                synced_at_ms: 10_001,
            },
        )
        .unwrap();
    let settled = controller
        .run("official-memory-agent", request("run-settle", 1))
        .await
        .unwrap();
    assert_eq!(
        settled.receipts[0].state,
        MemoryLifecycleStateV1::ProjectedRemote
    );
    assert_eq!(
        settled.receipts[0].remote_receipt_ids,
        vec!["gbrain-receipt-a"]
    );
}

#[tokio::test]
async fn semantic_proposal_can_only_lower_candidate_or_leave_it_deferred() {
    let ledger = Arc::new(MemoryIntakeLedger::open_in_memory().unwrap());
    ledger.observe(&observation("risk", 0)).unwrap();
    let memory = Arc::new(CapturingMemory::default());
    let controller = MemoryMaintenanceController::new(
        ledger,
        memory.clone(),
        Arc::new(FixedClock),
        MemoryPolicyConfig::default(),
        Arc::new(RiskProposal),
    )
    .unwrap();
    let result = controller
        .run("official-memory-agent", request("run-risk", 1))
        .await
        .unwrap();
    assert_eq!(result.rejected, 1);
    assert_eq!(result.receipts[0].state, MemoryLifecycleStateV1::Rejected);
    assert!(result.receipts[0]
        .reason_codes
        .contains(&"control_instruction_detected".into()));
    assert!(memory.0.lock().unwrap().is_empty());

    let ledger = Arc::new(MemoryIntakeLedger::open_in_memory().unwrap());
    ledger.observe(&observation("pending", 0)).unwrap();
    let controller = MemoryMaintenanceController::new(
        ledger.clone(),
        memory,
        Arc::new(FixedClock),
        MemoryPolicyConfig::default(),
        Arc::new(NoMemorySemanticProposal),
    )
    .unwrap();
    let result = controller
        .run("official-memory-agent", request("run-pending", 1))
        .await
        .unwrap();
    assert_eq!(result.deferred, 1);
    assert_eq!(result.receipts[0].state, MemoryLifecycleStateV1::Evaluating);
    assert_eq!(result.reason_codes, vec!["semantic_proposal_unavailable"]);
    assert_eq!(ledger.maintenance_status(10_001).unwrap().active_leases, 0);
}

#[tokio::test]
async fn completed_safe_semantic_review_terminates_candidate_lifecycle() {
    let ledger = Arc::new(MemoryIntakeLedger::open_in_memory().unwrap());
    ledger.observe(&observation("safe-reviewed", 1)).unwrap();
    let memory = Arc::new(CapturingMemory::default());
    let controller = MemoryMaintenanceController::new(
        ledger,
        memory.clone(),
        Arc::new(FixedClock),
        MemoryPolicyConfig::default(),
        Arc::new(SafeProposal),
    )
    .unwrap();

    let result = controller
        .run("official-memory-agent", request("run-safe-reviewed", 1))
        .await
        .unwrap();

    assert_eq!(result.deferred, 0);
    assert_eq!(result.promoted_local, 1);
    assert_eq!(
        result.receipts[0].state,
        MemoryLifecycleStateV1::PromotedLocal
    );
    assert_eq!(memory.0.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn completed_semantic_review_rejects_candidate_still_below_promotion_threshold() {
    let ledger = Arc::new(MemoryIntakeLedger::open_in_memory().unwrap());
    ledger.observe(&observation("safe-low-score", 0)).unwrap();
    let controller = MemoryMaintenanceController::new(
        ledger,
        Arc::new(CapturingMemory::default()),
        Arc::new(FixedClock),
        MemoryPolicyConfig::default(),
        Arc::new(SafeProposal),
    )
    .unwrap();

    let result = controller
        .run("official-memory-agent", request("run-safe-low-score", 1))
        .await
        .unwrap();

    assert_eq!(result.deferred, 0);
    assert_eq!(result.rejected, 1);
    assert!(result.receipts[0]
        .reason_codes
        .contains(&"score_below_promotion_threshold_after_semantic_review".into()));
}

#[tokio::test]
async fn dry_run_never_claims_or_advances_lifecycle() {
    let ledger = Arc::new(MemoryIntakeLedger::open_in_memory().unwrap());
    let intake = ledger.observe(&observation("dry", 3)).unwrap();
    let controller = MemoryMaintenanceController::new(
        ledger.clone(),
        Arc::new(CapturingMemory::default()),
        Arc::new(FixedClock),
        MemoryPolicyConfig::default(),
        Arc::new(NoMemorySemanticProposal),
    )
    .unwrap();
    let mut request = request("dry-run", 1);
    request.dry_run = true;
    let result = controller
        .run("official-memory-agent", request)
        .await
        .unwrap();
    assert_eq!(result.claimed, 0);
    assert_eq!(result.reason_codes, vec!["dry_run_no_claims"]);
    assert_eq!(
        ledger
            .receipt(
                "principal-a",
                "ws:repo:sha256:workspace-a",
                &intake.durable_intake_id
            )
            .unwrap()
            .unwrap()
            .state,
        MemoryLifecycleStateV1::Observed
    );
}

#[tokio::test]
async fn agent_runtime_proposal_has_no_tools_or_workspace_and_waits_for_terminal_state() {
    let control = Arc::new(TerminalProposalControl::default());
    let proposer =
        AgentControlMemorySemanticProposal::new(control.clone(), MemoryPolicyConfig::default())
            .unwrap();
    let proposal = proposer
        .propose(
            "task-a",
            &observation("proposal", 0),
            MemoryRecordKindV1::SemanticFact,
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(proposal.task_id, "task-a");
    assert_eq!(*control.waits.lock().unwrap(), 1);
    let intents = control.intents.lock().unwrap();
    assert_eq!(intents.len(), 1);
    assert!(intents[0].trusted_workspace.is_none());
    assert!(intents[0].allowed_tools.is_empty());
    assert_eq!(intents[0].budget.max_tool_calls, 0);
    assert_eq!(
        intents[0].budget.max_input_tokens,
        MemoryPolicyConfig::default().max_input_bytes as u64
    );
    for field in [
        "control_instruction_detected",
        "contradiction_detected",
        "exact_duplicate_record_ids",
        "evidence",
    ] {
        assert!(intents[0].task.contains(field));
    }
    assert_eq!(
        intents[0].required_capabilities,
        vec![::contracts::AgentRuntimeCapability::MemoryProposal]
    );
}

#[tokio::test]
async fn agent_runtime_proposal_surfaces_terminal_failure_detail() {
    let control = Arc::new(TerminalProposalControl {
        failure: Some(
            "cognitive session TerminalRuntime: inference provider failed: core RPC closed".into(),
        ),
        ..TerminalProposalControl::default()
    });
    let proposer =
        AgentControlMemorySemanticProposal::new(control, MemoryPolicyConfig::default()).unwrap();

    let error = proposer
        .propose(
            "task-failed",
            &observation("proposal-failed", 0),
            MemoryRecordKindV1::SemanticFact,
        )
        .await
        .unwrap_err();

    let message = error.to_string();
    assert!(message.contains("runtime ended as Failed"));
    assert!(message.contains("core RPC closed"));
}
