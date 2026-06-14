pub mod chat;
pub mod command;
#[cfg(all(feature = "input", feature = "display", feature = "a11y"))]
pub mod computer;
pub mod event;
pub mod input;
pub mod markdown;
pub mod skill;
pub mod status;
pub mod term_compat;

use std::io::{self, Stdout};
use std::time::Duration;

use crossterm::{
    event::{DisableBracketedPaste, DisableFocusChange, DisableMouseCapture, EnableBracketedPaste, EnableFocusChange, EnableMouseCapture},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Padding, Paragraph},
    Terminal,
};
use tokio::net::UnixStream;
use tokio::sync::mpsc;

use self::chat::ChatWidget;
use self::command::{BuiltinCommand, CommandType, parse_command};
#[cfg(all(feature = "input", feature = "display", feature = "a11y"))]
use self::computer::ComputerCommands;
use self::event::{Action, TuiEvent};
use self::input::InputArea;
use self::skill::SkillLoader;
use self::status::StatusBar;
use self::term_compat::TermCaps;

type Tui = Terminal<CrosstermBackend<Stdout>>;

/// Run the TUI, connecting to the daemon at `socket_path`.
pub async fn run(socket_path: &str) -> anyhow::Result<()> {
    // Detect terminal capabilities
    let caps = TermCaps::detect();

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

    let stream = match UnixStream::connect(socket_path).await {
        Ok(s) => s,
        Err(e) => {
            disable_raw_mode()?;
            execute!(
                terminal.backend_mut(),
                LeaveAlternateScreen,
                DisableBracketedPaste,
                DisableFocusChange,
                DisableMouseCapture
            )?;
            terminal.show_cursor()?;
            return Err(anyhow::anyhow!(
                "Cannot connect to daemon at {}: {}\n\nStart the daemon first:\n  argos daemon &",
                socket_path,
                e
            ));
        }
    };

    let (tx, rx) = mpsc::unbounded_channel::<TuiEvent>();

    let tx_clone = tx.clone();
    tokio::spawn(async move {
        loop {
            match tokio::task::spawn_blocking(crossterm::event::read).await {
                Ok(Ok(event)) => {
                    if let Some(tui_event) = TuiEvent::from_crossterm(event) {
                        if tx_clone.send(tui_event).is_err() {
                            break;
                        }
                    }
                }
                _ => break,
            }
        }
    });

    let tx_tick = tx.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_millis(100));
        loop {
            interval.tick().await;
            if tx_tick.send(TuiEvent::Tick).is_err() {
                break;
            }
        }
    });

    let mut app = App::new(stream, terminal, rx, caps);

    if let Ok(model) = std::env::var("OS_AGENT_MODEL") {
        app.status.provider_info = model.clone();
        app.status.model_name = model.clone();
        app.input.model_name = model;
    }

    app.chat.add_message(
        chat::Role::System,
        "Welcome to argos! Type a message to get started. Ctrl+C to quit.".to_string(),
    );

    let result = app.run(tx).await;

    disable_raw_mode()?;
    execute!(
        app.terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableBracketedPaste,
        DisableFocusChange,
        DisableMouseCapture
    )?;
    app.terminal.show_cursor()?;

    result
}

struct App {
    chat: ChatWidget,
    input: InputArea,
    status: StatusBar,
    terminal: Tui,
    event_rx: mpsc::UnboundedReceiver<TuiEvent>,
    stream: UnixStream,
    read_buf: Vec<u8>,
    running: bool,
    streaming: bool,
    response_buf: String,
    caps: TermCaps,
    skill_loader: SkillLoader,
    show_header: bool,
}

impl App {
    fn new(
        stream: UnixStream,
        terminal: Tui,
        event_rx: mpsc::UnboundedReceiver<TuiEvent>,
        caps: TermCaps,
    ) -> Self {
        let mut status = StatusBar::new(caps.clone());
        status.connected = true;

        let chat = ChatWidget::new(caps.clone());

        // Load skills from default directory
        let mut skill_loader = SkillLoader::new(SkillLoader::default_dir());
        if let Err(e) = skill_loader.load_all() {
            eprintln!("Warning: failed to load skills: {}", e);
        }

        Self {
            chat,
            input: InputArea::new(),
            status,
            terminal,
            event_rx,
            stream,
            read_buf: vec![0u8; 8192],
            running: true,
            streaming: false,
            response_buf: String::new(),
            caps,
            skill_loader,
            show_header: true,
        }
    }

