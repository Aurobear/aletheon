use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::super::approval_dialog::{ApprovalDialog, DialogDecision};
use super::super::chat::{ChatWidget, Role as ChatRole};
use super::super::App;
use super::submit::{submit_message, write_request};

use fabric::protocol::client::{ClientRpcRequest, TransientApprovalDecision};
use fabric::ui_event::CollaborationMode;

pub async fn handle_mouse(app: &mut App, mouse: crossterm::event::MouseEvent) {
    use crossterm::event::MouseEventKind;
    match mouse.kind {
        // Mouse wheel up: scroll up in pager, or scroll chat up
        MouseEventKind::ScrollUp => {
            if let Some(ref mut pager) = app.pager {
                pager.scroll_up(3);
            } else {
                app.chat.scroll_up(3);
            }
        }
        // Mouse wheel down: scroll down in pager, or scroll chat down
        MouseEventKind::ScrollDown => {
            if let Some(ref mut pager) = app.pager {
                pager.scroll_down(3);
            } else {
                app.chat.scroll_down(3);
            }
        }
        _ => {}
    }
}

pub async fn handle_key(app: &mut App, key: KeyEvent) {
    // If pager overlay is active, route key to pager
    if let Some(ref mut pager) = app.pager {
        if pager.handle_key(key) {
            app.pager = None; // close pager
        }
        return;
    }

    // Ctrl+T: open pager overlay
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('t') {
        app.pager = Some(super::super::pager::PagerOverlay::from_chat(
            &app.chat,
            "Transcript",
        ));
        return;
    }

    // If approval dialog is active, route key to dialog
    if app.pending_approval.is_some() {
        if let KeyCode::Char(c) = key.code {
            if let Some(decision) = ApprovalDialog::key_to_decision(c) {
                let dialog = app.pending_approval.take().unwrap();
                let decision = match decision {
                    DialogDecision::Approve => TransientApprovalDecision::Approve,
                    DialogDecision::ApproveForSession => {
                        TransientApprovalDecision::ApproveForSession
                    }
                    DialogDecision::Deny => TransientApprovalDecision::Deny,
                };
                let resp = ClientRpcRequest::approval_response(dialog.approval_id, decision)
                    .to_json_rpc(None)
                    .expect("typed approval response serializes");
                use tokio::io::AsyncWriteExt;
                let payload = serde_json::to_string(&resp).unwrap_or_default();
                let framed = format!("{}\n", payload);
                let _ = app.stream.write_all(framed.as_bytes()).await;
                let _ = app.stream.flush().await;
                app.chat.add_text(
                    ChatRole::System,
                    format!(
                        "Approval: {} ({})",
                        decision.as_str(),
                        dialog.action_summary
                    ),
                );
                return;
            }
        }
        // Any other key while dialog is open: ignore (except Ctrl+C to dismiss)
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            app.pending_approval = None;
            app.chat
                .add_text(ChatRole::System, "Approval cancelled (deny)".to_string());
        }
        return;
    }

    // Ctrl+C: cancel streaming / clear input / double-press quits
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
        // If streaming, send cancel to daemon
        if app.streaming {
            write_request(app, ClientRpcRequest::Cancel).await;
            return;
        }
        if app.input_buf.is_empty() {
            match app.last_ctrl_c {
                Some(t) if (app.clock.mono_now().0 - t.0) < 2000 => {
                    app.running = false;
                    return;
                }
                _ => {
                    app.last_ctrl_c = Some(app.clock.mono_now());
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
    if key.modifiers.contains(KeyModifiers::CONTROL)
        && key.code == KeyCode::Char('d')
        && app.input_buf.is_empty()
    {
        app.running = false;
        return;
    }

    // Ctrl+L: clear screen
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('l') {
        app.chat = ChatWidget::new(app.caps.clone());
        return;
    }

    // Ctrl+O: toggle thinking display
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('o') {
        app.stream_ctrl.toggle_thinking();
        return;
    }

    // Ctrl+B: toggle last tool card (find last ExecEntry in chat history)
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('b') {
        // Iterate entries in reverse, find the last ExecEntry and toggle it
        let call_id = {
            let mut found = None;
            for entry in app.chat.entries.iter().rev() {
                if let super::super::chat::ChatEntry::Exec(ref ee) = entry {
                    found = Some(ee.call_id.clone());
                    break;
                }
            }
            found
        };
        if let Some(cid) = call_id {
            app.chat.toggle_exec(&cid);
        }
        return;
    }

    // Ctrl+M: cycle collaboration mode
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('m') {
        let modes = [
            CollaborationMode::Default,
            CollaborationMode::Plan,
            CollaborationMode::Auto,
            CollaborationMode::Sandbox,
        ];
        let current = modes
            .iter()
            .position(|m| *m == app.app_state.mode)
            .unwrap_or(0);
        let next = modes[(current + 1) % modes.len()];
        write_request(app, ClientRpcRequest::mode_switch(next)).await;
        app.chat.add_text(
            ChatRole::System,
            format!("Switching to {} mode", next.display_name()),
        );
        return;
    }

    // Ctrl+P: toggle plan mode
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('p') {
        let target = if app.app_state.mode == CollaborationMode::Plan {
            CollaborationMode::Default
        } else {
            CollaborationMode::Plan
        };
        write_request(app, ClientRpcRequest::mode_switch(target)).await;
        app.chat.add_text(
            ChatRole::System,
            format!("Switching to {} mode", target.display_name()),
        );
        return;
    }

    // Ctrl+A: cursor to beginning of line
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('a') {
        let before = &app.input_buf[..app.cursor];
        if let Some(pos) = before.rfind('\n') {
            app.cursor = pos + 1;
        } else {
            app.cursor = 0;
        }
        return;
    }

    // Ctrl+E: cursor to end of line
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('e') {
        let after = &app.input_buf[app.cursor..];
        if let Some(pos) = after.find('\n') {
            app.cursor += pos;
        } else {
            app.cursor = app.input_buf.len();
        }
        return;
    }

    // Ctrl+W: delete word backward
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('w') {
        if app.cursor > 0 {
            // Skip trailing spaces
            let before = &app.input_buf[..app.cursor];
            let trimmed_end = before.trim_end().len();
            // Find start of word
            let trimmed = &before[..trimmed_end];
            let word_start = trimmed
                .rfind(|c: char| c.is_whitespace())
                .map(|p| p + 1)
                .unwrap_or(0);
            app.input_buf.drain(word_start..app.cursor);
            app.cursor = word_start;
            app.check_cjk();
        }
        return;
    }

    // Ctrl+K: delete from cursor to end of line
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('k') {
        let after = &app.input_buf[app.cursor..];
        let cut_len = after.find('\n').unwrap_or(after.len());
        app.input_buf.drain(app.cursor..app.cursor + cut_len);
        app.check_cjk();
        return;
    }

    // Ctrl+U: delete from cursor to beginning of line
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('u') {
        let before = &app.input_buf[..app.cursor];
        let cut_start = before.rfind('\n').map(|p| p + 1).unwrap_or(0);
        app.input_buf.drain(cut_start..app.cursor);
        app.cursor = cut_start;
        app.check_cjk();
        return;
    }

    match key.code {
        // Tab: trigger completion for slash commands
        KeyCode::Tab => {
            if app.input_buf.starts_with('/') {
                let commands: Vec<String> = vec![
                    "/help",
                    "/clear",
                    "/copy",
                    "/status",
                    "/reflect",
                    "/reflect_now",
                    "/evolution",
                    "/genome",
                    "/sessions",
                    "/resume",
                    "/compact",
                    "/model",
                    "/quit",
                    "/mode",
                    "/plan",
                    "/approve",
                    "/agents",
                    "/agent",
                    "/hooks",
                    "/skills",
                    "/skill",
                    "/interrupt",
                    "/context",
                ]
                .iter()
                .map(|s| s.to_string())
                .collect();
                app.completion.show(&app.input_buf, &commands);
            }
        }

        // Enter: submit (or accept completion, or Shift+Enter / Alt+Enter: newline)
        KeyCode::Enter => {
            // Accept completion if visible
            if app.completion.visible {
                if let Some(selected) = app.completion.selected() {
                    app.input_buf = selected.to_string();
                    app.cursor = app.input_buf.len();
                    app.completion.hide();
                }
                return;
            }

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
                app.pending_submit = Some(app.clock.mono_now());
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

        // Character input (skip control characters from Ctrl+letter)
        KeyCode::Char(c) => {
            if !key.modifiers.contains(KeyModifiers::CONTROL) {
                app.input_buf.insert(app.cursor, c);
                app.cursor += c.len_utf8();
                app.check_cjk();
            }
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

        // Up: completion prev, or history, or scroll chat
        KeyCode::Up => {
            if app.completion.visible {
                app.completion.prev();
            } else if let Some(entry) = app.history.up() {
                app.input_buf = entry.to_string();
                app.cursor = app.input_buf.len();
            } else {
                app.chat.scroll_up(5);
            }
        }
        // Down: completion next, or history, or scroll chat
        KeyCode::Down => {
            if app.completion.visible {
                app.completion.next();
            } else if let Some(entry) = app.history.down() {
                app.input_buf = entry.to_string();
                app.cursor = app.input_buf.len();
            } else {
                app.chat.scroll_down(5);
            }
        }

        // PageUp/PageDown: scroll chat
        KeyCode::PageUp => {
            app.chat.scroll_up(5);
        }
        KeyCode::PageDown => {
            app.chat.scroll_down(5);
        }

        // Escape: hide completion, or clear input
        KeyCode::Esc => {
            if app.completion.visible {
                app.completion.hide();
                return;
            }
            app.input_buf.clear();
            app.cursor = 0;
            app.has_cjk = false;
            app.pending_submit = None;
        }

        _ => {}
    }
}
