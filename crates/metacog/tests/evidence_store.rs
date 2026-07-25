//! Evidence store tests — append-only persistence with integrity checks.

use std::path::PathBuf;

use sha2::Digest;
use sha2::Sha256;

use fabric::types::metacognition_evidence::{
    EvidenceId, EvidenceItem, EvidenceKind, EvidenceTrust,
};
use fabric::types::metacognition_experience::ExperienceId;
use metacog::evidence::{AppendOutcome, EvidenceStore, JsonlEvidenceStore};

fn make_item(id: &str, exp_id: &str, digest: &str) -> EvidenceItem {
    let payload = serde_json::json!({"key": id});
    EvidenceItem {
        schema_version: 1,
        evidence_id: EvidenceId(id.into()),
        experience_id: ExperienceId(exp_id.into()),
        kind: EvidenceKind::ActionResult,
        source: "test".into(),
        producer: "test".into(),
        captured_at_ms: 100,
        payload,
        sha256: digest.into(),
        trust: EvidenceTrust::Authoritative,
        freshness_ms: None,
        redacted: false,
    }
}

fn item_with_digest(id: &str, exp_id: &str) -> EvidenceItem {
    let payload = serde_json::json!({"key": id});
    let bytes = serde_json::to_vec(&payload).unwrap();
    let digest = format!("{:x}", Sha256::digest(bytes));
    make_item(id, exp_id, &digest)
}

#[tokio::test]
async fn append_and_retrieve() {
    let store = JsonlEvidenceStore::in_memory();
    let item = item_with_digest("ev-1", "exp-1");
    let outcome = store.append(item.clone()).await.unwrap();
    assert_eq!(outcome, AppendOutcome::Appended);

    let retrieved = store.get(&EvidenceId("ev-1".into())).await.unwrap();
    assert!(retrieved.is_some());
    assert_eq!(retrieved.unwrap().evidence_id, EvidenceId("ev-1".into()));
}

#[tokio::test]
async fn duplicate_id_with_same_digest_is_idempotent() {
    let store = JsonlEvidenceStore::in_memory();
    let item = item_with_digest("ev-1", "exp-1");
    store.append(item.clone()).await.unwrap();
    let outcome = store.append(item).await.unwrap();
    assert_eq!(outcome, AppendOutcome::AlreadyPresent);

    let items = store
        .list_for_experience(&ExperienceId("exp-1".into()))
        .await
        .unwrap();
    assert_eq!(items.len(), 1);
}

#[tokio::test]
async fn duplicate_id_with_different_payload_is_rejected() {
    let store = JsonlEvidenceStore::in_memory();
    let item1 = item_with_digest("ev-1", "exp-1");
    store.append(item1).await.unwrap();

    // Different payload → different digest → conflict
    let payload2 = serde_json::json!({"different": "payload"});
    let digest2 = {
        let bytes = serde_json::to_vec(&payload2).unwrap();
        format!("{:x}", Sha256::digest(bytes))
    };
    let item2 = EvidenceItem {
        schema_version: 1,
        evidence_id: EvidenceId("ev-1".into()),
        experience_id: ExperienceId("exp-2".into()),
        kind: EvidenceKind::Assertion,
        source: "test".into(),
        producer: "test".into(),
        captured_at_ms: 100,
        payload: payload2,
        sha256: digest2,
        trust: EvidenceTrust::Authoritative,
        freshness_ms: None,
        redacted: false,
    };
    let result = store.append(item2).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn reopening_jsonl_rebuilds_index() {
    let dir = tempfile::tempdir().unwrap();
    let path: PathBuf = dir.path().join("evidence.jsonl");

    // Append to first store
    let store1 = JsonlEvidenceStore::open(path.clone()).unwrap();
    let item = item_with_digest("ev-1", "exp-1");
    store1.append(item).await.unwrap();

    // Reopen
    let store2 = JsonlEvidenceStore::open(path).unwrap();
    let retrieved = store2.get(&EvidenceId("ev-1".into())).await.unwrap();
    assert!(retrieved.is_some());
}

#[tokio::test]
async fn list_filters_by_experience() {
    let store = JsonlEvidenceStore::in_memory();
    store
        .append(item_with_digest("ev-1", "exp-a"))
        .await
        .unwrap();
    store
        .append(item_with_digest("ev-2", "exp-a"))
        .await
        .unwrap();
    store
        .append(item_with_digest("ev-3", "exp-b"))
        .await
        .unwrap();

    let a_items = store
        .list_for_experience(&ExperienceId("exp-a".into()))
        .await
        .unwrap();
    assert_eq!(a_items.len(), 2);

    let b_items = store
        .list_for_experience(&ExperienceId("exp-b".into()))
        .await
        .unwrap();
    assert_eq!(b_items.len(), 1);
}
