//! TUI interface — interactive terminal UI and CLI entry point.

pub mod app;
pub mod render;
pub mod response;
pub mod test_infra;

pub mod approval_dialog;
pub mod awareness;
pub mod chat;
pub mod command;
pub mod completion;
pub mod computer;

pub mod help_overlay;
pub mod history_search;
pub mod input;
pub mod markdown;
pub mod pager;
pub mod plan_view;
pub mod skill;
pub mod state;
pub mod status;
pub mod streaming;
pub mod subagent_view;
pub mod term_compat;

// CLI modules (formerly cli/)
pub mod cli;
pub mod debug;
pub mod goal;
pub(crate) mod rpc_client;
pub mod workflow;

// Re-export the main entry point
pub use cli::run;

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
use self::input::CommandHistory;
use self::plan_view::PlanViewState;
use self::skill::SkillLoader;
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
    let caps = TermCaps::detect();
    let clock: Arc<dyn Clock> = Arc::new(aletheon_kernel::chronos::SystemClock::new());

    let stream = match UnixStream::connect(socket_path).await {
        Ok(s) => s,
        Err(e) => {
            return Err(anyhow::anyhow!(
                "Cannot connect to daemon at {}: {}\n\nStart the daemon first:\n  aletheon daemon &",
                socket_path,
                e
            ));
        }
    };

    let model = std::env::var("OS_AGENT_MODEL").unwrap_or_default();
    let model_name = if model.is_empty() {
        "mimo-v2.5-pro".to_string()
    } else {
        model
    };

    // If not a TTY and no test input, fall back to simple line mode
    if (!atty::is(atty::Stream::Stdin) || !atty::is(atty::Stream::Stdout))
        && test_config.test_input.is_none()
    {
        return simple_line_mode(stream, caps, model_name, clock).await;
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
    chat: ChatWidget,
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
    response_buf: String,
    caps: TermCaps,
    skill_loader: SkillLoader,
    model_name: String,
    status: StatusBar,
    /// Last Ctrl+C press time (for double-press detection).
    last_ctrl_c: Option<fabric::MonoTime>,
    /// Whether input has CJK characters (affects Enter behavior).
    has_cjk: bool,
    /// Pending submit (delayed for IME composition).
    pending_submit: Option<fabric::MonoTime>,
    /// First render flag.
    first_render: bool,
    /// Pending approval dialog (shown as modal overlay).
    pending_approval: Option<approval_dialog::ApprovalDialog>,
    /// Streaming controller for incremental rendering
    stream_ctrl: StreamController,
    /// Current turn's token count
    turn_tokens: Option<(u32, u32)>,
    /// Cumulative tokens across all turns
    total_tokens: u32,
    /// Command history
    history: CommandHistory,
    /// Tab completion popup
    completion: CompletionPopup,
    /// Pager overlay (Ctrl+T to open, q/Esc to close)
    pager: Option<pager::PagerOverlay>,
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
    /// Clock for time-based operations (injectable for testing).
    pub clock: Arc<dyn Clock>,
}

impl App {
    fn new(stream: UnixStream, caps: TermCaps, model_name: String, clock: Arc<dyn Clock>) -> Self {
        let mut skill_loader = SkillLoader::new(SkillLoader::default_dir());
        if let Err(e) = skill_loader.load_all() {
            eprintln!("Warning: failed to load skills: {}", e);
        }
        let mut status = StatusBar::new(caps.clone());
        status.connected = true;
        status.model_name = model_name.clone();

        Self {
            chat: ChatWidget::new(caps.clone()),
            input_buf: String::new(),
            cursor: 0,
            stream,
            read_buf: vec![0u8; 8192],
            running: true,
            streaming: false,
            turn_active: false,
            response_buf: String::new(),
            caps,
            skill_loader,
            model_name,
            status,
            last_ctrl_c: None,
            has_cjk: false,
            pending_submit: None,
            first_render: true,
            pending_approval: None,
            stream_ctrl: StreamController::new(Arc::clone(&clock)),
            turn_tokens: None,
            total_tokens: 0,
            history: CommandHistory::new(),
            completion: CompletionPopup::new(),
            pager: None,
            frame_counter: 0,
            app_state: AppState::default(),
            plan_view: PlanViewState::default(),
            sub_agents: Vec::new(),
            current_iteration: 0,
            clock,
        }
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
