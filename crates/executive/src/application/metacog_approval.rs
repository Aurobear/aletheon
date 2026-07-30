//! Human-approved Metacog apply boundary.

use crate::application::approval_service::{ApprovalServiceError, DaseinMutationCoordinator};
use crate::application::governed_capability::GovernedPermitIssuer;
use async_trait::async_trait;
use fabric::{ApprovalSnapshot, CapabilityId};
use std::sync::Arc;

pub struct GovernedMetacogApplyCoordinator {
    permits: Arc<dyn GovernedPermitIssuer>,
    metacog: Arc<dyn metacog::MetacogService>,
}

impl GovernedMetacogApplyCoordinator {
    pub fn new(
        permits: Arc<dyn GovernedPermitIssuer>,
        metacog: Arc<dyn metacog::MetacogService>,
    ) -> Self {
        Self { permits, metacog }
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
            .permits
            .admit_system_modify(
                approval.owner_id.clone(),
                CapabilityId("metacog.apply".into()),
                "apply verified genome mutation".into(),
                mutation_id.to_string(),
            )
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
        self.permits
            .settle(&permit, result.is_ok())
            .await
            .map_err(|e| ApprovalServiceError::Store(e.to_string()))?;
        result
            .map(|_| ())
            .map_err(|e| ApprovalServiceError::Store(e.to_string()))
    }
}
