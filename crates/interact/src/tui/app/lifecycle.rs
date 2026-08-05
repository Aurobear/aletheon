use std::io;
use std::io::Write;
use std::sync::Arc;
use std::time::Duration;

use crossterm::event::Event;
use fabric::protocol::client::{ClientRpcRequest, TransientApprovalDecision};
use fabric::Clock;
use ratatui::Terminal;
use tokio::net::UnixStream;

use crate::tui::host_time::ClientTimer;
use fabric::Timer;

use super::super::response::{
    format_models, format_sessions, format_status, try_read_socket_with_recorder,
};
use super::super::term_compat::TermCaps;
use super::super::test_infra::{EventRecorder, FrameRecorder, TestConfig, TestInputReader};
use super::super::App;
use super::super::{
    command::{looks_like_command, BuiltinCommand, CommandType},
    registry::CommandRegistry,
};
use super::key_handler::{handle_key, handle_mouse};
use super::submit::submit_message;

fn initial_session_request(
    initial_session: crate::host::InitialSession,
) -> (ClientRpcRequest, super::super::PendingCommand) {
    match initial_session {
        crate::host::InitialSession::New => (
            ClientRpcRequest::SessionNew,
            super::super::PendingCommand::InitializeSession,
        ),
        crate::host::InitialSession::Resume(session_id) => (
            ClientRpcRequest::resume(session_id.0),
            super::super::PendingCommand::Resume {
                previous_session_id: None,
            },
        ),
        crate::host::InitialSession::Pick => (
            ClientRpcRequest::Sessions,
            super::super::PendingCommand::OpenSessionPicker,
        ),
    }
}

