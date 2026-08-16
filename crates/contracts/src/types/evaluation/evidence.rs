use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::types::metacognition_evidence::{EvidenceId, EvidenceItem};

use super::{EvaluationContractError, EvaluationContractId, EVALUATION_SCHEMA_V1};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct EvaluationSnapshotId(pub Uuid);

impl EvaluationSnapshotId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for EvaluationSnapshotId {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvaluationEvidenceSnapshot {
    pub schema_version: u16,
    pub snapshot_id: EvaluationSnapshotId,
    pub contract_id: EvaluationContractId,
    pub evidence: Vec<EvidenceItem>,
    pub sha256: String,
    pub created_at_ms: i64,
}

impl EvaluationEvidenceSnapshot {
    pub fn new(
        contract_id: EvaluationContractId,
        mut evidence: Vec<EvidenceItem>,
        created_at_ms: i64,
    ) -> Result<Self, EvaluationContractError> {
        evidence.sort_by(|left, right| left.evidence_id.cmp(&right.evidence_id));
        validate_evidence(&evidence)?;
        let sha256 = snapshot_digest(&contract_id, &evidence)?;
        Ok(Self {
            schema_version: EVALUATION_SCHEMA_V1,
            snapshot_id: EvaluationSnapshotId::new(),
            contract_id,
            evidence,
            sha256,
            created_at_ms,
        })
    }

    pub fn validate(&self) -> Result<(), EvaluationContractError> {
        if self.schema_version != EVALUATION_SCHEMA_V1 {
            return Err(EvaluationContractError::UnsupportedSchema(
                self.schema_version,
            ));
        }
        validate_evidence(&self.evidence)?;
        if snapshot_digest(&self.contract_id, &self.evidence)? != self.sha256 {
            return Err(EvaluationContractError::SnapshotDigestMismatch);
        }
        Ok(())
    }

    pub fn evidence_ids(&self) -> Vec<EvidenceId> {
        self.evidence
            .iter()
            .map(|item| item.evidence_id.clone())
            .collect()
    }
}

fn validate_evidence(evidence: &[EvidenceItem]) -> Result<(), EvaluationContractError> {
    let mut ids = HashSet::with_capacity(evidence.len());
    for item in evidence {
        if !ids.insert(item.evidence_id.clone()) {
            return Err(EvaluationContractError::DuplicateEvidence(
                item.evidence_id.0.clone(),
            ));
        }
        let bytes = serde_json::to_vec(&item.payload)
            .map_err(|error| EvaluationContractError::Serialization(error.to_string()))?;
        let digest = format!("{:x}", Sha256::digest(bytes));
        if digest != item.sha256 {
            return Err(EvaluationContractError::EvidenceDigestMismatch(
                item.evidence_id.0.clone(),
            ));
        }
    }
    Ok(())
}

fn snapshot_digest(
    contract_id: &EvaluationContractId,
    evidence: &[EvidenceItem],
) -> Result<String, EvaluationContractError> {
    let material = serde_json::json!({
        "contract_id": contract_id,
        "evidence": evidence,
    });
    let bytes = serde_json::to_vec(&material)
        .map_err(|error| EvaluationContractError::Serialization(error.to_string()))?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::metacognition_evidence::{EvidenceKind, EvidenceTrust};
    use crate::types::metacognition_experience::{ExperienceId, METACOGNITION_SCHEMA_V1};

    fn item(id: &str, payload: serde_json::Value) -> EvidenceItem {
        let bytes = serde_json::to_vec(&payload).unwrap();
        EvidenceItem {
            schema_version: METACOGNITION_SCHEMA_V1,
            evidence_id: EvidenceId(id.into()),
            experience_id: ExperienceId("experience".into()),
            kind: EvidenceKind::Observation,
            source: "test".into(),
            producer: "test".into(),
            captured_at_ms: 1,
            payload,
            sha256: format!("{:x}", Sha256::digest(bytes)),
            trust: EvidenceTrust::Authoritative,
            freshness_ms: None,
            redacted: false,
        }
    }

    #[test]
    fn evidence_snapshot_digest_detects_payload_changes() {
        let snapshot = EvaluationEvidenceSnapshot::new(
            EvaluationContractId::new(),
            vec![item("e-1", serde_json::json!({"ok": true}))],
            1,
        )
        .unwrap();
        assert!(snapshot.validate().is_ok());
        let mut tampered = snapshot;
        tampered.evidence[0].payload = serde_json::json!({"ok": false});
        assert!(matches!(
            tampered.validate(),
            Err(EvaluationContractError::EvidenceDigestMismatch(_))
        ));
    }

    #[test]
    fn evidence_snapshot_sorts_ids_before_hashing() {
        let snapshot = EvaluationEvidenceSnapshot::new(
            EvaluationContractId::new(),
            vec![
                item("e-2", serde_json::json!({"n": 2})),
                item("e-1", serde_json::json!({"n": 1})),
            ],
            1,
        )
        .unwrap();
        assert_eq!(snapshot.evidence[0].evidence_id.0, "e-1");
        assert_eq!(snapshot.evidence[1].evidence_id.0, "e-2");
    }
}
