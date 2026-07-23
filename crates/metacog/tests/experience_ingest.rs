//! Experience ingestion tests — validation of experience envelopes.

use std::collections::BTreeMap;
use std::sync::Arc;

use sha2::Digest;
use sha2::Sha256;

use fabric::types::metacognition_evidence::{
    EvidenceId, EvidenceItem, EvidenceKind, EvidenceTrust,
};
use fabric::types::metacognition_experience::{
    DomainId, ExperienceEnvelope, ExperienceId, ExperienceOutcome, SubjectId,
    METACOGNITION_SCHEMA_V1,
};
use metacog::evidence::{AppendOutcome, EvidenceStore, JsonlEvidenceStore};
use metacog::experience::{ExperienceIngestError, ExperienceIngestor, InMemoryExperienceStore};

fn make_envelope(id: &str, evidence_ids: Vec<&str>) -> ExperienceEnvelope {
    ExperienceEnvelope {
        schema_version: METACOGNITION_SCHEMA_V1,
        experience_id: ExperienceId(id.into()),
        domain: DomainId::new("synthetic").unwrap(),
        subject: SubjectId("test-component".into()),
        goal_ref: None,
        started_at_ms: 100,
        completed_at_ms: Some(200),
        outcome: ExperienceOutcome::Succeeded,
        correlations: BTreeMap::new(),
        evidence: evidence_ids
            .into_iter()
            .map(|e| EvidenceId(e.into()))
            .collect(),
    }
}

fn make_evidence(id: &str) -> EvidenceItem {
    let payload = serde_json::json!({"key": id});
    let bytes = serde_json::to_vec(&payload).unwrap();
    let digest = format!("{:x}", sha2::Sha256::digest(bytes));
    EvidenceItem {
        schema_version: 1,
        evidence_id: EvidenceId(id.into()),
        experience_id: ExperienceId("exp-1".into()),
        kind: EvidenceKind::ActionResult,
        source: "test".into(),
        producer: "test".into(),
        captured_at_ms: 150,
        payload,
        sha256: digest,
        trust: EvidenceTrust::Authoritative,
        freshness_ms: None,
        redacted: false,
    }
}

#[tokio::test]
async fn valid_experience_with_evidence_is_accepted() {
    let evidence_store = Arc::new(JsonlEvidenceStore::in_memory());
    let exp_store = InMemoryExperienceStore::new();
    let ingestor = ExperienceIngestor::new(evidence_store.clone());

    evidence_store.append(make_evidence("ev-1")).await.unwrap();
    let envelope = make_envelope("exp-1", vec!["ev-1"]);
    let outcome = ingestor.ingest(&envelope, &exp_store).await.unwrap();
    assert_eq!(outcome, AppendOutcome::Appended);
}

#[tokio::test]
async fn missing_evidence_is_rejected() {
    let evidence_store = Arc::new(JsonlEvidenceStore::in_memory());
    let exp_store = InMemoryExperienceStore::new();
    let ingestor = ExperienceIngestor::new(evidence_store);

    let envelope = make_envelope("exp-1", vec!["nonexistent"]);
    let result = ingestor.ingest(&envelope, &exp_store).await;
    assert!(matches!(
        result,
        Err(ExperienceIngestError::MissingEvidence(_))
    ));
}

#[tokio::test]
async fn duplicate_experience_is_idempotent() {
    let evidence_store = Arc::new(JsonlEvidenceStore::in_memory());
    let exp_store = InMemoryExperienceStore::new();
    let ingestor = ExperienceIngestor::new(evidence_store.clone());

    evidence_store.append(make_evidence("ev-1")).await.unwrap();
    let envelope = make_envelope("exp-1", vec!["ev-1"]);
    ingestor.ingest(&envelope, &exp_store).await.unwrap();
    let outcome = ingestor.ingest(&envelope, &exp_store).await.unwrap();
    assert_eq!(outcome, AppendOutcome::AlreadyPresent);
}

#[tokio::test]
async fn invalid_schema_is_rejected() {
    let evidence_store = Arc::new(JsonlEvidenceStore::in_memory());
    let exp_store = InMemoryExperienceStore::new();
    let ingestor = ExperienceIngestor::new(evidence_store);

    let mut envelope = make_envelope("exp-1", vec![]);
    envelope.schema_version = 999;
    let result = ingestor.ingest(&envelope, &exp_store).await;
    assert!(matches!(
        result,
        Err(ExperienceIngestError::UnsupportedSchema(999))
    ));
}

#[tokio::test]
async fn invalid_time_ordering_is_rejected() {
    let evidence_store = Arc::new(JsonlEvidenceStore::in_memory());
    let exp_store = InMemoryExperienceStore::new();
    let ingestor = ExperienceIngestor::new(evidence_store);

    let mut envelope = make_envelope("exp-1", vec![]);
    envelope.completed_at_ms = Some(50); // before started_at_ms (100)
    let result = ingestor.ingest(&envelope, &exp_store).await;
    assert!(matches!(
        result,
        Err(ExperienceIngestError::InvalidTimeOrdering)
    ));
}
