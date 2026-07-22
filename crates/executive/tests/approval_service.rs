use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use executive::approval::{ApprovalCreate, ApprovalDecision, ApprovalRepository};
use executive::goal::ObjectiveStore;
use executive::service::admin_service::{ApprovalOwner, PendingApprovals, ScopedApprovalCache};
use executive::service::approval_service::{
    ApprovalContext, ApprovalService, ApprovalServiceError, ApprovalUseCases,
    ResolveApprovalRequest,
};
use fabric::{
    ApprovalCategory, ApprovalRisk, ApprovalSubject, Clock, GoalSpec, GoalState, PrincipalId,
    ThreadId, TurnId,
};
use kernel::chronos::TestClock;
use tempfile::tempdir;

struct Fixture {
    service: ApprovalService,
    repository: Arc<Mutex<ApprovalRepository>>,
    owner: ApprovalContext,
    other: ApprovalContext,
    goal_id: fabric::GoalId,
}

#[tokio::test]
async fn another_principal_cannot_resolve_transient_approval() {
    let pending = PendingApprovals::default();
    let alice = ApprovalOwner::new(PrincipalId::local_uid(1001), ThreadId("a".into()));
    let bob = ApprovalOwner::new(PrincipalId::local_uid(1002), ThreadId("b".into()));
    let (sender, _receiver) = tokio::sync::oneshot::channel();
    let alice_connection = fabric::ConnectionId::new();
    let id = pending
        .insert(
            alice.clone(),
            TurnId::new(),
            "call-1".into(),
            "shell".into(),
            alice_connection,
            sender,
        )
        .await;
    let connection_error = pending
        .resolve_authenticated(
            &alice.principal_id,
            &fabric::ConnectionId::new(),
            &id,
            corpus::security::approval::ApprovalDecision::Approve,
        )
        .await
        .unwrap_err();
    assert!(connection_error
        .to_string()
        .contains("not owned by authenticated principal"));
    let error = pending
        .resolve(
            &bob,
            &id,
            corpus::security::approval::ApprovalDecision::Approve,
        )
        .await
        .unwrap_err();
    assert!(error
        .to_string()
        .contains("not owned by authenticated principal"));
}

#[tokio::test]
async fn session_grant_is_not_reused_by_another_principal() {
    let cache = ScopedApprovalCache::default();
    cache
        .allow_for_thread(PrincipalId::local_uid(1001), ThreadId("a".into()), "shell")
        .await;
    assert!(
        !cache
            .is_allowed(
                &PrincipalId::local_uid(1002),
                &ThreadId("a".into()),
                "shell",
            )
            .await
    );
}

impl Fixture {
    fn new(now_ms: i64) -> Self {
        let directory = tempdir().unwrap();
        let path = directory.keep().join("approvals.db");
        let owner_id = PrincipalId("authenticated-session".into());
        let store = ObjectiveStore::open(&path).unwrap();
        let goal = store
            .create_goal(
                &owner_id,
                "authenticated-session",
                "session",
                &GoalSpec {
                    original_intent: "approve change".into(),
                    desired_state: vec![],
                    constraints: vec![],
                    acceptance_criteria: vec![],
                    budget: Default::default(),
                },
            )
            .unwrap();
        let goal = store
            .transition_goal(goal.id, 0, GoalState::Running, None, &serde_json::json!({}))
            .unwrap();
        drop(store);
        let repository = Arc::new(Mutex::new(ApprovalRepository::open(&path).unwrap()));
        let clock: Arc<dyn Clock> = Arc::new(TestClock::new(now_ms, 0));
        let service = ApprovalService::new(
            repository.clone(),
            None,
            clock,
            Arc::new(tokio::sync::Mutex::new(None)),
        );
        Self {
            service,
            repository,
            owner: ApprovalContext {
                principal_id: owner_id,
                channel: "local_rpc".into(),
            },
            other: ApprovalContext {
                principal_id: PrincipalId("forged-owner".into()),
                channel: "local_rpc".into(),
            },
            goal_id: goal.id,
        }
    }

