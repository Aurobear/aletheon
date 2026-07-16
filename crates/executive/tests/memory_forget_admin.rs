use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use executive::core::config::ExecutiveConfig;
use executive::core::orchestrator::AletheonExecutive;
use executive::service::admin_service::{
    AdminResources, AdminService, AdminServiceError, AdminUseCases, SkillAdminPort,
};
use executive::service::request_use_cases::ProductionMemoryAdminUseCases;
use mnemosyne::{
    ForgetAuthority, ForgetPolicy, ForgetReceipt, ForgetSelector, MemoryAuthority, MemoryKind,
    MemoryMetadata, MemoryRecord, MemoryRecordId, MemoryScope, MemoryService, MemoryStatus,
    RetentionRepository,
};
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

struct NoopSkills;
#[async_trait]
impl SkillAdminPort for NoopSkills {
    async fn reload(&self) -> Result<usize, AdminServiceError> {
        Ok(0)
    }
}

struct RepositoryMemory {
    repository: Arc<RetentionRepository>,
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
        memory, repository, "owner",
    ));
    let service = AdminService::new(AdminResources {
        orchestrator: Arc::new(Mutex::new(AletheonExecutive::new(
            ExecutiveConfig::default(),
        ))),
        skills: Arc::new(NoopSkills),
        tool_catalog: Arc::new(|| Box::pin(async { vec![] })),
        hook_catalog: Arc::new(|| Box::pin(async { vec![] })),
        pending_approvals: Arc::new(Mutex::new(HashMap::new())),
        session_approvals: Arc::new(Mutex::new(HashMap::new())),
        daemon_cancel: CancellationToken::new(),
        google_sync: None,
        gbrain_worker: None,
        goal_worker: None,
        memory_admin: Some(memory_admin),
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
