//! DaseinModule ABI types — pure interfaces, zero implementations.
//!
//! Philosophy grounding:
//! - Stimmung: Heidegger's Befindlichkeit (attunement)
//! - TemporalStream: Husserl's inner time consciousness (retention-primal impression-protention)
//! - Bewandtnisganzheit: Heidegger's involvement whole (meaningful relational network)
//! - MutableSelfModel: Sartre's pour-soi (self-negating being-for-itself)
//! - CareStructure: Heidegger's Sorge (care = projection + thrownness + fallenness)

pub mod context;
pub mod event;
pub mod ops;
pub mod types;

pub use context::*;
pub use event::*;
pub use ops::*;
pub use types::*;

#[cfg(test)]
mod tests {
    use super::event::DaseinEvent;

    #[test]
    fn dasein_event_thinking_observed_serde_roundtrip() {
        let event = DaseinEvent::ThinkingObserved {
            text: "considering the implications".to_string(),
            turn: 3,
        };
        let json = serde_json::to_string(&event).unwrap();
        let deserialized: DaseinEvent = serde_json::from_str(&json).unwrap();
        match deserialized {
            DaseinEvent::ThinkingObserved { text, turn } => {
                assert_eq!(text, "considering the implications");
                assert_eq!(turn, 3);
            }
            other => panic!("expected ThinkingObserved, got {:?}", other),
        }
    }

    #[test]
    fn dasein_event_reasoning_observed_serde_roundtrip() {
        let event = DaseinEvent::ReasoningObserved {
            text: "need to check the error".to_string(),
            turn: 5,
            has_tool_calls: true,
        };
        let json = serde_json::to_string(&event).unwrap();
        let deserialized: DaseinEvent = serde_json::from_str(&json).unwrap();
        match deserialized {
            DaseinEvent::ReasoningObserved {
                text,
                turn,
                has_tool_calls,
            } => {
                assert_eq!(text, "need to check the error");
                assert_eq!(turn, 5);
                assert!(has_tool_calls);
            }
            other => panic!("expected ReasoningObserved, got {:?}", other),
        }
    }

    #[test]
    fn dasein_event_knowledge_asserted_serde_roundtrip() {
        let event = DaseinEvent::KnowledgeAsserted {
            assertions: vec!["sky is blue".to_string(), "water is wet".to_string()],
            confidence: 0.95,
        };
        let json = serde_json::to_string(&event).unwrap();
        let deserialized: DaseinEvent = serde_json::from_str(&json).unwrap();
        match deserialized {
            DaseinEvent::KnowledgeAsserted {
                assertions,
                confidence,
            } => {
                assert_eq!(assertions, vec!["sky is blue", "water is wet"]);
                assert!((confidence - 0.95).abs() < f64::EPSILON);
            }
            other => panic!("expected KnowledgeAsserted, got {:?}", other),
        }
    }

    #[test]
    fn dasein_event_thinking_observed_empty_text_roundtrip() {
        let event = DaseinEvent::ThinkingObserved {
            text: String::new(),
            turn: 0,
        };
        let json = serde_json::to_string(&event).unwrap();
        let deserialized: DaseinEvent = serde_json::from_str(&json).unwrap();
        match deserialized {
            DaseinEvent::ThinkingObserved { text, turn } => {
                assert!(text.is_empty());
                assert_eq!(turn, 0);
            }
            other => panic!("expected ThinkingObserved, got {:?}", other),
        }
    }

    #[test]
    fn dasein_event_knowledge_asserted_empty_assertions_roundtrip() {
        let event = DaseinEvent::KnowledgeAsserted {
            assertions: vec![],
            confidence: 0.0,
        };
        let json = serde_json::to_string(&event).unwrap();
        let deserialized: DaseinEvent = serde_json::from_str(&json).unwrap();
        match deserialized {
            DaseinEvent::KnowledgeAsserted {
                assertions,
                confidence,
            } => {
                assert!(assertions.is_empty());
                assert!((confidence).abs() < f64::EPSILON);
            }
            other => panic!("expected KnowledgeAsserted, got {:?}", other),
        }
    }
}
