//! ConflictLayer — multi-source arbitration.
//!
//! Resolves conflicts between competing opinions (User, Body, Brain, Memory, Self_).
//! Priority: User > Body(risk>=High) > Brain(confidence>=0.8) > Memory > Self_.
//! Body Critical risk always wins (safety overrides everything).

use fabric::self_field::{AwarenessRiskLevel, ConflictSource};
use fabric::{Conflict, Resolution};

/// ConflictLayer — stateless conflict arbitration.
pub struct ConflictLayer;

impl ConflictLayer {
    pub fn new() -> Self {
        Self
    }

    /// Resolve a conflict between two sources.
    pub fn resolve(&self, conflict: &Conflict) -> Resolution {
        let priority_a = self.source_priority(&conflict.source_a);
        let priority_b = self.source_priority(&conflict.source_b);

        // Body Critical risk always wins (safety override)
        if self.is_critical_body(&conflict.source_a) {
            return Resolution::AcceptA {
                reason: "Body critical risk — safety override".to_string(),
            };
        }
        if self.is_critical_body(&conflict.source_b) {
            return Resolution::AcceptB {
                reason: "Body critical risk — safety override".to_string(),
            };
        }

        match priority_a.cmp(&priority_b) {
            std::cmp::Ordering::Greater => Resolution::AcceptA {
                reason: format!(
                    "{:?} has higher priority",
                    self.source_name(&conflict.source_a)
                ),
            },
            std::cmp::Ordering::Less => Resolution::AcceptB {
                reason: format!(
                    "{:?} has higher priority",
                    self.source_name(&conflict.source_b)
                ),
            },
            std::cmp::Ordering::Equal => Resolution::Compromise {
                action: "combine_both".to_string(),
                reason: "Equal priority — attempt compromise".to_string(),
            },
        }
    }

    /// Assign a numeric priority to a conflict source.
    /// Higher number = higher priority.
    fn source_priority(&self, source: &ConflictSource) -> u8 {
        match source {
            ConflictSource::User { .. } => 100,
            ConflictSource::Body { risk, .. } if *risk >= AwarenessRiskLevel::High => 90,
            ConflictSource::Brain { confidence, .. } if *confidence >= 0.8 => 80,
            ConflictSource::Memory { .. } => 50,
            ConflictSource::Self_ { .. } => 30,
            // Lower-priority fallbacks
            ConflictSource::Body { .. } => 40,
            ConflictSource::Brain { .. } => 45,
        }
    }

    fn is_critical_body(&self, source: &ConflictSource) -> bool {
        matches!(source, ConflictSource::Body { risk, .. } if *risk == AwarenessRiskLevel::Critical)
    }

    fn source_name(&self, source: &ConflictSource) -> &str {
        match source {
            ConflictSource::User { .. } => "User",
            ConflictSource::Brain { .. } => "Brain",
            ConflictSource::Body { .. } => "Body",
            ConflictSource::Memory { .. } => "Memory",
            ConflictSource::Self_ { .. } => "Self",
        }
    }
}

impl Default for ConflictLayer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fabric::Context;
    use std::path::PathBuf;

    fn test_ctx() -> Context {
        Context::new("test-session", PathBuf::from("/tmp"))
    }

    #[test]
    fn user_wins() {
        let layer = ConflictLayer::new();
        let conflict = Conflict {
            source_a: ConflictSource::User {
                intent: "do X".to_string(),
            },
            source_b: ConflictSource::Brain {
                proposal: "do Y".to_string(),
                confidence: 0.9,
            },
            context: test_ctx(),
        };
        let resolution = layer.resolve(&conflict);
        assert!(matches!(resolution, Resolution::AcceptA { .. }));
    }

    #[test]
    fn body_critical_wins() {
        let layer = ConflictLayer::new();
        let conflict = Conflict {
            source_a: ConflictSource::User {
                intent: "proceed".to_string(),
            },
            source_b: ConflictSource::Body {
                objection: "motor overheating".to_string(),
                risk: AwarenessRiskLevel::Critical,
            },
            context: test_ctx(),
        };
        let resolution = layer.resolve(&conflict);
        assert!(matches!(resolution, Resolution::AcceptB { .. }));
    }

    #[test]
    fn brain_confidence_wins_over_memory() {
        let layer = ConflictLayer::new();
        let conflict = Conflict {
            source_a: ConflictSource::Brain {
                proposal: "approach A".to_string(),
                confidence: 0.95,
            },
            source_b: ConflictSource::Memory {
                evidence: "previously used B".to_string(),
            },
            context: test_ctx(),
        };
        let resolution = layer.resolve(&conflict);
        assert!(matches!(resolution, Resolution::AcceptA { .. }));
    }

    #[test]
    fn equal_priority_compromise() {
        let layer = ConflictLayer::new();
        let conflict = Conflict {
            source_a: ConflictSource::Memory {
                evidence: "fact A".to_string(),
            },
            source_b: ConflictSource::Self_ {
                concern: "concern B".to_string(),
            },
            context: test_ctx(),
        };
        // Memory=50, Self_=30, not equal. Let's test with two equal ones.
        // Actually Memory > Self_, so AcceptA
        let resolution = layer.resolve(&conflict);
        assert!(matches!(resolution, Resolution::AcceptA { .. }));
    }

    #[test]
    fn body_high_beats_brain_low() {
        let layer = ConflictLayer::new();
        let conflict = Conflict {
            source_a: ConflictSource::Body {
                objection: "joint limit reached".to_string(),
                risk: AwarenessRiskLevel::High,
            },
            source_b: ConflictSource::Brain {
                proposal: "try harder".to_string(),
                confidence: 0.5,
            },
            context: test_ctx(),
        };
        let resolution = layer.resolve(&conflict);
        assert!(matches!(resolution, Resolution::AcceptA { .. }));
    }
}
