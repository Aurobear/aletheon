use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use fabric::genome::{
    BoundarySpec, CareSpec, IdentitySpec, LifecycleSpec, MemorySpec, MutationSpec, Topology,
};
use fabric::meta::Recommendation;
use fabric::{
    ApprovalCategory, ApprovalId, ApprovalResolution, ApprovalRisk, ApprovalSnapshot,
    ApprovalStatus, ApprovalSubject, CapabilityId, CapabilityScope, Clock, Evaluation,
    ExecutionPermit, Genome, GoalId, MetaRuntimeOps, MigrationResult, MonoDeadline, MonoTime,
    MutationIntent, OperationId, PermitId, PrincipalId, ProcessId, RuntimeCandidate,
    SandboxDecision, Subsystem, SubsystemContext, SubsystemHealth, TestResult, Version,
};
use metacog::{
    ApplyMutation, DefaultMetacogService, GovernedMutationEvidence, MetacogError, MetacogService,
    MutationLifecycle, RetryDisposition, RollbackMutation, VerifyMutation,
};
use uuid::Uuid;

struct MockRuntime {
    migrations: AtomicUsize,
    rollbacks: AtomicUsize,
    fail_migration: bool,
}

impl MockRuntime {
    fn new() -> Self {
        Self {
            migrations: AtomicUsize::new(0),
            rollbacks: AtomicUsize::new(0),
            fail_migration: false,
        }
    }

    fn failing_migration() -> Self {
        Self {
            fail_migration: true,
            ..Self::new()
        }
    }
}

#[async_trait]
impl Subsystem for MockRuntime {
    fn name(&self) -> &str {
        "mock-metacog"
    }

    fn version(&self) -> Version {
        Version::new(1, 0, 0)
    }

    async fn init(&mut self, _ctx: &SubsystemContext) -> anyhow::Result<()> {
        Ok(())
    }

    async fn shutdown(&mut self) -> anyhow::Result<()> {
        Ok(())
    }

    async fn health(&self) -> SubsystemHealth {
        SubsystemHealth::Healthy
    }
}

#[async_trait]
impl MetaRuntimeOps for MockRuntime {
    async fn read_genome(&self) -> anyhow::Result<Genome> {
        Ok(genome("1.0.0"))
    }

    async fn generate_candidate(
        &self,
        _intent: &MutationIntent,
    ) -> anyhow::Result<RuntimeCandidate> {
        Ok(RuntimeCandidate {
            id: Uuid::from_u128(500),
            genome: genome("1.1.0"),
            changes: vec!["raise verification threshold".into()],
            generated_at: chrono::DateTime::UNIX_EPOCH,
        })
    }

    async fn sandbox_test(&self, _candidate: &RuntimeCandidate) -> anyhow::Result<TestResult> {
        Ok(TestResult {
            passed: true,
            tests_run: 1,
            tests_passed: 1,
            tests_failed: 0,
            failures: vec![],
            elapsed_ms: 4,
        })
    }

    async fn evaluate(
        &self,
        _candidate: &RuntimeCandidate,
        _test: &TestResult,
    ) -> anyhow::Result<Evaluation> {
        Ok(Evaluation {
            score: 0.95,
            strengths: vec!["bounded".into()],
            weaknesses: vec![],
            recommendation: Recommendation::Adopt,
        })
    }

    async fn migrate(&self, _candidate: &RuntimeCandidate) -> anyhow::Result<MigrationResult> {
        self.migrations.fetch_add(1, Ordering::SeqCst);
        if self.fail_migration {
            anyhow::bail!("injected ambiguous migration failure");
        }
        Ok(MigrationResult {
            success: true,
            from_version: "1.0.0".into(),
            to_version: "1.1.0".into(),
            memories_migrated: 0,
            identity_preserved: true,
            message: "applied".into(),
        })
    }

