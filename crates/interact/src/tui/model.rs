//! TUI presentation model and local connection state.

#[cfg(test)]
use std::collections::BTreeMap;
use std::sync::Arc;

use super::presentation::SubAgentHandle;
use ::contracts::Clock;
use gateway::client::{GatewayClient, UnixSocketTransport};

use super::agent_inspector;
use super::approval_dialog;
use super::chat::Role;
use super::checkpoint_picker;
use super::completion::CompletionPopup;
use super::controller::TuiController;
use super::diff_view;
use super::history_search;
use super::input::{CommandHistory, InputStateStore};
use super::pager;
use super::plan_view::PlanViewState;
use super::registry;
use super::render;
use super::session_picker;
use super::state::AppState;
use super::status::StatusBar;
use super::streaming::StreamController;
use super::term_compat::TermCaps;

/// Bounded local system-notice queue. It is presentation feedback only: it
/// has no renderer, scroll state, persistence, or authority semantics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SystemNotice {
    pub(crate) content: String,
}

pub(crate) struct SystemNoticeQueue {
    pub(crate) entries: Vec<SystemNotice>,
}

impl SystemNoticeQueue {
    pub(crate) fn new(_caps: TermCaps) -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    pub(crate) fn push(&mut self, role: Role, content: String) {
        if role == Role::System {
            self.entries.push(SystemNotice { content });
        }
    }
}

