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

use std::io::{self, Stdout, Write};
use std::time::{Duration, Instant};

use crossterm::{
    event::{
        DisableBracketedPaste, DisableFocusChange, DisableMouseCapture,
        EnableBracketedPaste, EnableFocusChange, EnableMouseCapture,
        Event, KeyCode, KeyEvent, KeyModifiers,
    },
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Padding, Paragraph},
    Terminal,
};
use tokio::net::UnixStream;

use self::chat::{ChatWidget, Role as ChatRole};
use self::command::{BuiltinCommand, CommandType, parse_command};
#[cfg(all(feature = "input", feature = "display", feature = "a11y"))]
use self::computer::ComputerCommands;
use self::skill::SkillLoader;
use self::status::StatusBar;
use self::term_compat::TermCaps;

/// Run the full TUI with raw mode, alternate screen, and IME-aware input.
pub async fn run(socket_path: &str) -> anyhow::Result<()> {
    let caps = TermCaps::detect();

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
    let model_name = if model.is_empty() { "mimo-v2.5-pro".to_string() } else { model };

    // If not a TTY, fall back to simple line mode
    if !atty::is(atty::Stream::Stdin) || !atty::is(atty::Stream::Stdout) {
        return simple_line_mode(stream, caps, model_name).await;
    }

    // Set up terminal
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

    let result = run_app(&mut terminal, stream, caps, model_name).await;

    // Restore terminal
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableBracketedPaste,
        DisableFocusChange,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

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
    response_buf: String,
    caps: TermCaps,
    skill_loader: SkillLoader,
    model_name: String,
    status: StatusBar,
    /// Last Ctrl+C press time (for double-press detection).
    last_ctrl_c: Option<Instant>,
    /// Whether input has CJK characters (affects Enter behavior).
    has_cjk: bool,
    /// Pending submit (delayed for IME composition).
    pending_submit: Option<Instant>,
    /// Scroll offset for chat area.
    scroll_offset: u16,
    /// First render flag.
    first_render: bool,
}

impl App {
    fn new(stream: UnixStream, caps: TermCaps, model_name: String) -> Self {
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
            response_buf: String::new(),
            caps,
            skill_loader,
            model_name,
            status,
            last_ctrl_c: None,
            has_cjk: false,
            pending_submit: None,
            scroll_offset: 0,
            first_render: true,
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
                || (0x30A0..=0x30FF).contains(&cp)  // Katakana
        });
    }
}

async fn run_app(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    stream: UnixStream,
    caps: TermCaps,
    model_name: String,
) -> anyhow::Result<()> {
    let mut app = App::new(stream, caps, model_name.clone());

    // Clear daemon session on startup (avoids stale data from previous runs)
    let clear_msg = serde_json::json!({"jsonrpc": "2.0", "method": "clear", "id": 0});
    use tokio::io::AsyncWriteExt;
    let _ = app.stream.write_all(format!("{}\n", clear_msg).as_bytes()).await;
    let _ = app.stream.flush().await;

    // Welcome message
    app.chat.add_message(
        ChatRole::System,
        "Welcome to aletheon! Type a message to get started.\nShift+Enter 换行 │ Enter 发送 │ Ctrl+C 清空/退出 │ /copy 复制 │ /help 帮助".to_string(),
    );

    while app.running {
        // Resize handling
        if let Ok(size) = terminal.size() {
            app.chat.set_width(size.width);
        }

        // Draw
        draw(terminal, &mut app)?;

        // Check pending submit (IME delay)
        if let Some(pending_time) = app.pending_submit {
            if pending_time.elapsed() > Duration::from_millis(100) {
                app.pending_submit = None;
                let text = app.input_buf.trim().to_string();
                if !text.is_empty() {
                    app.input_buf.clear();
                    app.cursor = 0;
                    app.has_cjk = false;
                    submit_message(&mut app, text).await;
                }
            }
        }

        // Poll for events (short timeout to allow spinner/submit updates)
        let poll_timeout = if app.streaming || app.pending_submit.is_some() {
            Duration::from_millis(50)
        } else {
            Duration::from_millis(200)
        };

        if crossterm::event::poll(poll_timeout)? {
            match crossterm::event::read()? {
                Event::Key(key) => {
                    handle_key(&mut app, key).await;
                }
                Event::Paste(text) => {
                    // Paste: insert at cursor
                    for ch in text.chars() {
                        app.input_buf.insert(app.cursor, ch);
                        app.cursor += ch.len_utf8();
                    }
                    app.check_cjk();
                }
                Event::Resize(w, _h) => {
                    app.chat.set_width(w);
                }
                _ => {}
            }
        }

        // Try reading daemon response
        if app.streaming {
            try_read_response(&mut app);
        }
    }

    Ok(())
}

