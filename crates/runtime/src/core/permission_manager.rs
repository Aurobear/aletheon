//! Runtime permission policy (Tier 2a).
//!
//! Owns the confirmation/whitelist/sandbox policy decisions that previously
//! lived inline in `dasein::review()`. Phase 1 reproduces the exact prior
//! rule; whitelist and sandbox-policy selection are future additions.
//!
//! Port of `dasein/src/core/mod.rs:389-398` (behavior-identical).

use base::context::Context;
use base::policy::permission_authority::PermissionAuthority;
use base::{PermissionLevel, RiskLevel, Verdict};

/// The Runtime's permission authority.
///
/// Currently ports the single inline rule from `dasein::review()`:
/// high care + insufficient permissions = RequireConfirmation.
/// Future: whitelist, sandbox-policy selection, per-action rules.
#[derive(Default, Clone)]
pub struct PermissionManager;

impl PermissionManager {
    pub fn new() -> Self {
        Self
    }
}

impl PermissionAuthority for PermissionManager {
    fn confirmation_verdict(
        &self,
        ctx: &Context,
        care_score: f64,
        action: &str,
    ) -> Option<Verdict> {
        // Exact port of crates/dasein/src/core/mod.rs:389-398.
        // Behavior-preserving: same threshold (0.8), same comparison
        // (max_level < SystemChange), same message format.
        if care_score > 0.8 && ctx.permissions.max_level() < PermissionLevel::SystemChange {
            return Some(Verdict::RequireConfirmation {
                reason: format!(
                    "High care relevance ({care_score:.2}) with insufficient permissions for action '{action}'"
                ),
                risk_level: RiskLevel::Medium,
            });
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use base::policy::permission_authority::PermissionAuthority;
    use std::path::PathBuf;

    #[test]
    fn high_care_insufficient_perms_requires_confirmation() {
        let mgr = PermissionManager::new();
        // Default Context has CapabilitySet::new() => max_level = ReadOnly
        let ctx = Context::new("t", PathBuf::from("/tmp"));
        let v = mgr.confirmation_verdict(&ctx, 0.9, "settings.update");
        assert!(matches!(v, Some(Verdict::RequireConfirmation { .. })));
    }

    #[test]
    fn low_care_no_opinion() {
        let mgr = PermissionManager::new();
        let ctx = Context::new("t", PathBuf::from("/tmp"));
        assert!(mgr.confirmation_verdict(&ctx, 0.1, "ls").is_none());
    }

    #[test]
    fn high_care_but_sufficient_perms_no_opinion() {
        let mgr = PermissionManager::new();
        // ctx with SystemChange-level capability
        use base::capability::{Capability, CapabilitySet};
        let mut perms = CapabilitySet::new();
        perms.add(Capability::new(
            "system.admin",
            base::PermissionLevel::SystemChange,
            "admin access",
        ));
        let mut ctx = Context::new("t", PathBuf::from("/tmp"));
        ctx.permissions = perms;
        assert!(mgr
            .confirmation_verdict(&ctx, 0.9, "settings.update")
            .is_none());
    }
}
