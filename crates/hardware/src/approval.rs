//! Typed, request-bound approval boundary for high-risk embodiment skills.

use ::contracts::types::embodiment::{
    skill_request_digest, DeviceId, SkillDescriptor, SkillId, SkillRequest,
};
use ::contracts::PrincipalId;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// Operator approval bound to one exact skill request. Policy/model confidence
/// is deliberately absent; only a trusted approval adapter can produce this.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HighRiskSkillApprovalReceipt {
    pub approval_id: String,
    pub principal: PrincipalId,
    pub device: DeviceId,
    pub skill: SkillId,
    pub request_digest: String,
    pub expires_at_unix_ms: i64,
}

impl HighRiskSkillApprovalReceipt {
    pub fn validate(
        &self,
        principal: &PrincipalId,
        request: &SkillRequest,
        now_unix_ms: i64,
    ) -> Result<(), String> {
        if self.approval_id.trim().is_empty() {
            return Err("high-risk approval id is empty".into());
        }
        if &self.principal != principal {
            return Err("high-risk approval principal does not match requester".into());
        }
        if self.device != request.device || self.skill != request.skill {
            return Err("high-risk approval is bound to a different device or skill".into());
        }
        let expected = skill_request_digest(request)?;
        if self.request_digest != expected {
            return Err("high-risk approval request digest does not match parameters".into());
        }
        if self.expires_at_unix_ms <= now_unix_ms {
            return Err("high-risk approval is expired".into());
        }
        Ok(())
    }
}

#[async_trait]
pub trait HighRiskSkillApprovalPort: Send + Sync {
    async fn authorize(
        &self,
        principal: &PrincipalId,
        request: &SkillRequest,
        descriptor: &SkillDescriptor,
    ) -> Result<HighRiskSkillApprovalReceipt, String>;
}

/// Production-safe default until a trusted interactive/durable approval adapter
/// is explicitly composed. It never derives authority from the Policy/VLA.
#[derive(Default)]
pub struct DenyHighRiskSkillApproval;

#[async_trait]
impl HighRiskSkillApprovalPort for DenyHighRiskSkillApproval {
    async fn authorize(
        &self,
        _principal: &PrincipalId,
        request: &SkillRequest,
        _descriptor: &SkillDescriptor,
    ) -> Result<HighRiskSkillApprovalReceipt, String> {
        Err(format!(
            "explicit operator approval is required for high-risk skill {}",
            request.skill.0
        ))
    }
}

/// Internal capability payload retained through Kernel admission/audit. The
/// provider receives only `request`; the receipt remains trusted host evidence.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EmbodimentCapabilityInput {
    pub request: SkillRequest,
    pub high_risk_approval: Option<HighRiskSkillApprovalReceipt>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn receipt_is_bound_to_exact_parameters_and_expiry() {
        let principal = PrincipalId("operator".into());
        let request = SkillRequest {
            skill: SkillId("robot.high-risk".into()),
            device: DeviceId("robot".into()),
            parameters: serde_json::json!({"force": 1}),
        };
        let mut receipt = HighRiskSkillApprovalReceipt {
            approval_id: "approval-1".into(),
            principal: principal.clone(),
            device: request.device.clone(),
            skill: request.skill.clone(),
            request_digest: skill_request_digest(&request).unwrap(),
            expires_at_unix_ms: 2_000,
        };
        receipt.validate(&principal, &request, 1_000).unwrap();

        let changed = SkillRequest {
            parameters: serde_json::json!({"force": 2}),
            ..request.clone()
        };
        assert!(receipt.validate(&principal, &changed, 1_000).is_err());
        receipt.request_digest = skill_request_digest(&request).unwrap();
        assert!(receipt.validate(&principal, &request, 2_000).is_err());
    }
}
