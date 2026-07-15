use agora::{AgoraOperation, AgoraPersistence, AgoraRegistry};
use async_trait::async_trait;
use fabric::include::agora::{AgoraService, WorkspaceCommitPermit};
use fabric::{AgoraOps, AgoraProposal, AgoraSpaceId, ProcessId};
use serde_json::json;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::{Mutex, Notify};
use uuid::Uuid;

fn author(id: u128) -> ProcessId {
    ProcessId(Uuid::from_u128(id))
}
fn proposal(
    space: &str,
    id: u128,
    base: u64,
    operation: AgoraOperation,
    author: ProcessId,
) -> AgoraProposal {
    AgoraProposal {
        id: Uuid::from_u128(id),
        space: AgoraSpaceId(space.into()),
        author,
        base_version: base,
        operation,
        evidence: Vec::new(),
        confidence: 1.0,
        expires_at_ms: None,
    }
}
fn permit(proposal: &AgoraProposal) -> WorkspaceCommitPermit {
    WorkspaceCommitPermit::issue_for(proposal, i64::MAX).unwrap()
}

#[tokio::test]
async fn transaction_same_base_proposals_cannot_both_commit() {
    let registry = Arc::new(AgoraRegistry::new(Arc::new(
        aletheon_kernel::chronos::TestClock::default(),
    )));
    let p1 = proposal(
        "s",
        1,
        0,
        AgoraOperation::PublishFact {
            key: "a".into(),
            value: json!(1),
        },
        author(1),
    );
    let p2 = proposal(
        "s",
        2,
        0,
        AgoraOperation::PublishFact {
            key: "b".into(),
            value: json!(2),
        },
        author(2),
    );
    let permit1 = permit(&p1);
    let permit2 = permit(&p2);
    AgoraService::propose(&*registry, p1).await.unwrap();
    AgoraService::propose(&*registry, p2).await.unwrap();
    let r1 = {
        let registry = registry.clone();
        tokio::spawn(
            async move { AgoraService::commit(&*registry, Uuid::from_u128(1), permit1).await },
        )
    };
    let r2 = {
        let registry = registry.clone();
        tokio::spawn(
            async move { AgoraService::commit(&*registry, Uuid::from_u128(2), permit2).await },
        )
    };
    let (r1, r2) = tokio::join!(r1, r2);
    assert_ne!(r1.unwrap().is_ok(), r2.unwrap().is_ok());
    assert_eq!(AgoraOps::version(&*registry, "s").await.unwrap(), 1);
}

#[tokio::test]
async fn transaction_claim_and_release_require_current_owner() {
    let registry = AgoraRegistry::new(Arc::new(aletheon_kernel::chronos::TestClock::default()));
    let owner = author(10);
    let other = author(11);
    let claim = proposal(
        "s",
        10,
        0,
        AgoraOperation::ClaimSharedObject {
            oid: "object".into(),
        },
        owner,
    );
    let bound = permit(&claim);
    AgoraService::propose(&registry, claim).await.unwrap();
    AgoraService::commit(&registry, Uuid::from_u128(10), bound)
        .await
        .unwrap();
    let duplicate = proposal(
        "s",
        11,
        1,
        AgoraOperation::ClaimSharedObject {
            oid: "object".into(),
        },
        other,
    );
    let bound = permit(&duplicate);
    AgoraService::propose(&registry, duplicate).await.unwrap();
    assert!(AgoraService::commit(&registry, Uuid::from_u128(11), bound)
        .await
        .is_err());
    let release = proposal(
        "s",
        12,
        1,
        AgoraOperation::ReleaseSharedObject {
            oid: "object".into(),
        },
        other,
    );
    let bound = permit(&release);
    AgoraService::propose(&registry, release).await.unwrap();
    assert!(AgoraService::commit(&registry, Uuid::from_u128(12), bound)
        .await
        .is_err());
    assert_eq!(AgoraOps::version(&registry, "s").await.unwrap(), 1);
}

#[derive(Default)]
struct ControlledLog {
    fail: AtomicBool,
    entered: Notify,
    release: Notify,
    block: AtomicBool,
    commits: Mutex<Vec<(String, fabric::AgoraCommit)>>,
}
#[async_trait]
impl AgoraPersistence for ControlledLog {
    async fn append_commit(
        &self,
        session: &str,
        commit: &fabric::AgoraCommit,
    ) -> anyhow::Result<()> {
        self.entered.notify_one();
        if self.block.load(Ordering::SeqCst) && session == "blocked" {
            self.release.notified().await;
        }
        if self.fail.load(Ordering::SeqCst) {
            anyhow::bail!("injected append failure");
        }
        self.commits
            .lock()
            .await
            .push((session.into(), commit.clone()));
        Ok(())
    }
    async fn recover(&self, session: &str) -> anyhow::Result<Vec<fabric::AgoraCommit>> {
        Ok(self
            .commits
            .lock()
            .await
            .iter()
            .filter(|(candidate, _)| candidate == session)
            .map(|(_, commit)| commit.clone())
            .collect())
    }
}

