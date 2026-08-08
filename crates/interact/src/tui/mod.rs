//! TUI interface — interactive terminal UI and CLI entry point.

pub mod app;
mod json_lines;
pub mod reducer;
pub mod render;
pub mod response;
pub mod session_protocol;
pub mod test_infra;

pub mod activity_detail;
pub mod approval_dialog;
pub mod awareness;
pub mod chat;
pub mod checkpoint_picker;
pub mod command;
pub mod completion;
pub mod conscious_core;
pub mod diff_view;
pub mod file_picker;

pub mod help_overlay;
pub mod history_search;
pub mod host_time;
pub mod input;
pub mod input_safety;
pub mod markdown;
pub mod pager;
pub mod plan_view;
pub mod registry;
pub mod session_picker;
pub mod state;
pub mod status;
pub mod streaming;
pub mod subagent_view;
pub mod task_console;
pub mod term_compat;

/// Build the local chat envelope. Keeping this in one place prevents the TUI,
/// line mode, and `-m` mode from silently diverging.
pub fn chat_request(message: &str, workspace: &fabric::WorkspacePolicy) -> serde_json::Value {
    crate::intent::rpc(crate::intent::submit_prompt(crate::intent::PromptIntent {
        surface: fabric::contract::command::ClientSurface::Tui,
        correlation_id: format!("tui-chat:{}", uuid::Uuid::new_v4()),
        content: message,
        session_id: None,
        workspace,
        requirements: Vec::new(),
        task_kind: None,
        permission_mode: crate::host::permission_mode_from_environment(),
        execution_target: fabric::ExecutionTargetSelection::default(),
    }))
    .to_json_rpc(Some(1))
    .expect("typed chat request serializes")
}

/// Restore terminal to normal state.
/// Useful when the terminal is stuck in raw mode or mouse capture mode.
pub fn restore_terminal() {
    let _ = disable_raw_mode();
    let _ = execute!(
        io::stderr(),
        LeaveAlternateScreen,
        DisableBracketedPaste,
        DisableFocusChange,
        DisableMouseCapture
    );
    let _ = execute!(io::stderr(), crossterm::cursor::Show);
}

use std::collections::BTreeMap;
use std::io;
use std::sync::Arc;