async fn handle_key(app: &mut App, key: KeyEvent) {
    // Ctrl+C: first press clears input, second press quits
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
        if app.input_buf.is_empty() {
            match app.last_ctrl_c {
                Some(t) if t.elapsed() < Duration::from_secs(2) => {
                    app.running = false;
                    return;
                }
                _ => {
                    app.last_ctrl_c = Some(Instant::now());
                    return;
                }
            }
        } else {
            app.input_buf.clear();
            app.cursor = 0;
            app.has_cjk = false;
            app.pending_submit = None;
            return;
        }
    }

    // Ctrl+D: quit
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('d') {
        if app.input_buf.is_empty() {
            app.running = false;
            return;
        }
    }

    // Ctrl+L: clear screen
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('l') {
        app.chat = ChatWidget::new(app.caps.clone());
        return;
    }

    match key.code {
        // Enter: submit (or Shift+Enter / Alt+Enter: newline)
        KeyCode::Enter => {
            // Shift+Enter or Alt+Enter → newline
            if key.modifiers.contains(KeyModifiers::SHIFT)
                || key.modifiers.contains(KeyModifiers::ALT)
            {
                app.input_buf.insert(app.cursor, '\n');
                app.cursor += 1;
                return;
            }

            // Check for `\` + Enter → newline (continuation)
            if app.input_buf.ends_with('\\') {
                app.input_buf.pop(); // remove trailing `\`
                app.cursor = app.input_buf.len();
                app.input_buf.insert(app.cursor, '\n');
                app.cursor += 1;
                return;
            }

            // Enter → submit (with CJK delay)
            let text = app.input_buf.trim().to_string();
            if text.is_empty() {
                return;
            }

            if app.has_cjk {
                // Delay submit to let IME finish composition
                // (OpenCode's double-defer pattern adapted for Rust)
                app.pending_submit = Some(Instant::now());
            } else {
                // No CJK: submit immediately
                app.input_buf.clear();
                app.cursor = 0;
                app.has_cjk = false;
                submit_message(app, text).await;
            }
        }

        // Backspace
        KeyCode::Backspace => {
            if app.cursor > 0 {
                let prev = app.input_buf[..app.cursor]
                    .char_indices()
                    .last()
                    .map(|(i, _)| i)
                    .unwrap_or(0);
                app.input_buf.replace_range(prev..app.cursor, "");
                app.cursor = prev;
                app.check_cjk();
            }
        }

        // Delete
        KeyCode::Delete => {
            if app.cursor < app.input_buf.len() {
                let next = app.input_buf[app.cursor..]
                    .char_indices()
                    .nth(1)
                    .map(|(i, _)| app.cursor + i)
                    .unwrap_or(app.input_buf.len());
                app.input_buf.replace_range(app.cursor..next, "");
                app.check_cjk();
            }
        }

        // Character input
        KeyCode::Char(c) => {
            app.input_buf.insert(app.cursor, c);
            app.cursor += c.len_utf8();
            app.check_cjk();
        }

        // Cursor movement
        KeyCode::Left => {
            if app.cursor > 0 {
                app.cursor = app.input_buf[..app.cursor]
                    .char_indices()
                    .last()
                    .map(|(i, _)| i)
                    .unwrap_or(0);
            }
        }
        KeyCode::Right => {
            if app.cursor < app.input_buf.len() {
                app.cursor = app.input_buf[app.cursor..]
                    .char_indices()
                    .nth(1)
                    .map(|(i, _)| app.cursor + i)
                    .unwrap_or(app.input_buf.len());
            }
        }
        KeyCode::Home => app.cursor = 0,
        KeyCode::End => app.cursor = app.input_buf.len(),

        // History (up/down) — scroll chat for now
        KeyCode::PageUp => {
            app.chat.scroll_up(5);
        }
        KeyCode::PageDown => {
            app.chat.scroll_down(5);
        }

        // Escape: clear input
        KeyCode::Esc => {
            app.input_buf.clear();
            app.cursor = 0;
            app.has_cjk = false;
            app.pending_submit = None;
        }

        _ => {}
    }
}

