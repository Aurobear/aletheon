//! Explicit child-agent resource settlement state machine.
//!
//! The engine deliberately depends on narrow ports. Resource ownership changes,
//! receipt durability, and lease deletion remain implemented by their owning
//! subsystems, while this module provides ordering, policy, and replay safety.

use std::sync::Arc;

#[cfg(test)]
use ::contracts::{
    AgentControlError, AgentControlErrorKind, AgentResourceClass, BackgroundResourceDecl,
    OperationId, ReparentContext, SettlementPhase, SettlementTerminal,
};
use async_trait::async_trait;
use runtime::EventSpine;

#[cfg(test)]
use super::AgentAdmissionLease;
pub use runtime::{
    recovery_disposition, settle_admission, terminal_with_memory_flush,
    FailClosedSettlementResourcePort, InMemorySettlementReceiptStore,
    ManagedSettlementResourcePort, NoopSettlementEvidenceSink, RecoveryResourceDisposition,
    RepositorySettlementLeasePort, SettlementEngine, SettlementEvidence, SettlementEvidenceSink,
    SettlementLeasePort, SettlementMetricSnapshot, SettlementMetrics, SettlementQuiescePort,
    SettlementReceiptStore, SettlementRequest, SettlementResourcePort, SpineSettlementEvidenceSink,
};

#[cfg(test)]
fn invalid(message: impl Into<String>) -> AgentControlError {
    AgentControlError {
        kind: AgentControlErrorKind::InvalidRequest,
        message: message.into(),
    }
}

pub struct AgentEvaluationProjectionSink {
    inner: crate::wiring::application::post_turn_projection::DurableDomainEvaluationSink,
}

impl AgentEvaluationProjectionSink {
    pub fn new(spine: Arc<dyn EventSpine>) -> Self {
        Self {
            inner:
                crate::wiring::application::post_turn_projection::DurableDomainEvaluationSink::new(
                    "agent_control",
                    spine,
                ),
        }
    }
}

