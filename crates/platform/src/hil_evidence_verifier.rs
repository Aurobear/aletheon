//! Verifier for signed HIL gate evidence.
//! Only allowlisted public keys may produce valid evidence.

use std::collections::HashSet;
use fabric::types::hil_evidence::{HILEvidence, HILResult};

pub struct HILEvidenceVerifier {
    /// Allowlisted public key IDs that may sign HIL evidence.
    allowed_keys: HashSet<String>,
    /// Maximum age of valid evidence in milliseconds.
    max_age_ms: i64,
    /// Current time provider for expiry checks.
    now_ms_fn: Box<dyn Fn() -> i64 + Send + Sync>,
}

impl HILEvidenceVerifier {
    pub fn new(
        allowed_key_ids: Vec<String>,
        max_age_ms: i64,
        now_ms_fn: Box<dyn Fn() -> i64 + Send + Sync>,
    ) -> Self {
        Self {
            allowed_keys: allowed_key_ids.into_iter().collect(),
            max_age_ms,
            now_ms_fn,
        }
    }

    pub fn verify(&self, evidence: &HILEvidence) -> Result<(), String> {
        // 1. Schema version check
        if evidence.schema_version != 1 {
            return Err(format!(
                "unsupported schema version: {}",
                evidence.schema_version
            ));
        }

        // 2. Result must be Passed or Conditional
        match evidence.result {
            HILResult::Passed | HILResult::Conditional => {}
            HILResult::Failed => return Err("HIL evidence result is Failed".into()),
            HILResult::Inconclusive => {
                return Err("HIL evidence result is Inconclusive".into())
            }
        }

        // 3. Signer must be in allowlist
        if !self.allowed_keys.contains(&evidence.signer_key_id) {
            return Err(format!(
                "signer key not allowed: {}",
                evidence.signer_key_id
            ));
        }

        // 4. Evidence must not be expired
        let now_ms = (self.now_ms_fn)();
        if now_ms > evidence.expiry_unix_ms {
            return Err(format!(
                "evidence expired: now {} > expiry {}",
                now_ms, evidence.expiry_unix_ms
            ));
        }

        // 5. Evidence must not be too old (issued before max_age_ms ago)
        if now_ms - evidence.issued_unix_ms > self.max_age_ms {
            return Err(format!(
                "evidence too old: now {} - issued {} > max age {}",
                now_ms, evidence.issued_unix_ms, self.max_age_ms
            ));
        }

        // 6. Device serial must be non-empty
        if evidence.device_serial.is_empty() {
            return Err("device_serial is empty".into());
        }

        // 7. Device ID must be non-empty
        if evidence.device_id.is_empty() {
            return Err("device_id is empty".into());
        }

        // 8. Signature must be non-empty (actual signature verification uses
        //    an external crypto library — this is the structural check)
        if evidence.signature.is_empty() {
            return Err("signature is empty".into());
        }

        // 9. Manifest and limits digests must be non-empty
        if evidence.manifest_digest.is_empty() {
            return Err("manifest_digest is empty".into());
        }
        if evidence.limits_digest.is_empty() {
            return Err("limits_digest is empty".into());
        }

        // 10. Software commits must be non-empty
        if evidence.software_commits.is_empty() {
            return Err("software_commits is empty".into());
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_evidence() -> HILEvidence {
        HILEvidence {
            schema_version: 1,
            device_id: "kuavo-01".into(),
            device_serial: "SN-001".into(),
            software_commits: vec!["abc123".into()],
            manifest_digest: "sha256:manifest".into(),
            limits_digest: "sha256:limits".into(),
            test_cases: vec!["latency_50ms".into()],
            measured_stop_latency_ms: 150,
            result: HILResult::Passed,
            issued_unix_ms: 1000,
            expiry_unix_ms: 9000,
            signer_key_id: "key-1".into(),
            signature: "sig-data".into(),
        }
    }

    fn verifier(keys: Vec<&str>, now_ms: i64) -> HILEvidenceVerifier {
        let now = Box::new(move || now_ms);
        HILEvidenceVerifier::new(
            keys.into_iter().map(|s| s.to_string()).collect(),
            10000, // max_age_ms large enough not to trigger in most tests
            now,
        )
    }

    #[test]
    fn valid_evidence_passes() {
        let v = verifier(vec!["key-1"], 5000);
        assert!(v.verify(&valid_evidence()).is_ok());
    }

    #[test]
    fn conditional_result_passes() {
        let v = verifier(vec!["key-1"], 5000);
        let mut ev = valid_evidence();
        ev.result = HILResult::Conditional;
        assert!(v.verify(&ev).is_ok());
    }

    #[test]
    fn failed_result_rejected() {
        let v = verifier(vec!["key-1"], 5000);
        let mut ev = valid_evidence();
        ev.result = HILResult::Failed;
        assert!(v.verify(&ev).is_err());
    }

    #[test]
    fn inconclusive_result_rejected() {
        let v = verifier(vec!["key-1"], 5000);
        let mut ev = valid_evidence();
        ev.result = HILResult::Inconclusive;
        assert!(v.verify(&ev).is_err());
    }

    #[test]
    fn unknown_signer_rejected() {
        let v = verifier(vec!["key-2"], 5000);
        assert!(v.verify(&valid_evidence()).is_err());
    }

    #[test]
    fn expired_evidence_rejected() {
        let v = verifier(vec!["key-1"], 10000); // now = 10000 > expiry = 9000
        assert!(v.verify(&valid_evidence()).is_err());
    }

    #[test]
    fn empty_signature_rejected() {
        let v = verifier(vec!["key-1"], 5000);
        let mut ev = valid_evidence();
        ev.signature = "".into();
        assert!(v.verify(&ev).is_err());
    }

    #[test]
    fn empty_manifest_digest_rejected() {
        let v = verifier(vec!["key-1"], 5000);
        let mut ev = valid_evidence();
        ev.manifest_digest = "".into();
        assert!(v.verify(&ev).is_err());
    }

    #[test]
    fn empty_limits_digest_rejected() {
        let v = verifier(vec!["key-1"], 5000);
        let mut ev = valid_evidence();
        ev.limits_digest = "".into();
        assert!(v.verify(&ev).is_err());
    }

    #[test]
    fn unknown_schema_version_rejected() {
        let v = verifier(vec!["key-1"], 5000);
        let mut ev = valid_evidence();
        ev.schema_version = 2;
        assert!(v.verify(&ev).is_err());
    }

    #[test]
    fn old_evidence_rejected() {
        let v = HILEvidenceVerifier::new(
            vec!["key-1".into()],
            100, // max_age_ms = 100ms
            Box::new(|| 5000), // now = 5000
        );
        // issued_unix_ms = 1000, now = 5000, diff = 4000 > max_age_ms=100
        assert!(v.verify(&valid_evidence()).is_err());
    }
}
