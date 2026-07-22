use corpus::tools::google::oauth::GoogleBinding;
use executive::approval::{
    ApprovalDecision, ApprovalRepository, ApprovalResolutionContext,
};
use executive::testing::channel::gmail::report::{
    GmailDeliveryOutcome, GmailReconciliation, GmailReportBoundary, GmailReportProvider,
    GmailSendResult,
};
use executive::testing::external::ExternalIdentityRepository;
use executive::goal::ObjectiveStore;
use fabric::{
    ApprovalId, ExternalCapabilityId, ExternalIdentityId, GoalBudget, GoalSpec, PrincipalId,
};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

struct Fixture {
    _dir: tempfile::TempDir,
    db_path: std::path::PathBuf,
    artifact_root: std::path::PathBuf,
    account: ExternalIdentityId,
    owner: PrincipalId,
    goal_id: fabric::GoalId,
}

impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("objectives.db");
        let artifact_root = dir.path().join("artifacts");
        let account = ExternalIdentityId::new();
        let owner = PrincipalId("owner".into());
        ExternalIdentityRepository::open(&db_path)
            .unwrap()
            .bind_google(
                &owner,
                GoogleBinding {
                    identity_id: account,
                    provider_subject: "subject".into(),
                    email: "owner@example.com".into(),
                    scopes: vec![ExternalCapabilityId::new("mail.read").unwrap()],
                },
                Some("work".into()),
                1,
            )
            .unwrap();
        let goal_id = ObjectiveStore::open(&db_path)
            .unwrap()
            .create_goal(
                &owner,
                "session",
                "session",
                &GoalSpec {
                    original_intent: "produce report".into(),
                    desired_state: vec![],
                    constraints: vec![],
                    acceptance_criteria: vec![],
                    budget: GoalBudget::default(),
                },
            )
            .unwrap()
            .id;
        Self {
            _dir: dir,
            db_path,
            artifact_root,
            account,
            owner,
            goal_id,
        }
    }

    fn boundary(&self) -> GmailReportBoundary {
        GmailReportBoundary::open(&self.db_path, &self.artifact_root).unwrap()
    }

    fn grant_send(&self) {
        rusqlite::Connection::open(&self.db_path)
            .unwrap()
            .execute(
                "UPDATE capability_grants SET scopes_json=?1
                 WHERE identity_id=?2 AND state='active'",
                rusqlite::params![
                    serde_json::to_string(&vec![
                        ExternalCapabilityId::new("mail.read").unwrap(),
                        ExternalCapabilityId::new("mail.send").unwrap(),
                    ])
                    .unwrap(),
                    self.account.to_string()
                ],
            )
            .unwrap();
    }

    fn revoke(&self) {
        let db = rusqlite::Connection::open(&self.db_path).unwrap();
        db.execute(
            "UPDATE capability_grants SET state='revoked',revoked_at_ms=500
             WHERE identity_id=?1",
            [self.account.to_string()],
        )
        .unwrap();
    }

    fn approve(
        &self,
        approval: &fabric::ApprovalSnapshot,
        now_ms: i64,
    ) -> fabric::ApprovalSnapshot {
        ApprovalRepository::open(&self.db_path)
            .unwrap()
            .resolve(
                approval.id,
                approval.version,
                &ApprovalResolutionContext {
                    principal_id: self.owner.clone(),
                    channel: "telegram".into(),
                },
                ApprovalDecision::Approve,
                now_ms,
            )
            .unwrap()
    }
}

#[derive(Default)]
struct Provider {
    sends: AtomicUsize,
    reconciles: AtomicUsize,
    send_results: Mutex<VecDeque<GmailSendResult>>,
    reconcile_results: Mutex<VecDeque<GmailReconciliation>>,
}

impl Provider {
    fn with(
        send: impl IntoIterator<Item = GmailSendResult>,
        reconcile: impl IntoIterator<Item = GmailReconciliation>,
    ) -> Self {
        Self {
            send_results: Mutex::new(send.into_iter().collect()),
            reconcile_results: Mutex::new(reconcile.into_iter().collect()),
            ..Self::default()
        }
    }
}

#[async_trait::async_trait]
impl GmailReportProvider for Provider {
    async fn send(
        &self,
        _: ExternalIdentityId,
        _: &str,
        _: &str,
        _: &str,
        _: &str,
    ) -> GmailSendResult {
        self.sends.fetch_add(1, Ordering::SeqCst);
        self.send_results
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or(GmailSendResult::Failed {
                error: "unexpected send".into(),
            })
    }

    async fn reconcile(&self, _: ExternalIdentityId, _: &str) -> GmailReconciliation {
        self.reconciles.fetch_add(1, Ordering::SeqCst);
        self.reconcile_results
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or(GmailReconciliation::Unknown)
    }
}

