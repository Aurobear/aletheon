use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::super::agent_inspector::AgentInspectorAction;
use super::super::approval_dialog::{ApprovalDialog, DialogDecision};
use super::super::chat::Role as ChatRole;
use super::super::checkpoint_picker::CheckpointPickerAction;
use super::super::session_picker::SessionPickerAction;
use super::super::SystemNoticeQueue;
use super::super::TuiModel;
use super::submit::{
    request_agent_inspector, submit_message, typed_resume_session, typed_set_collaboration_mode,
    typed_transaction_review, typed_transaction_settlement, typed_workspace_rewind,
};

use application::turn_control::CollaborationMode;
use gateway::client::CommandOutcome as GatewayCommandOutcome;
use gateway::protocol::{CancelActiveTurn, Command, SessionRef, SubmitApprovalChoice};

pub(crate) fn refresh_command_completion(app: &mut TuiModel) {
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
    app: &mut TuiModel,
    action: ::contracts::TransactionReviewAction,
    risk_acknowledged: bool,
) {
    let Some(_session_id) = app.app_state.session_id.clone() else {
        app.system_notices
            .push(ChatRole::System, "当前会话尚未初始化".to_string());
        return;
    };
    let Some(_transaction_id) = app
        .latest_patch
        .as_ref()
        .and_then(|patch| patch.transaction_id)
        .map(|id| id.0.to_string())
    else {
        app.system_notices.push(
            ChatRole::System,
            "当前差异没有 Host change transaction，无法执行 review action".to_string(),
        );
        return;
    };
    if typed_transaction_review(app, action, risk_acknowledged).await {
        return;
    }
    app.app_state.last_error = Some(
        "typed Gateway is unavailable for transaction review; refusing legacy fallback".into(),
    );
}

async fn request_latest_transaction_settlement(app: &mut TuiModel) {
    let Some(_session_id) = app.app_state.session_id.clone() else {
        return;
    };
    let Some(_transaction_id) = app
        .latest_patch
        .as_ref()
        .and_then(|patch| patch.transaction_id)
        .map(|id| id.0.to_string())
    else {
        return;
    };
    if typed_transaction_settlement(app).await {
        return;
    }
    app.app_state.last_error = Some(
        "typed Gateway is unavailable for transaction settlement; refusing legacy fallback".into(),
    );
}

/// Insert a bracketed-paste payload as inert editor text. Newlines and CJK
/// codepoints are preserved, and paste never invokes submit by itself.
pub(crate) fn insert_paste(app: &mut TuiModel, text: &str) {
    let text = super::super::input_safety::sanitize_paste(text);
    app.input_buf.insert_str(app.cursor, &text);
    app.cursor += text.len();
    if app.input_buf.trim_start().starts_with(['/', '@', '!']) {
        app.input_literal = true;
    }
    app.check_cjk();
    app.completion.hide();
}

