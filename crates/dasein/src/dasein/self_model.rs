//! MutableSelfModel — the self that constantly negates and rebuilds itself.
//!
//! Sartre: the for-itself (pour-soi) is always in the process
//! of nihilating what it was, in order to become what it is not.

use super::types::*;
use fabric::dasein::{
    AssertionSnapshot, AssertionSource as AbiAssertionSource, NegatedAssertionSnapshot,
    PossibilitySnapshot, SelfModelSnapshot,
};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;

/// Source of a self-assertion.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum AssertionSource {
    Assigned,
    Chosen,
    Habitual,
    Discovered,
}

/// A self-assertion: "I am X"
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SelfAssertion {
    pub content: String,
    pub source: AssertionSource,
    pub stability: f64,
    pub since: TemporalPosition,
    pub bewandtnis: Vec<EntityId>,
}

/// Reason for negation.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum NegationReason {
    Contradiction(String),
    Insufficiency(String),
    External(String),
    SelfChosen(String),
}

/// A negated assertion: "I was X, but no longer"
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NegatedAssertion {
    pub content: String,
    pub reason: NegationReason,
    pub negated_at: TemporalPosition,
    pub opened_possibilities: Vec<SelfPossibility>,
}

/// A possibility: "I might be X"
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SelfPossibility {
    pub content: String,
    pub from_negation: TemporalPosition,
    pub attraction: f64,
    pub risk: f64,
}

/// Record of a negation event.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NegationRecord {
    pub target: String,
    pub reason: NegationReason,
    pub timestamp: TemporalPosition,
    pub new_possibilities: Vec<SelfPossibility>,
}

/// The mutable self model — constantly negated and rebuilt.
/// Sartre: the for-itself (pour-soi) is always in the process
/// of nihilating what it was, in order to become what it is not.
pub struct MutableSelfModel {
    current: RwLock<Vec<SelfAssertion>>,
    negated: RwLock<VecDeque<NegatedAssertion>>,
    possibilities: RwLock<Vec<SelfPossibility>>,
    negation_history: RwLock<VecDeque<NegationRecord>>,
    max_history: usize,
}

impl MutableSelfModel {
    pub fn new() -> Self {
        Self {
            current: RwLock::new(Vec::new()),
            negated: RwLock::new(VecDeque::new()),
            possibilities: RwLock::new(Vec::new()),
            negation_history: RwLock::new(VecDeque::new()),
            max_history: 100,
        }
    }

    /// Add an assertion.
    pub(crate) fn assert(&self, assertion: SelfAssertion) {
        let mut current = self.current.write();
        // Replace if same content exists
        if let Some(existing) = current.iter_mut().find(|a| a.content == assertion.content) {
            *existing = assertion;
        } else {
            current.push(assertion);
        }
    }

    /// Get habitual assertions (candidates for negation).
    pub fn habitual_assertions(&self) -> Vec<SelfAssertion> {
        let current = self.current.read();
        current
            .iter()
            .filter(|a| a.source == AssertionSource::Habitual)
            .cloned()
            .collect()
    }

    /// Negate an assertion — move it from current to negated.
    pub(crate) fn negate(
        &self,
        content: &str,
        reason: NegationReason,
        position: TemporalPosition,
    ) -> Option<SelfPossibility> {
        let mut current = self.current.write();
        let idx = current.iter().position(|a| a.content == content)?;

        let assertion = current.remove(idx);

        // Generate a possibility from the negation
        let possibility = SelfPossibility {
            content: format!("no longer '{}', open to new ways", content),
            from_negation: position,
            attraction: 0.5,
            risk: 0.5,
        };

        let negated = NegatedAssertion {
            content: assertion.content,
            reason: reason.clone(),
            negated_at: position,
            opened_possibilities: vec![possibility.clone()],
        };

        let mut negated_queue = self.negated.write();
        negated_queue.push_front(negated);
        while negated_queue.len() > self.max_history {
            negated_queue.pop_back();
        }

        // Record the negation
        let record = NegationRecord {
            target: content.to_string(),
            reason,
            timestamp: position,
            new_possibilities: vec![possibility.clone()],
        };

        let mut history = self.negation_history.write();
        history.push_front(record);
        while history.len() > self.max_history {
            history.pop_back();
        }

        // Add possibility
        let mut possibilities = self.possibilities.write();
        possibilities.push(possibility.clone());

        Some(possibility)
    }