use crossterm::{
    event::{
        DisableBracketedPaste, DisableFocusChange, DisableMouseCapture, EnableBracketedPaste,
        EnableFocusChange, EnableMouseCapture,
    },
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use fabric::Clock;
use ratatui::{
    backend::{CrosstermBackend, TestBackend},
    Terminal,
};
use tokio::net::UnixStream;

use self::app::lifecycle::run_app;
use self::app::lifecycle::simple_line_mode;
use self::chat::ChatWidget;
use self::completion::CompletionPopup;
use self::input::{CommandHistory, InputStateStore};
use self::plan_view::PlanViewState;
use self::state::AppState;
use self::status::StatusBar;
use self::streaming::StreamController;
use self::term_compat::TermCaps;
pub use self::test_infra::TestConfig;

use fabric::ui_event::SubAgentHandle;

/// Run the full TUI with raw mode, alternate screen, and IME-aware input.
/// This is the original entry point (no test config).
pub async fn run_tui(socket_path: &str) -> anyhow::Result<()> {
    run_with_config(socket_path, TestConfig::default()).await
}

/// Run the full TUI with optional test configuration.
pub async fn run_with_config(socket_path: &str, test_config: TestConfig) -> anyhow::Result<()> {
    let cwd = std::env::current_dir()?;
    let workspace = fabric::WorkspaceSelection::default().resolve(&cwd)?;
    run_with_workspace_config(socket_path, test_config, workspace).await
}

pub async fn run_with_workspace_config(
    socket_path: &str,
    test_config: TestConfig,
    workspace: fabric::WorkspacePolicy,
) -> anyhow::Result<()> {
    run_with_workspace_requirements(socket_path, test_config, workspace, Vec::new()).await
}

pub async fn run_with_workspace_requirements(
    socket_path: &str,
    test_config: TestConfig,
    workspace: fabric::WorkspacePolicy,
    turn_requirements: Vec<fabric::TurnRequirement>,
) -> anyhow::Result<()> {
    run_with_workspace_requirements_and_task_kind(
        socket_path,
        test_config,
        workspace,
        turn_requirements,
        None,
        crate::host::InitialSession::New,
    )
    .await
}

pub async fn run_with_workspace_requirements_and_task_kind(
    socket_path: &str,
    test_config: TestConfig,
    workspace: fabric::WorkspacePolicy,
    turn_requirements: Vec<fabric::TurnRequirement>,
    task_kind: Option<fabric::TaskKind>,
    initial_session: crate::host::InitialSession,
) -> anyhow::Result<()> {
    let caps = TermCaps::detect();
    let clock: Arc<dyn Clock> = Arc::new(self::host_time::ClientClock::new());

    let stream = match UnixStream::connect(socket_path).await {
        Ok(s) => s,
        Err(e) => {
            return Err(anyhow::anyhow!(
                "daemon connection failed after readiness negotiation [socket_connect_failed] at {socket_path}: {e}; run `aletheon doctor --json`"
            ));
        }
    };

    let model = std::env::var("OS_AGENT_MODEL").unwrap_or_default();
    let model_name = if model.is_empty() {
        "default".to_string()
    } else {
        model
    };

    // If not a TTY and no test input, fall back to simple line mode
    if (!atty::is(atty::Stream::Stdin) || !atty::is(atty::Stream::Stdout))
        && test_config.test_input.is_none()
    {
        anyhow::ensure!(
            initial_session == crate::host::InitialSession::New,
            "session selection requires an interactive terminal; pass `aletheon run PROMPT --resume SESSION` for non-interactive use"
        );
        return simple_line_mode(
            stream,
            caps,
            model_name,
            clock,
            workspace,
            turn_requirements,
            task_kind,
        )
        .await;
    }

    // Check if we're in test mode (no TTY needed)
    let is_test_mode = test_config.test_input.is_some();

    let result = if is_test_mode {
        // In test mode, use a test backend (no real terminal)
        let backend = TestBackend::new(120, 40);
        let mut terminal = Terminal::new(backend)?;
        run_app(
            &mut terminal,
            stream,
            caps,
            model_name,
            test_config,
            true,
            clock,
            workspace.clone(),
            turn_requirements.clone(),
            task_kind,
            initial_session,
        )
        .await
    } else {
        // RAII guard that restores terminal state on drop.
        // Handles normal exit, panic, and signal-driven exit.
        struct TerminalGuard;
        impl Drop for TerminalGuard {
            fn drop(&mut self) {
                let _ = disable_raw_mode();
                let _ = execute!(
                    io::stderr(),
                    LeaveAlternateScreen,
                    DisableBracketedPaste,
                    DisableFocusChange,
                    DisableMouseCapture
                );
                let _ = execute!(io::stderr(), crossterm::cursor::Show);
            }
        }

        // Set up real terminal
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(
            stdout,
            EnterAlternateScreen,
            EnableBracketedPaste,
            EnableFocusChange,
            EnableMouseCapture
        )?;
        let backend = CrosstermBackend::new(stdout);
        let mut terminal = Terminal::new(backend)?;

        // Clear alternate screen completely (fixes dirty data from previous runs)
        terminal.clear()?;

        // Install RAII guard — dropped on any exit path (return, panic, signal)
        let _guard = TerminalGuard;

        // Install panic hook to ensure terminal is restored on panic/crash.
        // The Drop guard handles the actual cleanup; this hook just ensures
        // the panic message is visible by flushing stderr.
        let original_hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            // TerminalGuard::drop() will run during unwinding
            original_hook(info);
        }));

        // Install signal handler to restore terminal on SIGINT/SIGTERM.
        // Note: We do NOT call std::process::exit() here because that would
        // skip Drop destructors (including TerminalGuard). Instead, we force
        // cleanup directly and then exit via exit() — the guard is already
        // redundant at that point since we cleaned up manually.
        ctrlc::set_handler(move || {
            // Directly restore terminal state (can't rely on Drop here)
            let _ = disable_raw_mode();
            let _ = execute!(
                io::stderr(),
                LeaveAlternateScreen,
                DisableBracketedPaste,
                DisableFocusChange,
                DisableMouseCapture
            );
            let _ = execute!(io::stderr(), crossterm::cursor::Show);
            std::process::exit(130);
        })
        .expect("Error setting Ctrl-C handler");

        let result = run_app(
            &mut terminal,
            stream,
            caps,
            model_name,
            test_config,
            false,
            clock,
            workspace,
            turn_requirements,
            task_kind,
            initial_session,
        )
        .await;

        // TerminalGuard::drop() handles terminal cleanup.
        // Explicitly drop the guard before returning to ensure clean state.
        drop(_guard);

        result
    };

    result
}