#[tokio::test]
async fn durability_failure_is_not_visible_and_retry_applies_once() {
    let log = Arc::new(ControlledLog::default());
    log.fail.store(true, Ordering::SeqCst);
    let registry = AgoraRegistry::new_with_persistence(
        log.clone(),
        Arc::new(aletheon_kernel::chronos::TestClock::default()),
    );
    let p = proposal(
        "s",
        20,
        0,
        AgoraOperation::PublishFact {
            key: "k".into(),
            value: json!(1),
        },
        author(20),
    );
    let bound = permit(&p);
    AgoraService::propose(&registry, p).await.unwrap();
    assert!(
        AgoraService::commit(&registry, Uuid::from_u128(20), bound.clone())
            .await
            .is_err()
    );
    assert_eq!(AgoraOps::version(&registry, "s").await.unwrap(), 0);
    assert_eq!(AgoraOps::recall(&registry, "s", "k").await.unwrap(), None);
    log.fail.store(false, Ordering::SeqCst);
    AgoraService::commit(&registry, Uuid::from_u128(20), bound)
        .await
        .unwrap();
    assert_eq!(AgoraOps::version(&registry, "s").await.unwrap(), 1);
    assert_eq!(
        AgoraOps::recall(&registry, "s", "k").await.unwrap(),
        Some(json!(1))
    );
}

#[tokio::test]
async fn durability_io_does_not_hold_workspace_state_lock() {
    let log = Arc::new(ControlledLog::default());
    log.block.store(true, Ordering::SeqCst);
    let registry = Arc::new(AgoraRegistry::new_with_persistence(
        log.clone(),
        Arc::new(aletheon_kernel::chronos::TestClock::default()),
    ));
    let p = proposal(
        "blocked",
        30,
        0,
        AgoraOperation::PublishFact {
            key: "k".into(),
            value: json!(1),
        },
        author(30),
    );
    let bound = permit(&p);
    AgoraService::propose(&*registry, p).await.unwrap();
    let committing = {
        let registry = registry.clone();
        tokio::spawn(
            async move { AgoraService::commit(&*registry, Uuid::from_u128(30), bound).await },
        )
    };
    log.entered.notified().await;
    let snapshot = tokio::time::timeout(
        std::time::Duration::from_millis(100),
        AgoraOps::snapshot(&*registry, "blocked"),
    )
    .await;
    assert!(snapshot.is_ok(), "persistence held the workspace lock");
    assert_eq!(snapshot.unwrap().unwrap()["version"], json!(0));
    let other = proposal(
        "other",
        31,
        0,
        AgoraOperation::PublishFact {
            key: "other".into(),
            value: json!(2),
        },
        author(31),
    );
    let other_permit = permit(&other);
    AgoraService::propose(&*registry, other).await.unwrap();
    let other_commit = tokio::time::timeout(
        std::time::Duration::from_millis(100),
        AgoraService::commit(&*registry, Uuid::from_u128(31), other_permit),
    )
    .await;
    assert!(
        other_commit.is_ok(),
        "blocked space serialized an independent space"
    );
    log.release.notify_waiters();
    committing.await.unwrap().unwrap();
}

struct CorruptRecovery {
    commit: fabric::AgoraCommit,
}
#[async_trait]
impl AgoraPersistence for CorruptRecovery {
    async fn append_commit(&self, _: &str, _: &fabric::AgoraCommit) -> anyhow::Result<()> {
        Ok(())
    }
    async fn recover(&self, _: &str) -> anyhow::Result<Vec<fabric::AgoraCommit>> {
        Ok(vec![self.commit.clone()])
    }
}
#[tokio::test]
async fn recovery_rejects_wrong_space_or_checksum() {
    let p = proposal(
        "other",
        40,
        0,
        AgoraOperation::PublishFact {
            key: "k".into(),
            value: json!(1),
        },
        author(40),
    );
    let commit = fabric::AgoraCommit::from_proposal(&p, 1, 1, None).unwrap();
    let registry = AgoraRegistry::new_with_persistence(
        Arc::new(CorruptRecovery { commit }),
        Arc::new(aletheon_kernel::chronos::TestClock::default()),
    );
    assert!(registry.recover_session("s").await.is_err());
    assert_eq!(AgoraOps::version(&registry, "s").await.unwrap(), 0);
}