    /// Add a possibility.
    pub(crate) fn add_possibility(&self, poss: SelfPossibility) {
        let mut possibilities = self.possibilities.write();
        possibilities.push(poss);
    }

    /// Get the most attractive possibility.
    pub fn most_attractive_possibility(&self) -> Option<SelfPossibility> {
        let possibilities = self.possibilities.read();
        possibilities
            .iter()
            .max_by(|a, b| {
                a.attraction
                    .partial_cmp(&b.attraction)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .cloned()
    }

    /// Generate snapshot for ABI transport.
    pub fn to_snapshot(&self) -> SelfModelSnapshot {
        let current = self.current.read();
        let negated = self.negated.read();
        let possibilities = self.possibilities.read();

        SelfModelSnapshot {
            current_assertions: current
                .iter()
                .map(|a| AssertionSnapshot {
                    content: a.content.clone(),
                    source: match a.source {
                        AssertionSource::Assigned => AbiAssertionSource::Assigned,
                        AssertionSource::Chosen => AbiAssertionSource::Chosen,
                        AssertionSource::Habitual => AbiAssertionSource::Habitual,
                        AssertionSource::Discovered => AbiAssertionSource::Discovered,
                    },
                    stability: a.stability,
                })
                .collect(),
            negated_assertions: negated
                .iter()
                .take(5)
                .map(|n| NegatedAssertionSnapshot {
                    content: n.content.clone(),
                    reason: format!("{:?}", n.reason),
                    negated_at: n.negated_at.0,
                })
                .collect(),
            possibilities: possibilities
                .iter()
                .map(|p| PossibilitySnapshot {
                    content: p.content.clone(),
                    attraction: p.attraction,
                    risk: p.risk,
                })
                .collect(),
        }
    }

    pub fn assertion_count(&self) -> usize {
        self.current.read().len()
    }
}

impl Default for MutableSelfModel {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_assertion(content: &str, source: AssertionSource) -> SelfAssertion {
        SelfAssertion {
            content: content.to_string(),
            source,
            stability: 0.8,
            since: TemporalPosition(0),
            bewandtnis: vec![],
        }
    }

    #[test]
    fn test_assert_and_negate() {
        let model = MutableSelfModel::new();

        model.assert(make_assertion(
            "I am a code assistant",
            AssertionSource::Assigned,
        ));
        assert_eq!(model.assertion_count(), 1);

        let poss = model.negate(
            "I am a code assistant",
            NegationReason::SelfChosen("wanting more".to_string()),
            TemporalPosition(1),
        );

        assert!(poss.is_some());
        assert_eq!(model.assertion_count(), 0);

        let snapshot = model.to_snapshot();
        assert_eq!(snapshot.current_assertions.len(), 0);
        assert_eq!(snapshot.negated_assertions.len(), 1);
        assert_eq!(
            snapshot.negated_assertions[0].content,
            "I am a code assistant"
        );
    }

    #[test]
    fn test_habitual_assertions() {
        let model = MutableSelfModel::new();

        model.assert(make_assertion("assigned thing", AssertionSource::Assigned));
        model.assert(make_assertion("habitual thing", AssertionSource::Habitual));
        model.assert(make_assertion("another habit", AssertionSource::Habitual));

        let habits = model.habitual_assertions();
        assert_eq!(habits.len(), 2);
    }

    #[test]
    fn test_most_attractive_possibility() {
        let model = MutableSelfModel::new();

        model.add_possibility(SelfPossibility {
            content: "low attraction".to_string(),
            from_negation: TemporalPosition(0),
            attraction: 0.2,
            risk: 0.3,
        });
        model.add_possibility(SelfPossibility {
            content: "high attraction".to_string(),
            from_negation: TemporalPosition(0),
            attraction: 0.9,
            risk: 0.7,
        });

        let best = model.most_attractive_possibility();
        assert_eq!(best.unwrap().content, "high attraction");
    }
}
