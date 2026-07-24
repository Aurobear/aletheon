//! Pure policy helpers for conscious field action arbitration.

use fabric::dasein::CareActionKind;
use fabric::{ConsciousFieldReadout, FieldDecisionReason, SalienceVector};

/// Derive bounded action-proposal confidence and salience from a validated
/// conscious field readout.
pub fn proposal_salience(readout: &ConsciousFieldReadout) -> (f32, SalienceVector) {
    let confidence = (0.5 + 0.5 * readout.salience.confidence).clamp(0.0, 1.0);
    (
        confidence,
        SalienceVector {
            urgency: readout.concern_urgency.max(readout.salience.urgency),
            ..readout.salience
        },
    )
}

/// Return the conservative field reason that should defer an action.
///
/// A Dasein Negate decision takes precedence over competition outcome.
pub fn should_defer(
    readout: &ConsciousFieldReadout,
    selected: bool,
) -> Option<FieldDecisionReason> {
    if matches!(readout.care_action, Some(CareActionKind::Negate)) {
        Some(FieldDecisionReason::Negated)
    } else if !selected {
        Some(FieldDecisionReason::LostCompetition)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fabric::BroadcastEpoch;

    fn readout(action: Option<CareActionKind>) -> ConsciousFieldReadout {
        ConsciousFieldReadout {
            epoch: BroadcastEpoch(7),
            care_action: action,
            concern_urgency: 0.9,
            salience: SalienceVector {
                urgency: 0.4,
                goal_relevance: 0.3,
                self_relevance: 0.8,
                novelty: 0.2,
                confidence: 0.6,
                prediction_error: 0.1,
                affect_intensity: 0.5,
                social_relevance: 0.2,
            },
            precision: 0.9,
        }
    }

    #[test]
    fn proposal_uses_real_urgency_and_bounded_confidence() {
        let (confidence, salience) = proposal_salience(&readout(None));
        assert_eq!(salience.urgency, 0.9);
        assert_eq!(confidence, 0.8);
        assert!(confidence < 1.0);
    }

    #[test]
    fn negate_precedes_lost_competition() {
        assert_eq!(
            should_defer(&readout(Some(CareActionKind::Negate)), false),
            Some(FieldDecisionReason::Negated)
        );
        assert_eq!(
            should_defer(&readout(None), false),
            Some(FieldDecisionReason::LostCompetition)
        );
        assert_eq!(should_defer(&readout(None), true), None);
    }
}