#[test]
fn read_only_default_creates_local_artifact_and_never_authorizes_gmail_send() {
    let f = Fixture::new();
    let boundary = f.boundary();
    let report = boundary
        .create_local_report(
            f.goal_id,
            f.account,
            "Architecture report",
            "Trusted local report body.",
            100,
        )
        .unwrap();
    assert_eq!(
        report.artifact.scan_status,
        executive::testing::artifact::ArtifactScanStatus::Clean
    );
    assert!(report.telegram_summary.contains("Report ready"));
    assert!(f
        .artifact_root
        .join(&report.artifact.relative_path)
        .is_file());
    assert!(boundary
        .request_send_approval(&report, "recipient@example.com", 101, 1_000)
        .is_err());
    assert!(ApprovalRepository::open(&f.db_path)
        .unwrap()
        .list_pending(&f.owner, 101)
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn approval_binds_account_recipient_subject_body_and_report_hashes() {
    let f = Fixture::new();
    f.grant_send();
    let boundary = f.boundary();
    let report = boundary
        .create_local_report(f.goal_id, f.account, "Subject", "Body", 100)
        .unwrap();
    let approval = boundary
        .request_send_approval(&report, "recipient@example.com", 101, 200)
        .unwrap();
    let provider = Provider::with(
        [GmailSendResult::Sent {
            provider_message_id: "sent".into(),
        }],
        [],
    );
    assert!(boundary
        .deliver(
            ApprovalId::new(),
            &report,
            "recipient@example.com",
            110,
            &provider,
        )
        .await
        .is_err());
    assert!(boundary
        .deliver(
            approval.id,
            &report,
            "recipient@example.com",
            110,
            &provider,
        )
        .await
        .is_err());
    let approved = f.approve(&approval, 120);
    assert!(boundary
        .deliver(approved.id, &report, "changed@example.com", 121, &provider,)
        .await
        .is_err());
    let mut changed = report.clone();
    changed.body.push_str(" changed");
    assert!(boundary
        .deliver(
            approved.id,
            &changed,
            "recipient@example.com",
            121,
            &provider,
        )
        .await
        .is_err());
    assert!(boundary
        .deliver(
            approved.id,
            &report,
            "recipient@example.com",
            200,
            &provider,
        )
        .await
        .is_err());
    assert_eq!(provider.sends.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn successful_delivery_is_idempotent_and_revoked_write_grant_fails_closed() {
    let f = Fixture::new();
    f.grant_send();
    let boundary = f.boundary();
    let report = boundary
        .create_local_report(f.goal_id, f.account, "Subject", "Body", 100)
        .unwrap();
    let approval = boundary
        .request_send_approval(&report, "recipient@example.com", 101, 1_000)
        .unwrap();
    let approved = f.approve(&approval, 110);
    let provider = Provider::with(
        [GmailSendResult::Sent {
            provider_message_id: "provider-1".into(),
        }],
        [],
    );
    assert_eq!(
        boundary
            .deliver(
                approved.id,
                &report,
                "recipient@example.com",
                120,
                &provider,
            )
            .await
            .unwrap(),
        GmailDeliveryOutcome::Sent {
            provider_message_id: "provider-1".into()
        }
    );
    assert_eq!(
        boundary
            .deliver(
                approved.id,
                &report,
                "recipient@example.com",
                121,
                &provider,
            )
            .await
            .unwrap(),
        GmailDeliveryOutcome::AlreadySent {
            provider_message_id: "provider-1".into()
        }
    );
    assert_eq!(provider.sends.load(Ordering::SeqCst), 1);

    let f2 = Fixture::new();
    f2.grant_send();
    let boundary = f2.boundary();
    let report = boundary
        .create_local_report(f2.goal_id, f2.account, "Subject", "Body", 100)
        .unwrap();
    let approval = boundary
        .request_send_approval(&report, "recipient@example.com", 101, 1_000)
        .unwrap();
    let approved = f2.approve(&approval, 110);
    f2.revoke();
    assert!(boundary
        .deliver(
            approved.id,
            &report,
            "recipient@example.com",
            120,
            &provider,
        )
        .await
        .is_err());
    assert_eq!(provider.sends.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn ambiguous_timeout_requires_reconciliation_before_any_retry() {
    let f = Fixture::new();
    f.grant_send();
    let boundary = f.boundary();
    let report = boundary
        .create_local_report(f.goal_id, f.account, "Subject", "Body", 100)
        .unwrap();
    let approval = boundary
        .request_send_approval(&report, "recipient@example.com", 101, 1_000)
        .unwrap();
    let approved = f.approve(&approval, 110);
    let provider = Provider::with(
        [GmailSendResult::AmbiguousTimeout],
        [
            GmailReconciliation::Unknown,
            GmailReconciliation::Found {
                provider_message_id: "provider-after-timeout".into(),
            },
        ],
    );
    assert_eq!(
        boundary
            .deliver(
                approved.id,
                &report,
                "recipient@example.com",
                120,
                &provider,
            )
            .await
            .unwrap(),
        GmailDeliveryOutcome::Ambiguous
    );
    assert_eq!(
        boundary
            .deliver(
                approved.id,
                &report,
                "recipient@example.com",
                121,
                &provider,
            )
            .await
            .unwrap(),
        GmailDeliveryOutcome::ReconciliationRequired
    );
    assert_eq!(provider.sends.load(Ordering::SeqCst), 1);
    assert_eq!(
        boundary
            .deliver(
                approved.id,
                &report,
                "recipient@example.com",
                122,
                &provider,
            )
            .await
            .unwrap(),
        GmailDeliveryOutcome::AlreadySent {
            provider_message_id: "provider-after-timeout".into()
        }
    );
    assert_eq!(provider.sends.load(Ordering::SeqCst), 1);
    assert_eq!(provider.reconciles.load(Ordering::SeqCst), 2);
}
