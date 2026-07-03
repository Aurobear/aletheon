//! NegativityEngine — enables self-questioning.
//!
//! Sartre: the for-itself negates. It is what it is not,
//! and is not what it is.

use super::self_model::*;
use super::types::*;
use base::dasein::Stimmung;

/// The source of a negation.
#[derive(Clone, Debug)]
pub enum NegationSource {
    /// From the care structure
    CareStructure,
    /// From a world contradiction
    WorldContradiction,
    /// From a temporal surprise
    TemporalSurprise,
    /// From Angst
    AngstSignal,
}

/// A pending negation — something that needs to be questioned.
#[derive(Clone, Debug)]
pub enum PendingNegation {
    /// A habitual assertion that should be questioned
    HabitualAssertion(SelfAssertion),
    /// A contradiction in the world
    WorldContradiction(String),
    /// An expected pattern that didn't materialize
    TemporalSurprise(String),
    /// Angst signal
    AngstSignal(String),
}

/// The negativity engine — enables self-questioning.
/// Sartre: the for-itself negates. It is what it is not,
/// and is not what it is.
pub struct NegativityEngine {
    /// How often to question habits (in ticks)
    habit_question_interval: u64,
    /// Last tick at which habits were questioned
    last_habit_question: std::sync::atomic::AtomicU64,
}

impl NegativityEngine {
    pub fn new(habit_question_interval: u64) -> Self {
        Self {
            habit_question_interval,
            last_habit_question: std::sync::atomic::AtomicU64::new(0),
        }
    }

    /// Check if habits should be questioned this tick.
    pub fn should_question_habits(&self, current_tick: u64) -> bool {
        let last = self
            .last_habit_question
            .load(std::sync::atomic::Ordering::Relaxed);
        current_tick - last >= self.habit_question_interval
    }

    /// Mark that habits were questioned this tick.
    pub fn mark_habits_questioned(&self, tick: u64) {
        self.last_habit_question
            .store(tick, std::sync::atomic::Ordering::Relaxed);
    }

    /// Check for negation triggers from mood.
    pub fn check_mood_negation(mood: &Stimmung) -> Option<PendingNegation> {
        match mood {
            Stimmung::Angst { facing } => {
                Some(PendingNegation::AngstSignal(format!("{:?}", facing)))
            }
            Stimmung::Langeweile {
                depth: base::dasein::BoredomDepth::Deep,
            } => Some(PendingNegation::AngstSignal(
                "deep boredom — confronting meaninglessness".to_string(),
            )),
            _ => None,
        }
    }

    /// Generate possibilities from a negation.
    pub fn generate_possibilities(
        negation: &PendingNegation,
        position: TemporalPosition,
    ) -> Vec<SelfPossibility> {
        match negation {
            PendingNegation::HabitualAssertion(assertion) => {
                vec![
                    SelfPossibility {
                        content: format!("beyond '{}'", assertion.content),
                        from_negation: position,
                        attraction: 0.5,
                        risk: 0.5,
                    },
                    SelfPossibility {
                        content: format!("rechoosing '{}' consciously", assertion.content),
                        from_negation: position,
                        attraction: 0.6,
                        risk: 0.2,
                    },
                ]
            }
            PendingNegation::WorldContradiction(desc) => {
                vec![SelfPossibility {
                    content: format!("resolving: {}", desc),
                    from_negation: position,
                    attraction: 0.7,
                    risk: 0.4,
                }]
            }
            PendingNegation::TemporalSurprise(desc) => {
                vec![SelfPossibility {
                    content: format!("adapting to: {}", desc),
                    from_negation: position,
                    attraction: 0.6,
                    risk: 0.3,
                }]
            }
            PendingNegation::AngstSignal(desc) => {
                vec![
                    SelfPossibility {
                        content: format!("facing {}", desc),
                        from_negation: position,
                        attraction: 0.4,
                        risk: 0.8,
                    },
                    SelfPossibility {
                        content: "choosing freely despite uncertainty".to_string(),
                        from_negation: position,
                        attraction: 0.7,
                        risk: 0.6,
                    },
                ]
            }
        }
    }
}

impl Default for NegativityEngine {
    fn default() -> Self {
        Self::new(100) // question habits every 100 ticks
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_should_question_habits() {
        let engine = NegativityEngine::new(10);

        assert!(!engine.should_question_habits(5)); // only 5 ticks since 0
        assert!(engine.should_question_habits(10)); // 10 ticks since 0

        engine.mark_habits_questioned(10);
        assert!(!engine.should_question_habits(15)); // only 5 since 10
        assert!(engine.should_question_habits(20)); // 10 since 10
    }

    #[test]
    fn test_mood_negation_angst() {
        let mood = Stimmung::Angst {
            facing: base::dasein::AngstSource::Freedom,
        };
        let negation = NegativityEngine::check_mood_negation(&mood);
        assert!(matches!(negation, Some(PendingNegation::AngstSignal(_))));
    }

    #[test]
    fn test_mood_negation_deep_boredom() {
        let mood = Stimmung::Langeweile {
            depth: base::dasein::BoredomDepth::Deep,
        };
        let negation = NegativityEngine::check_mood_negation(&mood);
        assert!(matches!(negation, Some(PendingNegation::AngstSignal(_))));
    }

    #[test]
    fn test_mood_negation_calm() {
        let mood = Stimmung::Gelassenheit;
        let negation = NegativityEngine::check_mood_negation(&mood);
        assert!(negation.is_none());
    }

    #[test]
    fn test_generate_possibilities() {
        let negation = PendingNegation::HabitualAssertion(SelfAssertion {
            content: "always being safe".to_string(),
            source: AssertionSource::Habitual,
            stability: 0.9,
            since: TemporalPosition(0),
            bewandtnis: vec![],
        });

        let possibilities =
            NegativityEngine::generate_possibilities(&negation, TemporalPosition(5));
        assert_eq!(possibilities.len(), 2);
        assert!(possibilities[0].content.contains("beyond"));
        assert!(possibilities[1].content.contains("rechoosing"));
    }
}