    async fn rollback(&self) -> anyhow::Result<()> {
        self.rollbacks.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    fn current_version(&self) -> Version {
        Version::new(1, 0, 0)
    }
}

fn genome(version: &str) -> Genome {
    Genome {
        topology: Topology { subsystems: vec![] },
        identity: IdentitySpec {
            name: "aletheon".into(),
            description: version.into(),
            self_model: "bounded".into(),
        },
        boundary: BoundarySpec { rules: vec![] },
        care: CareSpec { priorities: vec![] },
        memory: MemorySpec {
            backends: vec![],
            compaction_strategy: "none".into(),
        },
        mutation: MutationSpec {
            allowed_targets: vec!["policy".into()],
            require_sandbox: true,
            require_self_field_approval: true,
        },
        lifecycle: LifecycleSpec {
            auto_compact: false,
            health_check_interval_secs: 30,
            max_idle_time_secs: 60,
        },
    }
}

fn verify_request(mutation_id: Uuid) -> VerifyMutation {
    VerifyMutation {
        mutation_id,
        intent: MutationIntent {
            target: "policy".into(),
            change: serde_json::json!({"verification_threshold": 0.9}),
            reason: "reduce unsafe adoption".into(),
            reversible: true,
        },
    }
}

fn evidence(
    mutation_id: Uuid,
    operation: &str,
    binding_name: &str,
    binding_value: &str,
) -> GovernedMutationEvidence {
    let permit_seed = if operation == "apply" { 701 } else { 711 };
    let subject = ApprovalSubject {
        category: ApprovalCategory::DaseinModification,
        goal_id: GoalId(41),
        attempt_id: None,
        job_id: None,
        attributes: BTreeMap::from([
            ("mutation_id".into(), mutation_id.to_string()),
            ("operation".into(), operation.into()),
            (binding_name.into(), binding_value.into()),
        ]),
        allowed_scope: vec![],
        apply_target: None,
    };
    let subject_hash = subject.subject_hash().unwrap();
    GovernedMutationEvidence {
        permit: ExecutionPermit {
            id: PermitId(Uuid::from_u128(permit_seed)),
            operation_id: OperationId(Uuid::from_u128(permit_seed + 1)),
            process_id: ProcessId(Uuid::from_u128(permit_seed + 2)),
            capability: CapabilityId(format!("metacog.{operation}")),
            granted_scope: CapabilityScope::default(),
            expires_at: MonoDeadline(MonoTime(10_000)),
            sandbox: SandboxDecision::Passed,
            budget_reservation: None,
            lease: None,
        },
        approval: ApprovalSnapshot {
            id: ApprovalId(Uuid::from_u128(704)),
            goal_id: GoalId(41),
            attempt_id: None,
            job_id: None,
            owner_id: PrincipalId("operator".into()),
            category: ApprovalCategory::DaseinModification,
            risk: ApprovalRisk::Critical,
            subject,
            subject_hash,
            summary: format!("approve metacog {operation}"),
            artifacts: vec![],
            created_at_ms: 0,
            expires_at_ms: 10_000,
            status: ApprovalStatus::Approved,
            version: 1,
            resolution: Some(ApprovalResolution::approved(
                PrincipalId("operator".into()),
                "local_rpc",
                1,
            )),
        },
    }
}

#[tokio::test]
async fn verify_apply_status_and_reopen_share_one_durable_lineage() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("mutation-state.json");
    let clock: Arc<dyn Clock> = Arc::new(aletheon_kernel::chronos::TestClock::new(0, 0));
    let runtime = Arc::new(MockRuntime::new());
    let service =
        DefaultMetacogService::with_state_path(runtime.clone(), clock.clone(), path.clone())
            .unwrap();
    let mutation_id = Uuid::from_u128(42);

    let verification = service.verify(verify_request(mutation_id)).await.unwrap();
    let apply = ApplyMutation {
        evidence: evidence(
            mutation_id,
            "apply",
            "verification_hash",
            &verification.verification_hash,
        ),
        verification: verification.clone(),
    };
    let receipt = service.apply(apply.clone()).await.unwrap();
    let repeated = service.apply(apply).await.unwrap();

    assert_eq!(receipt.receipt_hash, repeated.receipt_hash);
    assert_eq!(runtime.migrations.load(Ordering::SeqCst), 1);
    let status = service.status().await.unwrap();
    assert_eq!(status.lineage.len(), 1);
    assert_eq!(status.lineage[0].lifecycle, MutationLifecycle::Applied);

    drop(service);
    let reopened =
        DefaultMetacogService::with_state_path(Arc::new(MockRuntime::new()), clock, path).unwrap();
    let status = reopened.status().await.unwrap();
    assert_eq!(status.lineage.len(), 1);
    assert_eq!(
        status.lineage[0].receipt.as_ref().unwrap().receipt_hash,
        receipt.receipt_hash
    );
}

