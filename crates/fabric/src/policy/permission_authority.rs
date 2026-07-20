//! Runtime-owned permission policy contract (Tier 2a).
//!
//! The Self layer (`dasein`) delegates the "does this action need user
//! confirmation / is it permitted" decision to whatever implements this
//! trait, so the *policy* lives in the Runtime while identity/care/boundary
//! judgment stays in Self.
//!
//! Returning `None` means "no opinion" -- the caller falls back to its
//! default behavior. This keeps the trait additive: an un-wired system
//! behaves exactly as before.

use crate::context::Context;
use crate::Verdict;

/// Decides permission verdicts on behalf of the Runtime.
///
/// # Design
///
/// - `None` return = "defer to caller's default rule"
/// - Object-safe (takes `&self`, `Send + Sync`)
/// - Lives in `base` so `dasein` can hold a reference without depending on `runtime`
pub trait PermissionAuthority: Send + Sync {
    /// Given the current context, care relevance of an action, and the action
    /// name, decide whether it should be confirmed or gated.
    ///
    /// Returns `None` to defer to the caller's inline fallback rule.
    fn confirmation_verdict(&self, ctx: &Context, care_score: f64, action: &str)
        -> Option<Verdict>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::AwarenessRiskLevel;
    use std::path::PathBuf;

    struct AlwaysConfirm;
    impl PermissionAuthority for AlwaysConfirm {
        fn confirmation_verdict(
            &self,
            _ctx: &Context,
            _care: f64,
            action: &str,
        ) -> Option<Verdict> {
            Some(Verdict::RequireConfirmation {
                reason: format!("policy requires confirmation for '{action}'"),
                risk_level: AwarenessRiskLevel::Medium,
            })
        }
    }

    struct NeverOpinion;
    impl PermissionAuthority for NeverOpinion {
        fn confirmation_verdict(
            &self,
            _ctx: &Context,
            _care: f64,
            _action: &str,
        ) -> Option<Verdict> {
            None
        }
    }

    #[test]
    fn authority_can_return_a_verdict() {
        let ctx = Context::new("t", PathBuf::from("/tmp"));
        let v = AlwaysConfirm.confirmation_verdict(&ctx, 0.9, "system.reboot");
        assert!(matches!(v, Some(Verdict::RequireConfirmation { .. })));
    }

    #[test]
    fn authority_can_defer_by_returning_none() {
        let ctx = Context::new("t", PathBuf::from("/tmp"));
        let v = NeverOpinion.confirmation_verdict(&ctx, 0.9, "ls");
        assert!(v.is_none());
    }
}
