//! TUI-owned presentation state derived from daemon event strings.

use serde::{Deserialize, Serialize};

/// Awareness level for TUI status bar display.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AwarenessLevel {
    /// No critical issues detected.
    Confident,
    /// Hedging language or uncertainty detected.
    Hesitant,
    /// 3+ consecutive errors or impasse detected.
    Confused,
    /// Domain shift or new direction detected.
    Curious,
    /// CognitCore generating plan.
    Planning,
    /// Post-turn reflection running.
    Reflecting,
    /// Morphogenesis triggered.
    Evolving,
}

impl AwarenessLevel {
    /// Icon shown in TUI status bar.
    pub fn icon(&self) -> &'static str {
        match self {
            Self::Confident => "\u{1f49a}",
            Self::Hesitant => "\u{1f7e1}",
            Self::Confused => "\u{1f534}",
            Self::Curious => "\u{1f535}",
            Self::Planning => "\u{1f4cb}",
            Self::Reflecting => "\u{1f504}",
            Self::Evolving => "\u{26a1}",
        }
    }

    /// Human-readable name.
    pub fn display_name(&self) -> &'static str {
        match self {
            Self::Confident => "confident",
            Self::Hesitant => "hesitant",
            Self::Confused => "confused",
            Self::Curious => "curious",
            Self::Planning => "planning",
            Self::Reflecting => "reflecting",
            Self::Evolving => "evolving",
        }
    }

    /// Whether this level warrants an inline message in the chat.
    pub fn is_notable(&self) -> bool {
        matches!(
            self,
            Self::Hesitant | Self::Confused | Self::Curious | Self::Evolving
        )
    }
}

/// Sub-agent status for inline TUI display.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SubAgentStatus {
    Planning,
    Executing { current_step: String },
    WaitingApproval,
    Completed { summary: String },
    Failed { error: String },
}

/// Sub-agent handle for tracking spawned agents.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubAgentHandle {
    pub id: String,
    pub task: String,
    pub status: SubAgentStatus,
    pub parent_turn_id: String,
    pub spawned_at_ms: u64,
}

pub trait CollaborationModePresentation {
    fn icon(self) -> &'static str;
}

impl CollaborationModePresentation for application::turn_control::CollaborationMode {
    fn icon(self) -> &'static str {
        match self {
            Self::Default => "\u{1f4ac}",
            Self::Plan => "\u{1f4cb}",
            Self::Auto => "\u{26a1}",
            Self::Sandbox => "\u{1f512}",
        }
    }
}
