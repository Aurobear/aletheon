use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use executive::application::admin_service::{
    AdminResources, AdminRuntimePort, AdminService, AdminServiceError, AdminUseCases, ModeChange,
    SkillAdminPort,
};
use executive::application::request_use_cases::{ProductionMemoryAdminUseCases, RetentionAdminPort};
use mnemosyne::{
    ForgetAuthority, ForgetPolicy, ForgetReceipt, ForgetSelector, MemoryAuthority, MemoryKind,
    MemoryMetadata, MemoryRecord, MemoryRecordId, MemoryScope, MemoryService, MemoryStatus,
    RetentionRepository,
};
use tokio_util::sync::CancellationToken;

struct NoopSkills;

struct NoopAdminRuntime;

#[async_trait]
impl AdminRuntimePort for NoopAdminRuntime {
    async fn request_interrupt(&self, _reason: fabric::ui_event::InterruptReason) {}

    async fn switch_mode(&self, mode: fabric::ui_event::CollaborationMode) -> ModeChange {
        ModeChange {
            old: fabric::ui_event::CollaborationMode::Default,
            new: mode,
        }
    }
}
#[async_trait]
impl SkillAdminPort for NoopSkills {
    async fn reload(&self) -> Result<usize, AdminServiceError> {
        Ok(0)
    }
}

struct RepositoryMemory {
    repository: Arc<RetentionRepository>,
}

struct RepositoryRetentionAdmin {
    repository: Arc<RetentionRepository>,
}

impl RetentionAdminPort for RepositoryRetentionAdmin {
    fn compact(
        &self,
        owner: &str,
        now_ms: i64,
        policy: &mnemosyne::RetentionCompactionPolicy,
    ) -> anyhow::Result<mnemosyne::RetentionCompactionReport> {
        mnemosyne::RetentionCompactor::new(&self.repository).run(owner, now_ms, policy)
    }
}
#[async_trait]
impl MemoryService for RepositoryMemory {
    async fn record(&self, _: mnemosyne::ExperienceEvent) -> anyhow::Result<()> {
        Ok(())
    }
    async fn recall(&self, _: mnemosyne::RecallRequest) -> anyhow::Result<mnemosyne::RecallSet> {
        Ok(Default::default())
    }
    async fn consolidate(&self, _: MemoryScope) -> anyhow::Result<()> {
        Ok(())
    }
    async fn preview_forget(&self, policy: ForgetPolicy) -> anyhow::Result<ForgetReceipt> {
        self.repository.preview_forget(&policy, 10)
    }
    async fn forget(&self, policy: ForgetPolicy) -> anyhow::Result<ForgetReceipt> {
        self.repository.forget(&policy, 11)
    }
}

fn policy(requester: &str) -> ForgetPolicy {
    ForgetPolicy {
        request_id: "admin-forget-1".into(),
        selector: ForgetSelector::Exact {
            record_ids: vec![MemoryRecordId("decision-1".into())],
            within: MemoryScope::Global,
        },
        requester: requester.into(),
        reason: "authenticated erasure".into(),
        authority: ForgetAuthority::Elevated {
            proof: "admin-approval".into(),
        },
    }
}

#[tokio::test]
async fn authenticated_admin_requires_preview_and_returns_durable_receipt() {
    let dir = tempfile::tempdir().unwrap();
    let repository = Arc::new(RetentionRepository::open(dir.path().join("retention.db")).unwrap());
    repository
        .register(
            &MemoryRecord {
                id: MemoryRecordId("decision-1".into()),
                kind: MemoryKind::ArchitectureDecision,
                scope: MemoryScope::Global,
                content: "retire the legacy policy".into(),
                metadata: MemoryMetadata::local(
                    "decision-1",
                    "event-1",
                    DateTime::<Utc>::UNIX_EPOCH,
                ),
                status: MemoryStatus::Current,
                authority: MemoryAuthority::LocalEpisode,
                source_event_ids: vec!["event-1".into()],
                tags: Vec::new(),
            },
            0,
        )
        .unwrap();
    let memory: Arc<dyn MemoryService> = Arc::new(RepositoryMemory {
        repository: repository.clone(),
    });
    let memory_admin = Arc::new(ProductionMemoryAdminUseCases::new(
        memory,
        Arc::new(RepositoryRetentionAdmin { repository }),
        "owner",
    ));
    let service = AdminService::new(AdminResources {
        runtime: Arc::new(NoopAdminRuntime),
        skills: Arc::new(NoopSkills),
        tool_catalog: Arc::new(|| Box::pin(async { vec![] })),
        hook_catalog: Arc::new(|| Box::pin(async { vec![] })),
        pending_approvals: executive::application::admin_service::PendingApprovals::default(),
        session_approvals: executive::application::admin_service::ScopedApprovalCache::default(),
        daemon_cancel: CancellationToken::new(),
        external_sync: None,
        supplemental_memory_worker: None,
        goal_worker: None,
        runtime_shutdown: Arc::new(|| Box::pin(async { Ok(()) })),
        memory_admin: Some(memory_admin),
        agent_runs: None,
        agent_profiles: None,
        current_profile: None,
        profile_switch_events: Arc::new(
            executive::application::admin_service::NoopProfileSwitchEventSink,
        ),
        deployment_rollback: None,
    });
    assert!(
        service.forget_memory(policy("owner")).await.is_err(),
        "elevated execution cannot bypass preview"
    );
    assert!(
        service
            .preview_memory_forget(policy("attacker"))
            .await
            .is_err(),
        "requester is bound to authenticated principal"
    );
    assert!(service
        .preview_memory_forget(policy("owner"))
        .await
        .unwrap()
        .denied
        .is_empty());
    let receipt = service.forget_memory(policy("owner")).await.unwrap();
    assert_eq!(
        receipt.tombstoned,
        vec![MemoryRecordId("decision-1".into())]
    );
    assert_eq!(
        service.forget_memory(policy("owner")).await.unwrap(),
        receipt
    );
}
