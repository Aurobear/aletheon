//! Human-approved Metacog apply boundary.

use crate::application::approval_service::{ApprovalServiceError, DaseinMutationCoordinator};
use async_trait::async_trait;
use fabric::types::admission::RiskLevel;
use fabric::{
    AdmissionController, AdmissionRequest, ApprovalSnapshot, CapabilityId, CapabilityScope,
    OperationId, ProcessId, SandboxRequirement, UsageReport,
};
use std::sync::Arc;

pub struct GovernedMetacogApplyCoordinator {
    admission: Arc<dyn AdmissionController>,
    metacog: Arc<dyn metacog::MetacogService>,
}

impl GovernedMetacogApplyCoordinator {
    pub fn new(
        admission: Arc<dyn AdmissionController>,
        metacog: Arc<dyn metacog::MetacogService>,
    ) -> Self {
        Self { admission, metacog }
    }
}

#[async_trait]
impl DaseinMutationCoordinator for GovernedMetacogApplyCoordinator {
    async fn apply(&self, approval: ApprovalSnapshot) -> Result<(), ApprovalServiceError> {
        let mutation_id = approval
            .subject
            .attributes
            .get("mutation_id")
            .ok_or_else(|| {
                ApprovalServiceError::Store("metacog approval lacks mutation_id".into())
            })?
            .parse::<uuid::Uuid>()
            .map_err(|_| ApprovalServiceError::Store("invalid mutation_id".into()))?;
        if approval
            .subject
            .attributes
            .get("operation")
            .map(String::as_str)
            != Some("apply")
        {
            return Err(ApprovalServiceError::Store(
                "metacog approval operation is not apply".into(),
            ));
        }
        let verification_hash = approval
            .subject
            .attributes
            .get("verification_hash")
            .ok_or_else(|| {
                ApprovalServiceError::Store("metacog approval lacks verification_hash".into())
            })?;
        let status = self
            .metacog
            .status()
            .await
            .map_err(|e| ApprovalServiceError::Store(e.to_string()))?;
        let verification = status
            .lineage
            .into_iter()
            .find(|item| {
                item.mutation_id == mutation_id
                    && item.verification.verification_hash == *verification_hash
            })
            .ok_or_else(|| {
                ApprovalServiceError::Conflict(
                    "approval verification no longer matches durable mutation state".into(),
                )
            })?
            .verification;

        let permit = self
            .admission
            .admit(AdmissionRequest {
                operation_id: OperationId::new(),
                process_id: ProcessId::new(),
                principal: approval.owner_id.clone(),
                capability: CapabilityId("metacog.apply".into()),
                action: "apply verified genome mutation".into(),
                input_summary: mutation_id.to_string(),
                risk: RiskLevel::SystemModify,
                requested_scope: CapabilityScope::default(),
                budget: None,
                lease: None,
                sandbox: SandboxRequirement::NotRequired,
            })
            .await
            .map_err(|e| ApprovalServiceError::RuntimeUnavailable(e.to_string()))?;
        let result = self
            .metacog
            .apply(metacog::ApplyMutation {
                verification,
                evidence: metacog::GovernedMutationEvidence {
                    permit: permit.clone(),
                    approval,
                },
            })
            .await;
        self.admission
            .settle(
                permit.id,
                UsageReport {
                    permit_id: permit.id,
                    exit_code: Some(if result.is_ok() { 0 } else { 1 }),
                    ..Default::default()
                },
            )
            .await
            .map_err(|e| ApprovalServiceError::Store(e.to_string()))?;
        result
            .map(|_| ())
            .map_err(|e| ApprovalServiceError::Store(e.to_string()))
    }
}