/// Main TUI presentation model. It contains only view state and local input
/// preferences; transport ownership is delegated to [`TuiController`].
pub(crate) struct TuiModel {
    pub(crate) workspace: ::contracts::WorkspacePolicy,
    pub(crate) turn_requirements: Vec<::contracts::TurnRequirement>,
    pub(crate) requested_task_kind: Option<::contracts::TaskKind>,
    pub(crate) requested_permission: gateway::protocol::RequestedPermissionMode,
    /// Explicit runtime requirement consumed by the next submitted turn.
    pub(crate) next_agent_runtime: Option<String>,
    /// Bounded local system notices. They are never used as durable
    /// conversation, tool, or terminal truth; typed projections own those.
    pub(crate) system_notices: SystemNoticeQueue,
    pub(crate) input_buf: String,
    /// Cursor position in input_buf (byte index).
    pub(crate) cursor: usize,
    pub(crate) controller: TuiController,
    pub(crate) running: bool,
    pub(crate) streaming: bool,
    /// Whether a chat turn is active (between turn_start and turn_done).
    /// Unlike `streaming` (which controls the spinner and is reset by
    /// process_response), `turn_active` is only set by turn_start and
    /// cleared by turn_done. Used by auto-submit to know when the next
    /// message can be sent.
    pub(crate) turn_active: bool,
    /// Opaque Runtime-assigned turn reference returned by the typed Gateway
    /// for the currently submitted turn.  The TUI never mints or interprets
    /// this value; it is only compared with the projection's `TurnId` string
    /// so a stale completed turn cannot settle a newer request.
    pub(crate) active_turn_ref: Option<String>,
    pub(crate) caps: TermCaps,
    /// UI mutations which must happen only after their matching RPC succeeds.
    #[cfg(test)]
    pub(crate) pending_commands: BTreeMap<u64, PendingCommand>,
    /// Transport-only sequencing for fork-and-rewind. Recovery authority stays
    /// in the daemon; the client emits rewind only after a successful fork.
    pub(crate) deferred_checkpoint_rewind: Option<DeferredCheckpointRewind>,
    /// Test-only compatibility bookkeeping for legacy response fixtures.
    #[cfg(test)]
    pub(crate) pending_non_turn: std::collections::BTreeSet<u64>,
    /// Connection-local driver for the daemon-owned Session projection.
    /// Authoritative content lives in `app_state`; these fields only schedule
    /// bounded snapshot/page reads.
    pub(crate) projection_session_id: Option<String>,
    /// Requested canonical session. This is transport-local selection state;
    /// it is not a Session projection and must not be rendered as one.
    pub(crate) projection_target_session_id: Option<String>,
    pub(crate) projection_request_in_flight: bool,
    pub(crate) projection_polling: bool,
    pub(crate) projection_next_poll_at: ::contracts::MonoTime,
    pub(crate) model_name: String,
    pub(crate) status: StatusBar,
    /// Last Ctrl+C press time (for double-press detection).
    pub(crate) last_ctrl_c: Option<::contracts::MonoTime>,
    /// Local acknowledgement that the current turn received an explicit user
    /// cancellation request. Authoritative settlement still comes from the
    /// daemon's durable `TurnSettlement` item.
    pub(crate) turn_cancel_requested: bool,
    /// Whether input has CJK characters (affects Enter behavior).
    pub(crate) has_cjk: bool,
    /// A leading action sigil arrived through paste and remains inert until
    /// the buffer is cleared or a palette choice is explicitly accepted.
    pub(crate) input_literal: bool,
    /// Exact shell text awaiting a second explicit Enter. This is local
    /// presentation state only; execution authority remains in the Host.
    pub(crate) pending_shell_confirmation: Option<String>,
    /// Pending submit (delayed for IME composition).
    pub(crate) pending_submit: Option<::contracts::MonoTime>,
    /// First render flag.
    pub(crate) first_render: bool,
    /// Pending approval dialog (shown as modal overlay).
    pub(crate) pending_approval: Option<approval_dialog::ApprovalDialog>,
    /// Local selection cursor into the daemon-owned Activity projection.
    /// It never owns or mutates activity state.
    pub(crate) selected_activity: Option<usize>,
    pub(crate) detail: Option<diff_view::DiffView>,
    pub(crate) latest_diff: Option<String>,
    pub(crate) latest_patch: Option<::contracts::PatchDelta>,
    /// Streaming controller for incremental rendering
    pub(crate) stream_ctrl: StreamController,
    /// Command history
    pub(crate) history: CommandHistory,
    pub(crate) input_store: InputStateStore,
    pub(crate) input_dirty: bool,
    pub(crate) input_persist_at: ::contracts::MonoTime,
    pub(crate) history_search: Option<history_search::HistorySearchOverlay>,
    /// Tab completion popup
    pub(crate) completion: CompletionPopup,
    /// Pager overlay (Ctrl+T to open, q/Esc to close)
    pub(crate) pager: Option<pager::PagerOverlay>,
    /// Canonical session list with keyboard navigation and resume action.
    pub(crate) session_picker: Option<session_picker::SessionPicker>,
    /// Read-only navigator over canonical child Agent sessions.
    pub(crate) agent_inspector: Option<agent_inspector::AgentInspector>,
    /// Next bounded refresh for an open child Agent inspector.
    pub(crate) agent_inspector_next_refresh_at: ::contracts::MonoTime,
    /// Local selection cursor over the daemon-owned checkpoint list.
    pub(crate) checkpoint_picker: Option<checkpoint_picker::CheckpointPicker>,
    /// Transaction awaiting a second explicit rollback keypress because the
    /// Host reports only best-effort mutation coverage.
    pub(crate) review_risk_confirmation: Option<String>,
    /// Frame counter for spinner animation.
    pub(crate) frame_counter: u64,
    /// Centralized application state (mode, awareness, context).
    pub(crate) app_state: AppState,
    /// Plan view state for plan mode visualization.
    pub(crate) plan_view: PlanViewState,
    /// Active sub-agents for inline display.
    pub(crate) sub_agents: Vec<SubAgentHandle>,
    /// Current ReAct loop iteration (0 = first call, 1+ = after tool calls).
    pub(crate) current_iteration: usize,
    pub(crate) next_transient_item: u64,
    pub(crate) system_notice_cursor: usize,
    pub(crate) registry: registry::CommandRegistry,
    /// Clock for time-based operations (injectable for testing).
    pub(crate) clock: Arc<dyn Clock>,
}

