use std::io;
use std::io::Write;

use fabric::contract::command::ClientSurface;
use fabric::protocol::client::{ClientRequest, ClientRpcRequest, SnapshotRequest};
use fabric::ui_event::CollaborationMode;
use fabric::ui_event::InterruptReason;
use tokio::io::AsyncWriteExt;

use super::super::chat::Role as ChatRole;
use super::super::command::{looks_like_command, BuiltinCommand, CommandType};
use super::super::App;

pub(super) async fn write_request(app: &mut App, request: ClientRpcRequest) -> u64 {
    let request_id = app.next_request_id;
    app.next_request_id = app.next_request_id.saturating_add(1);
    let request = request
        .to_json_rpc(Some(request_id))
        .expect("typed client request serializes");
    let payload = serde_json::to_string(&request).unwrap_or_default();
    let framed = format!("{payload}\n");
    let _ = app.stream.write_all(framed.as_bytes()).await;
    let _ = app.stream.flush().await;
    request_id
}

pub(super) async fn write_protocol_request(
    app: &mut App,
    request: fabric::protocol::client::ClientRequest,
) -> u64 {
    let request_id = app.next_request_id;
    app.next_request_id = app.next_request_id.saturating_add(1);
    let request = request
        .to_json_rpc(request_id)
        .expect("typed Session projection request serializes");
    let payload = serde_json::to_string(&request).unwrap_or_default();
    let framed = format!("{payload}\n");
    let _ = app.stream.write_all(framed.as_bytes()).await;
    let _ = app.stream.flush().await;
    request_id
}

/// Send a typed protocol request whose response is handled by the streaming
/// response path.
async fn send_request(app: &mut App, request: ClientRpcRequest) {
    let expects_turn = matches!(request, ClientRpcRequest::SkillInvoke(_));
    let request_id = write_request(app, request).await;
    if !expects_turn {
        app.pending_non_turn.insert(request_id);
    }
    app.streaming = true;
    app.response_buf.clear();
    app.status.waiting = true;
}