/// Main TUI application state.
struct App {
    workspace: fabric::WorkspacePolicy,
    turn_requirements: Vec<fabric::TurnRequirement>,
    requested_task_kind: Option<fabric::TaskKind>,
    /// Compatibility-only V0 transcript recorder. It is never rendered and is
    /// never consulted for durable conversation, tool, or terminal truth; all
    /// visible state is projected into `app_state` through the reducer.
    compat_transcript: ChatWidget,
    input_buf: String,
    /// Cursor position in input_buf (byte index).
    cursor: usize,
    stream: UnixStream,
    read_buf: Vec<u8>,
    running: bool,
    streaming: bool,
    /// Whether a chat turn is active (between turn_start and turn_done).
    /// Unlike `streaming` (which controls the spinner and is reset by
    /// process_response), `turn_active` is only set by turn_start and
    /// cleared by turn_done. Used by auto-submit to know when the next
    /// message can be sent.
    turn_active: bool,
    response_buf: json_lines::JsonLineBuffer,
    caps: TermCaps,
    /// Monotonically increasing JSON-RPC request id.
    next_request_id: u64,
    /// UI mutations which must happen only after their matching RPC succeeds.
    pending_commands: BTreeMap<u64, PendingCommand>,
    /// Transport-only sequencing for fork-and-rewind. Recovery authority stays
    /// in the daemon; the client emits rewind only after a successful fork.
    deferred_checkpoint_rewind: Option<DeferredCheckpointRewind>,
    /// Non-turn RPCs whose result should stop the command spinner immediately.
    pending_non_turn: std::collections::BTreeSet<u64>,
    /// Connection-local driver for the daemon-owned Session projection.
    /// Authoritative content lives in `app_state`; these fields only schedule
    /// bounded snapshot/page reads.
    projection_session_id: Option<String>,
    /// Requested canonical session. This is transport-local selection state;
    /// it is not a Session projection and must not be rendered as one.
    projection_target_session_id: Option<String>,
    projection_request_in_flight: bool,
    projection_polling: bool,
    projection_next_poll_at: fabric::MonoTime,
    model_name: String,
    status: StatusBar,
    /// Last Ctrl+C press time (for double-press detection).
    last_ctrl_c: Option<fabric::MonoTime>,
    /// Whether input has CJK characters (affects Enter behavior).
    has_cjk: bool,
    /// A leading action sigil arrived through paste and remains inert until
    /// the buffer is cleared or a palette choice is explicitly accepted.
    input_literal: bool,
    /// Exact shell text awaiting a second explicit Enter. This is local
    /// presentation state only; execution authority remains in the Host.
    pending_shell_confirmation: Option<String>,
    /// Pending submit (delayed for IME composition).
    pending_submit: Option<fabric::MonoTime>,
    /// First render flag.
    first_render: bool,
    /// Pending approval dialog (shown as modal overlay).
    pending_approval: Option<approval_dialog::ApprovalDialog>,
    /// Local selection cursor into the daemon-owned Activity projection.
    /// It never owns or mutates activity state.
    selected_activity: Option<usize>,
    detail: Option<diff_view::DiffView>,
    latest_diff: Option<String>,
    latest_patch: Option<fabric::PatchDelta>,
    /// Streaming controller for incremental rendering
    stream_ctrl: StreamController,
    /// Command history
    history: CommandHistory,
    input_store: InputStateStore,
    input_dirty: bool,
    input_persist_at: fabric::MonoTime,
    history_search: Option<history_search::HistorySearchOverlay>,
    /// Tab completion popup
    completion: CompletionPopup,
    /// Pager overlay (Ctrl+T to open, q/Esc to close)
    pager: Option<pager::PagerOverlay>,
    /// Canonical session list with keyboard navigation and resume action.
    session_picker: Option<session_picker::SessionPicker>,
    /// Local selection cursor over the daemon-owned checkpoint list.
    checkpoint_picker: Option<checkpoint_picker::CheckpointPicker>,
    /// Transaction awaiting a second explicit rollback keypress because the
    /// Host reports only best-effort mutation coverage.
    review_risk_confirmation: Option<String>,
    /// Frame counter for spinner animation.
    frame_counter: u64,
    /// Centralized application state (mode, awareness, context).
    app_state: AppState,
    /// Plan view state for plan mode visualization.
    plan_view: PlanViewState,
    /// Active sub-agents for inline display.
    sub_agents: Vec<SubAgentHandle>,
    /// Current ReAct loop iteration (0 = first call, 1+ = after tool calls).
    current_iteration: usize,
    next_transient_item: u64,
    compat_projected_entries: usize,
    pub registry: registry::CommandRegistry,
    /// Clock for time-based operations (injectable for testing).
    pub clock: Arc<dyn Clock>,
}