impl TuiModel {
    #[cfg(test)]
    pub(crate) fn new(
        caps: TermCaps,
        model_name: String,
        clock: Arc<dyn Clock>,
        workspace: ::contracts::WorkspacePolicy,
        turn_requirements: Vec<::contracts::TurnRequirement>,
    ) -> Self {
        Self::new_with_gateway(
            None,
            None,
            caps,
            model_name,
            clock,
            workspace,
            turn_requirements,
            gateway::protocol::RequestedPermissionMode::Inherit,
        )
    }

    #[cfg(test)]
    pub(crate) fn new_with_gateway(
        _legacy_gateway: Option<()>,
        typed_gateway: Option<GatewayClient<UnixSocketTransport>>,
        caps: TermCaps,
        model_name: String,
        clock: Arc<dyn Clock>,
        workspace: ::contracts::WorkspacePolicy,
        turn_requirements: Vec<::contracts::TurnRequirement>,
        requested_permission: gateway::protocol::RequestedPermissionMode,
    ) -> Self {
        let mut status = StatusBar::new(caps.clone());
        status.connected = true;
        status.model_name = model_name.clone();
        let mut app_state = AppState::default();
        app_state.model_name = model_name.clone();

        let input_store = InputStateStore::for_workspace(&workspace);
        let (history, draft) = input_store.load();
        let cursor = draft.len();

        Self {
            workspace,
            turn_requirements,
            requested_task_kind: None,
            requested_permission,
            next_agent_runtime: None,
            system_notices: SystemNoticeQueue::new(caps.clone()),
            input_buf: draft,
            cursor,
            controller: TuiController::new(typed_gateway),
            running: true,
            streaming: false,
            turn_active: false,
            active_turn_ref: None,
            caps,
            #[cfg(test)]
            pending_commands: BTreeMap::new(),
            deferred_checkpoint_rewind: None,
            #[cfg(test)]
            pending_non_turn: std::collections::BTreeSet::new(),
            projection_session_id: None,
            projection_target_session_id: None,
            projection_request_in_flight: false,
            projection_polling: false,
            projection_next_poll_at: ::contracts::MonoTime(0),
            model_name,
            status,
            last_ctrl_c: None,
            turn_cancel_requested: false,
            has_cjk: false,
            input_literal: false,
            pending_shell_confirmation: None,
            pending_submit: None,
            first_render: true,
            pending_approval: None,
            selected_activity: None,
            detail: None,
            latest_diff: None,
            latest_patch: None,
            stream_ctrl: StreamController::new(Arc::clone(&clock)),
            history,
            input_store,
            input_dirty: false,
            input_persist_at: ::contracts::MonoTime(0),
            history_search: None,
            completion: CompletionPopup::new(),
            pager: None,
            session_picker: None,
            agent_inspector: None,
            agent_inspector_next_refresh_at: ::contracts::MonoTime(0),
            checkpoint_picker: None,
            review_risk_confirmation: None,
            frame_counter: 0,
            app_state,
            plan_view: PlanViewState::default(),
            sub_agents: Vec::new(),
            current_iteration: 0,
            next_transient_item: 0,
            system_notice_cursor: 0,
            registry: registry::CommandRegistry::new(),
            clock,
        }
    }

