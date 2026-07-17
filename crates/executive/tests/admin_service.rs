use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use corpus::security::approval::ApprovalDecision;
use executive::core::config::ExecutiveConfig;
use executive::core::orchestrator::AletheonExecutive;
use executive::service::admin_service::{
    AdminResources, AdminService, AdminServiceError, AdminUseCases, ApprovalOwner,
    DefaultSkillAdmin, PendingApprovals, ScopedApprovalCache, SkillAdminPort,
    TransientApprovalRequest,
};
use fabric::ui_event::{CollaborationMode, InterruptReason};
use tempfile::tempdir;
use tokio::sync::{oneshot, Mutex};
use tokio_util::sync::CancellationToken;

struct FailingSkillAdmin;

fn noop_runtime_shutdown(
) -> Arc<dyn Fn() -> executive::service::admin_service::RuntimeShutdownFuture + Send + Sync> {
    Arc::new(|| Box::pin(async { Ok(()) }))
}

#[async_trait::async_trait]
impl SkillAdminPort for FailingSkillAdmin {
    async fn reload(&self) -> Result<usize, AdminServiceError> {
        Err(AdminServiceError::Operation("reload failed".into()))
    }
}

fn setup(skills_dir: std::path::PathBuf) -> (AdminService, CancellationToken, Arc<Mutex<String>>) {
    let cancellation = CancellationToken::new();
    let cached_prefix = Arc::new(Mutex::new(String::new()));
    let skills = Arc::new(DefaultSkillAdmin::new(
        Arc::new(Mutex::new(corpus::SkillLoader::new(skills_dir))),
        cached_prefix.clone(),
        "system prompt".into(),
    ));
    let service = AdminService::new(AdminResources {
        orchestrator: Arc::new(Mutex::new(AletheonExecutive::new(
            ExecutiveConfig::default(),
        ))),
        skills,
        tool_catalog: Arc::new(|| Box::pin(async { vec![] })),
        hook_catalog: Arc::new(|| Box::pin(async { vec![] })),
        pending_approvals: PendingApprovals::default(),
        session_approvals: ScopedApprovalCache::default(),
        daemon_cancel: cancellation.clone(),
        google_sync: None,
        gbrain_worker: None,
        goal_worker: None,
        runtime_shutdown: noop_runtime_shutdown(),
        memory_admin: None,
        agent_runs: None,
    });
    (service, cancellation, cached_prefix)
}

#[tokio::test]
async fn skill_reload_rebuilds_prefix_and_missing_directory_is_bounded() {
    let directory = tempdir().unwrap();
    std::fs::write(
        directory.path().join("review.md"),
        "# Review\nReview changes safely.\n\nUse focused checks.\n",
    )
    .unwrap();
    let (service, _, prefix) = setup(directory.path().to_path_buf());
    assert_eq!(service.reload_skills().await.unwrap(), 1);
    assert!(prefix.lock().await.contains("Review"));

    let (missing, _, _) = setup(directory.path().join("missing"));
    assert_eq!(missing.reload_skills().await.unwrap(), 0);
}

#[tokio::test]
async fn skill_reload_failure_is_propagated_without_partial_protocol_state() {
    let cancellation = CancellationToken::new();
    let service = AdminService::new(AdminResources {
        orchestrator: Arc::new(Mutex::new(AletheonExecutive::new(
            ExecutiveConfig::default(),
        ))),
        skills: Arc::new(FailingSkillAdmin),
        tool_catalog: Arc::new(|| Box::pin(async { vec![] })),
        hook_catalog: Arc::new(|| Box::pin(async { vec![] })),
        pending_approvals: PendingApprovals::default(),
        session_approvals: ScopedApprovalCache::default(),
        daemon_cancel: cancellation,
        google_sync: None,
        gbrain_worker: None,
        goal_worker: None,
        runtime_shutdown: noop_runtime_shutdown(),
        memory_admin: None,
        agent_runs: None,
    });
    assert!(matches!(
        service.reload_skills().await,
        Err(AdminServiceError::Operation(message)) if message == "reload failed"
    ));
}

