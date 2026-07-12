//! Integration tests for Agora persistence adapter (Phase 3C).
//!
//! Verifies that AgoraRegistry with an InMemoryCommitLog survives
//! "restart" (new registry instance) and recovers committed state.

use agora::{AgoraOperation, AgoraPersistence, AgoraRegistry, InMemoryCommitLog};
use fabric::{AgoraOps, ProcessId};
use serde_json::json;
use std::sync::Arc;

fn test_author() -> ProcessId {
    ProcessId(uuid::Uuid::from_u128(4))
}

#[tokio::test]
async fn commit_then_recover() {
    let log = Arc::new(InMemoryCommitLog::new());

    let reg1 = AgoraRegistry::new_with_persistence(log.clone() as Arc<dyn AgoraPersistence>);

    let prop1 = reg1
        .propose(
            "s1",
            0,
            AgoraOperation::PublishFact {
                key: "alpha".into(),
                value: json!(10),
            },
            test_author(),
        )
        .await
        .unwrap();
    reg1.commit("s1", prop1.id).await.unwrap();

    let prop2 = reg1
        .propose(
            "s1",
            1,
            AgoraOperation::PublishFact {
                key: "beta".into(),
                value: json!(20),
            },
            test_author(),
        )
        .await
        .unwrap();
    reg1.commit("s1", prop2.id).await.unwrap();

    let reg2 = AgoraRegistry::new_with_persistence(log.clone() as Arc<dyn AgoraPersistence>);
    let replayed = reg2.recover_session("s1").await.unwrap();
    assert_eq!(replayed, 2, "expected 2 commits replayed");

    let snap = reg2.snapshot("s1").await.unwrap();
    assert_eq!(snap["version"], json!(2));
    assert_eq!(snap["blackboard"]["alpha"], json!(10));
    assert_eq!(snap["blackboard"]["beta"], json!(20));

    let changes = reg2.changes_since("s1", 0).await;
    assert_eq!(changes.len(), 2);

    let changes_since_1 = reg2.changes_since("s1", 1).await;
    assert_eq!(changes_since_1.len(), 1);
}

#[tokio::test]
async fn no_persistence_still_works() {
    let reg = AgoraRegistry::new();

    let prop = reg
        .propose(
            "s1",
            0,
            AgoraOperation::PublishFact {
                key: "k".into(),
                value: json!("v"),
            },
            test_author(),
        )
        .await
        .unwrap();
    reg.commit("s1", prop.id).await.unwrap();

    let snap = reg.snapshot("s1").await.unwrap();
    assert_eq!(snap["version"], json!(1));

    let replayed = reg.recover_session("s1").await.unwrap();
    assert_eq!(replayed, 0);

    let changes = reg.changes_since("s1", 0).await;
    assert_eq!(changes.len(), 1);
}

#[tokio::test]
async fn recover_session_is_idempotent() {
    let log = Arc::new(InMemoryCommitLog::new());

    let reg1 = AgoraRegistry::new_with_persistence(log.clone() as Arc<dyn AgoraPersistence>);
    let prop = reg1
        .propose(
            "s1",
            0,
            AgoraOperation::PublishFact {
                key: "x".into(),
                value: json!(1),
            },
            test_author(),
        )
        .await
        .unwrap();
    reg1.commit("s1", prop.id).await.unwrap();

    let reg2 = AgoraRegistry::new_with_persistence(log.clone() as Arc<dyn AgoraPersistence>);
    let n1 = reg2.recover_session("s1").await.unwrap();
    assert_eq!(n1, 1);

    let n2 = reg2.recover_session("s1").await.unwrap();
    assert_eq!(n2, 0, "second replay must skip already-applied commits");
    let snap = reg2.snapshot("s1").await.unwrap();
    assert_eq!(snap["version"], json!(1));
    assert_eq!(snap["blackboard"]["x"], json!(1));
}
