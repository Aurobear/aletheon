//! Approval-callback capability.
//!
//! This is not a [`CapabilityHandler`](crate::registry::CapabilityHandler) —
//! approval callbacks are recognized by `reply_to_action.is_some()` on the
//! inbound message, *before* [`classify_intent`](crate::intent::classify_intent)
//! runs, so it sits outside the intent-keyed registry. It is invoked
//! directly by the dispatcher (step 4 of `process()`), moved here verbatim
//! from the god-object router's `execute_approval_action`.

use std::sync::Arc;

use fabric::{ApprovalCategory, ApprovalId, PrincipalId};

use crate::ports::{ChannelApprovalDecision, ChannelApprovalPort};
use crate::registry::ApprovalResolverRegistry;

pub struct ApprovalExecutor {
    approval_port: Arc<dyn ChannelApprovalPort>,
    resolvers: Arc<ApprovalResolverRegistry>,
}

impl ApprovalExecutor {
    pub fn new(
        approval_port: Arc<dyn ChannelApprovalPort>,
        resolvers: Arc<ApprovalResolverRegistry>,
    ) -> Self {
        Self {
            approval_port,
            resolvers,
        }
    }

    pub async fn execute_approval_action(
        &self,
        principal: &str,
        action_data: &str,
        now_ms: i64,
    ) -> anyhow::Result<String> {
        let port = &self.approval_port;
        let (id, action) = action_data
            .split_once(':')
            .ok_or_else(|| anyhow::anyhow!("invalid approval action"))?;
        let id = ApprovalId(uuid::Uuid::parse_str(id)?);
        let principal_id = PrincipalId(principal.to_owned());
        let channel = "telegram".to_string();
        let result = match action {
            "view_diff" => {
                let approval = port
                    .get(id)?
                    .ok_or_else(|| anyhow::anyhow!("approval not found"))?;
                let artifact = approval
                    .artifacts
                    .iter()
                    .find(|artifact| artifact.kind == "diff")
                    .ok_or_else(|| anyhow::anyhow!("approval diff artifact is unavailable"))?;
                return Ok(format!(
                    "Verified diff reference: {} (sha256 {}).",
                    artifact.relative_path.display(),
                    artifact.sha256
                ));
            }
            "apply" | "confirm" => {
                let current = port
                    .get(id)?
                    .ok_or_else(|| anyhow::anyhow!("approval not found"))?;
                let resolved = port.resolve(
                    id,
                    current.version,
                    principal_id,
                    channel,
                    ChannelApprovalDecision::Approve,
                    now_ms,
                )?;
                (resolved, "approved")
            }
            "revision" | "edit" | "reject" => {
                let current = port
                    .get(id)?
                    .ok_or_else(|| anyhow::anyhow!("approval not found"))?;
                let reason = matches!(action, "revision" | "edit")
                    .then(|| "owner requested revision".to_string());
                let resolved = port.resolve(
                    id,
                    current.version,
                    principal_id,
                    channel,
                    ChannelApprovalDecision::Reject { reason },
                    now_ms,
                )?;
                (resolved, "rejected")
            }
            _ => anyhow::bail!("unknown approval action"),
        };
        if result.0.category == ApprovalCategory::ActivateGoal {
            let resolver = self
                .resolvers
                .resolve_category_only(ApprovalCategory::ActivateGoal)
                .ok_or_else(|| anyhow::anyhow!("Gmail draft executor is not configured"))?;
            resolver.execute_resolved(&result.0, action, now_ms).await?;
        } else if let Some(resolver) = self.resolvers.resolve(result.0.category) {
            resolver.execute_resolved(&result.0, action, now_ms).await?;
        }
        Ok(format!("Approval {}: {}", result.0.id, result.1))
    }
}
