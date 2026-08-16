//! Dasein-owned permission decision port.

use crate::core::contracts::Verdict;
use ::contracts::Context;

/// Optional authorization opinion consumed by SelfField policy review.
pub trait PermissionAuthority: Send + Sync {
    /// Return `None` to defer to SelfField's default policy.
    fn confirmation_verdict(&self, ctx: &Context, care_score: f64, action: &str)
        -> Option<Verdict>;
}

/// Default permission opinion used by the production composition root.
#[derive(Default, Clone)]
pub struct DefaultPermissionAuthority;

impl PermissionAuthority for DefaultPermissionAuthority {
    fn confirmation_verdict(
        &self,
        ctx: &Context,
        care_score: f64,
        action: &str,
    ) -> Option<Verdict> {
        use crate::core::contracts::AwarenessRiskLevel;
        use ::contracts::CapabilityLevel;

        if care_score > 0.8 && ctx.permissions.max_level() < CapabilityLevel::SystemChange {
            Some(Verdict::RequireConfirmation {
                reason: format!(
                    "High care relevance ({care_score:.2}) with insufficient permissions for action '{action}'"
                ),
                risk_level: AwarenessRiskLevel::Medium,
            })
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ::contracts::capability::{Capability, CapabilitySet};
    use ::contracts::CapabilityLevel;

    #[test]
    fn requires_confirmation_only_for_high_care_without_system_permission() {
        let authority = DefaultPermissionAuthority;
        let mut context = Context::new("test", std::path::PathBuf::from("/tmp"));
        assert!(matches!(
            authority.confirmation_verdict(&context, 0.9, "settings.update"),
            Some(Verdict::RequireConfirmation { .. })
        ));
        assert!(authority
            .confirmation_verdict(&context, 0.1, "settings.update")
            .is_none());

        let mut permissions = CapabilitySet::new();
        permissions.add(Capability::new(
            "system.admin",
            CapabilityLevel::SystemChange,
            "admin access",
        ));
        context.permissions = permissions;
        assert!(authority
            .confirmation_verdict(&context, 0.9, "settings.update")
            .is_none());
    }
}