#[tokio::test]
async fn mode_interrupt_catalogs_and_lists_use_the_port() {
    let directory = tempdir().unwrap();
    let (service, _, _) = setup(directory.path().to_path_buf());
    let change = service.switch_mode(CollaborationMode::Plan).await.unwrap();
    assert_eq!(change.old, CollaborationMode::Default);
    assert_eq!(change.new, CollaborationMode::Plan);
    service
        .interrupt(InterruptReason::UserCancelled)
        .await
        .unwrap();
    assert_eq!(service.model_catalog().await.unwrap().models.len(), 4);
    assert_eq!(
        service.switch_model("custom".into()).await.unwrap(),
        "custom"
    );
    assert!(service.tools().await.unwrap().is_empty());
    assert!(service.hooks().await.unwrap().is_empty());
    assert!(service.sub_agents().await.unwrap().is_empty());
}

#[tokio::test]
async fn transient_approval_and_shutdown_are_owned_by_admin_service() {
    let directory = tempdir().unwrap();
    let cancellation = CancellationToken::new();
    let runtime_shutdowns = Arc::new(AtomicUsize::new(0));
    let runtime_shutdowns_for_hook = runtime_shutdowns.clone();
    let (sender, receiver) = oneshot::channel();
    let pending = PendingApprovals::default();
    let session = ScopedApprovalCache::default();
    let principal_id = fabric::PrincipalId::local_uid(1001);
    let connection_id = fabric::ConnectionId::new();
    let thread_id = fabric::ThreadId("thread-a".into());
    let approval_id = pending
        .insert(
            ApprovalOwner::new(principal_id.clone(), thread_id.clone()),
            fabric::TurnId::new(),
            "call-1".into(),
            "bash_exec".into(),
            connection_id.clone(),
            sender,
        )
        .await;
    let service = AdminService::new(AdminResources {
        orchestrator: Arc::new(Mutex::new(AletheonExecutive::new(
            ExecutiveConfig::default(),
        ))),
        skills: Arc::new(DefaultSkillAdmin::new(
            Arc::new(Mutex::new(corpus::SkillLoader::new(
                directory.path().to_path_buf(),
            ))),
            Arc::new(Mutex::new(String::new())),
            String::new(),
        )),
        tool_catalog: Arc::new(|| Box::pin(async { vec![] })),
        hook_catalog: Arc::new(|| Box::pin(async { vec![] })),
        pending_approvals: pending,
        session_approvals: session.clone(),
        daemon_cancel: cancellation.clone(),
        google_sync: None,
        gbrain_worker: None,
        goal_worker: None,
        runtime_shutdown: Arc::new(move || {
            let runtime_shutdowns = runtime_shutdowns_for_hook.clone();
            Box::pin(async move {
                runtime_shutdowns.fetch_add(1, Ordering::SeqCst);
                Ok(())
            })
        }),
        memory_admin: None,
        agent_runs: None,
    });

    assert!(service
        .resolve_transient_approval(TransientApprovalRequest {
            principal_id: principal_id.clone(),
            connection_id,
            approval_id,
            decision: "always".into(),
        })
        .await
        .unwrap());
    assert_eq!(receiver.await.unwrap(), ApprovalDecision::ApproveForSession);
    assert!(
        session
            .is_allowed(&principal_id, &thread_id, "bash_exec")
            .await
    );
    service.shutdown().await.unwrap();
    assert!(cancellation.is_cancelled());
    assert_eq!(runtime_shutdowns.load(Ordering::SeqCst), 1);
}

#[test]
fn admin_rpc_has_no_concrete_runtime_registry_or_lock_access() {
    let source = include_str!("../src/impl/daemon/handler/rpc/rpc_admin.rs");
    assert!(source.contains("self.ports.admin"));
    for forbidden in [
        "subsystems",
        "SkillLoader",
        "ToolRegistry",
        "HookRegistry",
        ".lock()",
    ] {
        assert!(
            !source.contains(forbidden),
            "admin RPC must not contain {forbidden}"
        );
    }
}
