//! Centralized TUI application state.
//!
//! Extracts state fields from the monolithic App struct in mod.rs
//! into a focused module for clarity and testability.

use fabric::protocol::client::EventCursor;
use fabric::ui_event::{AwarenessLevel, CollaborationMode};
use fabric::{AgentSnapshot, ApprovalSnapshot, MonoTime, TurnTerminalStatus};
use serde::Serialize;
use std::collections::BTreeMap;

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
#[derive(Debug, Clone)]
pub struct ContextDisplay {
    pub used: usize,
    pub max: usize,
}

impl Default for ContextDisplay {
    fn default() -> Self {
        Self {
            used: 0,
            max: 200_000,
        }
    }
}

impl ContextDisplay {
    pub fn usage_percent(&self) -> f64 {
        if self.max == 0 {
            0.0
        } else {
            (self.used as f64 / self.max as f64) * 100.0
        }
    }

    pub fn display(&self) -> String {
        let used_k = self.used / 1000;
        let max_k = self.max / 1000;
        let pct = self.usage_percent();
        format!("ctx: {used_k}k/{max_k}k ({pct:.0}%)")
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UiItemStatus {
    Streaming,
    Completed,
    Failed,
}

#[derive(Debug, Clone, Serialize)]
pub struct UiItem {
    pub id: String,
    pub sequence: u64,
    pub kind: String,
    pub content: String,
    pub status: UiItemStatus,
    pub collapsed: bool,
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
    /// Total tokens used in session.
    pub total_tokens: u32,
    /// Tools used in current turn.
    pub turn_tool_count: usize,
    /// Whether currently streaming a response.
    pub streaming: bool,
    /// Whether a turn is active (between turn_start and turn_done).
    pub turn_active: bool,
    /// Current ReAct loop iteration (0 = first call, 1+ = after tool calls).
    pub current_iteration: usize,
    /// Last protocol event included in this state.
    pub cursor: EventCursor,
    pub session_id: Option<String>,
    pub provider_name: Option<String>,
    pub items: BTreeMap<String, UiItem>,
    pub approvals: BTreeMap<String, ApprovalSnapshot>,
    pub agents: BTreeMap<String, AgentSnapshot>,
    pub last_error: Option<String>,
    /// Semantic terminal projected from the canonical versioned turn stream.
    /// ACP and TUI derive this from the same `ClientEvent`, rather than from
    /// transport-specific success heuristics.
    pub last_terminal_status: Option<TurnTerminalStatus>,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            mode: CollaborationMode::Default,
            awareness: AwarenessState::default(),
            context: ContextDisplay::default(),
            model_name: "unknown".to_string(),
            total_tokens: 0,
            turn_tool_count: 0,
            streaming: false,
            turn_active: false,
            current_iteration: 0,
            cursor: EventCursor::origin(),
            session_id: None,
            provider_name: None,
            items: BTreeMap::new(),
            approvals: BTreeMap::new(),
            agents: BTreeMap::new(),
            last_error: None,
            last_terminal_status: None,
        }
    }
}

impl AppState {
    /// Format the status line for the built-in status bar.
    pub fn format_status_line(&self) -> String {
        let mode_str = format!("{} {}", self.mode.icon(), self.mode.display_name());
        let ctx_str = self.context.display();
        let token_str = format!("tokens: {}k", self.total_tokens / 1000);
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