    #[cfg(not(test))]
    pub(crate) fn new_with_gateway(
        typed_gateway: Option<GatewayClient<UnixSocketTransport>>,
        caps: TermCaps,
        model_name: String,
        clock: Arc<dyn Clock>,
        workspace: ::contracts::WorkspacePolicy,
        turn_requirements: Vec<::contracts::TurnRequirement>,
        requested_permission: gateway::protocol::RequestedPermissionMode,
    ) -> Self {
        let mut status = StatusBar::new(caps.clone());
        status.connected = true;
        status.model_name = model_name.clone();
        let mut app_state = AppState::default();
        app_state.model_name = model_name.clone();

        let input_store = InputStateStore::for_workspace(&workspace);
        let (history, draft) = input_store.load();
        let cursor = draft.len();

        Self {
            workspace,
            turn_requirements,
            requested_task_kind: None,
            requested_permission,
            next_agent_runtime: None,
            system_notices: SystemNoticeQueue::new(caps.clone()),
            input_buf: draft,
            cursor,
            controller: TuiController::new(typed_gateway),
            running: true,
            streaming: false,
            turn_active: false,
            active_turn_ref: None,
            caps,
            #[cfg(test)]
            pending_commands: BTreeMap::new(),
            deferred_checkpoint_rewind: None,
            #[cfg(test)]
            pending_non_turn: std::collections::BTreeSet::new(),
            projection_session_id: None,
            projection_target_session_id: None,
            projection_request_in_flight: false,
            projection_polling: false,
            projection_next_poll_at: ::contracts::MonoTime(0),
            model_name,
            status,
            last_ctrl_c: None,
            turn_cancel_requested: false,
            has_cjk: false,
            input_literal: false,
            pending_shell_confirmation: None,
            pending_submit: None,
            first_render: true,
            pending_approval: None,
            selected_activity: None,
            detail: None,
            latest_diff: None,
            latest_patch: None,
            stream_ctrl: StreamController::new(Arc::clone(&clock)),
            history,
            input_store,
            input_dirty: false,
            input_persist_at: ::contracts::MonoTime(0),
            history_search: None,
            completion: CompletionPopup::new(),
            pager: None,
            session_picker: None,
            agent_inspector: None,
            agent_inspector_next_refresh_at: ::contracts::MonoTime(0),
            checkpoint_picker: None,
            review_risk_confirmation: None,
            frame_counter: 0,
            app_state,
            plan_view: PlanViewState::default(),
            sub_agents: Vec::new(),
            current_iteration: 0,
            next_transient_item: 0,
            system_notice_cursor: 0,
            registry: registry::CommandRegistry::new(),
            clock,
        }
    }

