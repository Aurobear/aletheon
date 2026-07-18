//! Approval-callback capability.
//!
//! This is not a [`CapabilityHandler`](crate::r#impl::channel::registry::CapabilityHandler) —
//! approval callbacks are recognized by `reply_to_action.is_some()` on the
//! inbound message, *before* [`classify_intent`](crate::r#impl::channel::intent::classify_intent)
//! runs, so it sits outside the intent-keyed registry. It is invoked
//! directly by the dispatcher (step 4 of `process()`), moved here verbatim
//! from the god-object router's `execute_approval_action`.

use std::sync::{Arc, Mutex};

use fabric::{ApprovalCategory, ApprovalId, PrincipalId};

use crate::r#impl::approval::{ApprovalDecision, ApprovalRepository, ApprovalResolutionContext};
use crate::r#impl::channel::registry::ApprovalResolverRegistry;

pub struct ApprovalExecutor {
    approval_repository: Arc<Mutex<ApprovalRepository>>,
    resolvers: Arc<ApprovalResolverRegistry>,
}

impl ApprovalExecutor {
    pub fn new(
        approval_repository: Arc<Mutex<ApprovalRepository>>,
        resolvers: Arc<ApprovalResolverRegistry>,
    ) -> Self {
        Self {
            approval_repository,
            resolvers,
        }
    }

    pub async fn execute_approval_action(
        &self,
        principal: &str,
        action_data: &str,
        now_ms: i64,
    ) -> anyhow::Result<String> {
        let repository = &self.approval_repository;
        let (id, action) = action_data
            .split_once(':')
            .ok_or_else(|| anyhow::anyhow!("invalid approval action"))?;
        let id = ApprovalId(uuid::Uuid::parse_str(id)?);
        let context = ApprovalResolutionContext {
            principal_id: PrincipalId(principal.to_owned()),
            channel: "telegram".into(),
        };
        let result = match action {
            "view_diff" => {
                let repository = repository.lock().unwrap();
                let approval = repository
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
                let repository = repository.lock().unwrap();
                let current = repository
                    .get(id)?
                    .ok_or_else(|| anyhow::anyhow!("approval not found"))?;
                let resolved = repository.resolve(
                    id,
                    current.version,
                    &context,
                    ApprovalDecision::Approve,
                    now_ms,
                )?;
                (resolved, "approved")
            }
            "revision" | "edit" | "reject" => {
                let repository = repository.lock().unwrap();
                let current = repository
                    .get(id)?
                    .ok_or_else(|| anyhow::anyhow!("approval not found"))?;
                let reason = matches!(action, "revision" | "edit")
                    .then(|| "owner requested revision".to_string());
                let resolved = repository.resolve(
                    id,
                    current.version,
                    &context,
                    ApprovalDecision::Reject { reason },
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
            resolver
                .execute_resolved(&result.0, action, now_ms)
                .await?;
        } else if let Some(resolver) = self.resolvers.resolve(result.0.category) {
            resolver.execute_resolved(&result.0, action, now_ms).await?;
        }
        Ok(format!("Approval {}: {}", result.0.id, result.1))
    }
}
