use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::super::approval_dialog::{ApprovalDialog, DialogDecision};
use super::super::chat::{ChatWidget, Role as ChatRole};
use super::super::checkpoint_picker::CheckpointPickerAction;
use super::super::session_picker::SessionPickerAction;
use super::super::App;
use super::submit::{submit_message, write_protocol_request, write_request};

use fabric::protocol::client::{
    ClientRequest, ClientRpcRequest, SnapshotRequest, TransientApprovalDecision,
};
use fabric::ui_event::CollaborationMode;

pub(crate) fn refresh_command_completion(app: &mut App) {
    if app.input_literal {
        app.completion.hide();
    } else if app.input_buf.starts_with('/') {
        app.completion
            .show_commands(&app.input_buf, &app.registry, app.turn_active);
    } else if app.input_buf.starts_with('@') && !app.input_buf.contains(char::is_whitespace) {
        app.completion
            .show_attachments(&app.input_buf, &app.workspace);
    } else {
        app.completion.hide();
    }
}

async fn request_transaction_review(
    app: &mut App,
    action: fabric::TransactionReviewAction,
    risk_acknowledged: bool,
) {
    let Some(session_id) = app.app_state.session_id.clone() else {
        app.chat
            .add_text(ChatRole::System, "当前会话尚未初始化".to_string());
        return;
    };
    let Some(transaction_id) = app
        .latest_patch
        .as_ref()
        .and_then(|patch| patch.transaction_id)
        .map(|id| id.0.to_string())
    else {
        app.chat.add_text(
            ChatRole::System,
            "当前差异没有 Host change transaction，无法执行 review action".to_string(),
        );
        return;
    };
    let request_id = write_request(
        app,
        ClientRpcRequest::TransactionReview(fabric::TransactionReviewParams {
            session_id,
            transaction_id,
            action,
            risk_acknowledged,
        }),
    )
    .await;
    app.pending_commands
        .insert(request_id, super::super::PendingCommand::TransactionReview);
    app.pending_non_turn.insert(request_id);
    app.streaming = true;
    app.status.waiting = true;
}

async fn request_latest_transaction_settlement(app: &mut App) {
    let Some(session_id) = app.app_state.session_id.clone() else {
        return;
    };
    let Some(transaction_id) = app
        .latest_patch
        .as_ref()
        .and_then(|patch| patch.transaction_id)
        .map(|id| id.0.to_string())
    else {
        return;
    };
    let request_id = write_request(
        app,
        ClientRpcRequest::TransactionSettlementGet(fabric::TransactionSettlementGetParams {
            session_id,
            transaction_id,
        }),
    )
    .await;
    app.pending_commands.insert(
        request_id,
        super::super::PendingCommand::TransactionSettlementLatest,
    );
    app.pending_non_turn.insert(request_id);
}

/// Insert a bracketed-paste payload as inert editor text. Newlines and CJK
/// codepoints are preserved, and paste never invokes submit by itself.
pub(crate) fn insert_paste(app: &mut App, text: &str) {
    let text = super::super::input_safety::sanitize_paste(text);
    app.input_buf.insert_str(app.cursor, &text);
    app.cursor += text.len();
    if app.input_buf.trim_start().starts_with(['/', '@', '!']) {
        app.input_literal = true;
    }
    app.check_cjk();
    app.completion.hide();
}

