//! TUI interface — interactive terminal UI and CLI entry point.

pub mod app;
pub(crate) mod controller;
pub(crate) mod model;
pub mod reducer;
pub mod render;
pub mod response;
pub mod test_infra;

pub mod activity_detail;
pub mod agent_inspector;
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
pub mod presentation;
pub mod registry;
pub mod session_picker;
pub mod state;
pub mod status;
pub mod streaming;
pub mod subagent_view;
pub mod task_console;
pub mod term_compat;

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

use self::app::lifecycle::run_app;
use self::app::lifecycle::simple_line_mode;
#[cfg(test)]
pub(crate) use self::model::PendingCommand;
pub(crate) use self::model::{DeferredCheckpointRewind, SystemNoticeQueue, TuiModel};
use ::contracts::Clock;
use crossterm::{
    event::{
        DisableBracketedPaste, DisableFocusChange, DisableMouseCapture, EnableBracketedPaste,
        EnableFocusChange, EnableMouseCapture,
    },
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use gateway::client::{GatewayClient, UnixSocketTransport};
use ratatui::{
    backend::{CrosstermBackend, TestBackend},
    Terminal,
};

use self::term_compat::TermCaps;
pub use self::test_infra::TestConfig;

/// Run the full TUI with raw mode, alternate screen, and IME-aware input.
/// This is the original entry point (no test config).
pub async fn run_tui(socket_path: &str) -> anyhow::Result<()> {
    run_with_config(socket_path, TestConfig::default()).await
}

/// Run the full TUI with optional test configuration.
pub async fn run_with_config(socket_path: &str, test_config: TestConfig) -> anyhow::Result<()> {
    let cwd = std::env::current_dir()?;
    let workspace = ::contracts::WorkspaceSelection::default().resolve(&cwd)?;
    run_with_workspace_config(socket_path, test_config, workspace).await
}

pub async fn run_with_workspace_config(
    socket_path: &str,
    test_config: TestConfig,
    workspace: ::contracts::WorkspacePolicy,
) -> anyhow::Result<()> {
    run_with_workspace_requirements(socket_path, test_config, workspace, Vec::new()).await
}

pub async fn run_with_workspace_requirements(
    socket_path: &str,
    test_config: TestConfig,
    workspace: ::contracts::WorkspacePolicy,
    turn_requirements: Vec<::contracts::TurnRequirement>,
) -> anyhow::Result<()> {
    run_with_workspace_requirements_and_task_kind(
        socket_path,
        test_config,
        workspace,
        turn_requirements,
        None,
        crate::host::InitialSession::New,
        gateway::protocol::RequestedPermissionMode::Inherit,
    )
    .await
}

pub async fn run_with_workspace_requirements_and_task_kind(
    socket_path: &str,
    test_config: TestConfig,
    workspace: ::contracts::WorkspacePolicy,
    turn_requirements: Vec<::contracts::TurnRequirement>,
    task_kind: Option<::contracts::TaskKind>,
    initial_session: crate::host::InitialSession,
    requested_permission: gateway::protocol::RequestedPermissionMode,
) -> anyhow::Result<()> {
    let caps = TermCaps::detect();
    let clock: Arc<dyn Clock> = Arc::new(self::host_time::ClientClock::new());

    // Every real TUI invocation, including scripted acceptance mode, uses the
    // versioned Gateway transport.  The legacy JSON-RPC client is retained
    // only by unit-test fixtures (`TuiModel::new`) and is never opened by the
    // production composition root.
    let model = std::env::var("OS_AGENT_MODEL").unwrap_or_default();
    let model_name = if model.is_empty() {
        "default".to_string()
    } else {
        model
    };

    // The line adapter and the full TUI share the same typed Gateway path.
    let is_test_mode = test_config.test_input.is_some();
    let typed_gateway = Some(GatewayClient::new(
        UnixSocketTransport::connect(socket_path)
            .await
            .map_err(|error| anyhow::anyhow!("typed Gateway connection failed: {error}"))?,
    ));

    // If not a TTY and no test input, fall back to simple line mode.
    if (!atty::is(atty::Stream::Stdin) || !atty::is(atty::Stream::Stdout)) && !is_test_mode {
        anyhow::ensure!(
            initial_session == crate::host::InitialSession::New,
            "session selection requires an interactive terminal; pass `aletheon run PROMPT --resume SESSION` for non-interactive use"
        );
        return simple_line_mode(
            typed_gateway,
            caps,
            model_name,
            clock,
            workspace,
            turn_requirements,
            task_kind,
            requested_permission.clone(),
        )
        .await;
    }

    let result = if is_test_mode {
        // In test mode, use a test backend (no real terminal)
        let backend = TestBackend::new(120, 40);
        let mut terminal = Terminal::new(backend)?;
        run_app(
            &mut terminal,
            typed_gateway,
            caps,
            model_name,
            test_config,
            true,
            clock,
            workspace.clone(),
            turn_requirements.clone(),
            task_kind,
            initial_session,
            requested_permission.clone(),
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
            typed_gateway,
            caps,
            model_name,
            test_config,
            false,
            clock,
            workspace,
            turn_requirements,
            task_kind,
            initial_session,
            requested_permission,
        )
        .await;

        // TerminalGuard::drop() handles terminal cleanup.
        // Explicitly drop the guard before returning to ensure clean state.
        drop(_guard);

        result
    };

    result
}