async fn submit_message(app: &mut App, text: String) {
    // Check for /commands
    if text.starts_with('/') {
        let parsed = parse_command(&text);
        match parsed {
            Some(CommandType::Builtin(BuiltinCommand::Quit)) => {
                app.running = false;
                return;
            }
            Some(CommandType::Builtin(BuiltinCommand::Clear)) => {
                app.chat = ChatWidget::new(app.caps.clone());
                return;
            }
            Some(CommandType::Builtin(BuiltinCommand::Copy)) => {
                // Copy last assistant message to clipboard via OSC 52
                let last_assistant = app.chat.messages.iter().rev()
                    .find(|m| m.role == ChatRole::Assistant)
                    .map(|m| m.content.clone());
                match last_assistant {
                    Some(text) if !text.is_empty() => {
                        let encoded = base64_encode(&text);
                        // OSC 52: set clipboard to base64-encoded text
                        let osc = format!("\x1b]52;c;{}\x1b\\", encoded);
                        io::stdout().write_all(osc.as_bytes()).ok();
                        io::stdout().flush().ok();
                        app.chat.add_message(ChatRole::System, "已复制到剪贴板".to_string());
                    }
                    _ => {
                        app.chat.add_message(ChatRole::System, "没有可复制的内容".to_string());
                    }
                }
                return;
            }
            Some(CommandType::Builtin(BuiltinCommand::Help)) => {
                let help = "内置命令：\n  /help         显示帮助\n  /clear        清空对话\n  /copy         复制最后回复到剪贴板\n  /status (st)  查看自我演化状态\n  /reflect      查看反思记录\n  /reflect_now  执行即时反思\n  /evolution    查看演化历史\n  /genome       查看基因组\n  /quit         退出\n\n输入：\n  Shift+Enter 或 \\+Enter  换行\n  Enter                   发送\n  Ctrl+C                   清空/退出\n  Esc                      清空输入\n  PgUp/PgDn               滚动聊天";
                app.chat.add_message(ChatRole::System, help.to_string());
                return;
            }
            Some(CommandType::Builtin(BuiltinCommand::Status)) => {
                let msg = serde_json::json!({
                    "jsonrpc": "2.0", "method": "status", "id": 1
                });
                let payload = serde_json::to_string(&msg).unwrap_or_default();
                use tokio::io::AsyncWriteExt;
                let framed = format!("{}\n", payload);
                let _ = app.stream.write_all(framed.as_bytes()).await;
                let _ = app.stream.flush().await;
                app.chat.add_message(ChatRole::System, "查询状态中...".to_string());
                return;
            }
            Some(CommandType::Builtin(BuiltinCommand::Reflect)) => {
                let msg = serde_json::json!({
                    "jsonrpc": "2.0", "method": "reflect", "id": 1
                });
                let payload = serde_json::to_string(&msg).unwrap_or_default();
                use tokio::io::AsyncWriteExt;
                let framed = format!("{}\n", payload);
                let _ = app.stream.write_all(framed.as_bytes()).await;
                let _ = app.stream.flush().await;
                app.chat.add_message(ChatRole::System, "查询反思记录中...".to_string());
                return;
            }
            Some(CommandType::Builtin(BuiltinCommand::ReflectNow)) => {
                let msg = serde_json::json!({
                    "jsonrpc": "2.0", "method": "reflect_now", "id": 1
                });
                let payload = serde_json::to_string(&msg).unwrap_or_default();
                use tokio::io::AsyncWriteExt;
                let framed = format!("{}\n", payload);
                let _ = app.stream.write_all(framed.as_bytes()).await;
                let _ = app.stream.flush().await;
                app.chat.add_message(ChatRole::System, "执行即时反思中...".to_string());
                return;
            }
            Some(CommandType::Builtin(BuiltinCommand::Evolution)) => {
                let msg = serde_json::json!({
                    "jsonrpc": "2.0", "method": "evolution", "id": 1
                });
                let payload = serde_json::to_string(&msg).unwrap_or_default();
                use tokio::io::AsyncWriteExt;
                let framed = format!("{}\n", payload);
                let _ = app.stream.write_all(framed.as_bytes()).await;
                let _ = app.stream.flush().await;
                app.chat.add_message(ChatRole::System, "查询演化历史中...".to_string());
                return;
            }
            Some(CommandType::Builtin(BuiltinCommand::Genome)) => {
                let msg = serde_json::json!({
                    "jsonrpc": "2.0", "method": "genome", "id": 1
                });
                let payload = serde_json::to_string(&msg).unwrap_or_default();
                use tokio::io::AsyncWriteExt;
                let framed = format!("{}\n", payload);
                let _ = app.stream.write_all(framed.as_bytes()).await;
                let _ = app.stream.flush().await;
                app.chat.add_message(ChatRole::System, "查询基因组中...".to_string());
                return;
            }
            Some(CommandType::Builtin(_)) => return,
            Some(CommandType::Skill { name, args }) => {
                app.chat.add_message(ChatRole::User, text.clone());
                let skill = match app.skill_loader.get(&name) {
                    Some(s) => s.clone(),
                    None => {
                        app.chat.add_message(ChatRole::System, format!("未知技能: /{}", name));
                        return;
                    }
                };
                let message = if args.is_empty() {
                    skill.content.clone()
                } else {
                    format!("{}\n\nUser input: {}", skill.content, args)
                };
                app.chat.add_message(ChatRole::Assistant, String::new());
                send_to_daemon(app, &message).await;
                return;
            }
            None => {
                app.chat.add_message(ChatRole::System, "无效命令".to_string());
                return;
            }
        }
    }

    // Regular chat message
    app.chat.add_message(ChatRole::User, text.clone());
    app.chat.add_message(ChatRole::Assistant, String::new());
    send_to_daemon(app, &text).await;
}

