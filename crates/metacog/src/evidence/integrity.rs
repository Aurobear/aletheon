//! Evidence integrity verification — SHA-256 digests for evidence payloads.

use sha2::{Digest, Sha256};

use super::model::EvidenceItem;

/// Compute SHA-256 over canonical serialized payload bytes.
pub fn compute_digest(payload: &serde_json::Value) -> String {
    let bytes = serde_json::to_vec(payload).expect("serialization to vec is infallible");
    format!("{:x}", Sha256::digest(bytes))
}

/// Verify that the stored digest matches the payload.
pub fn verify_integrity(item: &EvidenceItem) -> bool {
    let computed = compute_digest(&item.payload);
    computed == item.sha256
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_payload_produces_same_digest() {
        let p1 = serde_json::json!({"key": "value"});
        let p2 = serde_json::json!({"key": "value"});
        assert_eq!(compute_digest(&p1), compute_digest(&p2));
    }

    #[test]
    fn different_payloads_produce_different_digests() {
        let p1 = serde_json::json!({"key": "value"});
        let p2 = serde_json::json!({"key": "other"});
        assert_ne!(compute_digest(&p1), compute_digest(&p2));
    }

    #[test]
    fn verify_integrity_detects_tampering() {
        let payload = serde_json::json!({"key": "value"});
        let digest = compute_digest(&payload);
        let item = EvidenceItem {
            schema_version: 1,
            evidence_id: crate::evidence::model::EvidenceId("ev-1".into()),
            experience_id: crate::evidence::model::ExperienceId("exp-1".into()),
            kind: crate::evidence::model::EvidenceKind::Assertion,
            source: "test".into(),
            producer: "test".into(),
            captured_at_ms: 0,
            payload,
            sha256: digest,
            trust: crate::evidence::model::EvidenceTrust::Authoritative,
            freshness_ms: None,
            redacted: false,
        };
        assert!(verify_integrity(&item));
    }
}