    async fn run(&mut self, _tx: mpsc::UnboundedSender<TuiEvent>) -> anyhow::Result<()> {
        self.render()?;

        while self.running {
            let event = self.event_rx.recv().await;
            match event {
                Some(event) => {
                    let action = self.handle_event(event);
                    match action {
                        Action::Submit(text) => {
                            self.handle_submit(text).await;
                        }
                        Action::Command(text) => {
                            self.handle_command(text).await;
                        }
                        Action::Quit => {
                            self.running = false;
                        }
                        Action::ScrollUp(n) => {
                            self.chat.scroll_up(n);
                        }
                        Action::ScrollDown(n) => {
                            self.chat.scroll_down(n);
                        }
                        Action::None => {}
                    }
                    self.render()?;
                }
                None => break,
            }
        }

        Ok(())
    }

    fn handle_event(&mut self, event: TuiEvent) -> Action {
        match event {
            TuiEvent::Key(key) => self.input.handle_key(key),
            TuiEvent::Resize => {
                self.render().ok();
                Action::None
            }
            TuiEvent::Tick => {
                if self.streaming {
                    self.try_read_response();
                }
                self.status.tick_spinner();
                self.render().ok();
                Action::None
            }
            TuiEvent::Paste(text) => {
                for ch in text.chars() {
                    self.input.handle_key(crossterm::event::KeyEvent::new(
                        crossterm::event::KeyCode::Char(ch),
                        crossterm::event::KeyModifiers::empty(),
                    ));
                }
                Action::None
            }
        }
    }

    async fn handle_command(&mut self, text: String) {
        let parsed = parse_command(&text);
        match parsed {
            Some(CommandType::Builtin(cmd)) => self.execute_builtin(cmd).await,
            Some(CommandType::Skill { name, args }) => self.execute_skill(&name, &args).await,
            None => {
                self.chat
                    .add_message(chat::Role::System, "Invalid command.".to_string());
            }
        }
    }

    async fn execute_builtin(&mut self, cmd: BuiltinCommand) {
        match cmd {
            BuiltinCommand::Help => self.show_help(),
            BuiltinCommand::Clear => self.send_clear().await,
            BuiltinCommand::Status => self.send_status().await,
            BuiltinCommand::Quit => {
                self.running = false;
            }
            BuiltinCommand::Computer { args } => {
                #[cfg(all(feature = "input", feature = "display", feature = "a11y"))]
                {
                    let computer = ComputerCommands::new_mock();
                    match computer.handle(&args) {
                        Ok(output) => self.chat.add_message(chat::Role::System, output),
                        Err(e) => self.chat.add_message(
                            chat::Role::System,
                            format!("Computer error: {e}"),
                        ),
                    }
                }
                #[cfg(not(all(feature = "input", feature = "display", feature = "a11y")))]
                {
                    let _ = args;
                    self.chat.add_message(
                        chat::Role::System,
                        "Computer commands require input, display, and a11y features.".to_string(),
                    );
                }
            }
            BuiltinCommand::Input => {
                self.chat.add_message(
                    chat::Role::System,
                    "请输入消息（支持中文输入法，回车发送，空行取消）：".to_string(),
                );
                self.render().ok();

                // Leave raw mode to allow IME input
                disable_raw_mode().ok();
                execute!(
                    self.terminal.backend_mut(),
                    LeaveAlternateScreen,
                    DisableBracketedPaste,
                    DisableFocusChange,
                    DisableMouseCapture
                ).ok();
                self.terminal.show_cursor().ok();

                // Read line with normal terminal mode (IME works here)
                let mut input = String::new();
                std::io::stdin().read_line(&mut input).ok();
                let text = input.trim().to_string();

                // Re-enter raw mode
                enable_raw_mode().ok();
                execute!(
                    self.terminal.backend_mut(),
                    EnterAlternateScreen,
                    EnableBracketedPaste,
                    EnableFocusChange,
                    EnableMouseCapture
                ).ok();
                self.terminal.clear().ok();

                if !text.is_empty() {
                    // Submit as a regular message
                    self.handle_submit(text).await;
                } else {
                    self.chat.add_message(chat::Role::System, "已取消".to_string());
                }
            }
        }
    }

    fn show_help(&mut self) {
        let caps = &self.caps;
        let vline = caps.vline();
        let mut help = format!("{vline} Built-in commands:\n");
        help.push_str(&format!("{vline}   /help     Show this help\n"));
        help.push_str(&format!("{vline}   /clear    Clear conversation\n"));
        help.push_str(&format!("{vline}   /status   Show daemon status\n"));
        help.push_str(&format!("{vline}   /input    Input with IME support (Chinese etc)\n"));
        help.push_str(&format!("{vline}   /quit     Exit\n"));

        let skills = self.skill_loader.list();
        if !skills.is_empty() {
            help.push_str(&format!("{vline}\n"));
            help.push_str(&format!("{vline} Skills:\n"));
            for skill in &skills {
                help.push_str(&format!("{vline}   /{}  {}\n", skill.name, skill.description));
            }
        }

        self.chat.add_message(chat::Role::System, help);
    }