async fn send_to_daemon(app: &mut App, text: &str) {
    let msg = serde_json::json!({
        "jsonrpc": "2.0",
        "method": "chat",
        "id": 1,
        "params": { "message": text },
    });
    let payload = serde_json::to_string(&msg).unwrap_or_default();
    let framed = format!("{}\n", payload);

    use tokio::io::AsyncWriteExt;
    if app.stream.write_all(framed.as_bytes()).await.is_err() {
        app.chat.add_message(ChatRole::System, "发送失败，请检查 daemon".to_string());
        return;
    }
    let _ = app.stream.flush().await;
    app.streaming = true;
    app.response_buf.clear();
    app.status.waiting = true;
}

fn try_read_response(app: &mut App) {
    loop {
        match app.stream.try_read(&mut app.read_buf) {
            Ok(0) => {
                app.streaming = false;
                app.status.waiting = false;
                app.chat.add_message(ChatRole::System, "连接断开".to_string());
                break;
            }
            Ok(n) => {
                let chunk = String::from_utf8_lossy(&app.read_buf[..n]);
                app.response_buf.push_str(&chunk);
                if let Some(msg) = serde_json::from_str::<serde_json::Value>(app.response_buf.trim()).ok() {
                    process_response(app, msg);
                    break;
                }
            }
            Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => break,
            Err(_) => {
                app.streaming = false;
                app.status.waiting = false;
                break;
            }
        }
    }
}

