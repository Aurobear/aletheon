//! Integration tests for the HILEvidenceVerifier public API.

use fabric::types::hil_evidence::{HILEvidence, HILResult};
use metacog::hil_evidence_verifier::HILEvidenceVerifier;

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
    HILEvidenceVerifier::new(
        keys.into_iter().map(|s| s.to_string()).collect(),
        10000,
        Box::new(move || now_ms),
    )
}

#[test]
fn valid_passed_evidence_accepted() {
    let v = verifier(vec!["key-1"], 5000);
    let ev = valid_evidence();
    assert!(
        v.verify(&ev).is_ok(),
        "valid passed evidence must be accepted"
    );
}

#[test]
fn valid_conditional_evidence_accepted() {
    let v = verifier(vec!["key-1"], 5000);
    let mut ev = valid_evidence();
    ev.result = HILResult::Conditional;
    assert!(
        v.verify(&ev).is_ok(),
        "conditional evidence must be accepted"
    );
}

#[test]
fn failed_evidence_rejected() {
    let v = verifier(vec!["key-1"], 5000);
    let mut ev = valid_evidence();
    ev.result = HILResult::Failed;
    assert!(v.verify(&ev).is_err());
}

#[test]
fn inconclusive_evidence_rejected() {
    let v = verifier(vec!["key-1"], 5000);
    let mut ev = valid_evidence();
    ev.result = HILResult::Inconclusive;
    assert!(v.verify(&ev).is_err());
}

#[test]
fn unauthorized_signer_rejected() {
    let v = verifier(vec!["authorized-key-99"], 5000);
    assert!(v.verify(&valid_evidence()).is_err());
}

#[test]
fn multiple_authorized_signers_accepted() {
    let v = verifier(vec!["key-a", "key-1", "key-z"], 5000);
    assert!(v.verify(&valid_evidence()).is_ok());
}

#[test]
fn expired_evidence_rejected() {
    let v = verifier(vec!["key-1"], 10000); // now exceeds expiry
    assert!(v.verify(&valid_evidence()).is_err());
}

#[test]
fn structural_validation_failures() {
    let v = verifier(vec!["key-1"], 5000);

    let mut ev = valid_evidence();
    ev.signature = "".into();
    assert!(v.verify(&ev).is_err(), "empty signature rejected");

    let mut ev = valid_evidence();
    ev.manifest_digest = "".into();
    assert!(v.verify(&ev).is_err(), "empty manifest digest rejected");

    let mut ev = valid_evidence();
    ev.limits_digest = "".into();
    assert!(v.verify(&ev).is_err(), "empty limits digest rejected");

    let mut ev = valid_evidence();
    ev.device_serial = "".into();
    assert!(v.verify(&ev).is_err(), "empty device serial rejected");

    let mut ev = valid_evidence();
    ev.device_id = "".into();
    assert!(v.verify(&ev).is_err(), "empty device id rejected");

    let mut ev = valid_evidence();
    ev.software_commits = vec![];
    assert!(v.verify(&ev).is_err(), "empty software commits rejected");
}

#[test]
fn unknown_schema_version_rejected() {
    let v = verifier(vec!["key-1"], 5000);
    let mut ev = valid_evidence();
    ev.schema_version = 3;
    assert!(v.verify(&ev).is_err());
}

#[test]
fn age_based_rejection() {
    let v = HILEvidenceVerifier::new(
        vec!["key-1".into()],
        500,               // max age 500ms
        Box::new(|| 5000), // now
    );
    // issued = 1000, now = 5000, diff = 4000 > 500
    assert!(v.verify(&valid_evidence()).is_err());
}