fn accept_selected_completion(app: &mut App) -> bool {
    let Some(selected) = app.completion.selected().map(ToOwned::to_owned) else {
        return false;
    };
    app.input_buf = selected;
    app.cursor = app.input_buf.len();
    app.input_literal = false;
    app.completion.hide();
    app.check_cjk();
    true
}

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
    if let Some(mut search) = app.history_search.take() {
        if search.handle_key(key) {
            if let Some(entry) = search.selected_entry() {
                app.input_buf = entry;
                app.cursor = app.input_buf.len();
                app.input_literal = false;
                app.check_cjk();
            }
        } else {
            app.history_search = Some(search);
        }
        return;
    }

    if let Some(mut picker) = app.session_picker.take() {
        match picker.handle_key(key) {
            SessionPickerAction::Continue => app.session_picker = Some(picker),
            SessionPickerAction::Close => {}
            SessionPickerAction::Resume(session_id) => {
                let request_id = write_protocol_request(
                    app,
                    ClientRequest::ReadSnapshot(SnapshotRequest {
                        session_id: fabric::SessionId(session_id.clone()),
                    }),
                )
                .await;
                app.pending_commands.insert(
                    request_id,
                    super::super::PendingCommand::ProjectionSnapshot {
                        session_id: session_id.clone(),
                    },
                );
                app.projection_target_session_id = Some(session_id.clone());
                app.projection_request_in_flight = true;
                app.chat
                    .add_text(ChatRole::System, format!("恢复会话 {session_id}..."));
            }
        }
        return;
    }

    if let Some(mut picker) = app.checkpoint_picker.take() {
        match picker.handle_key(key) {
            CheckpointPickerAction::Continue => app.checkpoint_picker = Some(picker),
            CheckpointPickerAction::Close => {}
            CheckpointPickerAction::RewindCode { prompt_index } => {
                let Some(session_id) = app.app_state.session_id.clone() else {
                    app.chat
                        .add_text(ChatRole::System, "当前会话尚未初始化".to_string());
                    return;
                };
                let request_id = write_request(
                    app,
                    ClientRpcRequest::WorkspaceRewind(
                        fabric::protocol::client::WorkspaceRewindParams {
                            session_id: fabric::SessionId(session_id),
                            prompt_index,
                        },
                    ),
                )
                .await;
                app.pending_commands.insert(
                    request_id,
                    super::super::PendingCommand::CheckpointRewind {
                        child_session_id: None,
                    },
                );
                app.pending_non_turn.insert(request_id);
                app.streaming = true;
                app.status.waiting = true;
                app.chat.add_text(
                    ChatRole::System,
                    format!("请求恢复工作区检查点 {prompt_index}…"),
                );
            }
            action @ (CheckpointPickerAction::ForkSession { .. }
            | CheckpointPickerAction::ForkAndRewind { .. }) => {
                let Some(session_id) = app.app_state.session_id.clone() else {
                    app.chat
                        .add_text(ChatRole::System, "当前会话尚未初始化".to_string());
                    return;
                };
                let (through_sequence, prompt_index) = match action {
                    CheckpointPickerAction::ForkSession { through_sequence } => {
                        (through_sequence, None)
                    }
                    CheckpointPickerAction::ForkAndRewind {
                        through_sequence,
                        prompt_index,
                    } => (through_sequence, Some(prompt_index)),
                    _ => unreachable!(),
                };
                let request_id = write_request(
                    app,
                    ClientRpcRequest::SessionFork(fabric::protocol::client::SessionForkParams {
                        session_id: fabric::SessionId(session_id.clone()),
                        through_sequence,
                    }),
                )
                .await;
                app.pending_commands.insert(
                    request_id,
                    super::super::PendingCommand::CheckpointFork {
                        parent_session_id: session_id,
                        prompt_index,
                    },
                );
                app.pending_non_turn.insert(request_id);
                app.streaming = true;
                app.status.waiting = true;
                app.chat.add_text(
                    ChatRole::System,
                    if prompt_index.is_some() {
                        "创建历史会话分支，成功后再恢复代码…".to_string()
                    } else {
                        "创建历史会话分支…".to_string()
                    },
                );
            }
        }
        return;
    }

    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('r') {
        app.history_search = Some(super::super::history_search::HistorySearchOverlay::new(
            app.history.entries().to_vec(),
        ));
        return;
    }

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

    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('d') {
        app.detail = if app.detail.is_some() {
            None
        } else {
            app.latest_patch
                .as_ref()
                .map(super::super::diff_view::DiffView::from_patch_delta)
                .or_else(|| {
                    app.latest_diff
                        .clone()
                        .map(super::super::diff_view::DiffView::new)
                })
        };
        if let Some(detail) = app.detail.as_mut() {
            detail.project_findings(
                app.app_state
                    .tasks
                    .iter()
                    .flat_map(|task| task.review_findings.clone())
                    .collect(),
            );
        }
        if app.detail.is_some() {
            request_latest_transaction_settlement(app).await;
        }
        return;
    }
    if app.detail.is_some() {
        match key.code {
            KeyCode::Char('a') => {
                app.review_risk_confirmation = None;
                request_transaction_review(app, fabric::TransactionReviewAction::Accept, false)
                    .await;
                return;
            }
            KeyCode::Char('p') => {
                app.review_risk_confirmation = None;
                request_transaction_review(app, fabric::TransactionReviewAction::Repair, false)
                    .await;
                return;
            }
            KeyCode::Char('x') => {
                let coverage = app
                    .latest_patch
                    .as_ref()
                    .and_then(|patch| patch.mutation_coverage);
                let transaction_id = app
                    .latest_patch
                    .as_ref()
                    .and_then(|patch| patch.transaction_id)
                    .map(|id| id.0.to_string());
                match (coverage, transaction_id) {
                    (Some(fabric::change_transaction::MutationCoverage::Full), Some(_)) => {
                        request_transaction_review(
                            app,
                            fabric::TransactionReviewAction::Rollback,
                            false,
                        )
                        .await;
                    }
                    (
                        Some(fabric::change_transaction::MutationCoverage::BestEffort),
                        Some(transaction_id),
                    ) if app.review_risk_confirmation.as_deref()
                        == Some(transaction_id.as_str()) =>
                    {
                        app.review_risk_confirmation = None;
                        request_transaction_review(
                            app,
                            fabric::TransactionReviewAction::Rollback,
                            true,
                        )
                        .await;
                    }
                    (
                        Some(fabric::change_transaction::MutationCoverage::BestEffort),
                        Some(transaction_id),
                    ) => {
                        app.review_risk_confirmation = Some(transaction_id);
                        app.chat.add_text(
                            ChatRole::System,
                            "这是 best-effort rollback，可能残留外部副作用；再次按 x 显式确认风险"
                                .to_string(),
                        );
                    }
                    (Some(fabric::change_transaction::MutationCoverage::NonRollbackable), _) => {
                        app.chat.add_text(
                            ChatRole::System,
                            "Host 声明该事务不可回滚；未发送 rollback 请求".to_string(),
                        );
                    }
                    _ => app.chat.add_text(
                        ChatRole::System,
                        "缺少 Host transaction/coverage，未发送 rollback 请求".to_string(),
                    ),
                }
                return;
            }
            _ => {}
        }
    }
    if let Some(detail) = app.detail.as_mut() {
        match key.code {
            KeyCode::Esc => {
                app.detail = None;
                return;
            }
            KeyCode::Char('j') | KeyCode::Down => {
                detail.select_next();
                detail.scroll_down();
                return;
            }
            KeyCode::Char('k') | KeyCode::Up => {
                detail.select_previous();
                detail.scroll_up();
                return;
            }
            KeyCode::Char('f') => {
                if let Some(diff) = app.latest_diff.clone() {
                    app.pager = Some(super::super::pager::PagerOverlay::new("Diff", diff));
                }
                return;
            }
            _ => {}
        }
    }

    // If approval dialog is active, route key to dialog
    if app.pending_approval.is_some() {
        if matches!(key.code, KeyCode::Char('j') | KeyCode::Down) {
            if let Some(dialog) = app.pending_approval.as_mut() {
                dialog.scroll = dialog.scroll.saturating_add(1);
            }
            return;
        }
        if matches!(key.code, KeyCode::Char('k') | KeyCode::Up) {
            if let Some(dialog) = app.pending_approval.as_mut() {
                dialog.scroll = dialog.scroll.saturating_sub(1);
            }
            return;
        }
        if let KeyCode::Char(c) = key.code {
            if let Some(decision) = ApprovalDialog::key_to_decision(c) {
                let dialog = app.pending_approval.take().unwrap();
                let scope_hint = match decision {
                    DialogDecision::ApprovePathForSession => {
                        dialog.scope_subject.as_ref().and_then(|subject| {
                            subject.path_candidates.first().cloned().map(|path_root| {
                                fabric::protocol::client::TransientApprovalScopeHint {
                                    path_root,
                                    subject_version: subject.subject_version,
                                    subject_sha256: subject.subject_sha256.clone(),
                                }
                            })
                        })
                    }
                    _ => None,
                };
                if decision == DialogDecision::ApprovePathForSession && scope_hint.is_none() {
                    app.pending_approval = Some(dialog);
                    return;
                }
                let decision = match decision {
                    DialogDecision::Approve => TransientApprovalDecision::Approve,
                    DialogDecision::ApproveForSession => {
                        TransientApprovalDecision::ApproveForSession
                    }
                    DialogDecision::Deny => TransientApprovalDecision::Deny,
                    DialogDecision::ApprovePathForSession => {
                        TransientApprovalDecision::ApprovePathForSession
                    }
                };
                let request = match scope_hint {
                    Some(hint) => {
                        ClientRpcRequest::scoped_approval_response(dialog.approval_id, hint)
                    }
                    None => ClientRpcRequest::approval_response(dialog.approval_id, decision),
                };
                let resp = request
                    .to_json_rpc(None)
                    .expect("typed approval response serializes");
                use tokio::io::AsyncWriteExt;
                let payload = serde_json::to_string(&resp).unwrap_or_default();
                let framed = format!("{payload}\n");
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
        // While streaming, the first press requests cancellation and the
        // second press remains an unconditional escape hatch.  Do not wait
        // for the daemon to acknowledge cancellation before allowing exit:
        // an unavailable provider or wedged turn may never send that event.
        if app.streaming {
            let now = app.clock.mono_now();
            if matches!(
                app.last_ctrl_c,
                Some(t) if now.0.saturating_sub(t.0) < 2000
            ) {
                app.running = false;
                return;
            }
            app.last_ctrl_c = Some(now);
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
            app.input_literal = false;
            app.pending_submit = None;
            app.completion.hide();
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

    // Alt+Up/Down: navigate the daemon-projected activity timeline.
    if key.modifiers.contains(KeyModifiers::ALT) && key.code == KeyCode::Up {
        let len = app.app_state.activities.len();
        app.selected_activity = if len == 0 {
            None
        } else {
            Some(app.selected_activity.unwrap_or(len).saturating_sub(1))
        };
        return;
    }
    if key.modifiers.contains(KeyModifiers::ALT) && key.code == KeyCode::Down {
        let len = app.app_state.activities.len();
        app.selected_activity = if len == 0 {
            None
        } else {
            Some(
                app.selected_activity
                    .map_or(0, |index| index.saturating_add(1).min(len - 1)),
            )
        };
        return;
    }

    // Ctrl+B: show/hide authoritative activity details, defaulting to the tail.
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('b') {
        app.selected_activity = if app.selected_activity.is_some() {
            None
        } else {
            app.app_state.activities.len().checked_sub(1)
        };
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
        // Tab: accept the selected slash-command completion.
        KeyCode::Tab => {
            if app.input_buf.starts_with('/') {
                app.completion
                    .show_commands(&app.input_buf, &app.registry, app.turn_active);
                accept_selected_completion(app);
            }
        }

        // Shift+Tab: move backward without accepting so users can inspect options.
        KeyCode::BackTab => {
            if app.input_buf.starts_with('/') {
                app.completion
                    .show_commands(&app.input_buf, &app.registry, app.turn_active);
                app.completion.prev();
            }
        }

        // Enter: submit (or accept completion, or Shift+Enter / Alt+Enter: newline)
        KeyCode::Enter => {
            // Accept completion if visible
            if app.completion.visible {
                if accept_selected_completion(app) {
                    return;
                }
                app.completion.hide();
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
                refresh_command_completion(app);
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
                refresh_command_completion(app);
            }
        }

        // Character input (skip control characters from Ctrl+letter)
        KeyCode::Char(c) => {
            if !key.modifiers.contains(KeyModifiers::CONTROL) {
                app.input_buf.insert(app.cursor, c);
                app.cursor += c.len_utf8();
                app.check_cjk();
                refresh_command_completion(app);
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
            app.input_literal = false;
            app.pending_submit = None;
        }

        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::host_time::ClientClock;
    use crate::tui::term_compat::TermCaps;
    use crate::tui::App;
    use std::sync::Arc;

    async fn streaming_app() -> App {
        let (stream, _peer) = tokio::net::UnixStream::pair().unwrap();
        let workspace =
            fabric::WorkspacePolicy::from_resolved_roots("/tmp".into(), vec![]).unwrap();
        let mut app = App::new(
            stream,
            TermCaps {
                true_color: false,
                unicode: false,
                width: 80,
                height: 24,
            },
            "test".into(),
            Arc::new(ClientClock::new()),
            workspace,
            Vec::new(),
        );
        app.streaming = true;
        app
    }

    async fn idle_app() -> App {
        let mut app = streaming_app().await;
        app.streaming = false;
        app
    }

    fn ctrl_c() -> KeyEvent {
        KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)
    }

    fn review_patch(coverage: fabric::change_transaction::MutationCoverage) -> fabric::PatchDelta {
        fabric::PatchDelta {
            transaction_id: Some(fabric::change_transaction::ChangeTransactionId(
                uuid::Uuid::nil(),
            )),
            mutation_coverage: Some(coverage),
            applied: vec![],
            failed: vec![],
            files_changed: vec![],
            diff_preview: Some("diff".into()),
            diff_artifact: None,
            diff_preview_truncated: false,
        }
    }

    #[tokio::test]
    async fn best_effort_rollback_requires_two_explicit_keypresses() {
        let mut app = idle_app().await;
        app.app_state.session_id = Some("session-a".into());
        app.latest_patch = Some(review_patch(
            fabric::change_transaction::MutationCoverage::BestEffort,
        ));
        app.detail = app
            .latest_patch
            .as_ref()
            .map(crate::tui::diff_view::DiffView::from_patch_delta);

        handle_key(&mut app, KeyEvent::from(KeyCode::Char('x'))).await;
        assert!(app.review_risk_confirmation.is_some());
        assert!(app.pending_commands.is_empty());

        handle_key(&mut app, KeyEvent::from(KeyCode::Char('x'))).await;
        assert!(app.review_risk_confirmation.is_none());
        assert!(app
            .pending_commands
            .values()
            .any(|pending| *pending == crate::tui::PendingCommand::TransactionReview));
    }

    #[tokio::test]
    async fn non_rollbackable_transaction_never_sends_a_review_action() {
        let mut app = idle_app().await;
        app.app_state.session_id = Some("session-a".into());
        app.latest_patch = Some(review_patch(
            fabric::change_transaction::MutationCoverage::NonRollbackable,
        ));
        app.detail = app
            .latest_patch
            .as_ref()
            .map(crate::tui::diff_view::DiffView::from_patch_delta);

        handle_key(&mut app, KeyEvent::from(KeyCode::Char('x'))).await;
        assert!(app.pending_commands.is_empty());
    }

    #[tokio::test]
    async fn first_ctrl_c_requests_cancel_without_exiting_streaming_turn() {
        let mut app = streaming_app().await;

        handle_key(&mut app, ctrl_c()).await;

        assert!(app.running);
        assert!(app.last_ctrl_c.is_some());
    }

    #[tokio::test]
    async fn second_ctrl_c_exits_even_when_streaming_never_stops() {
        let mut app = streaming_app().await;

        handle_key(&mut app, ctrl_c()).await;
        handle_key(&mut app, ctrl_c()).await;

        assert!(!app.running);
    }

    #[tokio::test]
    async fn tab_accepts_selected_slash_command_completion() {
        let mut app = idle_app().await;
        app.input_buf = "/mem".to_string();
        app.cursor = app.input_buf.len();
        refresh_command_completion(&mut app);

        handle_key(&mut app, KeyEvent::from(KeyCode::Tab)).await;

        assert_eq!(app.input_buf, "/memory");
        assert_eq!(app.cursor, app.input_buf.len());
        assert!(!app.completion.visible);
    }

    #[tokio::test]
    async fn u_tui_005_cjk_multiline_paste_is_inert_and_submits_once_after_ime_delay() {
        let mut app = idle_app().await;
        insert_paste(&mut app, "第一行\n第二行");

        assert_eq!(app.input_buf, "第一行\n第二行");
        assert_eq!(app.cursor, app.input_buf.len());
        assert!(app.has_cjk);
        assert!(app.pending_submit.is_none());

        handle_key(&mut app, KeyEvent::from(KeyCode::Enter)).await;
        assert!(app.pending_submit.is_some());
        assert_eq!(app.input_buf, "第一行\n第二行");
    }

    #[tokio::test]
    async fn u_input_005_pasted_action_prefixes_remain_literal() {
        for value in ["/clear", "@secret", "!rm -rf workspace"] {
            let mut app = idle_app().await;
            insert_paste(&mut app, value);
            assert!(app.input_literal);
            assert!(!app.completion.visible);
        }
    }
}