    /// Build the immutable presentation view consumed by `TuiRenderer`.
    /// Renderer code never receives this model directly, so transport, clock,
    /// persistence and command queues remain outside the rendering boundary.
    pub(crate) fn view(&self) -> render::TuiView<'_> {
        render::TuiView {
            caps: &self.caps,
            input_buf: &self.input_buf,
            cursor: self.cursor,
            has_cjk: self.has_cjk,
            status: &self.status,
            pending_approval: &self.pending_approval,
            detail: &self.detail,
            completion: &self.completion,
            app_state: &self.app_state,
            stream_ctrl: &self.stream_ctrl,
            pager: &self.pager,
            session_picker: &self.session_picker,
            agent_inspector: &self.agent_inspector,
            checkpoint_picker: &self.checkpoint_picker,
            history_search: &self.history_search,
            workspace_name: self
                .workspace
                .cwd()
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("workspace"),
            selected_activity: self.selected_activity,
            next_agent_runtime: self.next_agent_runtime.as_deref(),
        }
    }

    pub(crate) fn mark_frame_rendered(&mut self) {
        self.first_render = false;
        self.frame_counter = self.frame_counter.wrapping_add(1);
    }

    /// Project the current live assistant stream onto the canonical reducer
    /// surface so text is visible before the durable projection commits it
    /// (U1-AUDIT-001). The reducer drops the overlay once a durable assistant
    /// item with the same or later sequence arrives, preventing duplication.
    pub(crate) fn dispatch_live_assistant_text(&mut self) {
        let text = self.stream_ctrl.current_text();
        let sequence = self.app_state.cursor.sequence;
        let _ = crate::tui::reducer::reduce(
            &mut self.app_state,
            crate::tui::reducer::UiAction::LiveAssistantText { text, sequence },
        );
    }

    /// Project a bounded local command response onto the same visible item
    /// map as durable and live conversation. These entries are explicitly
    /// ephemeral and are discarded on snapshot/reconnect.
    pub(crate) fn show_transient_assistant(&mut self, text: impl Into<String>) {
        self.next_transient_item = self.next_transient_item.saturating_add(1);
        let session = self
            .app_state
            .session_id
            .as_deref()
            .unwrap_or("unbound-session");
        let turn = self
            .app_state
            .active_turn_id
            .map(|turn| turn.0.to_string())
            .unwrap_or_else(|| "no-turn".into());
        let id = format!(
            "local:{session}:{turn}:assistant:{}",
            self.next_transient_item
        );
        self.app_state.items.insert(
            id.clone(),
            super::state::UiItem {
                id,
                sequence: self
                    .app_state
                    .cursor
                    .sequence
                    .saturating_add(self.next_transient_item),
                kind: "assistant".into(),
                content: text.into(),
                status: super::state::UiItemStatus::Streaming,
                collapsed: false,
            },
        );
    }

    /// Replace any in-flight assistant representation with one bounded
    /// transient item. The daemon can deliver the same completion as live text
    /// and a typed command result; those transport observations must not become
    /// separate conversation messages.
    #[cfg(test)]
    pub(crate) fn replace_transient_assistant(&mut self, text: impl Into<String>) {
        self.app_state.items.retain(|id, item| {
            item.kind != "assistant"
                || item.status != super::state::UiItemStatus::Streaming
                || !(id.starts_with("live:") || id.starts_with("local:"))
        });
        self.show_transient_assistant(text);
    }

    /// Project only new system notices into the local visible overlay.
    /// User/assistant/tool business truth is never read from this recorder.
    pub(crate) fn sync_system_notices(&mut self) {
        let start = self
            .system_notice_cursor
            .min(self.system_notices.entries.len());
        let notices = self.system_notices.entries[start..]
            .iter()
            .map(|notice| notice.content.clone())
            .collect::<Vec<_>>();
        self.system_notice_cursor = self.system_notices.entries.len();
        for notice in notices {
            self.show_transient_assistant(notice);
        }
    }

    pub(crate) fn persist_input_state(&mut self) {
        self.input_store.save(&self.history, &self.input_buf);
        self.input_dirty = false;
    }

    pub(crate) fn mark_input_dirty(&mut self) {
        self.input_dirty = true;
        self.input_persist_at = ::contracts::MonoTime(self.clock.mono_now().0.saturating_add(500));
    }

    pub(crate) fn check_cjk(&mut self) {
        self.has_cjk = self.input_buf.chars().any(|c| {
            let cp = c as u32;
            // CJK Unified Ideographs + common ranges
            (0x4E00..=0x9FFF).contains(&cp)   // CJK Unified
                || (0x3400..=0x4DBF).contains(&cp)  // CJK Extension A
                || (0x3000..=0x303F).contains(&cp)  // CJK Symbols
                || (0xFF00..=0xFFEF).contains(&cp)  // Fullwidth
                || (0xAC00..=0xD7AF).contains(&cp)  // Korean Hangul
                || (0x3040..=0x309F).contains(&cp)  // Hiragana
                || (0x30A0..=0x30FF).contains(&cp) // Katakana
        });
    }
}

#[cfg(test)]
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PendingCommand {
    InitializeSession,
    InitializeSkills,
    NewSession {
        clear_screen: bool,
    },
    OpenSessionPicker,
    OpenAgentInspector {
        focus: Option<String>,
    },
    OpenCheckpointPicker,
    CheckpointFork {
        parent_session_id: String,
        prompt_index: Option<u64>,
    },
    CheckpointRewind {
        child_session_id: Option<String>,
    },
    TransactionReview,
    TransactionSettlementLatest,
    ProjectionSnapshot {
        session_id: String,
    },
    ProjectionEvents {
        session_id: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DeferredCheckpointRewind {
    pub(crate) parent_session_id: String,
    pub(crate) child_session_id: String,
    pub(crate) prompt_index: u64,
}