fn process_response(app: &mut App, msg: serde_json::Value) {
    if let Some(result) = msg.get("result") {
        if let Some(text) = result.get("response").and_then(|v| v.as_str()) {
            // Standard chat response
            let display = format!("{}\n\n💡 /reflect to see reflections", text);
            app.chat.update_last_message(display);
        } else if let Some(entries) = result.get("reflections") {
            // /reflect response — format reflection entries
            let formatted = format_reflections(entries);
            app.chat.update_last_message(formatted);
        } else if let Some(genome) = result.get("genome") {
            // /genome response — format genome JSON
            let formatted = format_genome(genome);
            app.chat.update_last_message(formatted);
        } else if let Some(evo) = result.get("evolution") {
            // /evolution response — format evolution history
            let formatted = format_evolution(evo);
            app.chat.update_last_message(formatted);
        } else if let Some(status) = result.get("status") {
            // /status response — rich self-evolution state
            let formatted = format_status(status);
            app.chat.update_last_message(formatted);
        }
    } else if let Some(error) = msg.get("error") {
        let err = error.get("message").and_then(|v| v.as_str()).unwrap_or("Unknown error");
        app.chat.update_last_message(format!("Error: {}", err));
    }
    app.streaming = false;
    app.status.waiting = false;
    app.response_buf.clear();
}

/// Format reflection entries for display.
fn format_reflections(entries: &serde_json::Value) -> String {
    let empty = vec![];
    let arr = entries.as_array().unwrap_or(&empty);
    if arr.is_empty() {
        return "No reflections found.".to_string();
    }
    let mut lines = Vec::new();
    lines.push(format!("=== Reflections ({}) ===\n", arr.len()));
    for (i, entry) in arr.iter().enumerate() {
        let _trigger = entry.get("trigger").and_then(|v| {
            if let Some(s) = v.as_str() { Some(s.to_string()) }
            else { serde_json::to_string(v).ok() }
        }).unwrap_or_else(|| "unknown".to_string());
        let task = entry.get("task_summary").and_then(|v| v.as_str()).unwrap_or("");
        let outcome = entry.get("outcome").and_then(|v| {
            if let Some(s) = v.as_str() { Some(s.to_string()) }
            else { serde_json::to_string(v).ok() }
        }).unwrap_or_else(|| "unknown".to_string());
        let confidence = entry.get("confidence").and_then(|v| v.as_f64()).unwrap_or(0.0);
        let timestamp = entry.get("timestamp").and_then(|v| v.as_str()).unwrap_or("");

        lines.push(format!(
            "[{}] #{} {} ({}) conf={:.0}%",
            timestamp, i + 1, task, outcome, confidence * 100.0
        ));

        if let Some(arr) = entry.get("learned").and_then(|v| v.as_array()) {
            for l in arr {
                if let Some(s) = l.as_str() {
                    lines.push(format!("  learned: {}", s));
                }
            }
        }
        if let Some(arr) = entry.get("behavior_changes").and_then(|v| v.as_array()) {
            for c in arr {
                if let Some(s) = c.as_str() {
                    lines.push(format!("  changed: {}", s));
                }
            }
        }
        if let Some(arr) = entry.get("what_worked").and_then(|v| v.as_array()) {
            for w in arr {
                if let Some(s) = w.as_str() {
                    lines.push(format!("  worked: {}", s));
                }
            }
        }
        if let Some(arr) = entry.get("what_failed").and_then(|v| v.as_array()) {
            for f in arr {
                if let Some(s) = f.as_str() {
                    lines.push(format!("  failed: {}", s));
                }
            }
        }
        lines.push(String::new());
    }
    lines.join("\n")
}

/// Format genome for display.
fn format_genome(genome: &serde_json::Value) -> String {
    if let Some(s) = genome.as_str() {
        return s.to_string();
    }
    serde_json::to_string_pretty(genome).unwrap_or_else(|_| format!("{:?}", genome))
}

