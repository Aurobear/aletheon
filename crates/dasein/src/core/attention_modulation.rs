//! SelfField ↔ Dasein attention modulation — R2a (ObserveOnly).
//!
//! Conscious-core plan R2, phase R2a. SelfField's keyword `CareLayer` and the
//! DaseinModule's phenomenological `CareStructure` are two independent "selves"
//! that never inform each other. R2a closes the **module→layer read** direction
//! in observe-only mode: during `review`, SelfField reads the DaseinModule care
//! decision and urgent concerns, computes what it *would* set the attention
//! care-score to, and records a receipt — but does **not** change the verdict or
//! the actual attention. Enforcement (using the modulated score) is a later
//! step, gated on a validated distribution.
//!
//! Invariant: modulation only ever *raises* attention (more caution); it never
//! drops below the keyword baseline, so it can never relax a review decision.

use crate::dasein::care_structure::CareAction;
use serde::{Deserialize, Serialize};

/// The mode the modulation ran in. R2a is always [`ModulationMode::ObserveOnly`];
/// [`ModulationMode::Enforce`] is reserved for the later enforcing step.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModulationMode {
    ObserveOnly,
    Enforce,
}

/// Structured, auditable record of one SelfField modulation evaluation.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SelfFieldModulationReceipt {
    pub mode: ModulationMode,
    pub action: String,
    /// The keyword CareLayer score actually used for the verdict.
    pub keyword_care_score: f64,
    /// What the score would become if SelfField honored the DaseinModule state.
    pub would_be_care_score: f64,
    pub care_action: &'static str,
    pub urgent_concerns: usize,
}

/// Compute what SelfField's attention care-score would become if it honored the
/// DaseinModule's current care state. Monotone toward caution: the result is
/// never below the keyword baseline (modulation only adds attention).
pub fn would_be_care_score(keyword: f64, care_action: &CareAction, urgent_concerns: usize) -> f64 {
    let mut score = keyword;
    // Care wanting deliberation or self-questioning raises attention.
    match care_action {
        CareAction::Deliberate(_) | CareAction::Negate(_) => score += 0.2,
        CareAction::Wait(_) | CareAction::Direct(_) => {}
    }
    // Each urgent concern (capped) adds attention.
    score += 0.1 * (urgent_concerns.min(3) as f64);
    score.clamp(keyword, 1.0)
}

/// A short stable label for a care action, for the receipt and metrics.
pub fn care_action_label(action: &CareAction) -> &'static str {
    match action {
        CareAction::Deliberate(_) => "deliberate",
        CareAction::Direct(_) => "direct",
        CareAction::Wait(_) => "wait",
        CareAction::Negate(_) => "negate",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deliberate_raises_score() {
        let s = would_be_care_score(0.5, &CareAction::Deliberate("x".into()), 0);
        assert!(s > 0.5);
    }

    #[test]
    fn negate_raises_score() {
        let s = would_be_care_score(0.5, &CareAction::Negate("x".into()), 0);
        assert!(s > 0.5);
    }

    #[test]
    fn direct_does_not_lower_baseline() {
        let s = would_be_care_score(0.5, &CareAction::Direct("x".into()), 0);
        assert_eq!(s, 0.5);
    }

    #[test]
    fn urgent_concerns_raise_score() {
        let s = would_be_care_score(0.3, &CareAction::Wait("x".into()), 2);
        assert!((s - 0.5).abs() < 1e-9);
    }

    #[test]
    fn never_below_baseline_and_clamped() {
        assert!(would_be_care_score(0.9, &CareAction::Wait("x".into()), 0) >= 0.9);
        assert!(would_be_care_score(0.95, &CareAction::Negate("x".into()), 3) <= 1.0);
    }
}
