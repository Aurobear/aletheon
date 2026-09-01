//! Centralized TUI application state.
//!
//! Extracts state fields from the monolithic TuiModel struct in mod.rs
//! into a focused module for clarity and testability.

use super::presentation::AwarenessLevel;
use super::presentation::CollaborationModePresentation;
use ::contracts::protocol::client::EventCursor;
use ::contracts::{
    AgentSnapshot, ApprovalSnapshot, EvaluationReceiptRef, ExecutionTargetSelection, MonoTime,
    TurnTerminalStatus,
};
use application::turn_control::CollaborationMode;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone)]
struct PendingExecutionTargetSelection {
    selection: ExecutionTargetSelection,
    after_sequence: u64,
}

/// Tracks the current awareness level from brain signals.
#[derive(Debug, Clone)]
pub struct AwarenessState {
    pub level: AwarenessLevel,
    pub context: String,
    pub changed_at: MonoTime,
}

impl Default for AwarenessState {
    fn default() -> Self {
        Self {
            level: AwarenessLevel::Confident,
            context: String::new(),
            changed_at: MonoTime(0),
        }
    }
}

impl AwarenessState {
    pub fn update(&mut self, level: AwarenessLevel, context: String, now: MonoTime) {
        self.level = level;
        self.context = context;
        self.changed_at = now;
    }

    /// Whether to show an inline message (transitions to notable states).
    pub fn should_show_inline(&self, now: MonoTime) -> bool {
        let elapsed_ms = now.0.saturating_sub(self.changed_at.0);
        self.level.is_notable() && elapsed_ms < 5000
    }
}

/// Context window usage tracking for the TUI.
#[derive(Debug, Clone, Default)]
pub struct ContextDisplay {
    pub used: Option<usize>,
    pub max: Option<usize>,
}

impl ContextDisplay {
    pub fn usage_percent(&self) -> Option<f64> {
        let max = self.max.filter(|max| *max > 0)?;
        self.used.map(|used| (used as f64 / max as f64) * 100.0)
    }