/// Format evolution history for display.
fn format_evolution(evo: &serde_json::Value) -> String {
    if let Some(s) = evo.as_str() {
        return s.to_string();
    }
    if let Some(arr) = evo.as_array() {
        if arr.is_empty() {
            return "No evolution history found.".to_string();
        }
        let mut lines = Vec::new();
        lines.push(format!("=== Evolution History ({}) ===\n", arr.len()));
        for entry in arr {
            lines.push(serde_json::to_string_pretty(entry).unwrap_or_else(|_| format!("{:?}", entry)));
            lines.push(String::new());
        }
        return lines.join("\n");
    }
    // Handle object form with version/message fields
    serde_json::to_string_pretty(evo).unwrap_or_else(|_| format!("{:?}", evo))
}

/// Format status response for display.
fn format_status(status: &serde_json::Value) -> String {
    let session_id = status.get("session_id").and_then(|v| v.as_str()).unwrap_or("unknown");
    let turn_count = status.get("turn_count").and_then(|v| v.as_u64()).unwrap_or(0);
    let reflection_count = status.get("reflection_count").and_then(|v| v.as_u64()).unwrap_or(0);
    let evolution_count = status.get("evolution_count").and_then(|v| v.as_u64()).unwrap_or(0);
    let boundary_rules = status.get("boundary_rules").and_then(|v| v.as_u64()).unwrap_or(0);
    let boundary_immutable = status.get("boundary_immutable").and_then(|v| v.as_u64()).unwrap_or(0);
    let attention_focus = status.get("attention_focus").and_then(|v| v.as_str()).unwrap_or("");

    let mut lines = Vec::new();
    lines.push("=== Aletheon Status ===".to_string());
    lines.push(format!("Session: {}", &session_id[..8.min(session_id.len())]));
    lines.push(format!("Turns: {}", turn_count));
    lines.push(format!("Reflections: {}", reflection_count));
    lines.push(format!("Evolutions: {}", evolution_count));
    lines.push(String::new());
    lines.push("Care Weights:".to_string());

    if let Some(cares) = status.get("care_weights").and_then(|v| v.as_array()) {
        for care in cares {
            let topic = care.get("topic").and_then(|v| v.as_str()).unwrap_or("?");
            let weight = care.get("weight").and_then(|v| v.as_f64()).unwrap_or(0.0);
            lines.push(format!("  {}: {:.2}", topic, weight));
        }
    }

    lines.push(String::new());
    lines.push(format!("Boundary Rules: {} (immutable: {})", boundary_rules, boundary_immutable));

    let focus_display = if attention_focus.is_empty() { "none" } else { attention_focus };
    lines.push(format!("Attention Focus: {}", focus_display));

    lines.join("\n")
}

fn draw(terminal: &mut Terminal<CrosstermBackend<Stdout>>, app: &mut App) -> anyhow::Result<()> {
    let chat_ref = &app.chat;
    let caps_ref = &app.caps;
    let model_name = &app.model_name;
    let input_buf = &app.input_buf;
    let cursor = app.cursor;
    let has_cjk = app.has_cjk;
    let first_render = app.first_render;
    let status_ref = &app.status;

    terminal.draw(|f| {
        let size = f.area();

        // Layout: header(2) | chat(min) | input(3) | status(1)
        let header_rows: u16 = if first_render { 3 } else { 1 };
        let input_rows: u16 = 3; // border + input + hint

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(header_rows),
                Constraint::Min(1),
                Constraint::Length(input_rows),
                Constraint::Length(1),
            ])
            .split(size);

        // ── Header ──
        render_header(f, chunks[0], caps_ref, model_name, first_render);

        // ── Chat area ──
        let chat_block = Block::default()
            .borders(Borders::NONE)
            .padding(Padding::horizontal(1));
        let chat_inner = chat_block.inner(chunks[1]);
        f.render_widget(chat_block, chunks[1]);
        f.render_widget(chat_ref.render_widget(), chat_inner);

        // ── Input area ──
        render_input(f, chunks[2], caps_ref, input_buf, cursor, has_cjk);

        // ── Status bar ──
        f.render_widget(status_ref.render_widget(), chunks[3]);
    })?;

    app.first_render = false;
    Ok(())
}