    async fn send_clear(&mut self) {
        let msg = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "clear",
            "id": 1,
        });
        let payload = serde_json::to_string(&msg).unwrap_or_default();
        let framed = format!("{}\n", payload);

        use tokio::io::AsyncWriteExt;
        if self.stream.write_all(framed.as_bytes()).await.is_ok() {
            self.stream.flush().await.ok();
        }

        self.chat = ChatWidget::new(self.caps.clone());
        self.chat
            .add_message(chat::Role::System, "Conversation cleared.".to_string());
    }

    async fn send_status(&mut self) {
        let msg = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "status",
            "id": 1,
        });
        let payload = serde_json::to_string(&msg).unwrap_or_default();
        let framed = format!("{}\n", payload);

        use tokio::io::AsyncWriteExt;
        if self.stream.write_all(framed.as_bytes()).await.is_ok() {
            self.stream.flush().await.ok();
        }

        self.chat
            .add_message(chat::Role::System, "Requesting status...".to_string());
    }

    async fn execute_skill(&mut self, name: &str, args: &str) {
        let skill = match self.skill_loader.get(name) {
            Some(s) => s.clone(),
            None => {
                self.chat.add_message(
                    chat::Role::System,
                    format!("Unknown skill: /{}. Type /help for available skills.", name),
                );
                return;
            }
        };

        let term_width = self.terminal.size().map(|s| s.width).unwrap_or(80);
        self.chat.set_width(term_width);

        // Show user message
        let user_display = if args.is_empty() {
            format!("/{}", name)
        } else {
            format!("/{} {}", name, args)
        };
        self.chat.add_message(chat::Role::User, user_display);

        // Build message with skill context
        let message = if args.is_empty() {
            skill.content.clone()
        } else {
            format!("{}\n\nUser input: {}", skill.content, args)
        };

        // Send to daemon with skill as system context
        let msg = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "chat",
            "id": 1,
            "params": {
                "message": message,
                "system": skill.content,
            },
        });
        let payload = serde_json::to_string(&msg).unwrap_or_default();
        let framed = format!("{}\n", payload);

        use tokio::io::AsyncWriteExt;
        if self.stream.write_all(framed.as_bytes()).await.is_ok() {
            self.stream.flush().await.ok();
            self.streaming = true;
            self.response_buf.clear();
            self.status.waiting = true;
            self.chat
                .add_message(chat::Role::Assistant, String::new());
        } else {
            self.status.connected = false;
            self.chat.add_message(
                chat::Role::System,
                "Failed to send message. Is the daemon running?".to_string(),
            );
        }
    }

    async fn handle_submit(&mut self, text: String) {
        self.show_header = false;
        let term_width = self.terminal.size().map(|s| s.width).unwrap_or(80);
        self.chat.set_width(term_width);
        self.chat.add_message(chat::Role::User, text.clone());

        let msg = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "chat",
            "id": 1,
            "params": { "message": text },
        });
        let payload = serde_json::to_string(&msg).unwrap_or_default();
        let framed = format!("{}\n", payload);

        use tokio::io::AsyncWriteExt;
        if self.stream.write_all(framed.as_bytes()).await.is_ok() {
            self.stream.flush().await.ok();
            self.streaming = true;
            self.response_buf.clear();
            self.status.waiting = true;
            self.chat.add_message(chat::Role::Assistant, String::new());
        } else {
            self.status.connected = false;
            self.chat.add_message(
                chat::Role::System,
                "Failed to send message. Is the daemon running?".to_string(),
            );
        }
    }

    fn try_read_response(&mut self) {
        loop {
            match self.stream.try_read(&mut self.read_buf) {
                Ok(0) => {
                    self.streaming = false;
                    self.status.waiting = false;
                    self.status.connected = false;
                    self.chat.add_message(
                        chat::Role::System,
                        "Connection to daemon lost.".to_string(),
                    );
                    break;
                }
                Ok(n) => {
                    let chunk = String::from_utf8_lossy(&self.read_buf[..n]);
                    self.response_buf.push_str(&chunk);

                    if let Some(msg) = self.try_parse_response() {
                        self.process_response(msg);
                    }

                    if !self.response_buf.is_empty() {
                        self.chat.update_last_message(self.response_buf.clone());
                        self.status.waiting = false;
                    }
                }
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                    break;
                }
                Err(_) => {
                    self.streaming = false;
                    self.status.waiting = false;
                    break;
                }
            }
        }
    }

    fn try_parse_response(&self) -> Option<serde_json::Value> {
        let trimmed = self.response_buf.trim();
        if trimmed.is_empty() {
            return None;
        }
        serde_json::from_str(trimmed).ok()
    }

    fn process_response(&mut self, msg: serde_json::Value) {
        if let Some(result) = msg.get("result") {
            if let Some(text) = result.get("response").and_then(|v| v.as_str()) {
                self.chat.update_last_message(text.to_string());
            } else if let Some(text) = result.as_str() {
                self.chat.update_last_message(text.to_string());
            } else if let Some(text) = result.get("content").and_then(|v| v.as_str()) {
                self.chat.update_last_message(text.to_string());
            } else {
                let pretty = serde_json::to_string_pretty(result)
                    .unwrap_or_else(|_| format!("{:?}", result));
                self.chat.update_last_message(pretty);
            }
            self.streaming = false;
            self.status.waiting = false;
            self.response_buf.clear();
        } else if let Some(error) = msg.get("error") {
            let error_msg = if let Some(obj) = error.as_object() {
                let code = obj.get("code").and_then(|v| v.as_i64()).unwrap_or(0);
                let message = obj
                    .get("message")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Unknown error");
                format!("Error ({code}): {message}")
            } else {
                format!("Error: {}", error)
            };
            self.chat.update_last_message(error_msg);
            self.streaming = false;
            self.status.waiting = false;
            self.response_buf.clear();
        } else if msg.get("method").is_some() {
            self.response_buf.clear();
        }
    }

    fn render(&mut self) -> anyhow::Result<()> {
        let chat_ref = &self.chat;
        let input_ref = &self.input;
        let status_ref = &self.status;
        let caps_ref = &self.caps;
        let show_header = self.show_header;

        self.terminal.draw(|f| {
            let header_rows: u16 = if show_header { 3 } else { 1 };
            let input_rows: u16 = 3; // border + input + hint

            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(header_rows),
                    Constraint::Min(1),
                    Constraint::Length(input_rows),
                    Constraint::Length(1),
                ])
                .split(f.area());

            // Header / title bar
            if show_header {
                let bg = caps_ref.color(20, 20, 60);
                let model_display = if status_ref.model_name.is_empty() {
                    "no model"
                } else {
                    &status_ref.model_name
                };
                let conn = if status_ref.connected { "connected" } else { "disconnected" };
                let vsep = if caps_ref.unicode { "  │  " } else { "  |  " };

                let line1 = Line::from(Span::styled(
                    "  argos v0.1.0",
                    Style::default().fg(Color::White),
                ));
                let line2 = Line::from(vec![
                    Span::styled(
                        format!("  model: {model_display}{vsep}{conn}"),
                        Style::default().fg(Color::DarkGray),
                    ),
                ]);
                let hints = if caps_ref.unicode {
                    "  Type a message. Ctrl+L 中文 │ /help for commands"
                } else {
                    "  Type a message. Ctrl+L CJK | /help for commands"
                };
                let line3 = Line::from(Span::styled(hints, Style::default().fg(Color::DarkGray)));

                let header_lines = vec![line1, line2, line3];
                let header_widget = Paragraph::new(header_lines).style(Style::default().bg(bg));
                f.render_widget(header_widget, chunks[0]);
            } else {
                // Collapsed single-line title bar
                let title_bg = caps_ref.color(20, 20, 60);
                let model_display = if status_ref.model_name.is_empty() {
                    ""
                } else {
                    &status_ref.model_name
                };
                let title = if model_display.is_empty() {
                    " argos".to_string()
                } else {
                    format!(" argos  │  {model_display}")
                };
                let title_line = Line::from(Span::styled(title, Style::default().fg(Color::White)));
                let title_widget = Paragraph::new(title_line).style(Style::default().bg(title_bg));
                f.render_widget(title_widget, chunks[0]);
            }

            // Chat area
            let chat_block = Block::default()
                .borders(Borders::NONE)
                .padding(Padding::horizontal(1));
            let chat_inner = chat_block.inner(chunks[1]);
            f.render_widget(chat_block, chunks[1]);
            f.render_widget(chat_ref.render_widget(), chat_inner);

            // Input area (3 rows: border + input + hint)
            let input_area = ratatui::layout::Rect {
                x: chunks[2].x,
                y: chunks[2].y,
                width: chunks[2].width,
                height: chunks[2].height,
            };
            f.render_widget(input_ref.render_widget(caps_ref), input_area);

            // Status bar
            f.render_widget(status_ref.render_widget(), chunks[3]);
        })?;

        Ok(())
    }
}
