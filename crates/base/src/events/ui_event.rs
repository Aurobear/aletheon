//! UI event types for TUI display.
//!
//! These events flow from Runtime -> TUI via the IPC channel.
//! They extend the existing `Event` enum in `event_sink.rs` with
//! TUI-specific display events.

use serde::{Deserialize, Serialize};
use crate::{Plan, Critique};

/// Collaboration mode (user-facing).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CollaborationMode {
    /// Normal operation: SelfField reviews, approval for destructive tools.
    Default,
    /// Read-only explore + plan generation. User approves before execution.
    Plan,
    /// Full autonomy: no approval prompts.
    Auto,
    /// All side-effect tools run in sandbox first.
    Sandbox,
}

impl Default for CollaborationMode {
    fn default() -> Self {
        Self::Default
    }
}

impl CollaborationMode {
    /// Icon shown in TUI status bar.
    pub fn icon(&self) -> &'static str {
        match self {
            Self::Default => "\u{1f4ac}",
            Self::Plan => "\u{1f4cb}",
            Self::Auto => "\u{26a1}",
            Self::Sandbox => "\u{1f512}",
        }
    }

    /// Human-readable name.
    pub fn display_name(&self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::Plan => "plan",
            Self::Auto => "auto",
            Self::Sandbox => "sandbox",
        }
    }

    /// Mode-specific system prompt suffix injected into the LLM context.
    pub fn system_prompt_suffix(&self) -> &'static str {
        match self {
            Self::Default => "Operate normally. Ask for user approval before destructive operations.",
            Self::Plan => "You are in PLAN MODE. You may only use read-only tools (glob, grep, read, web_fetch). Generate a detailed plan. Do NOT execute any mutations. Wait for user approval before proceeding.",
            Self::Auto => "You are in AUTO MODE. Execute without asking for approval. Be thorough and autonomous. Persist until the task is fully handled.",
            Self::Sandbox => "You are in SANDBOX MODE. All side-effect operations run in a sandbox first. Review sandbox results before applying to the real environment.",
        }
    }
}

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
    /// BrainCore generating plan.
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
        matches!(self, Self::Hesitant | Self::Confused | Self::Curious | Self::Evolving)
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

/// Explicit sub-agent lifecycle state (control-plane; distinct from the
/// UI-facing `SubAgentStatus`). Roadmap M-E: Created -> Running -> Waiting ->
/// Completed -> Destroyed, with Failed as an alternate terminal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SubAgentState {
    Created,
    Running,
    Waiting,
    Completed,
    Failed,
    Destroyed,
}

impl SubAgentState {
    /// Whether a transition from `self` to `next` is legal.
    ///
    /// `Destroyed` is reachable from any non-terminal state (teardown may run at
    /// any time) but is itself terminal. `Completed`/`Failed` only advance to
    /// `Destroyed`.
    pub fn can_transition_to(&self, next: &SubAgentState) -> bool {
        use SubAgentState::*;
        matches!(
            (self, next),
            (Created, Running)
                | (Created, Failed)
                | (Created, Destroyed)
                | (Running, Waiting)
                | (Running, Completed)
                | (Running, Failed)
                | (Running, Destroyed)
                | (Waiting, Running)
                | (Waiting, Completed)
                | (Waiting, Failed)
                | (Waiting, Destroyed)
                | (Completed, Destroyed)
                | (Failed, Destroyed)
        )
    }
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

/// Plan update for plan mode visualization.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanUpdate {
    /// Version number (1-indexed, increments on each critique-revise cycle).
    pub version: usize,
    /// The plan at this version.
    pub plan: Plan,
    /// Critique of the previous version (None for v1).
    pub critique: Option<Vec<Critique>>,
    /// Whether the plan is ready for user approval.
    pub ready_for_approval: bool,
}

/// Evolution progress for TUI display.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum EvolutionStage {
    Reflecting { reflections_accumulated: usize },
    PatternDetected { pattern: String },
    MorphogenesisTriggered { proposal: String },
    LineageRecorded { entries: usize },
}

/// Interrupt reason.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum InterruptReason {
    /// User pressed Ctrl+C during streaming.
    UserCancelled,
    /// Turn exceeded timeout.
    Timeout,
    /// Token/cost budget exceeded.
    BudgetExceeded,
}

/// Events emitted by the Runtime for TUI display.
///
/// These flow over the existing JSON-RPC notification channel
/// with `method: "event"` and a `type` field discriminator.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum UiEvent {
    // === Existing events (kept for compatibility) ===
    /// Streaming text delta.
    TextDelta { text: String },
    /// Thinking/reasoning text delta.
    ThinkingDelta { text: String },
    /// Tool call started.
    ToolCallStart { id: String, name: String, input: serde_json::Value },
    /// Tool call completed.
    ToolCallResult { id: String, output: String, success: bool },
    /// Token/cost usage update.
    Usage { tokens_in: u32, tokens_out: u32, cache_hit_tokens: u32, cache_miss_tokens: u32 },
    /// Turn completed.
    TurnDone { response: String, interrupted: bool },
    /// Error occurred.
    Error { message: String },
    /// Approval requested for a tool.
    ApprovalRequest { id: String, tool: String, input: serde_json::Value, risk: String },

    // === NEW events for the overhaul ===
    /// Brain awareness signal changed.
    AwarenessChanged { level: AwarenessLevel, context: String },
    /// Plan mode update (new version or critique).
    PlanUpdate(PlanUpdate),
    /// Sub-agent status changed.
    SubAgentStatusChanged { agent_id: String, status: SubAgentStatus },
    /// Collaboration mode changed.
    ModeChanged { old: CollaborationMode, new: CollaborationMode },
    /// Evolution progress update.
    EvolutionProgress { stage: EvolutionStage },
    /// Context usage update.
    ContextUpdate { used: usize, max: usize },
    /// Model switched.
    ModelSwitch { from: String, to: String },
    /// Interrupt acknowledged.
    Interrupted { reason: InterruptReason },
    /// Compaction started.
    CompactionStarted,
    /// Compaction completed.
    CompactionDone { summary_chars: usize },
}

#[cfg(test)]
mod subagent_state_tests {
    use super::SubAgentState;

    #[test]
    fn legal_forward_path_is_allowed() {
        use SubAgentState::*;
        assert!(Created.can_transition_to(&Running));
        assert!(Running.can_transition_to(&Waiting));
        assert!(Waiting.can_transition_to(&Running));
        assert!(Running.can_transition_to(&Completed));
        assert!(Completed.can_transition_to(&Destroyed));
    }

    #[test]
    fn destroy_is_reachable_from_every_non_terminal_state() {
        use SubAgentState::*;
        for s in [Created, Running, Waiting, Completed, Failed] {
            assert!(s.can_transition_to(&Destroyed), "{s:?} -> Destroyed must be legal");
        }
    }

    #[test]
    fn illegal_transitions_are_rejected() {
        use SubAgentState::*;
        assert!(!Created.can_transition_to(&Completed), "must run before completing");
        assert!(!Completed.can_transition_to(&Running), "terminal-forward: no resurrection");
        assert!(!Destroyed.can_transition_to(&Running), "Destroyed is terminal");
        assert!(!Destroyed.can_transition_to(&Destroyed), "no self-loop on Destroyed");
    }
}