    fn create(&self, category: ApprovalCategory, nonce: usize) -> fabric::ApprovalSnapshot {
        self.repository
            .lock()
            .unwrap()
            .create(ApprovalCreate {
                subject: ApprovalSubject {
                    category,
                    goal_id: self.goal_id,
                    attempt_id: None,
                    job_id: None,
                    attributes: BTreeMap::from([("nonce".into(), nonce.to_string())]),
                    allowed_scope: vec![],
                    apply_target: None,
                },
                risk: ApprovalRisk::High,
                summary: format!("bounded approval {nonce}"),
                artifacts: vec![],
                created_at_ms: 10 + nonce as i64,
                expires_at_ms: 10_000,
            })
            .unwrap()
    }
}

#[tokio::test]
async fn forged_owner_cannot_show_or_resolve() {
    let fixture = Fixture::new(100);
    let approval = fixture.create(ApprovalCategory::SendMail, 1);
    assert!(matches!(
        fixture
            .service
            .show(fixture.other.clone(), approval.id)
            .await,
        Err(ApprovalServiceError::Forbidden(_))
    ));
    assert!(matches!(
        fixture
            .service
            .resolve(ResolveApprovalRequest {
                context: fixture.other.clone(),
                approval_id: approval.id,
                version: approval.version,
                decision: ApprovalDecision::Approve,
            })
            .await,
        Err(ApprovalServiceError::Forbidden(_))
    ));
}

#[tokio::test]
async fn replay_is_idempotent_and_stale_version_conflicts() {
    let fixture = Fixture::new(100);
    let approval = fixture.create(ApprovalCategory::SendMail, 1);
    let request = ResolveApprovalRequest {
        context: fixture.owner.clone(),
        approval_id: approval.id,
        version: approval.version,
        decision: ApprovalDecision::Approve,
    };
    let first = fixture.service.resolve(request.clone()).await.unwrap();
    let replay = fixture.service.resolve(request).await.unwrap();
    assert_eq!(replay.id, first.id);
    assert_eq!(replay.version, first.version);

    let stale = fixture.create(ApprovalCategory::SendMail, 2);
    assert!(matches!(
        fixture
            .service
            .resolve(ResolveApprovalRequest {
                context: fixture.owner.clone(),
                approval_id: stale.id,
                version: stale.version + 1,
                decision: ApprovalDecision::Approve,
            })
            .await,
        Err(ApprovalServiceError::Conflict(_))
    ));
}

#[tokio::test]
async fn approved_apply_requires_the_optional_runtime() {
    let fixture = Fixture::new(100);
    let approval = fixture.create(ApprovalCategory::ApplyCode, 1);
    assert!(matches!(
        fixture
            .service
            .resolve(ResolveApprovalRequest {
                context: fixture.owner.clone(),
                approval_id: approval.id,
                version: approval.version,
                decision: ApprovalDecision::Approve,
            })
            .await,
        Err(ApprovalServiceError::RuntimeUnavailable(_))
    ));
}

#[tokio::test]
async fn list_is_owner_scoped_expiry_aware_and_bounded() {
    let fixture = Fixture::new(100);
    for nonce in 0..105 {
        fixture.create(ApprovalCategory::SendMail, nonce);
    }
    assert_eq!(
        fixture
            .service
            .list(fixture.owner.clone())
            .await
            .unwrap()
            .len(),
        100
    );
    assert!(fixture
        .service
        .list(fixture.other.clone())
        .await
        .unwrap()
        .is_empty());

    let expired = Fixture::new(20_000);
    expired.create(ApprovalCategory::SendMail, 1);
    assert!(expired
        .service
        .list(expired.owner)
        .await
        .unwrap()
        .is_empty());
}

#[test]
fn approval_rpc_has_no_repository_clock_or_lock_access() {
    let source = include_str!("../src/host/daemon/handler/rpc/rpc_approval.rs");
    assert!(source.contains("self.ports.approvals"));
    for forbidden in [
        "subsystems",
        "ApprovalRepository",
        "ApplyCoordinator",
        ".lock()",
        "wall_now",
    ] {
        assert!(
            !source.contains(forbidden),
            "approval RPC must not contain {forbidden}"
        );
    }
}
