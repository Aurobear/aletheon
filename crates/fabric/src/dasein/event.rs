use serde::{Deserialize, Serialize};

use crate::dasein::types::{ReadinessState, Stimmung};

// ═══ Dasein Events ═══

/// Events flowing into and out of the DaseinModule.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum DaseinEvent {
    // External events
    UserInput {
        content: String,
    },
    SystemEvent {
        source: String,
        content: String,
    },
    TimerTick,

    // Thinking/observation events
    ThinkingObserved {
        text: String,
        turn: usize,
    },
    ReasoningObserved {
        text: String,
        turn: usize,
        has_tool_calls: bool,
    },
    KnowledgeAsserted {
        assertions: Vec<String>,
        confidence: f64,
    },

    // Internal events
    NegationCompleted {
        target: String,
        new_possibilities: Vec<String>,
    },
    MoodShift {
        from: Stimmung,
        to: Stimmung,
        reason: String,
    },
    BewandtnisChange {
        entity_id: String,
        old_state: ReadinessState,
        new_state: ReadinessState,
    },
    TemporalEvent {
        kind: TemporalEventKind,
        content: String,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum TemporalEventKind {
    RetentionFaded,
    ProtentionRealized,
    ProtentionSurprised,
    PatternDetected,
}