fn render_header(f: &mut ratatui::Frame, area: Rect, caps: &TermCaps, model_name: &str, show_full: bool) {
    let bg = caps.color(20, 20, 60);

    if show_full {
        let vsep = if caps.unicode { "  │  " } else { "  |  " };
        let line1 = Line::from(Span::styled(
            "  aletheon v0.1.0",
            Style::default().fg(Color::White),
        ));
        let line2 = Line::from(Span::styled(
            format!("  model: {model_name}{vsep}connected"),
            Style::default().fg(Color::DarkGray),
        ));
        let hints = if caps.unicode {
            "  Shift+Enter 换行 │ Enter 发送 │ Ctrl+C 退出 │ /help"
        } else {
            "  Shift+Enter newline | Enter send | Ctrl+C quit | /help"
        };
        let line3 = Line::from(Span::styled(hints, Style::default().fg(Color::DarkGray)));

        let header = Paragraph::new(vec![line1, line2, line3]).style(Style::default().bg(bg));
        f.render_widget(header, area);
    } else {
        let title = format!("  aletheon  │  {model_name}");
        let line = Line::from(Span::styled(title, Style::default().fg(Color::White)));
        let header = Paragraph::new(line).style(Style::default().bg(bg));
        f.render_widget(header, area);
    }
}

fn render_input(f: &mut ratatui::Frame, area: Rect, caps: &TermCaps, buf: &str, cursor: usize, has_cjk: bool) {
    let border_h = caps.hline();
    let prompt = if caps.unicode { "❯ " } else { "> " };

    // Row 0: separator line
    let sep = format!("  {}", border_h.repeat(area.width.saturating_sub(4) as usize));
    let sep_line = Line::from(Span::styled(sep, Style::default().fg(Color::DarkGray)));
    f.render_widget(Paragraph::new(sep_line), Rect { height: 1, ..area });

    // Row 1: input text with cursor
    let input_area = Rect { y: area.y + 1, height: 1, ..area };
    let mut spans = vec![
        Span::styled(prompt, Style::default().fg(Color::Green)),
    ];

    // Split buffer at cursor for cursor display
    let before = &buf[..cursor.min(buf.len())];
    let after = &buf[cursor.min(buf.len())..];

    if !before.is_empty() {
        spans.push(Span::styled(before, Style::default().fg(Color::White)));
    }

    // Cursor character (reverse video)
    let cursor_char = after.chars().next().map(|c| c.to_string()).unwrap_or_else(|| " ".to_string());
    spans.push(Span::styled(
        cursor_char,
        Style::default().fg(Color::Black).bg(Color::White).add_modifier(Modifier::BOLD),
    ));

    let rest = if after.chars().count() > 1 {
        &after[after.char_indices().nth(1).map(|(i, _)| i).unwrap_or(after.len())..]
    } else {
        ""
    };
    if !rest.is_empty() {
        spans.push(Span::styled(rest, Style::default().fg(Color::White)));
    }

    // CJK indicator
    if has_cjk {
        spans.push(Span::styled("  [CJK]", Style::default().fg(Color::DarkGray)));
    }

    let input_line = Paragraph::new(Line::from(spans));
    f.render_widget(input_line, input_area);

    // Row 2: hint line
    let hint_area = Rect { y: area.y + 2, height: 1, ..area };
    let hint = if has_cjk {
        "  Enter 发送(延迟) │ Shift+Enter 换行 │ Esc 清空"
    } else {
        "  Enter 发送 │ Shift+Enter 换行 │ Esc 清空"
    };
    let hint_line = Paragraph::new(Line::from(Span::styled(hint, Style::default().fg(Color::DarkGray))));
    f.render_widget(hint_line, hint_area);
}