pub async fn run_app<B: ratatui::backend::Backend>(
    terminal: &mut Terminal<B>,
    stream: UnixStream,
    caps: TermCaps,
    model_name: String,
    test_config: TestConfig,
    is_test_mode: bool,
    clock: Arc<dyn Clock>,
    workspace: fabric::WorkspacePolicy,
    turn_requirements: Vec<fabric::TurnRequirement>,
    task_kind: Option<fabric::TaskKind>,
    initial_session: crate::host::InitialSession,
) -> anyhow::Result<()> {
    let mut app = App::new(
        stream,
        caps,
        model_name.clone(),
        clock,
        workspace,
        turn_requirements,
    );
    app.requested_task_kind = task_kind;

    // ── Test infrastructure setup ──
    let mut frame_recorder: Option<FrameRecorder> = test_config
        .record_frames
        .as_ref()
        .and_then(|p| FrameRecorder::new(p).ok());

    let mut event_recorder: Option<EventRecorder> = test_config
        .record_events
        .as_ref()
        .and_then(|p| EventRecorder::new(p).ok());

    let mut test_input: Option<TestInputReader> = test_config
        .test_input
        .as_ref()
        .and_then(|p| TestInputReader::new(p, test_config.auto_submit).ok());

    let test_start = app.clock.mono_now();
    let test_timeout = Duration::from_secs(test_config.test_timeout);
    let mut needs_redraw = true;

    // Populate completion/help from the daemon-owned Skill catalog. The
    // registry retains its last valid catalog if a later refresh fails.
    // The top-level CLI chooses whether this terminal owns a fresh session,
    // resumes an explicit session, or opens the daemon-backed history picker.
    let (initial_request, initial_pending) = initial_session_request(initial_session);
    let init_id = super::submit::write_request(&mut app, initial_request).await;
    app.pending_commands.insert(init_id, initial_pending);
    let skills_id = super::submit::write_request(&mut app, ClientRpcRequest::SkillsList).await;
    app.pending_commands
        .insert(skills_id, super::super::PendingCommand::InitializeSkills);

    // If test mode with auto_submit, submit the first line immediately
    if let Some(ref mut reader) = test_input {
        if reader.auto_submit {
            if let Some(line) = reader.next_line() {
                submit_message(&mut app, line).await;
            }
        }
    }

    while app.running {
        // Test timeout check
        if test_input.is_some()
            && (app.clock.mono_now().0 - test_start.0) >= test_timeout.as_millis() as u64
        {
            app.running = false;
            break;
        }

        // Resize handling
        if let Ok(size) = terminal.size() {
            app.chat.set_width(size.width);
        }

        // Redraw only when state changed. This keeps idle and scroll handling
        // cheap instead of rebuilding the complete terminal frame on a timer.
        if needs_redraw {
            super::super::render::draw::draw_with_recorder(
                terminal,
                &mut app,
                &mut frame_recorder,
            )?;
            needs_redraw = false;
        }

        // Check pending submit (IME delay)
        if let Some(pending_time) = app.pending_submit {
            if (app.clock.mono_now().0 - pending_time.0) > 100 {
                app.pending_submit = None;
                needs_redraw = true;
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
        // Skip event polling in test mode (no terminal to poll)
        if !is_test_mode {
            let poll_timeout = if app.streaming || app.pending_submit.is_some() {
                Duration::from_millis(50)
            } else {
                Duration::from_millis(200)
            };

            if crossterm::event::poll(poll_timeout)? {
                match crossterm::event::read()? {
                    Event::Key(key) => {
                        handle_key(&mut app, key).await;
                        needs_redraw = true;
                    }
                    Event::Paste(text) => {
                        // Paste: insert at cursor
                        for ch in text.chars() {
                            app.input_buf.insert(app.cursor, ch);
                            app.cursor += ch.len_utf8();
                        }
                        app.check_cjk();
                        super::key_handler::refresh_command_completion(&mut app);
                        needs_redraw = true;
                    }
                    Event::Resize(w, _h) => {
                        app.chat.set_width(w);
                        needs_redraw = true;
                    }
                    Event::Mouse(mouse) => {
                        handle_mouse(&mut app, mouse).await;
                        needs_redraw = true;
                    }
                    _ => {}
                }
            }
        } else {
            // In test mode, wait for socket to be readable (with timeout)
            // This properly registers with the tokio reactor so we wake up
            // when the daemon sends data, instead of busy-polling with try_read.
            tokio::select! {
                result = app.stream.readable() => {
                    if result.is_err() {
                        app.running = false;
                        break;
                    }
                }
                _ = ClientTimer.sleep(Duration::from_millis(200)) => {}
            }
        }

        // Try reading daemon response (with optional event recording)
        needs_redraw |= try_read_socket_with_recorder(&mut app, &mut event_recorder);

        // Check if a turn just completed and we should auto-submit next line
        if let Some(ref mut reader) = test_input {
            // Use turn_active (set by turn_start, cleared by turn_done) instead
            // of streaming (which is also cleared by process_response and would
            // trigger premature auto-submit before the turn actually completes).
            if !app.turn_active {
                if let Some(next) = reader.on_turn_done() {
                    // Small delay to let the UI update before next turn
                    ClientTimer.sleep(Duration::from_millis(100)).await;
                    submit_message(&mut app, next).await;
                    needs_redraw = true;
                }
            }
            // All inputs consumed and last turn done
            if reader.done && !app.turn_active {
                app.running = false;
            }
        }

        if app.streaming {
            app.status.tick_spinner();
            needs_redraw = true;
        }
    }

    Ok(())
}

/// Simple line-based mode for non-TTY (piped) input.
pub async fn simple_line_mode(
    mut stream: UnixStream,
    _caps: TermCaps,
    model_name: String,
    _clock: Arc<dyn Clock>,
    workspace: fabric::WorkspacePolicy,
    turn_requirements: Vec<fabric::TurnRequirement>,
    task_kind: Option<fabric::TaskKind>,
) -> anyhow::Result<()> {
    use tokio::io::AsyncWriteExt;

    println!("aletheon v0.1.0 (model: {model_name})");
    println!("Type your message and press Enter. /quit to exit.\n");

    let stdin = io::stdin();
    let mut read_buf = vec![0u8; 8192];
    let mut response_buf = super::super::json_lines::JsonLineBuffer::default();
    let registry = CommandRegistry::new();

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
        // Resolve slash syntax through the same catalog as the full TUI. The
        // line adapter only projects typed commands it can render safely; it
        // never forwards unknown command text to the model as a chat prompt.
        let request = if looks_like_command(trimmed) {
            match registry.parse(trimmed) {
                Some(CommandType::Builtin(BuiltinCommand::Quit)) => break,
                Some(CommandType::Builtin(BuiltinCommand::Help)) => {
                    println!("{}", registry.help_text());
                    continue;
                }
                Some(CommandType::Builtin(BuiltinCommand::Clear)) => ClientRpcRequest::Clear,
                Some(CommandType::Builtin(BuiltinCommand::Status)) => {
                    crate::intent::rpc(crate::intent::status(
                        fabric::contract::command::ClientSurface::Tui,
                        format!("line-status:{}", uuid::Uuid::new_v4()),
                        None,
                    ))
                }
                Some(CommandType::Builtin(BuiltinCommand::Sessions)) => ClientRpcRequest::Sessions,
                Some(CommandType::Builtin(BuiltinCommand::Resume { id })) if !id.is_empty() => {
                    ClientRpcRequest::resume(id)
                }
                Some(CommandType::Builtin(BuiltinCommand::Compact)) => ClientRpcRequest::Compact,
                Some(CommandType::Builtin(BuiltinCommand::Model)) => ClientRpcRequest::ModelList,
                Some(CommandType::Skill { name, args }) => {
                    ClientRpcRequest::skill_invoke(name, args, &workspace)
                }
                Some(CommandType::Unknown {
                    name, suggestions, ..
                }) => {
                    if suggestions.is_empty() {
                        println!("Unknown command: /{name}");
                    } else {
                        println!(
                            "Unknown command: /{name}. Did you mean {}?",
                            suggestions.join(", ")
                        );
                    }
                    continue;
                }
                Some(CommandType::Builtin(_)) | None => {
                    println!("Command is unavailable in line mode: {trimmed}");
                    continue;
                }
            }
        } else {
            crate::intent::rpc(crate::intent::submit_prompt(crate::intent::PromptIntent {
                surface: fabric::contract::command::ClientSurface::Tui,
                correlation_id: format!("line:{}", uuid::Uuid::new_v4()),
                content: trimmed,
                session_id: None,
                workspace: &workspace,
                requirements: turn_requirements.clone(),
                task_kind,
                permission_mode: crate::host::permission_mode_from_environment(),
            }))
        };
        let msg = request.to_json_rpc(Some(1))?;
        let payload = serde_json::to_string(&msg)?;
        stream.write_all(format!("{payload}\n").as_bytes()).await?;
        stream.flush().await?;

        // Wait for response — drain out-of-band notifications until we get
        // the actual JSON-RPC response (identified by having "id" + "result"/"error").
        // Use Timer::timeout for clean timeout handling.
        let timeout_duration = Duration::from_secs(120);

        let result = ClientTimer.timeout(timeout_duration, async {
            loop {
                // Wait for stream to be readable
                match stream.readable().await {
                    Ok(()) => {}
                    Err(e) => {
                        eprintln!("Error: {e}");
                        return Ok::<(), anyhow::Error>(());
                    }
                }

                match stream.try_read(&mut read_buf) {
                    Ok(0) => {
                        println!("Connection lost");
                        return Ok(());
                    }
                    Ok(n) => {
                        response_buf.push(&read_buf[..n]);
                        loop {
                            let line = match response_buf.take_line() {
                                Ok(Some(line)) => line,
                                Ok(None) => break,
                                Err(error) => {
                                    eprintln!(
                                        "Error: daemon protocol contained invalid UTF-8: {error}"
                                    );
                                    return Ok(());
                                }
                            };
                            if let Ok(msg) =
                                serde_json::from_str::<serde_json::Value>(line.trim())
                            {
                            // Handle out-of-band approval_request notification
                            if msg.get("method").and_then(|v| v.as_str()) == Some("approval_request")
                                && msg.get("result").is_none()
                                && msg.get("id").is_none()
                            {
                                let params = &msg["params"];
                                let tool = params["tool"].as_str().unwrap_or("?");
                                let action_summary = params["action_summary"].as_str().unwrap_or("");
                                let risk_level = params["risk_level"].as_str().unwrap_or("");
                                let approval_id = params["approval_id"].as_str().unwrap_or("");
                                println!(
                                    "\n⚠  Approval required [{risk_level}] {tool}\n   {action_summary}\n   Approve? [y]es / [a]lways / [N]o: ",
                                );
                                io::stdout().flush()?;
                                let mut line = String::new();
                                let decision = match stdin.read_line(&mut line) {
                                    Ok(0) | Err(_) => TransientApprovalDecision::Deny,
                                    Ok(_) => match line.trim().to_lowercase().as_str() {
                                        "y" | "yes" => TransientApprovalDecision::Approve,
                                        "a" | "always" => {
                                            TransientApprovalDecision::ApproveForSession
                                        }
                                        _ => TransientApprovalDecision::Deny,
                                    },
                                };
                                let resp = ClientRpcRequest::approval_response(
                                    approval_id,
                                    decision,
                                )
                                .to_json_rpc(None)?;
                                let payload = serde_json::to_string(&resp)?;
                                stream
                                    .write_all(format!("{payload}\n").as_bytes())
                                    .await?;
                                stream.flush().await?;
                                continue; // go back to wait for the actual response
                            }

                            // Skip out-of-band notifications (method: "event", etc.)
                            // These are streaming events from the ReAct loop — not the
                            // final JSON-RPC response.  A real response has "id" and
                            // either "result" or "error".
                            let is_notification = msg.get("method").is_some()
                                && msg.get("id").is_none_or(|v| v.is_null());
                            if is_notification {
                                if let Some(receipt) = evaluation_receipt_from_protocol_message(&msg)
                                {
                                    println!(
                                        "{}",
                                        super::super::reducer::format_evaluation_receipt_ref(
                                            &receipt
                                        )
                                    );
                                }
                                // Print streaming events that carry text content
                                if let Some(event_type) = msg.pointer("/params/type").and_then(|v| v.as_str()) {
                                    match event_type {
                                        "text" | "text_delta" => {
                                            // Skip text_delta in simple_line_mode to avoid
                                            // duplicate output (final response has full text)
                                        }
                                        "tool_call_start" => {
                                            if let Some(name) = msg.pointer("/params/tool").and_then(|v| v.as_str()) {
                                                eprintln!("\n🔧 [{name}]");
                                            }
                                        }
                                        "tool_result" => {
                                            // Optionally show tool results inline
                                        }
                                        "error" => {
                                            if let Some(err) = msg.pointer("/params/message").and_then(|v| v.as_str()) {
                                                eprintln!("\n❌ {err}");
                                            }
                                        }
                                        _ => {} // silently skip other event types
                                    }
                                }
                                io::stdout().flush()?;
                                continue; // keep waiting for the actual response
                            }

                            // This is the actual JSON-RPC response — process it
                            if let Some(text) = msg["result"]["response"].as_str() {
                                println!("\n{text}\n");
                            } else if !msg["result"]["status"].is_null() {
                                println!("\n{}\n", format_status(&msg["result"]["status"]));
                            } else if !msg["result"]["sessions"].is_null() {
                                println!("\n{}\n", format_sessions(&msg["result"]["sessions"]));
                            } else if !msg["result"]["models"].is_null() {
                                println!("\n{}\n", format_models(&msg["result"]));
                            } else if let Some(msg_text) = msg["result"]["message"].as_str() {
                                println!("\n{msg_text}\n");
                            } else if let Some(err) = msg["error"]["message"].as_str() {
                                eprintln!("Error: {err}\n");
                            }
                                return Ok(());
                            }
                        }
                    }
                    Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                        ClientTimer.sleep(Duration::from_millis(50)).await;
                    }
                    Err(_) => return Ok(()),
                }
            }
        }).await;

        match result {
            Ok(inner) => inner?,
            Err(_) => {
                eprintln!("\n⏰ Timeout: no response after 120s");
            }
        }
    }

    Ok(())
}

fn evaluation_receipt_from_protocol_message(
    message: &serde_json::Value,
) -> Option<fabric::EvaluationReceiptRef> {
    use fabric::protocol::client::{ClientEvent as ProtocolEvent, ClientMessage, ItemPhase};

    let candidate = message
        .get("params")
        .or_else(|| message.get("result"))
        .unwrap_or(message);
    let message = serde_json::from_value::<ClientMessage<ProtocolEvent>>(candidate.clone()).ok()?;
    match message.into_v1().ok()? {
        ProtocolEvent::Item(item) if item.phase == ItemPhase::Completed => {
            item.item.and_then(|record| match record.payload {
                fabric::ItemPayload::EvaluationReceiptRef { receipt } => Some(receipt),
                _ => None,
            })
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initial_session_selection_uses_typed_session_requests() {
        let cases = [
            (crate::host::InitialSession::New, "session.new"),
            (
                crate::host::InitialSession::Resume(fabric::SessionId("session-7".into())),
                "resume",
            ),
            (crate::host::InitialSession::Pick, "sessions"),
        ];
        for (selection, method) in cases {
            let (request, _) = initial_session_request(selection);
            assert_eq!(request.to_json_rpc(Some(1)).unwrap()["method"], method);
        }
    }
}