#[tokio::test]
async fn apply_and_rollback_fail_closed_without_bound_governance_evidence() {
    let clock: Arc<dyn Clock> = Arc::new(aletheon_kernel::chronos::TestClock::new(0, 0));
    let runtime = Arc::new(MockRuntime::new());
    let service = DefaultMetacogService::in_memory(runtime.clone(), clock);
    let mutation_id = Uuid::from_u128(43);
    let verification = service.verify(verify_request(mutation_id)).await.unwrap();
    let wrong = ApplyMutation {
        evidence: evidence(mutation_id, "rollback", "verification_hash", "wrong"),
        verification: verification.clone(),
    };

    let error = service.apply(wrong).await.unwrap_err();
    assert!(matches!(error, MetacogError::Unauthorized(_)));
    assert_eq!(error.retry_disposition(), RetryDisposition::Never);
    assert_eq!(runtime.migrations.load(Ordering::SeqCst), 0);

    let applied = service
        .apply(ApplyMutation {
            evidence: evidence(
                mutation_id,
                "apply",
                "verification_hash",
                &verification.verification_hash,
            ),
            verification,
        })
        .await
        .unwrap();
    let rollback_error = service
        .rollback(RollbackMutation {
            mutation_id,
            applied_receipt_hash: applied.receipt_hash.clone(),
            evidence: evidence(mutation_id, "rollback", "applied_receipt_hash", "forged"),
        })
        .await
        .unwrap_err();
    assert!(matches!(rollback_error, MetacogError::Unauthorized(_)));
    assert_eq!(runtime.rollbacks.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn governed_rollback_is_idempotent_and_persisted() {
    let clock: Arc<dyn Clock> = Arc::new(aletheon_kernel::chronos::TestClock::new(0, 0));
    let runtime = Arc::new(MockRuntime::new());
    let service = DefaultMetacogService::in_memory(runtime.clone(), clock);
    let mutation_id = Uuid::from_u128(44);
    let verification = service.verify(verify_request(mutation_id)).await.unwrap();
    let applied = service
        .apply(ApplyMutation {
            evidence: evidence(
                mutation_id,
                "apply",
                "verification_hash",
                &verification.verification_hash,
            ),
            verification,
        })
        .await
        .unwrap();
    let request = RollbackMutation {
        mutation_id,
        applied_receipt_hash: applied.receipt_hash.clone(),
        evidence: evidence(
            mutation_id,
            "rollback",
            "applied_receipt_hash",
            &applied.receipt_hash,
        ),
    };

    let first = service.rollback(request.clone()).await.unwrap();
    let second = service.rollback(request).await.unwrap();
    assert_eq!(first.receipt_hash, second.receipt_hash);
    assert_eq!(runtime.rollbacks.load(Ordering::SeqCst), 1);
    assert_eq!(
        service.status().await.unwrap().lineage[0].lifecycle,
        MutationLifecycle::RolledBack
    );
}

#[tokio::test]
async fn ambiguous_apply_is_durable_and_never_replayed() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("mutation-state.json");
    let clock: Arc<dyn Clock> = Arc::new(aletheon_kernel::chronos::TestClock::new(0, 0));
    let runtime = Arc::new(MockRuntime::failing_migration());
    let service =
        DefaultMetacogService::with_state_path(runtime.clone(), clock.clone(), path.clone())
            .unwrap();
    let mutation_id = Uuid::from_u128(45);
    let verification = service.verify(verify_request(mutation_id)).await.unwrap();
    let request = ApplyMutation {
        evidence: evidence(
            mutation_id,
            "apply",
            "verification_hash",
            &verification.verification_hash,
        ),
        verification,
    };

    let first = service.apply(request.clone()).await.unwrap_err();
    assert!(matches!(first, MetacogError::ReconciliationRequired(_)));
    assert_eq!(first.retry_disposition(), RetryDisposition::Never);
    let second = service.apply(request).await.unwrap_err();
    assert!(matches!(second, MetacogError::ReconciliationRequired(_)));
    assert_eq!(runtime.migrations.load(Ordering::SeqCst), 1);

    drop(service);
    let reopened =
        DefaultMetacogService::with_state_path(Arc::new(MockRuntime::new()), clock, path).unwrap();
    assert_eq!(
        reopened.status().await.unwrap().lineage[0].lifecycle,
        MutationLifecycle::Applying
    );
}