pub async fn submit_message(app: &mut App, text: String) {
    let literal_input = std::mem::take(&mut app.input_literal);
    if !literal_input && text.starts_with('!') {
        let command = text.trim_start_matches('!').trim().to_owned();
        if command.is_empty() {
            app.input_buf = text;
            app.cursor = app.input_buf.len();
            app.app_state.last_error = Some("shell command cannot be empty".into());
            return;
        }
        if app.turn_active {
            app.input_buf = format!("!{command}");
            app.cursor = app.input_buf.len();
            app.app_state.last_error =
                Some("shell command is unavailable while another turn is active".into());
            return;
        }
        if app.pending_shell_confirmation.as_deref() != Some(command.as_str()) {
            app.pending_shell_confirmation = Some(command.clone());
            app.input_buf = format!("!{command}");
            app.cursor = app.input_buf.len();
            app.compat_transcript.add_text(
                ChatRole::System,
                format!(
                    "Shell confirmation\nWorkspace: {}\nPermission: {:?}\nTransaction coverage: non-rollbackable\nHost policy and approval still apply. Press Enter again to submit or Esc to cancel.",
                    app.workspace.cwd().display(),
                    crate::host::permission_mode_from_environment(),
                ),
            );
            return;
        }
        app.pending_shell_confirmation = None;
        app.history.push(format!("!{command}"));
        app.persist_input_state();
        app.compat_transcript
            .add_text(ChatRole::User, format!("!{command}"));
        send_shell_to_daemon(app, &command).await;
        return;
    }
    app.pending_shell_confirmation = None;
    // Check for /commands (but NOT absolute paths like /home/... — those are chat)
    if !literal_input && looks_like_command(&text) {
        let parsed = app.registry.parse(&text);
        match parsed {
            Some(CommandType::Builtin(BuiltinCommand::Quit)) => {
                app.running = false;
                return;
            }
            Some(CommandType::Builtin(BuiltinCommand::Clear)) => {
                let request = app.app_state.session_id.clone().map_or(
                    ClientRpcRequest::SessionNew,
                    |session_id| {
                        ClientRpcRequest::SessionNewFor(fabric::protocol::client::SessionParams {
                            session_id,
                        })
                    },
                );
                let request_id = write_request(app, request).await;
                app.pending_commands.insert(
                    request_id,
                    super::super::PendingCommand::NewSession { clear_screen: true },
                );
                app.compat_transcript
                    .add_text(ChatRole::System, "正在创建新会话…".to_string());
                return;
            }
            Some(CommandType::Builtin(BuiltinCommand::New)) => {
                let request = app.app_state.session_id.clone().map_or(
                    ClientRpcRequest::SessionNew,
                    |session_id| {
                        ClientRpcRequest::SessionNewFor(fabric::protocol::client::SessionParams {
                            session_id,
                        })
                    },
                );
                let request_id = write_request(app, request).await;
                app.pending_commands.insert(
                    request_id,
                    super::super::PendingCommand::NewSession {
                        clear_screen: false,
                    },
                );
                app.compat_transcript
                    .add_text(ChatRole::System, "正在创建新会话…".to_string());
                return;
            }
            Some(CommandType::Builtin(BuiltinCommand::Copy)) => {
                // Copy last assistant message to clipboard via OSC 52
                let last_assistant = app
                    .app_state
                    .items
                    .values()
                    .filter(|item| item.kind == "assistant")
                    .max_by_key(|item| item.sequence)
                    .map(|item| item.content.clone());
                match last_assistant {
                    Some(text) if !text.is_empty() => {
                        let encoded = base64_encode(&text);
                        // OSC 52: set clipboard to base64-encoded text
                        let osc = format!("\x1b]52;c;{encoded}\x1b\\");
                        io::stdout().write_all(osc.as_bytes()).ok();
                        io::stdout().flush().ok();
                        app.compat_transcript
                            .add_text(ChatRole::System, "已复制到剪贴板".to_string());
                    }
                    _ => {
                        app.compat_transcript
                            .add_text(ChatRole::System, "没有可复制的内容".to_string());
                    }
                }
                return;
            }
            Some(CommandType::Builtin(BuiltinCommand::Help)) => {
                let help = app.registry.help_text();
                app.compat_transcript.add_text(ChatRole::System, help);
                return;
            }
            Some(CommandType::Builtin(BuiltinCommand::Status)) => {
                let Some(session_id) = app.app_state.session_id.clone() else {
                    app.compat_transcript.add_text(
                        ChatRole::System,
                        "会话仍在初始化，请稍后重试 /status".to_string(),
                    );
                    return;
                };
                send_request(
                    app,
                    crate::intent::rpc(crate::intent::status(
                        ClientSurface::Tui,
                        format!("tui-status:{}", uuid::Uuid::new_v4()),
                        Some(fabric::SessionId(session_id)),
                    )),
                )
                .await;
                return;
            }
            Some(CommandType::Builtin(BuiltinCommand::Sessions)) => {
                let request_id = write_protocol_request(app, ClientRequest::ReadSessions).await;
                app.pending_commands
                    .insert(request_id, super::super::PendingCommand::OpenSessionPicker);
                app.pending_non_turn.insert(request_id);
                app.streaming = true;
                app.status.waiting = true;
                app.compat_transcript
                    .add_text(ChatRole::System, "查询会话列表中...".to_string());
                return;
            }
            Some(CommandType::Builtin(BuiltinCommand::Resume { id })) => {
                if id.is_empty() {
                    let request_id = write_protocol_request(app, ClientRequest::ReadSessions).await;
                    app.pending_commands
                        .insert(request_id, super::super::PendingCommand::OpenSessionPicker);
                    app.pending_non_turn.insert(request_id);
                    app.streaming = true;
                    app.status.waiting = true;
                    app.compat_transcript
                        .add_text(ChatRole::System, "查询可恢复会话中...".to_string());
                    return;
                }
                let request_id = write_protocol_request(
                    app,
                    ClientRequest::ReadSnapshot(SnapshotRequest {
                        session_id: fabric::SessionId(id.clone()),
                    }),
                )
                .await;
                app.pending_commands.insert(
                    request_id,
                    super::super::PendingCommand::ProjectionSnapshot {
                        session_id: id.clone(),
                    },
                );
                app.projection_target_session_id = Some(id.clone());
                app.projection_request_in_flight = true;
                app.compat_transcript
                    .add_text(ChatRole::System, format!("恢复会话 {id}..."));
                return;
            }
            Some(CommandType::Builtin(BuiltinCommand::Compact)) => {
                let Some(session_id) = app.app_state.session_id.clone() else {
                    app.compat_transcript.add_text(
                        ChatRole::System,
                        "会话仍在初始化，请稍后重试 /compact".to_string(),
                    );
                    return;
                };
                send_request(
                    app,
                    ClientRpcRequest::CompactFor(fabric::protocol::client::SessionParams {
                        session_id,
                    }),
                )
                .await;
                app.compat_transcript
                    .add_text(ChatRole::System, "压缩上下文中...".to_string());
                return;
            }
            Some(CommandType::Builtin(BuiltinCommand::Fork)) => {
                let Some(session_id) = app.app_state.session_id.clone() else {
                    app.compat_transcript.add_text(
                        ChatRole::System,
                        "当前会话尚未初始化，无法创建分支".to_string(),
                    );
                    return;
                };
                send_request(
                    app,
                    ClientRpcRequest::SessionFork(fabric::protocol::client::SessionForkParams {
                        session_id: fabric::SessionId(session_id),
                        through_sequence: app.app_state.cursor.sequence,
                    }),
                )
                .await;
                return;
            }
            Some(CommandType::Builtin(BuiltinCommand::Rewind { prompt_index })) => {
                let Some(session_id) = app.app_state.session_id.clone() else {
                    app.compat_transcript.add_text(
                        ChatRole::System,
                        "当前会话尚未初始化，无法恢复工作区检查点".to_string(),
                    );
                    return;
                };
                if prompt_index.trim().is_empty() {
                    let request_id =
                        write_request(app, ClientRpcRequest::checkpoint_list(session_id, 64)).await;
                    app.pending_commands.insert(
                        request_id,
                        super::super::PendingCommand::OpenCheckpointPicker,
                    );
                    app.pending_non_turn.insert(request_id);
                    app.streaming = true;
                    app.status.waiting = true;
                    app.compat_transcript
                        .add_text(ChatRole::System, "查询工作区检查点中…".to_string());
                    return;
                }
                let Ok(prompt_index) = prompt_index.parse::<u64>() else {
                    app.compat_transcript.add_text(
                        ChatRole::System,
                        "用法：/rewind <prompt-index>（必须使用 daemon 提供的检查点索引）"
                            .to_string(),
                    );
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
                app.compat_transcript.add_text(
                    ChatRole::System,
                    format!("请求恢复工作区检查点 {prompt_index}…"),
                );
                return;
            }
            Some(CommandType::Builtin(BuiltinCommand::Permissions)) => {
                app.compat_transcript.add_text(
                    ChatRole::System,
                    format!(
                        "=== Permissions ===\nWorking directory: {}\nWorkspace roots:\n{}",
                        app.workspace.cwd().display(),
                        app.workspace
                            .writable_roots()
                            .iter()
                            .map(|path| format!("  - {}", path.display()))
                            .collect::<Vec<_>>()
                            .join("\n")
                    ),
                );
                return;
            }
            Some(CommandType::Builtin(BuiltinCommand::Model)) => {
                send_request(app, ClientRpcRequest::ModelList).await;
                app.compat_transcript
                    .add_text(ChatRole::System, "查询可用模型中...".to_string());
                return;
            }
            // ── New P2 commands ──
            Some(CommandType::Builtin(BuiltinCommand::Mode { name })) => {
                let mode = if name.is_empty() {
                    // Cycle to next mode
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
                    modes[(current + 1) % modes.len()]
                } else {
                    match name.as_str() {
                        "plan" => CollaborationMode::Plan,
                        "auto" => CollaborationMode::Auto,
                        "sandbox" => CollaborationMode::Sandbox,
                        _ => CollaborationMode::Default,
                    }
                };
                write_request(app, ClientRpcRequest::mode_switch(mode)).await;
                app.compat_transcript.add_text(
                    ChatRole::System,
                    format!("Switching mode to: {}", mode.display_name()),
                );
                return;
            }
            Some(CommandType::Builtin(BuiltinCommand::Target { value })) => {
                if app.turn_active {
                    app.app_state.last_error =
                        Some("execution target cannot change while a turn is active".into());
                    return;
                }
                let parts = value.split_whitespace().collect::<Vec<_>>();
                let next = match parts.as_slice() {
                    ["general"] => Ok(fabric::ExecutionTargetSelection::general(
                        fabric::ExecutionTargetSource::UserCommand,
                    )),
                    ["robot", device] | ["robot", device, "simulation"] => {
                        fabric::ExecutionTargetSelection::robot(
                            *device,
                            fabric::types::embodiment::ExecutionEnvironment::Simulation,
                            fabric::ExecutionTargetSource::UserCommand,
                        )
                    }
                    ["robot", device, "hil"] => fabric::ExecutionTargetSelection::robot(
                        *device,
                        fabric::types::embodiment::ExecutionEnvironment::Hil,
                        fabric::ExecutionTargetSource::UserCommand,
                    ),
                    ["robot", device, "real"] => fabric::ExecutionTargetSelection::robot(
                        *device,
                        fabric::types::embodiment::ExecutionEnvironment::Real,
                        fabric::ExecutionTargetSource::UserCommand,
                    ),
                    _ => Err(
                        "usage: /target general | /target robot <device> [simulation|hil|real]"
                            .into(),
                    ),
                };
                match next {
                    Ok(selection) => {
                        let label = match &selection.target {
                            fabric::ExecutionTarget::General => "general".to_string(),
                            fabric::ExecutionTarget::Robot {
                                device_id,
                                environment,
                            } => format!("robot:{}/{}", device_id.0, environment.as_str()),
                        };
                        app.app_state
                            .select_execution_target_for_next_turn(selection);
                        app.compat_transcript.add_text(
                            ChatRole::System,
                            format!(
                                "Execution target set to {label}; the next turn will persist this typed selection."
                            ),
                        );
                    }
                    Err(error) => app.app_state.last_error = Some(error),
                }
                return;
            }
            Some(CommandType::Builtin(BuiltinCommand::Runtime { value })) => {
                if app.turn_active {
                    app.app_state.last_error =
                        Some("agent runtime cannot change while a turn is active".into());
                    return;
                }
                let value = value.trim();
                if value.eq_ignore_ascii_case("auto") || value.eq_ignore_ascii_case("clear") {
                    app.next_agent_runtime = None;
                    app.compat_transcript.add_text(
                        ChatRole::System,
                        "Agent runtime selection reset to auto for the next turn".to_string(),
                    );
                } else if value.is_empty()
                    || value.len() > 128
                    || value.chars().any(char::is_whitespace)
                {
                    app.app_state.last_error = Some("usage: /runtime <runtime-id|auto>".into());
                } else {
                    app.next_agent_runtime = Some(value.to_owned());
                    app.compat_transcript.add_text(
                        ChatRole::System,
                        format!(
                            "Agent runtime {value} is required for the next turn; completion requires its terminal receipt"
                        ),
                    );
                }
                return;
            }
            Some(CommandType::Builtin(BuiltinCommand::Agents)) => {
                let request_id = write_request(app, ClientRpcRequest::SubAgents).await;
                app.pending_commands.insert(
                    request_id,
                    super::super::PendingCommand::OpenAgentInspector { focus: None },
                );
                app.pending_non_turn.insert(request_id);
                return;
            }
            Some(CommandType::Builtin(BuiltinCommand::AgentDetail { id })) => {
                let request_id = write_request(app, ClientRpcRequest::SubAgents).await;
                app.pending_commands.insert(
                    request_id,
                    super::super::PendingCommand::OpenAgentInspector { focus: Some(id) },
                );
                app.pending_non_turn.insert(request_id);
                return;
            }
            Some(CommandType::Builtin(BuiltinCommand::Skills)) => {
                send_request(app, ClientRpcRequest::SkillsList).await;
                app.compat_transcript
                    .add_text(ChatRole::System, "查询技能列表中...".to_string());
                return;
            }
            Some(CommandType::Builtin(BuiltinCommand::Memory)) => {
                send_request(app, ClientRpcRequest::SessionMemory).await;
                app.compat_transcript
                    .add_text(ChatRole::System, "查询记忆中...".to_string());
                return;
            }
            Some(CommandType::Builtin(BuiltinCommand::MemorySearch { query })) => {
                if query.is_empty() {
                    app.compat_transcript
                        .add_text(ChatRole::System, "用法：/memory search <query>".to_string());
                } else {
                    send_request(
                        app,
                        ClientRpcRequest::memory_search(query, app.app_state.session_id.clone()),
                    )
                    .await;
                    app.compat_transcript
                        .add_text(ChatRole::System, "搜索记忆中...".to_string());
                }
                return;
            }
            Some(CommandType::Builtin(BuiltinCommand::MemoryStatus)) => {
                send_request(app, ClientRpcRequest::MemoryStatus).await;
                app.compat_transcript
                    .add_text(ChatRole::System, "查询记忆状态中...".to_string());
                return;
            }
            Some(CommandType::Builtin(BuiltinCommand::SkillRun { name, args })) => {
                if !app.registry.is_skill(&name) {
                    app.compat_transcript.add_text(
                        ChatRole::System,
                        format!("Skill 不可用：{name}。请先运行 /skills 刷新目录。"),
                    );
                    return;
                }
                app.compat_transcript.add_text(ChatRole::User, text.clone());
                let request = match app.app_state.session_id.clone() {
                    Some(session_id) => ClientRpcRequest::skill_invoke_for(
                        name,
                        args,
                        fabric::SessionId(session_id),
                        &app.workspace,
                    ),
                    None => ClientRpcRequest::skill_invoke(name, args, &app.workspace),
                };
                send_request(app, request).await;
                return;
            }
            Some(CommandType::Builtin(BuiltinCommand::Interrupt)) => {
                write_request(
                    app,
                    ClientRpcRequest::interrupt(InterruptReason::UserCancelled),
                )
                .await;
                app.compat_transcript
                    .add_text(ChatRole::System, "Interrupt sent".to_string());
                return;
            }
            Some(CommandType::Builtin(BuiltinCommand::Context)) => {
                let mode = &app.app_state.mode;
                let msg = format!(
                    "{}\nMode: {} {}\nCumulative provider usage: {}k tokens\nAwareness: {} {}",
                    app.app_state.context_diagnostic(),
                    mode.icon(),
                    mode.display_name(),
                    app.app_state.total_tokens / 1000,
                    app.app_state.awareness.level.icon(),
                    app.app_state.awareness.level.display_name(),
                );
                // Task Console renders only daemon-projected conversation
                // state.  Keep this local, read-only diagnostic out of that
                // canonical projection while still making it visible.
                app.pager = Some(super::super::pager::PagerOverlay::new(
                    "Context diagnostics",
                    msg,
                ));
                return;
            }
            Some(CommandType::Builtin(BuiltinCommand::Profile)) => {
                write_request(app, ClientRpcRequest::AgentProfileList).await;
                app.compat_transcript
                    .add_text(ChatRole::System, "Querying agent profiles...".to_string());
                return;
            }
            Some(CommandType::Builtin(BuiltinCommand::ProfileSet { name })) => {
                write_request(app, ClientRpcRequest::agent_profile_set(name.clone())).await;
                app.compat_transcript.add_text(
                    ChatRole::System,
                    format!("Switching agent profile to: {name}"),
                );
                return;
            }
            Some(CommandType::Builtin(BuiltinCommand::Diff)) => {
                if let Some(delta) = app.latest_patch.as_ref() {
                    app.detail = Some(super::super::diff_view::DiffView::from_patch_delta(delta));
                    return;
                }
                app.compat_transcript.add_text(
                    ChatRole::System,
                    "No authoritative checkpoint diff is available for this turn".to_string(),
                );
                return;
            }
            Some(CommandType::Builtin(BuiltinCommand::Mention { path })) => {
                if path.is_empty() {
                    app.compat_transcript
                        .add_text(ChatRole::System, "用法: /mention <path>".to_string());
                } else {
                    app.input_buf = format!("@{path} ");
                    app.cursor = app.input_buf.len();
                    app.compat_transcript
                        .add_text(ChatRole::System, format!("已将 @{path} 加入输入框"));
                }
                return;
            }
            Some(CommandType::Builtin(BuiltinCommand::Input)) => {
                app.input_buf.push('\n');
                app.cursor = app.input_buf.len();
                app.compat_transcript.add_text(
                    ChatRole::System,
                    "多行输入已开启；继续输入，使用 Alt+Enter 提交".to_string(),
                );
                return;
            }
            Some(CommandType::Skill { name, args }) => {
                app.compat_transcript.add_text(ChatRole::User, text.clone());
                let request = match app.app_state.session_id.clone() {
                    Some(session_id) => ClientRpcRequest::skill_invoke_for(
                        name,
                        args,
                        fabric::SessionId(session_id),
                        &app.workspace,
                    ),
                    None => ClientRpcRequest::skill_invoke(name, args, &app.workspace),
                };
                send_request(app, request).await;
                return;
            }
            Some(CommandType::Unknown {
                name, suggestions, ..
            }) => {
                let hint = if suggestions.is_empty() {
                    "输入 /help 查看可用命令".to_string()
                } else {
                    format!("你是否想输入：{}", suggestions.join("、"))
                };
                app.compat_transcript
                    .add_text(ChatRole::System, format!("未知命令 /{name}。{hint}"));
                return;
            }
            None => {
                app.compat_transcript
                    .add_text(ChatRole::System, "无效命令".to_string());
                return;
            }
        }
    }

    // Regular chat message
    if !literal_input {
        if let Err(error) = super::super::input_safety::resolve_attachments(&text, &app.workspace) {
            app.input_buf = text;
            app.cursor = app.input_buf.len();
            app.app_state.last_error = Some(format!("attachment rejected: {error}"));
            return;
        }
    }
    app.history.push(text.clone());
    app.persist_input_state();
    app.compat_transcript.add_text(ChatRole::User, text.clone());
    // Assistant entry created lazily on first response delta so it renders
    // after any tool/reflection logs (ordering fix).
    send_to_daemon(app, &text).await;
}

async fn send_shell_to_daemon(app: &mut App, command: &str) {
    let request_id = app.next_request_id;
    let request = crate::intent::rpc(crate::intent::execute_shell(
        format!("tui-shell:{request_id}"),
        command,
        app.app_state.session_id.clone().map(fabric::SessionId),
        &app.workspace,
        crate::host::permission_mode_from_environment(),
    ));
    write_request(app, request).await;
    app.streaming = true;
    app.response_buf.clear();
    app.status.waiting = true;
    app.app_state.streaming = true;
}

pub async fn send_to_daemon(app: &mut App, text: &str) {
    let request_id = app.next_request_id;
    app.next_request_id = app.next_request_id.saturating_add(1);
    let mut requirements = app.turn_requirements.clone();
    if let Some(runtime_id) = app.next_agent_runtime.as_ref() {
        requirements.push(fabric::TurnRequirement::InvokeAgentRuntime {
            runtime_id: runtime_id.clone(),
        });
    }
    let task_kind = app
        .next_agent_runtime
        .as_ref()
        .map(|_| fabric::TaskKind::Coding)
        .or(app.requested_task_kind);
    let request = crate::intent::rpc(crate::intent::submit_prompt(crate::intent::PromptIntent {
        surface: ClientSurface::Tui,
        correlation_id: format!("tui:{request_id}"),
        content: text,
        session_id: app.app_state.session_id.clone().map(fabric::SessionId),
        workspace: &app.workspace,
        requirements,
        task_kind,
        permission_mode: crate::host::permission_mode_from_environment(),
        execution_target: app.app_state.execution_target_for_submission().clone(),
    }));
    let msg = request
        .to_json_rpc(Some(request_id))
        .expect("typed chat request serializes");
    let payload = serde_json::to_string(&msg).unwrap_or_default();
    let framed = format!("{payload}\n");

    if app.stream.write_all(framed.as_bytes()).await.is_err() {
        app.compat_transcript
            .add_text(ChatRole::System, "发送失败，请检查 daemon".to_string());
        return;
    }
    if app.stream.flush().await.is_err() {
        app.compat_transcript
            .add_text(ChatRole::System, "发送失败，请检查 daemon".to_string());
        return;
    }
    app.next_agent_runtime = None;
    app.streaming = true;
    app.response_buf.clear();
    app.status.waiting = true;
    app.app_state.streaming = true;
}

/// Simple base64 encoder (no external dependency).
fn base64_encode(input: &str) -> String {
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let bytes = input.as_bytes();
    let mut result = String::with_capacity(bytes.len().div_ceil(3) * 4);
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

#[cfg(test)]
mod secure_shell_tests {
    use super::*;
    use crate::tui::host_time::ClientClock;
    use crate::tui::term_compat::TermCaps;
    use tokio::io::AsyncReadExt;

    #[tokio::test]
    async fn shell_intent_requires_confirmation_then_uses_typed_host_rpc() {
        let (stream, mut peer) = tokio::net::UnixStream::pair().unwrap();
        let workspace =
            fabric::WorkspacePolicy::from_resolved_roots("/tmp".into(), vec![]).unwrap();
        let mut app = App::new(
            stream,
            TermCaps {
                color: true,
                true_color: false,
                unicode: false,
                width: 80,
                height: 24,
            },
            "test-model".into(),
            std::sync::Arc::new(ClientClock::default()),
            workspace,
            vec![],
        );

        submit_message(&mut app, "!printf governed".into()).await;
        assert_eq!(
            app.pending_shell_confirmation.as_deref(),
            Some("printf governed")
        );
        assert_eq!(app.input_buf, "!printf governed");

        app.input_buf.clear();
        app.cursor = 0;
        submit_message(&mut app, "!printf governed".into()).await;
        let mut bytes = vec![0; 4096];
        let read = peer.read(&mut bytes).await.unwrap();
        let request: serde_json::Value =
            serde_json::from_slice(bytes[..read].strip_suffix(b"\n").unwrap()).unwrap();
        assert_eq!(request["method"], "client.intent");
        assert_eq!(request["params"]["command"]["command"], "execute_shell");
        assert_eq!(
            request["params"]["command"]["arguments"]["command"],
            "printf governed"
        );
        assert!(app.pending_shell_confirmation.is_none());
    }

    #[tokio::test]
    async fn pending_robot_target_survives_stale_projection_and_is_sent_on_wire() {
        let (stream, mut peer) = tokio::net::UnixStream::pair().unwrap();
        let workspace =
            fabric::WorkspacePolicy::from_resolved_roots("/tmp".into(), vec![]).unwrap();
        let mut app = App::new(
            stream,
            TermCaps {
                color: true,
                true_color: false,
                unicode: false,
                width: 80,
                height: 24,
            },
            "test-model".into(),
            std::sync::Arc::new(ClientClock::default()),
            workspace,
            vec![],
        );
        app.app_state.cursor.sequence = 10;
        let robot = fabric::ExecutionTargetSelection::robot(
            "robot-1",
            fabric::types::embodiment::ExecutionEnvironment::Simulation,
            fabric::ExecutionTargetSource::UserCommand,
        )
        .unwrap();
        app.app_state
            .select_execution_target_for_next_turn(robot.clone());

        app.app_state
            .reconcile_projected_execution_target(&fabric::ExecutionTargetSelection::default(), 1);
        send_to_daemon(&mut app, "perform a safe simulation step").await;

        let mut bytes = vec![0; 4096];
        let read = peer.read(&mut bytes).await.unwrap();
        let request: serde_json::Value =
            serde_json::from_slice(bytes[..read].strip_suffix(b"\n").unwrap()).unwrap();
        let selection = &request["params"]["command"]["arguments"]["execution_target"];
        assert_eq!(selection["target"]["kind"], "robot");
        assert_eq!(selection["target"]["device_id"], "robot-1");
        assert_eq!(selection["target"]["environment"], "simulation");
        assert_eq!(selection["source"], "user_command");
        assert_eq!(app.app_state.execution_target_for_submission(), &robot);
    }

    #[tokio::test]
    async fn next_runtime_becomes_typed_coding_requirement_and_is_consumed() {
        let (stream, mut peer) = tokio::net::UnixStream::pair().unwrap();
        let workspace =
            fabric::WorkspacePolicy::from_resolved_roots("/tmp".into(), vec![]).unwrap();
        let mut app = App::new(
            stream,
            TermCaps {
                color: true,
                true_color: false,
                unicode: false,
                width: 80,
                height: 24,
            },
            "test-model".into(),
            std::sync::Arc::new(ClientClock::default()),
            workspace,
            vec![],
        );
        app.next_agent_runtime = Some("pi-rpc".into());

        send_to_daemon(&mut app, "review the repository").await;

        let mut bytes = vec![0; 4096];
        let read = peer.read(&mut bytes).await.unwrap();
        let request: serde_json::Value =
            serde_json::from_slice(bytes[..read].strip_suffix(b"\n").unwrap()).unwrap();
        let arguments = &request["params"]["command"]["arguments"];
        assert_eq!(arguments["task_kind"], "coding");
        assert_eq!(
            arguments["requirements"][0]["InvokeAgentRuntime"]["runtime_id"],
            "pi-rpc"
        );
        assert!(app.next_agent_runtime.is_none());
    }
}

#[cfg(test)]
mod governance_command_tests {
    use super::super::super::command::CommandType;
    use super::super::super::registry::CommandRegistry;

    #[test]
    fn internal_governance_text_has_no_tui_dispatch() {
        let registry = CommandRegistry::new();
        for command in [
            "/reflect",
            "/reflect_now",
            "/evolution",
            "/genome",
            "/hooks",
            "/task coding",
            "/evaluation",
            "/approve",
            "/plan",
            "/computer",
        ] {
            assert!(
                matches!(registry.parse(command), Some(CommandType::Unknown { .. })),
                "{command} still has a TUI dispatch path"
            );
        }
    }
}