impl App {
    fn new(
        stream: UnixStream,
        caps: TermCaps,
        model_name: String,
        clock: Arc<dyn Clock>,
        workspace: fabric::WorkspacePolicy,
        turn_requirements: Vec<fabric::TurnRequirement>,
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
            compat_transcript: ChatWidget::new(caps.clone()),
            input_buf: draft,
            cursor,
            stream,
            read_buf: vec![0u8; 8192],
            running: true,
            streaming: false,
            turn_active: false,
            response_buf: json_lines::JsonLineBuffer::default(),
            caps,
            next_request_id: 1,
            pending_commands: BTreeMap::new(),
            deferred_checkpoint_rewind: None,
            pending_non_turn: std::collections::BTreeSet::new(),
            projection_session_id: None,
            projection_target_session_id: None,
            projection_request_in_flight: false,
            projection_polling: false,
            projection_next_poll_at: fabric::MonoTime(0),
            model_name,
            status,
            last_ctrl_c: None,
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
            input_persist_at: fabric::MonoTime(0),
            history_search: None,
            completion: CompletionPopup::new(),
            pager: None,
            session_picker: None,
            checkpoint_picker: None,
            review_risk_confirmation: None,
            frame_counter: 0,
            app_state,
            plan_view: PlanViewState::default(),
            sub_agents: Vec::new(),
            current_iteration: 0,
            next_transient_item: 0,
            compat_projected_entries: 0,
            registry: registry::CommandRegistry::new(),
            clock,
        }
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

    /// Project a bounded local command/compatibility response onto the same
    /// visible item map as durable and live conversation. These entries are
    /// explicitly ephemeral and are discarded on snapshot/reconnect.
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
            self::state::UiItem {
                id,
                sequence: self
                    .app_state
                    .cursor
                    .sequence
                    .saturating_add(self.next_transient_item),
                kind: "assistant".into(),
                content: text.into(),
                status: self::state::UiItemStatus::Streaming,
                collapsed: false,
            },
        );
    }

    /// Mirror only new V0 system notices into reducer-owned visible state.
    /// User/assistant/tool business truth is never read from this recorder.
    pub(crate) fn sync_compat_notices(&mut self) {
        let start = self
            .compat_projected_entries
            .min(self.compat_transcript.entries.len());
        let notices = self.compat_transcript.entries[start..]
            .iter()
            .filter_map(|entry| match entry {
                self::chat::ChatEntry::Text(message)
                    if message.role == self::chat::Role::System =>
                {
                    Some(message.content.clone())
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        self.compat_projected_entries = self.compat_transcript.entries.len();
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
        self.input_persist_at = fabric::MonoTime(self.clock.mono_now().0.saturating_add(500));
    }

    fn check_cjk(&mut self) {
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

#[derive(Debug, Clone, PartialEq, Eq)]
enum PendingCommand {
    InitializeSession,
    InitializeSkills,
    NewSession {
        clear_screen: bool,
    },
    OpenSessionPicker,
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
struct DeferredCheckpointRewind {
    parent_session_id: String,
    child_session_id: String,
    prompt_index: u64,
}

#[cfg(test)]
mod working_dir_tests {
    #[test]
    fn chat_request_contains_the_resolved_workspace() {
        let workspace = fabric::WorkspacePolicy::from_resolved_roots(
            "/tmp/project".into(),
            vec!["/tmp/shared".into()],
        )
        .unwrap();
        let request = super::chat_request("inspect this project", &workspace);
        assert_eq!(request["method"], "client.intent");
        let prompt = &request["params"]["command"]["arguments"];
        assert_eq!(prompt["content"], "inspect this project");
        assert_eq!(prompt["workspace"]["cwd"], "/tmp/project");
        assert_eq!(
            prompt["workspace"]["writable_roots"],
            serde_json::json!(["/tmp/project", "/tmp/shared"])
        );
    }
}
