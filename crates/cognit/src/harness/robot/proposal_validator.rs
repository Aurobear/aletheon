//! Proposal validator — validates SkillProposals against registered skill descriptors.
//! Never trusts provider-supplied schema; validates against live ListSkills output.

use fabric::types::embodiment::SkillDescriptor;
use fabric::types::skill_proposal::SkillProposal;

#[derive(Debug)]
pub struct ValidationError {
    pub field: String,
    pub message: String,
}

/// Validate a SkillProposal against registered skill descriptors.
pub fn validate_proposal(
    proposal: &SkillProposal,
    allowed_skills: &[SkillDescriptor],
    _now_ms: i64,
    _max_frame_age_ms: i64,
) -> Result<(), Vec<ValidationError>> {
    let mut errors = Vec::new();

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

    // 2. Device must match
    if proposal.device != descriptor.device {
        errors.push(ValidationError {
            field: "device_id".into(),
            message: format!(
                "proposal device '{}' does not match descriptor device '{}'",
                proposal.device.0, descriptor.device.0
            ),
        });
    }

    // 3. Confidence must be in [0,1]
    if proposal.confidence < 0.0 || proposal.confidence > 1.0 {
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

    // 5. Provenance digest required
    if proposal.provenance.digest.is_empty() {
        errors.push(ValidationError {
            field: "provenance.digest".into(),
            message: "policy provenance digest is required".into(),
        });
    }

    // 6. Expected outcome must validate
    if let Err(e) = proposal.expected_outcome.validate() {
        errors.push(ValidationError {
            field: "expected_outcome".into(),
            message: format!("invalid expected outcome: {}", e),
        });
    }

    // 7. Validate parameters against skill schema
    if let Some(schema) = descriptor.input_schema.as_object() {
        if let Some(required_fields) = schema.get("required") {
            if let Some(required_arr) = required_fields.as_array() {
                if let Some(params) = proposal.parameters.as_object() {
                    for req in required_arr {
                        if let Some(req_key) = req.as_str() {
                            if !params.contains_key(req_key) {
                                errors.push(ValidationError {
                                    field: format!("parameters.{}", req_key),
                                    message: format!("missing required parameter '{}'", req_key),
                                });
                            }
                        }
                    }
                }
            }
        }
    }

    // 8. Reject proposals that look like raw actuation (joint/torque in params)
    if proposal
        .parameters
        .as_object()
        .map_or(false, |p| {
            p.contains_key("joint")
                || p.contains_key("torque")
                || p.contains_key("topic")
                || p.contains_key("actuator")
        })
    {
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

#[cfg(test)]
mod tests {
    use super::*;
    use fabric::types::embodiment::{DeviceId, RiskClass, SkillId};
    use fabric::types::expected_outcome::{ExpectedOutcome, OutcomePredicate};
    use fabric::types::skill_proposal::{PolicyProvenance, SkillProposal};

    fn allowed_skills() -> Vec<SkillDescriptor> {
        vec![SkillDescriptor {
            skill: SkillId("kuavo.stance".into()),
            device: DeviceId("bot".into()),
            summary: "stable stance".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {},
                "required": []
            }),
            risk: RiskClass::Low,
            timeout_ms: 10000,
            cancellable: false,
            preconditions: vec![],
            success_criteria: vec![],
        }]
    }

    fn valid_proposal() -> SkillProposal {
        SkillProposal {
            skill: SkillId("kuavo.stance".into()),
            device: DeviceId("bot".into()),
            parameters: serde_json::json!({}),
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
                digest: "sha256:abc123".into(),
            },
        }
    }

    #[test]
    fn valid_proposal_passes() {
        assert!(validate_proposal(&valid_proposal(), &allowed_skills(), 1000, 5000).is_ok());
    }

    #[test]
    fn unknown_skill_rejected() {
        let mut p = valid_proposal();
        p.skill = SkillId("nonexistent.skill".into());
        assert!(validate_proposal(&p, &allowed_skills(), 1000, 5000).is_err());
    }

    #[test]
    fn confidence_out_of_range() {
        let mut p = valid_proposal();
        p.confidence = 1.5;
        assert!(validate_proposal(&p, &allowed_skills(), 1000, 5000).is_err());
        p.confidence = -0.1;
        assert!(validate_proposal(&p, &allowed_skills(), 1000, 5000).is_err());
    }

    #[test]
    fn raw_joint_params_rejected() {
        let mut p = valid_proposal();
        p.parameters = serde_json::json!({"joint": [0.1, 0.2]});
        assert!(validate_proposal(&p, &allowed_skills(), 1000, 5000).is_err());
    }

    #[test]
    fn missing_provenance_digest_rejected() {
        let mut p = valid_proposal();
        p.provenance.digest = "".into();
        assert!(validate_proposal(&p, &allowed_skills(), 1000, 5000).is_err());
    }

    #[test]
    fn too_many_frame_refs_rejected() {
        let mut p = valid_proposal();
        p.frame_refs = vec!["a".into(), "b".into(), "c".into(), "d".into(), "e".into()];
        assert!(validate_proposal(&p, &allowed_skills(), 1000, 5000).is_err());
    }
}
