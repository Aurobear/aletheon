//! Proposal validator — validates SkillProposals against registered skill descriptors.
//! Never trusts provider-supplied schema; validates against live ListSkills output.

use ::contracts::types::embodiment::{DeviceId, SkillDescriptor};
use ::contracts::types::expected_outcome::{get_path, OutcomePredicate};
use ::contracts::types::skill_proposal::{GoalAlignment, SkillProposal};
use ::contracts::types::world_state::WorldSnapshot;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationError {
    pub field: String,
    pub message: String,
}

/// Validate a SkillProposal against registered skill descriptors.
pub fn validate_proposal(
    proposal: &SkillProposal,
    requested_device: &DeviceId,
    allowed_skills: &[SkillDescriptor],
    snapshots: &[WorldSnapshot],
) -> Result<(), Vec<ValidationError>> {
    let mut errors = Vec::new();

    // A safety fallback is a typed refusal to fulfill the requested goal. It is
    // valid Policy evidence, but never a normal action proposal: the Harness
    // fails closed into its own SafeStop authority instead of executing the
    // fallback and mistaking its predicate for goal completion.
    if proposal.goal_alignment == GoalAlignment::SafetyFallback {
        errors.push(ValidationError {
            field: "goal_alignment".into(),
            message: "policy selected a safety fallback rather than a direct goal action".into(),
        });
    }

    // 1. Skill must be registered
    let descriptor = match allowed_skills.iter().find(|s| s.skill == proposal.skill) {
        Some(d) => d,
        None => {
            errors.push(ValidationError {
                field: "skill_id".into(),
                message: format!(
                    "skill '{}' is not registered for device '{}'",
                    proposal.skill.0, proposal.device.0
                ),
            });
            return Err(errors);
        }
    };

    // 2. Device must match both the host request and negotiated descriptor.
    if &proposal.device != requested_device {
        errors.push(ValidationError {
            field: "device_id".into(),
            message: format!(
                "proposal device '{}' does not match requested device '{}'",
                proposal.device.0, requested_device.0
            ),
        });
    }
    if proposal.device != descriptor.device {
        errors.push(ValidationError {
            field: "device_id".into(),
            message: format!(
                "proposal device '{}' does not match descriptor device '{}'",
                proposal.device.0, descriptor.device.0
            ),
        });
    }

    // 3. Confidence must be finite and in [0,1]. NaN otherwise bypasses both
    // comparison branches.
    if !proposal.confidence.is_finite() || proposal.confidence < 0.0 || proposal.confidence > 1.0 {
        errors.push(ValidationError {
            field: "confidence".into(),
            message: format!("confidence {} out of range [0,1]", proposal.confidence),
        });
    }

    // 4. Frame refs bounded
    if proposal.frame_refs.len() > 4 {
        errors.push(ValidationError {
            field: "frame_refs".into(),
            message: format!("too many frame refs: {} > 4", proposal.frame_refs.len()),
        });
    }
    for (index, frame) in proposal.frame_refs.iter().enumerate() {
        if let Err(message) = frame.validate() {
            errors.push(ValidationError {
                field: format!("frame_refs[{index}]"),
                message,
            });
        }
    }

    // 5. Every provenance fact is typed. GrpcPolicyProvider host-binds provider
    // and protocol to startup connection facts before this validator runs.
    for (field, value) in [
        ("provider", proposal.provenance.provider.as_str()),
        ("model", proposal.provenance.model.as_str()),
        ("version", proposal.provenance.version.as_str()),
        (
            "protocol_version",
            proposal.provenance.protocol_version.as_str(),
        ),
        ("digest", proposal.provenance.digest.as_str()),
    ] {
        if value.trim().is_empty() {
            errors.push(ValidationError {
                field: format!("provenance.{field}"),
                message: format!("policy provenance {field} is required"),
            });
        }
    }

    // 6. Expected outcome must validate
    if let Err(e) = proposal.expected_outcome.validate() {
        errors.push(ValidationError {
            field: "expected_outcome".into(),
            message: format!("invalid expected outcome: {e}"),
        });
    }
    validate_expected_outcome_context(
        &proposal.expected_outcome.predicate,
        requested_device,
        snapshots,
        &mut errors,
    );
    if proposal.expected_outcome.timeout_ms == 0
        || proposal.expected_outcome.timeout_ms > descriptor.timeout_ms
    {
        errors.push(ValidationError {
            field: "expected_outcome.timeout_ms".into(),
            message: format!(
                "timeout {} must be within 1..={} from the negotiated skill descriptor",
                proposal.expected_outcome.timeout_ms, descriptor.timeout_ms
            ),
        });
    }
    if proposal.expected_outcome.stable_window_ms > proposal.expected_outcome.timeout_ms
        || proposal.expected_outcome.stable_window_ms > descriptor.timeout_ms
    {
        errors.push(ValidationError {
            field: "expected_outcome.stable_window_ms".into(),
            message: format!(
                "stable window {} exceeds the proposal/descriptor timeout cap",
                proposal.expected_outcome.stable_window_ms
            ),
        });
    }

    // 7. Validate the complete parameter object against the exact descriptor
    // negotiated at startup. Unsupported schema semantics were rejected before
    // this allowlist was exposed.
    if let Err(message) = descriptor.validate_parameters(&proposal.parameters) {
        errors.push(ValidationError {
            field: "parameters".into(),
            message,
        });
    }

    // 8. Reject proposals that look like raw actuation (joint/torque in params)
    if proposal.parameters.as_object().is_some_and(|p| {
        p.contains_key("joint")
            || p.contains_key("torque")
            || p.contains_key("topic")
            || p.contains_key("actuator")
    }) {
        errors.push(ValidationError {
            field: "parameters".into(),
            message:
                "raw actuation parameters (joint/torque/topic/actuator) are forbidden in proposals"
                    .into(),
        });
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

fn validate_expected_outcome_context(
    predicate: &OutcomePredicate,
    requested_device: &DeviceId,
    snapshots: &[WorldSnapshot],
    errors: &mut Vec<ValidationError>,
) {
    match predicate {
        OutcomePredicate::Equals { path, .. } | OutcomePredicate::NotEquals { path, .. } => {
            validate_observed_path(path, false, requested_device, snapshots, errors);
        }
        OutcomePredicate::Range { path, .. } | OutcomePredicate::Change { path, .. } => {
            validate_observed_path(path, true, requested_device, snapshots, errors);
        }
        OutcomePredicate::All { predicates } | OutcomePredicate::Any { predicates } => {
            for child in predicates {
                validate_expected_outcome_context(child, requested_device, snapshots, errors);
            }
        }
    }
}

/// A proposal may use an unqualified path (`mode`) for a single-schema world or
/// a schema-qualified path (`base_twist.linear.x`). At least one fresh typed
/// snapshot must prove that the path is observable before execution is allowed.
fn validate_observed_path(
    path: &str,
    numeric: bool,
    requested_device: &DeviceId,
    snapshots: &[WorldSnapshot],
    errors: &mut Vec<ValidationError>,
) {
    let mut path_exists = false;
    let mut numeric_path_exists = false;
    for snapshot in snapshots.iter().filter(|snapshot| {
        snapshot.device == *requested_device
            && !snapshot.stale
            && !snapshot.schema.trim().is_empty()
            && snapshot.schema_version > 0
    }) {
        let qualified_prefix = format!("{}.", snapshot.schema);
        let candidate = path.strip_prefix(&qualified_prefix).unwrap_or(path);
        if let Some(value) = get_path(&snapshot.payload, candidate) {
            path_exists = true;
            numeric_path_exists |= value.as_f64().is_some();
        }
    }
    if !path_exists {
        errors.push(ValidationError {
            field: "expected_outcome.predicate.path".into(),
            message: format!(
                "path '{path}' is not observable in any fresh typed snapshot for '{}'",
                requested_device.0
            ),
        });
    } else if numeric && !numeric_path_exists {
        errors.push(ValidationError {
            field: "expected_outcome.predicate.path".into(),
            message: format!("path '{path}' is not numeric in any fresh typed snapshot"),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ::contracts::types::embodiment::{DeviceId, RiskClass, SkillId};
    use ::contracts::types::expected_outcome::{ExpectedOutcome, OutcomePredicate};
    use ::contracts::types::skill_proposal::{PolicyProvenance, SkillProposal};
    use ::contracts::MonoTime;

    fn allowed_skills() -> Vec<SkillDescriptor> {
        vec![SkillDescriptor {
            skill: SkillId("kuavo.stance".into()),
            device: DeviceId("bot".into()),
            summary: "stable stance".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {},
                "required": [],
                "additionalProperties": false
            }),
            risk: RiskClass::Low,
            timeout_ms: 10000,
            cancellable: false,
            preconditions: vec!["ready".into()],
            success_criteria: vec!["stable".into()],
        }]
    }

    fn valid_proposal() -> SkillProposal {
        SkillProposal {
            skill: SkillId("kuavo.stance".into()),
            device: DeviceId("bot".into()),
            parameters: serde_json::json!({}),
            goal_alignment: ::contracts::types::skill_proposal::GoalAlignment::Direct,
            expected_outcome: ExpectedOutcome {
                predicate: OutcomePredicate::Equals {
                    path: "mode".into(),
                    value: serde_json::json!("stance"),
                },
                freshness_ms: 500,
                stable_window_ms: 0,
                timeout_ms: 5000,
            },
            confidence: 0.9,
            frame_refs: vec![],
            provenance: PolicyProvenance {
                provider: "test-vla".into(),
                model: "v1".into(),
                version: "1.0".into(),
                protocol_version: "1.0".into(),
                digest: "sha256:abc123".into(),
            },
        }
    }

    fn snapshots() -> Vec<WorldSnapshot> {
        vec![WorldSnapshot {
            device: DeviceId("bot".into()),
            schema: "robot.state".into(),
            schema_version: 1,
            sequence: 1,
            payload: serde_json::json!({"mode": "idle", "speed": 0.0}),
            observed_at: MonoTime(1),
            valid_until: None,
            stale: false,
        }]
    }

    fn validate(proposal: &SkillProposal) -> Result<(), Vec<ValidationError>> {
        validate_proposal(
            proposal,
            &DeviceId("bot".into()),
            &allowed_skills(),
            &snapshots(),
        )
    }

    #[test]
    fn valid_proposal_passes() {
        assert!(validate(&valid_proposal()).is_ok());
    }

    #[test]
    fn safety_fallback_is_not_executable_as_a_goal_action() {
        let mut proposal = valid_proposal();
        proposal.goal_alignment = GoalAlignment::SafetyFallback;
        let errors = validate(&proposal).unwrap_err();
        assert!(errors.iter().any(|error| {
            error.field == "goal_alignment" && error.message.contains("safety fallback")
        }));
    }

    #[test]
    fn unknown_skill_rejected() {
        let mut p = valid_proposal();
        p.skill = SkillId("nonexistent.skill".into());
        assert!(validate(&p).is_err());
    }

    #[test]
    fn confidence_out_of_range() {
        let mut p = valid_proposal();
        p.confidence = 1.5;
        assert!(validate(&p).is_err());
        p.confidence = -0.1;
        assert!(validate(&p).is_err());
        p.confidence = f32::NAN;
        assert!(validate(&p).is_err());
    }

    #[test]
    fn raw_joint_params_rejected() {
        let mut p = valid_proposal();
        p.parameters = serde_json::json!({"joint": [0.1, 0.2]});
        assert!(validate(&p).is_err());
    }

    #[test]
    fn missing_provenance_digest_rejected() {
        let mut p = valid_proposal();
        p.provenance.digest = "".into();
        assert!(validate(&p).is_err());
    }

    #[test]
    fn too_many_frame_refs_rejected() {
        let mut p = valid_proposal();
        p.frame_refs = (0..5).map(|_| sample_frame()).collect();
        assert!(validate(&p).is_err());
    }

    #[test]
    fn requested_device_and_snapshot_schema_are_authoritative() {
        let proposal = valid_proposal();
        assert!(validate_proposal(
            &proposal,
            &DeviceId("other".into()),
            &allowed_skills(),
            &snapshots(),
        )
        .is_err());

        let mut invalid_schema = snapshots();
        invalid_schema[0].schema_version = 0;
        let errors = validate_proposal(
            &proposal,
            &DeviceId("bot".into()),
            &allowed_skills(),
            &invalid_schema,
        )
        .unwrap_err();
        assert!(errors
            .iter()
            .any(|error| error.field == "expected_outcome.predicate.path"));
    }

    #[test]
    fn expected_path_and_numeric_type_must_be_observable() {
        let mut proposal = valid_proposal();
        proposal.expected_outcome.predicate = OutcomePredicate::Equals {
            path: "missing.path".into(),
            value: serde_json::json!(true),
        };
        assert!(validate(&proposal).is_err());

        proposal.expected_outcome.predicate = OutcomePredicate::Range {
            path: "mode".into(),
            min: Some(0.0),
            max: Some(1.0),
        };
        assert!(validate(&proposal).is_err());

        proposal.expected_outcome.predicate = OutcomePredicate::Range {
            path: "robot.state.speed".into(),
            min: Some(0.0),
            max: Some(1.0),
        };
        assert!(validate(&proposal).is_ok());
    }

    #[test]
    fn expected_timeout_and_stable_window_obey_descriptor_cap() {
        let mut proposal = valid_proposal();
        proposal.expected_outcome.timeout_ms = 10_001;
        assert!(validate(&proposal).is_err());

        proposal.expected_outcome.timeout_ms = 5_000;
        proposal.expected_outcome.stable_window_ms = 5_001;
        assert!(validate(&proposal).is_err());

        proposal.expected_outcome.timeout_ms = 0;
        proposal.expected_outcome.stable_window_ms = 0;
        assert!(validate(&proposal).is_err());
    }

    #[test]
    fn parameter_outside_negotiated_hard_bound_is_rejected() {
        let proposal = SkillProposal {
            parameters: serde_json::json!({"linear_x": 0.3}),
            goal_alignment: ::contracts::types::skill_proposal::GoalAlignment::Direct,
            ..valid_proposal()
        };
        let mut descriptors = allowed_skills();
        descriptors[0].input_schema = serde_json::json!({
            "type": "object",
            "properties": {
                "linear_x": {
                    "type": "number",
                    "minimum": -0.2,
                    "maximum": 0.2
                }
            },
            "required": ["linear_x"],
            "additionalProperties": false
        });

        let errors = validate_proposal(
            &proposal,
            &DeviceId("bot".into()),
            &descriptors,
            &snapshots(),
        )
        .unwrap_err();
        assert!(errors.iter().any(|error| error.field == "parameters"));
    }

    #[test]
    fn every_policy_provenance_fact_is_required() {
        let mut proposal = valid_proposal();
        for field in ["provider", "model", "version", "protocol", "digest"] {
            let mut candidate = proposal.clone();
            match field {
                "provider" => candidate.provenance.provider.clear(),
                "model" => candidate.provenance.model.clear(),
                "version" => candidate.provenance.version.clear(),
                "protocol" => candidate.provenance.protocol_version.clear(),
                "digest" => candidate.provenance.digest.clear(),
                _ => unreachable!(),
            }
            assert!(validate(&candidate).is_err(), "field {field}");
        }
        proposal.provenance.provider = "host-bound".into();
        assert!(validate(&proposal).is_ok());
    }

    fn sample_frame() -> ::contracts::types::frame::FrameRef {
        let digest = "a".repeat(64);
        ::contracts::types::frame::FrameRef {
            uri: format!("artifact://sha256/{digest}"),
            sha256: digest,
            mime_type: "image/jpeg".into(),
            width: 1,
            height: 1,
            byte_len: 1,
            source_time_ms: 1,
            camera_id: "camera".into(),
            frame_id: 1,
        }
    }
}