/// Simple line-based mode for non-TTY (piped) input.
async fn simple_line_mode(mut stream: UnixStream, _caps: TermCaps, model_name: String) -> anyhow::Result<()> {
    use tokio::io::AsyncWriteExt;

    println!("aletheon v0.1.0 (model: {})", model_name);
    println!("Type your message and press Enter. /quit to exit.\n");

    let stdin = io::stdin();
    let mut read_buf = vec![0u8; 8192];

    loop {
        print!("> ");
        io::stdout().flush()?;

        let mut input = String::new();
        match stdin.read_line(&mut input) {
            Ok(0) => break,
            Ok(_) => {}
            Err(_) => break,
        }

        let trimmed = input.trim();
        if trimmed.is_empty() {
            continue;
        }
        if trimmed == "/quit" || trimmed == "/exit" {
            break;
        }

        // Determine JSON-RPC method from slash commands
        let msg = if trimmed.starts_with('/') {
            let cmd = trimmed.strip_prefix('/').unwrap_or(trimmed);
            let (name, _args) = match cmd.find(' ') {
                Some(i) => (&cmd[..i], cmd[i + 1..].trim()),
                None => (cmd, ""),
            };
            match name {
                "reflect" | "r" => serde_json::json!({
                    "jsonrpc": "2.0", "method": "reflect", "id": 1
                }),
                "reflect_now" | "rn" => serde_json::json!({
                    "jsonrpc": "2.0", "method": "reflect_now", "id": 1
                }),
                "evolution" | "evo" => serde_json::json!({
                    "jsonrpc": "2.0", "method": "evolution", "id": 1
                }),
                "genome" | "gene" => serde_json::json!({
                    "jsonrpc": "2.0", "method": "genome", "id": 1
                }),
                "clear" => serde_json::json!({
                    "jsonrpc": "2.0", "method": "clear", "id": 1
                }),
                "status" | "st" => serde_json::json!({
                    "jsonrpc": "2.0", "method": "status", "id": 1
                }),
                _ => serde_json::json!({
                    "jsonrpc": "2.0", "method": "chat", "id": 1,
                    "params": { "message": trimmed }
                }),
            }
        } else {
            serde_json::json!({
                "jsonrpc": "2.0", "method": "chat", "id": 1,
                "params": { "message": trimmed }
            })
        };
        let payload = serde_json::to_string(&msg)?;
        stream.write_all(format!("{}\n", payload).as_bytes()).await?;
        stream.flush().await?;

        // Wait for response
        loop {
            match stream.try_read(&mut read_buf) {
                Ok(0) => { println!("Connection lost"); return Ok(()); }
                Ok(n) => {
                    let chunk = String::from_utf8_lossy(&read_buf[..n]);
                    if let Ok(msg) = serde_json::from_str::<serde_json::Value>(chunk.trim()) {
                        if let Some(text) = msg["result"]["response"].as_str() {
                            println!("\n{}\n", text);
                        } else if !msg["result"]["reflections"].is_null() {
                            println!("\n{}\n", format_reflections(&msg["result"]["reflections"]));
                        } else if !msg["result"]["genome"].is_null() {
                            println!("\n{}\n", format_genome(&msg["result"]["genome"]));
                        } else if !msg["result"]["evolution"].is_null() {
                            println!("\n{}\n", format_evolution(&msg["result"]["evolution"]));
                        } else if !msg["result"]["status"].is_null() {
                            println!("\n{}\n", format_status(&msg["result"]["status"]));
                        } else if let Some(err) = msg["error"]["message"].as_str() {
                            eprintln!("Error: {}\n", err);
                        }
                        break;
                    }
                }
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
                Err(_) => break,
            }
        }
    }

    Ok(())
}

/// Simple base64 encoder (no external dependency).
fn base64_encode(input: &str) -> String {
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let bytes = input.as_bytes();
    let mut result = String::with_capacity((bytes.len() + 2) / 3 * 4);
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = chunk.get(1).map(|&b| b as u32).unwrap_or(0);
        let b2 = chunk.get(2).map(|&b| b as u32).unwrap_or(0);
        let triple = (b0 << 16) | (b1 << 8) | b2;
        result.push(CHARS[((triple >> 18) & 0x3F) as usize] as char);
        result.push(CHARS[((triple >> 12) & 0x3F) as usize] as char);
        if chunk.len() > 1 {
            result.push(CHARS[((triple >> 6) & 0x3F) as usize] as char);
        } else {
            result.push('=');
        }
        if chunk.len() > 2 {
            result.push(CHARS[(triple & 0x3F) as usize] as char);
        } else {
            result.push('=');
        }
    }
    result
}
