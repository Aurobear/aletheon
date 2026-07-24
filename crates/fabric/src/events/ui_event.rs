//! UI event types for TUI display.
//!
//! These events flow from Runtime -> TUI via the IPC channel.
//! They extend the existing `Event` enum in `event_sink.rs` with
//! TUI-specific display events.

use crate::{Critique, Plan};
use serde::{Deserialize, Serialize};

/// Collaboration mode (user-facing).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum CollaborationMode {
    /// Normal operation: SelfField reviews, approval for destructive tools.
    #[default]
    Default,
    /// Read-only explore + plan generation. User approves before execution.
    Plan,
    /// Full autonomy: no approval prompts.
    Auto,
    /// All side-effect tools run in sandbox first.
    Sandbox,
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

/// Client-facing event produced by the daemon and consumed by the TUI/CLI.
///
/// This is the canonical wire-protocol type shared between daemon and client.
/// Every variant maps to an event notification sent over the Unix socket.
///
/// daemon:  executive::Event -> ClientEvent -> serde_json -> socket
/// client:  socket -> serde_json -> ClientEvent -> display handler
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientEvent {
    // ── Turn lifecycle ──
    TurnStarted {
        iteration: usize,
    },
    TurnDone,
    Error {
        message: String,
    },

    // ── Streaming text ──
    TextDelta {
        text: String,
    },
    ThinkingDelta {
        text: String,
    },

    // ── Tool calls ──
    ToolCallStart {
        call_id: String,
        tool: String,
        args: serde_json::Value,
    },
    /// Emitted when streaming tool args are complete — carries the real args.
    ToolCallComplete {
        call_id: String,
        tool: String,
        args: serde_json::Value,
    },
    ToolCallResult {
        call_id: String,
        tool: String,
        output: String,
        is_error: bool,
        elapsed_ms: u64,
    },
    ToolProgress {
        call_id: String,
        tool: String,
        kind: String,
        payload: serde_json::Value,
    },
    PatchProgress {
        status: String,
        path: Option<String>,
        operation: Option<String>,
        error: Option<String>,
        applied_count: Option<usize>,
        failed_count: Option<usize>,
    },

    // ── Bookkeeping ──
    Usage {
        tokens_in: u64,
        tokens_out: u64,
    },
    ContextUpdate {
        max_tokens: u64,
        used_tokens: u64,
    },
    GoalSet {
        goal: String,
        sub_goals: Vec<String>,
    },
    ModelSwitch {
        model: String,
    },

    // ── Awareness / collaboration ──
    AwarenessChanged {
        level: String,
        context: String,
    },
    PlanUpdate {
        version: u32,
        plan: String,
        critique: Option<String>,
        ready_for_approval: bool,
    },
    SubAgentStatus {
        agent_id: String,
        task: String,
        status: String,
    },
    ModeChanged {
        new: String,
    },

    // ── Limits / interruptions ──
    Interrupted,
    BudgetExceeded {
        limit: u64,
    },
    CircuitBreakerTripped {
        reason: String,
    },
    CompactionTriggered,
    CompactionCompleted {
        strategy: String,
        tokens_before: u64,
        tokens_after: u64,
        evicted_messages: u64,
    },
    Reflection {
        summary: String,
    },
}

impl ClientEvent {
    /// Forward-compatible client decoding: unknown additive event variants are
    /// ignored rather than terminating the socket/TUI event loop.
    pub fn decode_if_known(value: serde_json::Value) -> Option<Self> {
        serde_json::from_value(value).ok()
    }
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
            assert!(
                s.can_transition_to(&Destroyed),
                "{s:?} -> Destroyed must be legal"
            );
        }
    }

    #[test]
    fn illegal_transitions_are_rejected() {
        use SubAgentState::*;
        assert!(
            !Created.can_transition_to(&Completed),
            "must run before completing"
        );
        assert!(
            !Completed.can_transition_to(&Running),
            "terminal-forward: no resurrection"
        );
        assert!(
            !Destroyed.can_transition_to(&Running),
            "Destroyed is terminal"
        );
        assert!(
            !Destroyed.can_transition_to(&Destroyed),
            "no self-loop on Destroyed"
        );
    }
}

#[cfg(test)]
mod client_event_compatibility_tests {
    use super::ClientEvent;

    #[test]
    fn unknown_additive_event_is_ignored_without_panic() {
        assert!(ClientEvent::decode_if_known(serde_json::json!({
            "type": "future_progress_shape",
            "payload": {"pct": 50}
        }))
        .is_none());
    }
}
