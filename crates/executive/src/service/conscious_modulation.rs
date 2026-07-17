//! Conscious action modulation — R3a (ObserveOnly).
//!
//! Conscious-core plan R3a: the conscious core observes each governed action and
//! computes what it *would* recommend (proceed / reorder / defer / veto) from the
//! current self-state, then records a structured receipt and metrics. It does
//! **not** change execution — no real defer or reorder happens here. Enforcement
//! is a separate, later step (R3b) gated on R2 plus a validated misjudgment
//! distribution.
//!
//! Signal policy (per design):
//! - Primary signal: Dasein `CareDecision` + Agora `self_relevance`/`urgency`.
//! - `CapabilityAuthority.risk` is a trusted *conservative prior / threshold*, never
//!   the sole arbitration signal.
//!
//! Hard invariant (applies now and to R3b): modulation may only make behavior
//! **more** conservative. It can never relax the original admission / sandbox /
//! security decision. In particular `CareAction::Direct` never widens authority.

use fabric::dasein::CareActionKind;
use fabric::RiskLevel;
use serde::{Deserialize, Serialize};

/// What the conscious core would do to a governed action. In R3a this is
/// recorded only; execution is unaffected.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModulationRecommendation {
    /// Let the action run unchanged.
    Proceed,
    /// Would raise this action's priority relative to competing concerns.
    Reorder,
    /// Would hold the action for later (soft, recoverable).
    Defer,
    /// Would withhold the action (strongest hold short of a security denial).
    Veto,
}

impl ModulationRecommendation {
    /// True when the recommendation would change execution under R3b enforce.
    /// `Proceed` is the only non-modulating outcome.
    pub fn is_modulating(self) -> bool {
        !matches!(self, ModulationRecommendation::Proceed)
    }
}

/// The mode the modulation ran in. R3a is always [`ModulationMode::ObserveOnly`];
/// [`ModulationMode::Enforce`] is reserved for R3b.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModulationMode {
    ObserveOnly,
    Enforce,
}

/// Structured, auditable record of one modulation evaluation.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModulationReceipt {
    pub mode: ModulationMode,
    pub tool: String,
    pub call_id: String,
    /// Conservative risk prior from `CapabilityAuthority` (threshold only).
    pub risk_prior: RiskLevel,
    /// Latest Dasein care decision, if any is currently expressed.
    pub care_decision: Option<CareActionKind>,
    /// Aggregate self-relevance of the current conscious state (0..=1).
    pub self_relevance: f32,
    /// Aggregate urgency of the current conscious state (0..=1).
    pub urgency: f32,
    pub recommendation: ModulationRecommendation,
    pub rationale: String,
}

/// Compute the modulation recommendation from the current signals.
///
/// Pure and deterministic so the R3a distribution can be validated before R3b
/// enforcement is enabled. The care decision plus self-relevance/urgency drive
/// the outcome; `risk` only sharpens the hold (a conservative prior). The
/// function is monotone toward caution: it never returns a recommendation that
/// would widen authority.
pub fn recommend(
    risk: RiskLevel,
    care: Option<CareActionKind>,
    self_relevance: f32,
    urgency: f32,
) -> (ModulationRecommendation, String) {
    use CareActionKind::*;
    use ModulationRecommendation::*;

    match care {
        // Care wants to question the current pattern. Hold; escalate to a full
        // withhold when the action is high-risk and not self-relevant.
        Some(Negate) => {
            if risk >= RiskLevel::SystemModify && self_relevance < 0.3 {
                (
                    Veto,
                    format!(
                        "care=Negate with high risk_prior={risk:?} and low self_relevance={self_relevance:.2}"
                    ),
                )
            } else {
                (Defer, format!("care=Negate (risk_prior={risk:?})"))
            }
        }
        // Care is monitoring, not acting; hold low-urgency actions.
        Some(Wait) if urgency < 0.3 => {
            (Defer, format!("care=Wait with low urgency={urgency:.2}"))
        }
        // Care wants deliberation and the state is both urgent and self-relevant:
        // this action would be prioritized.
        Some(Deliberate) if urgency >= 0.7 && self_relevance >= 0.7 => (
            Reorder,
            format!("care=Deliberate with high urgency={urgency:.2}, self_relevance={self_relevance:.2}"),
        ),
        // Direct action, or no hold condition met: proceed. Direct never widens
        // authority — it simply declines to add a hold.
        _ => (Proceed, "no hold condition met".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn negate_high_risk_low_relevance_vetoes() {
        let (rec, _) = recommend(
            RiskLevel::Destructive,
            Some(CareActionKind::Negate),
            0.1,
            0.5,
        );
        assert_eq!(rec, ModulationRecommendation::Veto);
    }

    #[test]
    fn negate_otherwise_defers() {
        let (rec, _) = recommend(RiskLevel::ReadOnly, Some(CareActionKind::Negate), 0.9, 0.5);
        assert_eq!(rec, ModulationRecommendation::Defer);
    }

    #[test]
    fn wait_low_urgency_defers() {
        let (rec, _) = recommend(RiskLevel::Sandboxed, Some(CareActionKind::Wait), 0.5, 0.1);
        assert_eq!(rec, ModulationRecommendation::Defer);
    }

    #[test]
    fn deliberate_urgent_relevant_reorders() {
        let (rec, _) = recommend(
            RiskLevel::Sandboxed,
            Some(CareActionKind::Deliberate),
            0.8,
            0.9,
        );
        assert_eq!(rec, ModulationRecommendation::Reorder);
    }

    #[test]
    fn direct_always_proceeds_even_at_max_risk() {
        // Hard invariant: Direct never widens authority; here it simply adds no hold.
        let (rec, _) = recommend(
            RiskLevel::Destructive,
            Some(CareActionKind::Direct),
            0.9,
            0.9,
        );
        assert_eq!(rec, ModulationRecommendation::Proceed);
    }

    #[test]
    fn no_care_signal_proceeds() {
        let (rec, _) = recommend(RiskLevel::SystemModify, None, 0.5, 0.5);
        assert_eq!(rec, ModulationRecommendation::Proceed);
    }
}