    pub fn display(&self) -> String {
        match (self.used, self.max) {
            (Some(used), Some(max)) if max > 0 => {
                let pct = self.usage_percent().unwrap_or_default();
                format!(
                    "ctx: {} / {} ({pct:.0}%)",
                    compact_tokens(used as u64),
                    compact_tokens(max as u64)
                )
            }
            (None, Some(max)) => format!(
                "ctx: unknown / {} (missing active occupancy)",
                compact_tokens(max as u64)
            ),
            _ => "ctx: unknown (missing context capacity projection)".into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UiItemStatus {
    Streaming,
    Completed,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UiItem {
    pub id: String,
    pub sequence: u64,
    pub kind: String,
    pub content: String,
    pub status: UiItemStatus,
    pub collapsed: bool,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct TurnActivity {
    pub inference_rounds: usize,
    pub provider_retries: usize,
    pub tool_calls: usize,
    pub succeeded: usize,
    pub denied: usize,
    pub failed: usize,
}

/// Cached, terminal-width-specific conversation layout.
///
/// Content mutations mark the composed layout dirty, while scroll-only frames
/// reuse `wrapped_lines` and copy only the visible terminal window.
#[derive(Debug)]
pub(crate) struct ConversationRenderCache {
    pub(crate) width: u16,
    pub(crate) items: BTreeMap<String, Vec<ratatui::text::Line<'static>>>,
    pub(crate) wrapped_lines: Vec<ratatui::text::Line<'static>>,
    pub(crate) dirty: bool,
}

impl Default for ConversationRenderCache {
    fn default() -> Self {
        Self {
            width: 0,
            items: BTreeMap::new(),
            wrapped_lines: Vec::new(),
            dirty: true,
        }
    }
}

impl UiItem {
    pub fn streaming(id: String) -> Self {
        Self {
            id,
            sequence: 0,
            kind: "assistant".into(),
            content: String::new(),
            status: UiItemStatus::Streaming,
            collapsed: false,
        }
    }
}

/// Centralized application state.
#[derive(Debug)]
pub struct AppState {
    /// Current collaboration mode.
    pub mode: CollaborationMode,
    /// Brain awareness level.
    pub awareness: AwarenessState,
    /// Context window usage.
    pub context: ContextDisplay,
    /// Current model name.
    pub model_name: String,
    /// Explicit target for the next turn, then reconciled from the durable
    /// UserMessage start boundary when the Session projection advances.
    pub execution_target: ExecutionTargetSelection,
    /// Client-local selection protected from older projection pages until a
    /// newer durable UserMessage confirms the same typed target.
    pending_execution_target: Option<PendingExecutionTargetSelection>,
    /// Provider-reported cumulative tokens in the active Task projection,
    /// extended by usage events observed on this connection.
    pub total_tokens: u64,
    /// Provider-reported prompt tokens accumulated across the active turn's
    /// inference rounds. This is billed work, not context occupancy.
    pub turn_input_tokens: u64,
    /// Provider-reported completion tokens accumulated across the active turn.
    pub turn_output_tokens: u64,
    /// Tools used in current turn.
    pub turn_tool_count: usize,
    /// Authoritative per-turn activity counters; never inferred from prose.
    pub turn_activity: TurnActivity,
    /// Whether currently streaming a response.
    pub streaming: bool,
    /// Whether a turn is active (between turn_start and turn_done).
    pub turn_active: bool,
    /// Current ReAct loop iteration (0 = first call, 1+ = after tool calls).
    pub current_iteration: usize,
    /// Last protocol event included in this state.
    pub cursor: EventCursor,
    pub session_id: Option<String>,
    /// Exact turn identity from the versioned client protocol. Ephemeral
    /// overlay keys use this instead of global text positions.
    pub active_turn_id: Option<::contracts::TurnId>,
    /// Compatibility streams do not carry a turn id. Keep a presentation-only
    /// generation for stable overlay keys; it is deliberately not a domain ID
    /// and is never serialized or sent back to the daemon.
    pub(crate) live_turn_key: Option<u64>,
    pub(crate) next_live_turn_key: u64,
    pub provider_name: Option<String>,
    pub items: BTreeMap<String, UiItem>,
    /// Scrollback offset for the actually rendered Task Console conversation.
    /// Zero follows the tail; larger values move upward.
    pub conversation_scroll: u16,
    /// Rendered conversation lines keyed by item id, plus the composed,
    /// terminal-width-specific layout. Durable items and scroll-only frames
    /// reuse it; content changes or resize invalidate it.
    /// Held in a RefCell because the Task Console renders through an immutable
    /// `&AppState` view but needs to memoize between frames.
    pub(crate) conversation_render: std::cell::RefCell<ConversationRenderCache>,
    pub approvals: BTreeMap<String, ApprovalSnapshot>,
    pub agents: BTreeMap<String, AgentSnapshot>,
    /// Daemon-owned Session/Task/Activity projection. These fields are
    /// replaced atomically by a schema-versioned read snapshot and are never
    /// inferred from chat text or local widgets.
    pub projected_session: Option<::contracts::SessionRecord>,
    pub tasks: Vec<::contracts::TaskSnapshot>,
    pub activities: Vec<::contracts::ActivitySnapshot>,
    /// Ephemeral activity identities currently overlaid in `activities`.
    ///
    /// The reducer owns insertion, terminal updates, and durable reconciliation;
    /// renderers never reconstruct tool state from the legacy chat transcript.
    pub(crate) live_activity_ids: BTreeSet<String>,
    pub last_error: Option<String>,
    /// Semantic terminal projected from the canonical versioned turn stream.
    /// ACP and TUI derive this from the same `ClientEvent`, rather than from
    /// transport-specific success heuristics.
    pub last_terminal_status: Option<TurnTerminalStatus>,
    /// Latest bounded evaluation summary observed in the canonical Session
    /// stream. Full evidence remains in the evaluation store.
    pub latest_evaluation: Option<EvaluationReceiptRef>,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            mode: CollaborationMode::Default,
            awareness: AwarenessState::default(),
            context: ContextDisplay::default(),
            model_name: "unknown".to_string(),
            execution_target: ExecutionTargetSelection::default(),
            pending_execution_target: None,
            total_tokens: 0,
            turn_input_tokens: 0,
            turn_output_tokens: 0,
            turn_tool_count: 0,
            turn_activity: TurnActivity::default(),
            streaming: false,
            turn_active: false,
            current_iteration: 0,
            cursor: EventCursor::origin(),
            session_id: None,
            active_turn_id: None,
            live_turn_key: None,
            next_live_turn_key: 0,
            provider_name: None,
            items: BTreeMap::new(),
            conversation_scroll: 0,
            conversation_render: std::cell::RefCell::new(ConversationRenderCache::default()),
            approvals: BTreeMap::new(),
            agents: BTreeMap::new(),
            projected_session: None,
            tasks: Vec::new(),
            activities: Vec::new(),
            live_activity_ids: BTreeSet::new(),
            last_error: None,
            last_terminal_status: None,
            latest_evaluation: None,
        }
    }
}

impl AppState {
    pub(crate) fn invalidate_conversation_render(&mut self) {
        self.conversation_render.get_mut().dirty = true;
    }
}

impl AppState {
    pub fn select_execution_target_for_next_turn(&mut self, selection: ExecutionTargetSelection) {
        self.pending_execution_target = Some(PendingExecutionTargetSelection {
            selection: selection.clone(),
            after_sequence: self.cursor.sequence,
        });
        self.execution_target = selection;
    }

    pub fn execution_target_for_submission(&self) -> &ExecutionTargetSelection {
        self.pending_execution_target
            .as_ref()
            .map_or(&self.execution_target, |pending| &pending.selection)
    }

    pub fn has_pending_execution_target(&self) -> bool {
        self.pending_execution_target.is_some()
    }

    pub fn reset_execution_target_for_session(&mut self) {
        self.execution_target = ExecutionTargetSelection::default();
        self.pending_execution_target = None;
    }

    pub(crate) fn reconcile_projected_execution_target(
        &mut self,
        selection: &ExecutionTargetSelection,
        sequence: u64,
    ) {
        if let Some(pending) = self.pending_execution_target.as_ref() {
            if sequence <= pending.after_sequence || selection != &pending.selection {
                return;
            }
        }
        self.execution_target = selection.clone();
        self.pending_execution_target = None;
    }

    pub fn latest_context_budget(&self) -> Option<&::contracts::ContextBudgetProjection> {
        self.tasks
            .iter()
            .find(|task| {
                matches!(
                    task.phase,
                    ::contracts::TaskPhase::Active | ::contracts::TaskPhase::Interrupted
                )
            })
            .or_else(|| self.tasks.first())
            .and_then(|task| task.runtime_facts.as_ref())
            .and_then(|facts| facts.context_budget.as_deref())
    }

    pub fn context_status(&self) -> String {
        self.latest_context_budget().map_or_else(
            || self.context.display(),
            |budget| {
                format!(
                    "history {} / {} · window {}",
                    compact_tokens(budget.current_history_tokens.get()),
                    compact_tokens(budget.admissible_history_tokens.get()),
                    compact_tokens(budget.model_context_tokens.get()),
                )
            },
        )
    }

    pub fn context_pressure_percent(&self) -> Option<f64> {
        self.latest_context_budget().map_or_else(
            || self.context.usage_percent(),
            |budget| {
                let available = budget.admissible_history_tokens.get();
                (available > 0)
                    .then(|| budget.current_history_tokens.get() as f64 / available as f64 * 100.0)
            },
        )
    }

    /// Read-only, secret-safe diagnostic used by `/context`.
    pub fn context_diagnostic(&self) -> String {
        let Some(budget) = self.latest_context_budget() else {
            return format!(
                "Context budget unavailable: missing canonical turn-start projection\nLegacy event: {}",
                self.context.display()
            );
        };
        format!(
            "Model: {}\nWindow: {} (source: {} / {})\nProfile input: {} (source: {} / {})\nHistory: {} / {} · compaction threshold {} (source: {} / {})\nCosts: output reserve {} · system+skills {} · tools {} · pending {} · safety {}\nRollout: root remaining {} · child limit {} · current Agent remaining {}",
            budget.model_spec,
            compact_tokens(budget.model_context_tokens.get()),
            budget.model_source.kind.as_str(),
            budget.model_source.label,
            compact_tokens(budget.profile_input_limit_tokens.get()),
            budget.profile_source.kind.as_str(),
            budget.profile_source.label,
            compact_tokens(budget.current_history_tokens.get()),
            compact_tokens(budget.admissible_history_tokens.get()),
            compact_tokens(budget.compaction_threshold_tokens.get()),
            budget.history_source.kind.as_str(),
            budget.history_source.label,
            compact_tokens(budget.reserved_output_tokens.get()),
            compact_tokens(budget.system_and_skill_tokens.get()),
            compact_tokens(budget.tool_schema_tokens.get()),
            compact_tokens(budget.pending_input_tokens.get()),
            compact_tokens(budget.safety_margin_tokens.get()),
            rollout_diagnostic(&budget.rollout.root_remaining_tokens),
            rollout_diagnostic(&budget.rollout.child_limit_tokens),
            rollout_diagnostic(&budget.rollout.current_agent_remaining_tokens),
        )
    }

    /// Format the status line for the built-in status bar.
    pub fn format_status_line(&self) -> String {
        let mode_str = format!("{} {}", self.mode.icon(), self.mode.display_name());
        let ctx_str = self.context_status();
        let token_str = format!(
            "turn tokens: {} in / {} out",
            self.turn_input_tokens, self.turn_output_tokens
        );
        let aware_str = format!(
            "{} {}",
            self.awareness.level.icon(),
            self.awareness.level.display_name()
        );
        let tools_str = format!("{} tools", self.turn_tool_count);

        format!(
            "{} | {} | {} | {} | {} | {}",
            mode_str, self.model_name, ctx_str, token_str, aware_str, tools_str
        )
    }
}

fn rollout_diagnostic(value: &::contracts::RolloutBudgetValue) -> String {
    match value {
        ::contracts::RolloutBudgetValue::Known { value, source } => format!(
            "{} (source: {} / {})",
            compact_tokens(value.get()),
            source.kind.as_str(),
            source.label
        ),
        ::contracts::RolloutBudgetValue::Unknown { source, reason } => format!(
            "unknown ({}, source: {} / {})",
            reason.as_str(),
            source.kind.as_str(),
            source.label
        ),
    }
}

fn compact_tokens(tokens: u64) -> String {
    if tokens >= 1_000 {
        format!("{}k", grouped_decimal(tokens / 1_000))
    } else {
        grouped_decimal(tokens)
    }
}

fn grouped_decimal(value: u64) -> String {
    let digits = value.to_string();
    let mut output = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, ch) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            output.push(',');
        }
        output.push(ch);
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_context_capacity_has_a_reason_and_no_numeric_fallback() {
        let state = AppState::default();
        let status = state.context_status();
        let diagnostic = state.context_diagnostic();
        assert!(status.contains("unknown"));
        assert!(status.contains("missing context capacity projection"));
        assert!(diagnostic.contains("missing canonical turn-start projection"));
        assert!(!status.contains("200k"));
        assert!(!diagnostic.contains("200k"));
    }
}