fn accept_selected_completion(app: &mut TuiModel) -> bool {
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

pub async fn handle_mouse(app: &mut TuiModel, mouse: crossterm::event::MouseEvent) {
    use crossterm::event::MouseEventKind;
    match mouse.kind {
        // Mouse wheel up: scroll the pager or the visible Task Console.
        MouseEventKind::ScrollUp => {
            if let Some(ref mut pager) = app.pager {
                pager.scroll_up(3);
            } else {
                app.app_state.conversation_scroll =
                    app.app_state.conversation_scroll.saturating_add(3);
            }
        }
        // Mouse wheel down: scroll the pager or the visible Task Console.
        MouseEventKind::ScrollDown => {
            if let Some(ref mut pager) = app.pager {
                pager.scroll_down(3);
            } else {
                app.app_state.conversation_scroll =
                    app.app_state.conversation_scroll.saturating_sub(3);
            }
        }
        _ => {}
    }
}

pub async fn handle_key(app: &mut TuiModel, key: KeyEvent) {
    if let Some(mut inspector) = app.agent_inspector.take() {
        match inspector.handle_key(key) {
            AgentInspectorAction::Continue => app.agent_inspector = Some(inspector),
            AgentInspectorAction::Close => {}
            AgentInspectorAction::Refresh => {
                let focus = inspector.focus_id();
                app.agent_inspector = Some(inspector);
                request_agent_inspector(app, focus).await;
            }
        }
        return;
    }
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
                if typed_resume_session(app, session_id.clone()).await {
                    return;
                }
                app.app_state.last_error = Some(
                    "typed Gateway is unavailable for session resume; refusing legacy fallback"
                        .into(),
                );
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
                    app.system_notices
                        .push(ChatRole::System, "当前会话尚未初始化".to_string());
                    return;
                };
                if typed_workspace_rewind(app, session_id.clone(), prompt_index, None).await {
                    return;
                }
                app.app_state.last_error = Some(
                    "typed Gateway is unavailable for workspace rewind; refusing legacy fallback"
                        .into(),
                );
            }
            action @ (CheckpointPickerAction::ForkSession { .. }
            | CheckpointPickerAction::ForkAndRewind { .. }) => {
                let Some(session_id) = app.app_state.session_id.clone() else {
                    app.system_notices
                        .push(ChatRole::System, "当前会话尚未初始化".to_string());
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
                if let Some(client) = app.controller.typed_gateway.as_mut() {
                    let outcome = client
                        .send(Command::ForkSession(
                            gateway::protocol::ForkSessionRequest {
                                session: SessionRef(session_id.clone()),
                                through_sequence,
                            },
                        ))
                        .await;
                    match outcome {
                        Ok(GatewayCommandOutcome::Forked { session }) => {
                            if let Some(prompt_index) = prompt_index {
                                app.deferred_checkpoint_rewind =
                                    Some(super::super::DeferredCheckpointRewind {
                                        parent_session_id: session_id,
                                        child_session_id: session.0,
                                        prompt_index,
                                    });
                            } else {
                                app.app_state.session_id = Some(session.0.clone());
                                app.projection_target_session_id = Some(session.0);
                                app.projection_session_id = None;
                                app.projection_request_in_flight = false;
                                app.projection_polling = false;
                            }
                            return;
                        }
                        Ok(other) => {
                            app.app_state.last_error =
                                Some(format!("Gateway fork receipt invalid: {other:?}"));
                            return;
                        }
                        Err(error) => {
                            app.app_state.last_error =
                                Some(format!("Gateway fork failed: {error}"));
                            return;
                        }
                    }
                }
                app.app_state.last_error = Some(
                    "typed Gateway is unavailable for session fork; refusing legacy fallback"
                        .into(),
                );
                app.system_notices.push(
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
        app.pager = Some(super::super::pager::PagerOverlay::from_state(
            &app.app_state,
            &app.caps,
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
                request_transaction_review(
                    app,
                    ::contracts::TransactionReviewAction::Accept,
                    false,
                )
                .await;
                return;
            }
            KeyCode::Char('p') => {
                app.review_risk_confirmation = None;
                request_transaction_review(
                    app,
                    ::contracts::TransactionReviewAction::Repair,
                    false,
                )
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
                    (Some(::contracts::change_transaction::MutationCoverage::Full), Some(_)) => {
                        request_transaction_review(
                            app,
                            ::contracts::TransactionReviewAction::Rollback,
                            false,
                        )
                        .await;
                    }
                    (
                        Some(::contracts::change_transaction::MutationCoverage::BestEffort),
                        Some(transaction_id),
                    ) if app.review_risk_confirmation.as_deref()
                        == Some(transaction_id.as_str()) =>
                    {
                        app.review_risk_confirmation = None;
                        request_transaction_review(
                            app,
                            ::contracts::TransactionReviewAction::Rollback,
                            true,
                        )
                        .await;
                    }
                    (
                        Some(::contracts::change_transaction::MutationCoverage::BestEffort),
                        Some(transaction_id),
                    ) => {
                        app.review_risk_confirmation = Some(transaction_id);
                        app.system_notices.push(
                            ChatRole::System,
                            "这是 best-effort rollback，可能残留外部副作用；再次按 x 显式确认风险"
                                .to_string(),
                        );
                    }
                    (
                        Some(::contracts::change_transaction::MutationCoverage::NonRollbackable),
                        _,
                    ) => {
                        app.system_notices.push(
                            ChatRole::System,
                            "Host 声明该事务不可回滚；未发送 rollback 请求".to_string(),
                        );
                    }
                    _ => app.system_notices.push(
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
                                ::contracts::protocol::client::TransientApprovalScopeHint {
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
                let approved = !matches!(decision, DialogDecision::Deny);
                if scope_hint.is_some() {
                    // The typed contract intentionally does not guess a path
                    // scope. Until the server exposes a typed scope field,
                    // fail closed rather than sending the legacy RPC.
                    app.app_state.last_error =
                        Some("path-scoped approval is not available on the typed Gateway".into());
                } else if app.controller.has_typed_gateway() && app.app_state.session_id.is_some() {
                    let outcome = app
                        .controller
                        .typed_gateway
                        .as_mut()
                        .expect("typed Gateway checked")
                        .send(Command::SubmitApproval(SubmitApprovalChoice {
                            session: SessionRef(
                                app.app_state
                                    .session_id
                                    .clone()
                                    .expect("typed session checked"),
                            ),
                            choice_id: dialog.approval_id.clone(),
                            approved,
                            version: 0,
                            reason: None,
                        }))
                        .await;
                    if let Err(error) = outcome {
                        app.system_notices.push(
                            ChatRole::System,
                            format!("Gateway approval failed: {error}"),
                        );
                    }
                } else {
                    app.app_state.last_error = Some(
                        "typed Gateway approval route is unavailable for this connection".into(),
                    );
                }
                app.system_notices.push(
                    ChatRole::System,
                    format!(
                        "Approval: {} ({})",
                        match decision {
                            DialogDecision::Approve => "approve",
                            DialogDecision::ApproveForSession => "approve-for-session",
                            DialogDecision::ApprovePathForSession => "approve-path-for-session",
                            DialogDecision::Deny => "deny",
                        },
                        dialog.action_summary
                    ),
                );
                return;
            }
        }
        // Any other key while dialog is open: ignore (except Ctrl+C to dismiss)
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            app.pending_approval = None;
            app.system_notices
                .push(ChatRole::System, "Approval cancelled (deny)".to_string());
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
            app.turn_cancel_requested = true;
            if let (Some(client), Some(session_id)) = (
                app.controller.typed_gateway.as_mut(),
                app.app_state.session_id.clone(),
            ) {
                let outcome = client
                    .send(Command::CancelActiveTurn(CancelActiveTurn {
                        session: SessionRef(session_id),
                    }))
                    .await;
                if !matches!(outcome, Ok(GatewayCommandOutcome::Cancelled)) {
                    app.system_notices.push(
                        ChatRole::System,
                        format!("Gateway cancellation failed: {outcome:?}"),
                    );
                }
            } else {
                app.app_state.last_error = Some(
                    "typed Gateway is unavailable for cancellation; refusing legacy fallback"
                        .into(),
                );
            }
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
        app.system_notices = SystemNoticeQueue::new(app.caps.clone());
        app.system_notice_cursor = 0;
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

    // Ctrl+G: open the live, read-only child Agent inspector. This remains
    // available while the parent turn is active.
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('g') {
        request_agent_inspector(app, None).await;
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
        if typed_set_collaboration_mode(app, next).await {
            app.system_notices.push(
                ChatRole::System,
                format!("Switching to {} mode", next.display_name()),
            );
            return;
        }
        app.app_state.last_error = Some(
            "typed Gateway is unavailable for collaboration mode; refusing legacy fallback".into(),
        );
        app.system_notices.push(
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
        if typed_set_collaboration_mode(app, target).await {
            app.system_notices.push(
                ChatRole::System,
                format!("Switching to {} mode", target.display_name()),
            );
            return;
        }
        app.app_state.last_error = Some(
            "typed Gateway is unavailable for collaboration mode; refusing legacy fallback".into(),
        );
        app.system_notices.push(
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
            // Accept completion if visible and it would change the input.  The
            // renderer refreshes discovery on every frame, so an exact command
            // remains visible after the first acceptance.  Treating that exact
            // match as another acceptance would make commands such as
            // `/context` impossible to submit.
            if app.completion.visible {
                let selection_changes_input = app
                    .completion
                    .selected()
                    .is_some_and(|selected| selected != app.input_buf);
                if selection_changes_input && accept_selected_completion(app) {
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
            app.pending_shell_confirmation = None;
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
            app.pending_shell_confirmation = None;
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
                app.pending_shell_confirmation = None;
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
                app.app_state.conversation_scroll =
                    app.app_state.conversation_scroll.saturating_add(5);
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
                app.app_state.conversation_scroll =
                    app.app_state.conversation_scroll.saturating_sub(5);
            }
        }

        // PageUp/PageDown: scroll the visible Task Console conversation.
        KeyCode::PageUp => {
            app.app_state.conversation_scroll = app.app_state.conversation_scroll.saturating_add(5);
        }
        KeyCode::PageDown => {
            app.app_state.conversation_scroll = app.app_state.conversation_scroll.saturating_sub(5);
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
            app.pending_shell_confirmation = None;
        }

        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::host_time::ClientClock;
    use crate::tui::term_compat::TermCaps;
    use crate::tui::TuiModel;
    use std::sync::Arc;

    async fn streaming_app() -> TuiModel {
        let workspace =
            ::contracts::WorkspacePolicy::from_resolved_roots("/tmp".into(), vec![]).unwrap();
        let mut app = TuiModel::new(
            TermCaps {
                color: true,
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

    async fn idle_app() -> TuiModel {
        let mut app = streaming_app().await;
        app.streaming = false;
        app
    }

    fn ctrl_c() -> KeyEvent {
        KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)
    }

    fn review_patch(
        coverage: ::contracts::change_transaction::MutationCoverage,
    ) -> ::contracts::PatchDelta {
        ::contracts::PatchDelta {
            transaction_id: Some(::contracts::change_transaction::ChangeTransactionId(
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
            ::contracts::change_transaction::MutationCoverage::BestEffort,
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
            .app_state
            .last_error
            .as_deref()
            .is_some_and(|error| error.contains("typed Gateway")));
    }

    #[tokio::test]
    async fn non_rollbackable_transaction_never_sends_a_review_action() {
        let mut app = idle_app().await;
        app.app_state.session_id = Some("session-a".into());
        app.latest_patch = Some(review_patch(
            ::contracts::change_transaction::MutationCoverage::NonRollbackable,
        ));
        app.detail = app
            .latest_patch
            .as_ref()
            .map(crate::tui::diff_view::DiffView::from_patch_delta);

        handle_key(&mut app, KeyEvent::from(KeyCode::Char('x'))).await;
        assert!(app.pending_commands.is_empty());
    }

    #[tokio::test]
    async fn u_tui_002_long_command_streams_incrementally_and_first_ctrl_c_fails_closed_without_typed_gateway(
    ) {
        let mut app = streaming_app().await;
        for (sequence, delta) in [(1, "first line\n"), (2, "second line\n")] {
            crate::tui::reducer::reduce(
                &mut app.app_state,
                crate::tui::reducer::UiAction::Item(::contracts::protocol::client::ItemEvent {
                    cursor: ::contracts::protocol::client::EventCursor {
                        sequence,
                        event_id: Some(format!("progress-{sequence}")),
                    },
                    item_id: "five-minute-command".into(),
                    phase: ::contracts::protocol::client::ItemPhase::Streaming,
                    delta: Some(delta.into()),
                    item: None,
                    error: None,
                }),
            );
        }
        assert_eq!(
            app.app_state.items["five-minute-command"].content,
            "first line\nsecond line\n"
        );

        handle_key(&mut app, ctrl_c()).await;

        assert!(app.running);
        assert!(app.last_ctrl_c.is_some());
        assert!(app
            .app_state
            .last_error
            .as_deref()
            .is_some_and(|error| error.contains("typed Gateway")));
    }

    #[tokio::test]
    async fn second_ctrl_c_exits_even_when_streaming_never_stops() {
        let mut app = streaming_app().await;

        handle_key(&mut app, ctrl_c()).await;
        handle_key(&mut app, ctrl_c()).await;

        assert!(!app.running);
    }

    #[tokio::test]
    async fn ctrl_g_requires_typed_gateway_for_agent_inspector() {
        let mut app = streaming_app().await;
        app.turn_active = true;

        handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('g'), KeyModifiers::CONTROL),
        )
        .await;

        assert!(app
            .app_state
            .last_error
            .as_deref()
            .is_some_and(|error| error.contains("typed Gateway")));
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
    async fn enter_submits_an_exact_slash_command_instead_of_reaccepting_it() {
        let mut app = idle_app().await;
        app.input_buf = "/context".to_string();
        app.cursor = app.input_buf.len();
        refresh_command_completion(&mut app);
        assert!(app.completion.visible);

        handle_key(&mut app, KeyEvent::from(KeyCode::Enter)).await;

        assert!(app.input_buf.is_empty());
        assert_eq!(app.cursor, 0);
        assert!(!app.completion.visible);
        assert!(app.pager.is_some());
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