#[async_trait]
impl application::evaluation_projection::EvaluationProjectionSink
    for AgentEvaluationProjectionSink
{
    fn name(&self) -> &'static str {
        "agent_control"
    }

    async fn project(
        &self,
        record: &application::evaluation_projection::EvaluationProjectionRecord,
    ) -> anyhow::Result<()> {
        application::evaluation_projection::EvaluationProjectionSink::project(&self.inner, record)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ::contracts::AgentRecoveryDecision;
    use adapters_sqlite::SqliteSettlementReceiptStore;
    use parking_lot::Mutex as ParkingMutex;
    mod audit_tests;
    #[derive(Default)]
    struct FakeAdmission {
        settled: usize,
        revoked: usize,
    }

    #[async_trait]
    impl AgentAdmissionLease for FakeAdmission {
        async fn mark_running(&mut self) -> Result<(), AgentControlError> {
            Ok(())
        }

        async fn settle(
            &mut self,
            _usage: &::contracts::AttemptUsage,
        ) -> Result<(), AgentControlError> {
            self.settled += 1;
            Ok(())
        }

        async fn revoke(&mut self) -> Result<(), AgentControlError> {
            self.revoked += 1;
            Ok(())
        }

        async fn transfer_remaining_to(
            &mut self,
            _parent: ::contracts::AgentId,
            _usage: &::contracts::AttemptUsage,
        ) -> Result<::contracts::BudgetTransferReceipt, AgentControlError> {
            Err(invalid("fake admission transfer is unavailable"))
        }
    }

    #[derive(Default)]
    struct FakeResources {
        context: ParkingMutex<Option<ReparentContext>>,
        foreground: ParkingMutex<Vec<String>>,
        terminated: ParkingMutex<Vec<String>>,
        reparented: ParkingMutex<Vec<(String, String, String)>>,
        fail_foreground: ParkingMutex<bool>,
    }

    impl FakeResources {
        fn allowing() -> Self {
            Self {
                context: ParkingMutex::new(Some(ReparentContext {
                    parent_authority_covers: true,
                    parent_budget_accepts: true,
                    notification_route_transferable: true,
                })),
                ..Default::default()
            }
        }
    }

    #[async_trait]
    impl SettlementResourcePort for FakeResources {
        fn reparent_context(
            &self,
            _resource: &BackgroundResourceDecl,
            _parent_owner: &str,
        ) -> ReparentContext {
            self.context.lock().clone().unwrap_or(ReparentContext {
                parent_authority_covers: false,
                parent_budget_accepts: false,
                notification_route_transferable: false,
            })
        }

        async fn settle_foreground(
            &self,
            resource: &BackgroundResourceDecl,
            _action_key: &str,
        ) -> Result<(), AgentControlError> {
            self.foreground.lock().push(resource.resource_id.clone());
            if *self.fail_foreground.lock() {
                Err(runtime("foreground still live"))
            } else {
                Ok(())
            }
        }

        async fn terminate(
            &self,
            resource: &BackgroundResourceDecl,
            _reason: &str,
            _action_key: &str,
        ) -> Result<(), AgentControlError> {
            self.terminated.lock().push(resource.resource_id.clone());
            Ok(())
        }

        async fn reparent(
            &self,
            resource: &BackgroundResourceDecl,
            old_owner: &str,
            new_owner: &str,
            _action_key: &str,
        ) -> Result<(), AgentControlError> {
            self.reparented.lock().push((
                resource.resource_id.clone(),
                old_owner.into(),
                new_owner.into(),
            ));
            Ok(())
        }
    }

    #[derive(Default)]
    struct FakeLeases {
        calls: ParkingMutex<Vec<(String, String)>>,
    }

    #[async_trait]
    impl SettlementLeasePort for FakeLeases {
        async fn release(
            &self,
            lease_key: &str,
            expected_owner: &str,
        ) -> Result<bool, AgentControlError> {
            let mut calls = self.calls.lock();
            let call = (lease_key.to_string(), expected_owner.to_string());
            if calls.contains(&call) {
                return Ok(false);
            }
            calls.push(call);
            Ok(true)
        }
    }

    #[derive(Default)]
    struct FakeEvidence(ParkingMutex<Vec<SettlementEvidence>>);

    #[async_trait]
    impl SettlementEvidenceSink for FakeEvidence {
        async fn record(&self, evidence: SettlementEvidence) -> Result<(), AgentControlError> {
            self.0.lock().push(evidence);
            Ok(())
        }
    }

    struct Harness {
        engine: SettlementEngine,
        resources: Arc<FakeResources>,
        leases: Arc<FakeLeases>,
        evidence: Arc<FakeEvidence>,
    }

    fn harness(resources: FakeResources) -> Harness {
        let resources = Arc::new(resources);
        let leases = Arc::new(FakeLeases::default());
        let evidence = Arc::new(FakeEvidence::default());
        Harness {
            engine: SettlementEngine::new(
                Arc::new(InMemorySettlementReceiptStore::default()),
                resources.clone(),
                leases.clone(),
                evidence.clone(),
            ),
            resources,
            leases,
            evidence,
        }
    }

    fn request() -> SettlementRequest {
        SettlementRequest {
            agent_id: "agent".into(),
            attempt_id: "attempt".into(),
            generation: "generation".into(),
            old_owner: "child".into(),
            parent_owner: Some("parent".into()),
            terminal: SettlementTerminal::Completed,
            lease_keys: vec!["execution".into(), "admission".into()],
            settled_at_ms: 42,
        }
    }

    fn resource(
        id: &str,
        class: AgentResourceClass,
        survive_child: bool,
    ) -> BackgroundResourceDecl {
        BackgroundResourceDecl {
            resource_id: id.into(),
            class,
            survive_child,
        }
    }

    #[tokio::test]
    async fn settlement_replay_does_not_repeat_release_or_reparent() {
        let harness = harness(FakeResources::allowing());
        let resources = vec![resource(
            "background",
            AgentResourceClass::BackgroundCommand,
            true,
        )];
        let first = harness
            .engine
            .settle(request(), resources.clone())
            .await
            .unwrap();
        let replay = harness.engine.settle(request(), resources).await.unwrap();

        assert_eq!(first, replay);
        assert_eq!(harness.resources.reparented.lock().len(), 1);
        assert_eq!(harness.leases.calls.lock().len(), 2);
        assert!(harness
            .evidence
            .0
            .lock()
            .iter()
            .any(|event| matches!(event, SettlementEvidence::IdempotentReplay { .. })));
    }

    #[tokio::test]
    async fn metrics_are_fixed_cardinality_and_count_reparent_denial_and_replay() {
        let resources = Arc::new(FakeResources::allowing());
        let metrics = Arc::new(SettlementMetrics::default());
        let engine = SettlementEngine::with_metrics(
            Arc::new(InMemorySettlementReceiptStore::default()),
            resources.clone(),
            Arc::new(FakeLeases::default()),
            Arc::new(FakeEvidence::default()),
            metrics.clone(),
        );
        let allowed = vec![resource(
            "background",
            AgentResourceClass::BackgroundCommand,
            true,
        )];
        engine.settle(request(), allowed.clone()).await.unwrap();
        engine.settle(request(), allowed).await.unwrap();

        *resources.context.lock() = None;
        let mut denied_request = request();
        denied_request.attempt_id = "denied-attempt".into();
        engine
            .settle(
                denied_request,
                vec![resource(
                    "notify",
                    AgentResourceClass::NotificationRoute,
                    true,
                )],
            )
            .await
            .unwrap();

        let snapshot = metrics.snapshot();
        assert_eq!(snapshot.reparent_background_command_total, 1);
        assert_eq!(snapshot.reparent_notification_route_total, 0);
        assert_eq!(snapshot.reparent_denied_total, 1);
        assert_eq!(snapshot.settlement_idempotent_replay_total, 1);
        let names = snapshot.named().map(|(name, _)| name);
        assert_eq!(names.len(), 5);
        assert_eq!(
            names,
            [
                "settlement_duration_ms",
                "reparent_total.background_command",
                "reparent_total.notification_route",
                "reparent_denied_total",
                "settlement_idempotent_replay_total",
            ]
            .map(String::from)
        );
    }

    #[tokio::test]
    async fn undeclared_survivor_is_killed_and_never_reparented() {
        let harness = harness(FakeResources::allowing());
        let receipt = harness
            .engine
            .settle(
                request(),
                vec![resource(
                    "background",
                    AgentResourceClass::BackgroundCommand,
                    false,
                )],
            )
            .await
            .unwrap();
        assert!(receipt.reparented.is_empty());
        assert_eq!(&*harness.resources.terminated.lock(), &["background"]);
    }

    #[tokio::test]
    async fn budget_or_authority_denial_kills_resource_with_evidence() {
        let resources = FakeResources::allowing();
        resources
            .context
            .lock()
            .as_mut()
            .unwrap()
            .parent_budget_accepts = false;
        let harness = harness(resources);
        harness
            .engine
            .settle(
                request(),
                vec![resource(
                    "background",
                    AgentResourceClass::BackgroundCommand,
                    true,
                )],
            )
            .await
            .unwrap();
        assert_eq!(&*harness.resources.terminated.lock(), &["background"]);
        assert!(harness.evidence.0.lock().iter().any(|event| matches!(
            event,
            SettlementEvidence::ResourceTerminated { reason, .. }
                if reason.contains("budget")
        )));
    }

    #[tokio::test]
    async fn transferable_notification_route_is_reparented_with_receipt() {
        let harness = harness(FakeResources::allowing());
        let receipt = harness
            .engine
            .settle(
                request(),
                vec![resource(
                    "notify",
                    AgentResourceClass::NotificationRoute,
                    true,
                )],
            )
            .await
            .unwrap();
        assert_eq!(receipt.reparented.len(), 1);
        assert_eq!(receipt.reparented[0].old_owner, "child");
        assert_eq!(receipt.reparented[0].new_owner, "parent");
    }

    #[tokio::test]
    async fn foreground_failure_forces_termination_before_terminal() {
        let resources = FakeResources::allowing();
        *resources.fail_foreground.lock() = true;
        let harness = harness(resources);
        let receipt = harness
            .engine
            .settle(
                request(),
                vec![resource(
                    "foreground",
                    AgentResourceClass::ForegroundCommand,
                    false,
                )],
            )
            .await
            .unwrap();
        assert!(matches!(
            receipt.terminal,
            SettlementTerminal::Failed { .. }
        ));
        assert_eq!(&*harness.resources.terminated.lock(), &["foreground"]);
    }

    #[test]
    fn recovery_decisions_distinguish_resume_finalize_and_reclaim() {
        assert_eq!(
            recovery_disposition(AgentRecoveryDecision::Resume),
            RecoveryResourceDisposition::RetainForResume
        );
        assert_eq!(
            recovery_disposition(AgentRecoveryDecision::Finalize),
            RecoveryResourceDisposition::ReplaySettlement
        );
        assert_eq!(
            recovery_disposition(AgentRecoveryDecision::Reclaim),
            RecoveryResourceDisposition::TerminateAndReclaim
        );
        assert_eq!(
            recovery_disposition(AgentRecoveryDecision::Interrupt),
            RecoveryResourceDisposition::TerminateAndReclaim
        );
    }

    #[tokio::test]
    async fn resume_recovery_disposition_retains_resources_without_settlement() {
        let harness = harness(FakeResources::allowing());
        let resources = vec![resource(
            "background",
            AgentResourceClass::BackgroundCommand,
            true,
        )];
        if recovery_disposition(AgentRecoveryDecision::Resume)
            != RecoveryResourceDisposition::RetainForResume
        {
            harness.engine.settle(request(), resources).await.unwrap();
        }
        assert!(harness.resources.reparented.lock().is_empty());
        assert!(harness.resources.terminated.lock().is_empty());
        assert!(harness.leases.calls.lock().is_empty());
    }

    #[tokio::test]
    async fn crash_recovery_settlement_reopens_idempotently_and_owner_checks_leases() {
        use super::super::{
            agent_workspace_id, AgentResourceLease, AgentResourceLeaseKind, AgentRunProjection,
            AgentRunRecord,
        };
        use ::contracts::{
            AgentBudget, AgentContextFork, AgentHandle, AgentProfileId, AgentRunStatus,
            AgentSnapshot, AgentSpawnRequest, OperationId, ProcessId, RuntimeId,
            RuntimeResumability,
        };
        use adapters_sqlite::runtime_agent::SqliteAgentRunProjection;

        let path = std::env::temp_dir().join(format!(
            "aletheon-settlement-reopen-{}.sqlite",
            uuid::Uuid::new_v4()
        ));
        let agent = ::contracts::AgentId::new();
        let matching = AgentResourceLease {
            lease_key: "execution:matching".into(),
            agent_id: agent,
            kind: AgentResourceLeaseKind::Execution,
            owner: "process:child".into(),
            expires_at_ms: 100,
            worktree_root: None,
            worktree_path: None,
            expected_head: None,
        };
        let wrong_owner = AgentResourceLease {
            lease_key: "execution:wrong-owner".into(),
            owner: "process:other".into(),
            ..matching.clone()
        };
        let repository = Arc::new(SqliteAgentRunProjection::open(&path).unwrap());
        let process_id = ProcessId::new();
        let request = AgentSpawnRequest {
            root_agent_id: agent,
            parent_agent_id: None,
            parent_process_id: None,
            profile_id: AgentProfileId("worker".into()),
            runtime_id: RuntimeId("test".into()),
            trusted_workspace: None,
            delegator_authority: None,
            cognitive_binding: None,
            task: "settlement recovery fixture".into(),
            context: AgentContextFork::None,
            broadcast_refs: vec![],
            allowed_tools: vec![],
            background_decls: vec![],
            budget: AgentBudget {
                max_input_tokens: 1,
                max_output_tokens: 1,
                max_tool_calls: 1,
                max_elapsed_ms: 1,
                max_cost_usd: None,
                max_depth: 1,
            },
        };
        repository
            .create(&AgentRunRecord {
                snapshot: AgentSnapshot {
                    handle: AgentHandle {
                        agent_id: agent,
                        root_agent_id: agent,
                        parent_agent_id: None,
                        process_id,
                        operation_id: OperationId::new(),
                        runtime_id: request.runtime_id.clone(),
                        profile_id: request.profile_id.clone(),
                    },
                    status: AgentRunStatus::Queued,
                    result: None,
                    created_at_ms: 0,
                    started_at_ms: None,
                    ended_at_ms: None,
                    last_error: None,
                },
                request_hash: SqliteAgentRunProjection::request_hash(&request).unwrap(),
                workspace_id: agent_workspace_id(agent),
                root_process_id: process_id,
                broadcast_refs: vec![],
                request,
                version: 0,
                retain_until_ms: 1_000,
                resumability: RuntimeResumability::Never,
                recovery: None,
            })
            .await
            .unwrap();
        repository.put_resource_lease(&matching).await.unwrap();
        repository.put_resource_lease(&wrong_owner).await.unwrap();
        let recovery_request = SettlementRequest {
            agent_id: agent.0.to_string(),
            attempt_id: "restart-attempt".into(),
            generation: "restart-generation".into(),
            old_owner: "process:child".into(),
            parent_owner: None,
            terminal: SettlementTerminal::Failed {
                reason: "restart".into(),
            },
            lease_keys: vec![matching.lease_key.clone(), wrong_owner.lease_key.clone()],
            settled_at_ms: 200,
        };
        SqliteSettlementReceiptStore::migrate(&path).unwrap();
        let engine = SettlementEngine::new(
            Arc::new(SqliteSettlementReceiptStore::open(&path).unwrap()),
            Arc::new(FailClosedSettlementResourcePort::new(
                tokio_util::sync::CancellationToken::new(),
            )),
            Arc::new(RepositorySettlementLeasePort::new(repository.clone())),
            Arc::new(NoopSettlementEvidenceSink),
        );
        let first = engine
            .settle(recovery_request.clone(), Vec::new())
            .await
            .unwrap();
        assert_eq!(first.released_leases, vec![matching.lease_key.clone()]);
        let remaining = repository
            .list_agent_resource_leases(agent, 10)
            .await
            .unwrap();
        assert_eq!(remaining, vec![wrong_owner.clone()]);
        drop(engine);
        drop(repository);

        let reopened_repository = Arc::new(SqliteAgentRunProjection::open(&path).unwrap());
        let reopened = SettlementEngine::new(
            Arc::new(SqliteSettlementReceiptStore::open(&path).unwrap()),
            Arc::new(FailClosedSettlementResourcePort::new(
                tokio_util::sync::CancellationToken::new(),
            )),
            Arc::new(RepositorySettlementLeasePort::new(
                reopened_repository.clone(),
            )),
            Arc::new(NoopSettlementEvidenceSink),
        );
        let replay = reopened.settle(recovery_request, Vec::new()).await.unwrap();
        assert_eq!(replay, first);
        assert_eq!(
            reopened_repository
                .list_agent_resource_leases(agent, 10)
                .await
                .unwrap(),
            vec![wrong_owner]
        );
        drop(reopened);
        drop(reopened_repository);
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn admission_adapter_settles_success_and_revokes_failure() {
        let usage = ::contracts::AttemptUsage::default();
        let mut succeeded = FakeAdmission::default();
        settle_admission(&mut succeeded, &SettlementTerminal::Completed, Some(&usage))
            .await
            .unwrap();
        assert_eq!((succeeded.settled, succeeded.revoked), (1, 0));

        let mut failed = FakeAdmission::default();
        settle_admission(
            &mut failed,
            &SettlementTerminal::Failed {
                reason: "runtime".into(),
            },
            Some(&usage),
        )
        .await
        .unwrap();
        assert_eq!((failed.settled, failed.revoked), (0, 1));
    }

    #[test]
    fn memory_flush_error_prevents_completed_receipt() {
        let terminal = terminal_with_memory_flush(
            SettlementTerminal::Completed,
            Some(runtime("vault unavailable")),
        );
        assert!(matches!(
            terminal,
            SettlementTerminal::Failed { reason } if reason.contains("vault unavailable")
        ));
    }

    fn runtime(message: &str) -> AgentControlError {
        AgentControlError {
            kind: AgentControlErrorKind::Runtime,
            message: message.into(),
        }
    }
}
