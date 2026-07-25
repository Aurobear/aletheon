//! Table-driven integration tests for wire↔domain conversions.

use hardware::grpc::convert;
use hardware::grpc::wire;

#[test]
fn every_risk_class_converts() {
    let cases = [
        (wire::RiskClass::Read as i32, "Read"),
        (wire::RiskClass::Low as i32, "Low"),
        (wire::RiskClass::Medium as i32, "Medium"),
        (wire::RiskClass::High as i32, "High"),
    ];
    for (wire_val, expected_debug) in &cases {
        let domain = convert::to_risk_class(*wire_val).unwrap();
        assert_eq!(format!("{domain:?}").as_str(), *expected_debug);
    }
}

#[test]
fn every_skill_outcome_converts() {
    let cases = [
        (
            wire::SkillOutcome::Succeeded as i32,
            "",
            fabric::types::embodiment::SkillOutcome::Succeeded,
        ),
        (
            wire::SkillOutcome::Failed as i32,
            "motor stall",
            fabric::types::embodiment::SkillOutcome::Failed {
                reason: "motor stall".into(),
            },
        ),
        (
            wire::SkillOutcome::Cancelled as i32,
            "",
            fabric::types::embodiment::SkillOutcome::Cancelled,
        ),
        (
            wire::SkillOutcome::TimedOut as i32,
            "",
            fabric::types::embodiment::SkillOutcome::TimedOut,
        ),
    ];
    for (wire_val, reason, expected) in &cases {
        let domain = convert::to_skill_outcome(*wire_val, reason.to_string()).unwrap();
        assert_eq!(domain, *expected);
    }
}

#[test]
fn failure_reason_is_preserved_from_wire() {
    let wr = wire::SkillResult {
        operation_id: "00000000-0000-4000-8000-000000000042".into(),
        skill_id: "kuavo.move".into(),
        device_id: "bot".into(),
        outcome: wire::SkillOutcome::Failed as i32,
        failure_reason: "obstacle detected".into(),
        ..Default::default()
    };
    let dr = convert::to_skill_result(&wr).unwrap();
    match dr.outcome {
        fabric::types::embodiment::SkillOutcome::Failed { reason } => {
            assert_eq!(reason, "obstacle detected");
        }
        other => panic!("expected Failed, got {other:?}"),
    }
}

#[test]
fn invalid_uuid_in_result_is_rejected() {
    let wr = wire::SkillResult {
        operation_id: "not-a-uuid".into(),
        skill_id: "s".into(),
        device_id: "d".into(),
        outcome: wire::SkillOutcome::Succeeded as i32,
        ..Default::default()
    };
    assert!(convert::to_skill_result(&wr).is_err());
}

#[test]
fn evidence_refs_are_collected() {
    let wr = wire::SkillResult {
        operation_id: "00000000-0000-4000-8000-000000000001".into(),
        skill_id: "s".into(),
        device_id: "d".into(),
        outcome: wire::SkillOutcome::Succeeded as i32,
        evidence: vec![
            wire::EvidenceRef {
                kind: "rosbag".into(),
                uri: "artifact://b/1".into(),
            },
            wire::EvidenceRef {
                kind: "log".into(),
                uri: "artifact://l/2".into(),
            },
        ],
        ..Default::default()
    };
    let dr = convert::to_skill_result(&wr).unwrap();
    assert_eq!(dr.evidence.len(), 2);
    assert_eq!(dr.evidence[0].kind, "rosbag");
    assert_eq!(dr.evidence[1].kind, "log");
}

#[test]
fn timestamp_mapping_is_non_negative() {
    let wo = wire::Observation {
        source_unix_ms: -100, // negative wire timestamp → clamped to 0
        received_unix_ms: 50,
        ..Default::default()
    };
    let obs = convert::to_observation(&wo).unwrap();
    assert_eq!(obs.source_time.0, 0);
}

#[test]
fn missing_frame_ref_is_none() {
    let wo = wire::Observation {
        frame_ref: String::new(),
        ..Default::default()
    };
    let obs = convert::to_observation(&wo).unwrap();
    assert_eq!(obs.frame_ref, None);
}

#[test]
fn skill_descriptor_preserves_all_fields() {
    let wsd = wire::SkillDescriptor {
        skill_id: "kuavo.stance".into(),
        device_id: "kuavo-mujoco-01".into(),
        summary: "stable stance".into(),
        risk: wire::RiskClass::Low as i32,
        timeout_ms: 10000,
        cancellable: true,
        preconditions: vec!["fresh state".into()],
        success_criteria: vec!["stance confirmed".into()],
        ..Default::default()
    };
    let dsd = convert::to_skill_descriptor(&wsd).unwrap();
    assert_eq!(dsd.skill.0, "kuavo.stance");
    assert_eq!(dsd.device.0, "kuavo-mujoco-01");
    assert_eq!(dsd.timeout_ms, 10000);
    assert!(dsd.cancellable);
    assert_eq!(dsd.preconditions.len(), 1);
    assert_eq!(dsd.success_criteria.len(), 1);
}

#[test]
fn no_conversion_depends_on_display_strings() {
    // All conversions use structured enum matching, never Display parsing.
    // This test verifies the conversion functions exist and compile.
    // The unit tests in grpc::convert cover exhaustive enum matching.

    // RiskClass: uses i32 → wire::RiskClass → domain::RiskClass mapping
    let rc = convert::to_risk_class(wire::RiskClass::High as i32).unwrap();
    assert_eq!(rc, fabric::types::embodiment::RiskClass::High);

    // SkillOutcome: uses i32 → wire::SkillOutcome → domain::SkillOutcome
    let so =
        convert::to_skill_outcome(wire::SkillOutcome::Cancelled as i32, String::new()).unwrap();
    assert_eq!(so, fabric::types::embodiment::SkillOutcome::Cancelled);
}
