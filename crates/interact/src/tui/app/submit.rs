use crate::tui::presentation::CollaborationModePresentation;

use application::turn_control::CollaborationMode;
use gateway::client::CommandOutcome as GatewayCommandOutcome;
use gateway::protocol::{
    CheckpointListQuery, Command, ExecuteShellRequest, ForkSessionRequest, MemorySnapshotQuery,
    Query, RequestSessionCreation, RequestedExecutionTarget, RestoreWorkspaceCheckpoint,
    ResumeSessionReference, ReviewTransactionRequest, SessionRef, SubmitPromptRequest,
    WorkspaceRestoreOutcome,
};

use super::super::chat::Role as ChatRole;
use super::super::command::{looks_like_command, BuiltinCommand, CommandType};
use super::super::TuiModel;

fn typed_route_required(app: &mut TuiModel, operation: &str) {
    app.app_state.last_error = Some(format!(
        "typed Gateway is unavailable for {operation}; refusing legacy Session RPC fallback"
    ));
}

pub(super) async fn request_agent_inspector(app: &mut TuiModel, focus: Option<String>) {
    #[cfg(test)]
    if app.pending_commands.values().any(|pending| {
        matches!(
            pending,
            super::super::PendingCommand::OpenAgentInspector { .. }
        )
    }) {
        return;
    }
    if let Some(client) = app.controller.typed_gateway.as_mut() {
        let value = client
            .query(Query::AgentCatalog(gateway::protocol::AgentCatalogQuery))
            .await;
        match value {
            Ok(value) => {
                let agents = value
                    .get("agents")
                    .cloned()
                    .unwrap_or_else(|| serde_json::Value::Array(Vec::new()));
                match super::super::agent_inspector::AgentInspector::from_json(
                    &agents,
                    focus.as_deref(),
                ) {
                    Ok(inspector) => app.agent_inspector = Some(inspector),
                    Err(error) => app.app_state.last_error = Some(error.to_string()),
                }
            }
            Err(error) => app.app_state.last_error = Some(error.to_string()),
        }
    } else {
        typed_route_required(app, "agent inspector");
    }
    app.agent_inspector_next_refresh_at =
        ::contracts::MonoTime(app.clock.mono_now().0.saturating_add(1_000));
}

async fn typed_create_session(app: &mut TuiModel, clear_screen: bool) -> bool {
    let Some(client) = app.controller.typed_gateway.as_mut() else {
        return false;
    };
    let result = client
        .send(Command::CreateSession(RequestSessionCreation {
            principal_hint: None,
            workspace: Some(app.workspace.cwd().to_string_lossy().into_owned()),
        }))
        .await;
    let session = match result {
        Ok(GatewayCommandOutcome::Created { session }) => session,
        Ok(other) => {
            app.system_notices.push(
                ChatRole::System,
                format!("Gateway new-session receipt invalid: {other:?}"),
            );
            return true;
        }
        Err(error) => {
            app.system_notices.push(
                ChatRole::System,
                format!("Gateway new-session failed: {error}"),
            );
            return true;
        }
    };
    if clear_screen {
        app.system_notices = super::super::SystemNoticeQueue::new(app.caps.clone());
        app.system_notice_cursor = 0;
    }
    app.app_state.reset_execution_target_for_session();
    app.app_state.session_id = Some(session.0.clone());
    app.active_turn_ref = None;
    app.turn_active = false;
    app.streaming = false;
    app.app_state.turn_active = false;
    app.app_state.streaming = false;
    app.projection_target_session_id = Some(session.0.clone());
    app.projection_session_id = None;
    app.projection_request_in_flight = false;
    app.projection_polling = false;
    app.system_notices
        .push(ChatRole::System, format!("已创建新会话：{}", session.0));
    true
}

pub(super) async fn typed_resume_session(app: &mut TuiModel, session_id: String) -> bool {
    let Some(client) = app.controller.typed_gateway.as_mut() else {
        return false;
    };
    let result = client
        .send(Command::ResumeSession(ResumeSessionReference {
            reference: session_id,
        }))
        .await;
    let session = match result {
        Ok(GatewayCommandOutcome::Resumed { session }) => session,
        Ok(other) => {
            app.system_notices.push(
                ChatRole::System,
                format!("Gateway resume receipt invalid: {other:?}"),
            );
            return true;
        }
        Err(error) => {
            app.system_notices
                .push(ChatRole::System, format!("Gateway resume failed: {error}"));
            return true;
        }
    };
    app.app_state.session_id = Some(session.0.clone());
    app.active_turn_ref = None;
    app.turn_active = false;
    app.streaming = false;
    app.app_state.turn_active = false;
    app.app_state.streaming = false;
    app.projection_target_session_id = Some(session.0.clone());
    app.projection_session_id = None;
    app.projection_request_in_flight = false;
    app.projection_polling = false;
    app.system_notices
        .push(ChatRole::System, format!("恢复会话 {}...", session.0));
    true
}

async fn typed_session_list(app: &mut TuiModel) -> bool {
    let Some(client) = app.controller.typed_gateway.as_mut() else {
        return false;
    };
    let result = client.query(Query::SessionList).await;
    match result {
        Ok(value) => {
            match serde_json::from_value::<::contracts::protocol::client::SessionListSnapshot>(
                value,
            ) {
                Ok(list)
                    if list.schema_version == ::contracts::SESSION_READ_MODEL_SCHEMA_VERSION =>
                {
                    match serde_json::to_value(list.sessions)
                        .map_err(|error| error.to_string())
                        .and_then(|sessions| {
                            super::super::session_picker::SessionPicker::from_json(
                                &sessions,
                                app.app_state.session_id.clone(),
                            )
                            .map_err(|error| error.to_string())
                        }) {
                        Ok(picker) => app.session_picker = Some(picker),
                        Err(error) => app
                            .system_notices
                            .push(ChatRole::System, format!("无法打开会话列表：{error}")),
                    }
                }
                Ok(list) => app.system_notices.push(
                    ChatRole::System,
                    format!(
                        "无法打开会话列表：unsupported schema {}",
                        list.schema_version
                    ),
                ),
                Err(error) => app
                    .system_notices
                    .push(ChatRole::System, format!("无法打开会话列表：{error}")),
            }
        }
        Err(error) => app.system_notices.push(
            ChatRole::System,
            format!("Typed Gateway session list failed: {error}"),
        ),
    }
    true
}

/// Refresh the daemon-owned skill catalog through the typed Gateway query.
/// This keeps command completion projection-only and removes the last startup
/// dependency on the legacy `skills.list` Session connection.
pub(super) async fn refresh_typed_skill_catalog(app: &mut TuiModel) -> bool {
    let Some(client) = app.controller.typed_gateway.as_mut() else {
        return false;
    };
    match client
        .query(gateway::protocol::Query::SkillCatalog(
            gateway::protocol::SkillCatalogQuery,
        ))
        .await
    {
        Ok(value) => {
            let skills = value
                .get("skills")
                .cloned()
                .unwrap_or_else(|| serde_json::Value::Array(Vec::new()));
            app.registry.set_skills_from_json(&skills);
        }
        Err(error) => {
            app.registry.set_skills_from_json(&serde_json::Value::Null);
            app.app_state.last_error = Some(format!("Typed Gateway skill catalog failed: {error}"));
        }
    }
    true
}

pub(super) async fn typed_model_catalog(app: &mut TuiModel) -> bool {
    let Some(client) = app.controller.typed_gateway.as_mut() else {
        return false;
    };
    match client
        .query(gateway::protocol::Query::ModelCatalog(
            gateway::protocol::ModelCatalogQuery,
        ))
        .await
    {
        Ok(value) => {
            app.app_state.last_error = None;
            app.system_notices.push(
                ChatRole::System,
                super::super::response::format_models(&value),
            );
        }
        Err(error) => {
            app.app_state.last_error = Some(format!("Typed Gateway model catalog failed: {error}"));
        }
    }
    true
}

pub(super) async fn typed_set_collaboration_mode(
    app: &mut TuiModel,
    mode: CollaborationMode,
) -> bool {
    let Some(client) = app.controller.typed_gateway.as_mut() else {
        return false;
    };
    let requested = match mode {
        CollaborationMode::Default => gateway::protocol::RequestedCollaborationMode::Default,
        CollaborationMode::Plan => gateway::protocol::RequestedCollaborationMode::Plan,
        CollaborationMode::Auto => gateway::protocol::RequestedCollaborationMode::Auto,
        CollaborationMode::Sandbox => gateway::protocol::RequestedCollaborationMode::Sandbox,
    };
    match client
        .send(Command::SetCollaborationMode(
            gateway::protocol::SetCollaborationModeRequest { mode: requested },
        ))
        .await
    {
        Ok(GatewayCommandOutcome::CollaborationModeUpdated { mode }) => {
            app.app_state.mode = match mode {
                gateway::protocol::RequestedCollaborationMode::Default => {
                    CollaborationMode::Default
                }
                gateway::protocol::RequestedCollaborationMode::Plan => CollaborationMode::Plan,
                gateway::protocol::RequestedCollaborationMode::Auto => CollaborationMode::Auto,
                gateway::protocol::RequestedCollaborationMode::Sandbox => {
                    CollaborationMode::Sandbox
                }
            };
            true
        }
        Ok(other) => {
            app.app_state.last_error = Some(format!("Gateway mode receipt invalid: {other:?}"));
            true
        }
        Err(error) => {
            app.app_state.last_error = Some(format!("Gateway mode change failed: {error}"));
            true
        }
    }
}

pub(super) async fn typed_set_agent_profile(app: &mut TuiModel, profile: String) -> bool {
    let Some(client) = app.controller.typed_gateway.as_mut() else {
        return false;
    };
    match client
        .send(Command::SetAgentProfile(
            gateway::protocol::SetAgentProfileRequest { profile },
        ))
        .await
    {
        Ok(GatewayCommandOutcome::AgentProfileUpdated { profile }) => {
            app.system_notices
                .push(ChatRole::System, format!("Agent profile set to {profile}"));
            true
        }
        Ok(other) => {
            app.app_state.last_error = Some(format!("Gateway profile receipt invalid: {other:?}"));
            true
        }
        Err(error) => {
            app.app_state.last_error = Some(format!("Gateway profile change failed: {error}"));
            true
        }
    }
}

async fn typed_agent_profile_catalog(app: &mut TuiModel) -> bool {
    let Some(client) = app.controller.typed_gateway.as_mut() else {
        return false;
    };
    match client
        .query(Query::AgentProfileCatalog(
            gateway::protocol::AgentProfileCatalogQuery,
        ))
        .await
    {
        Ok(value) => {
            let profiles = value
                .get("profiles")
                .cloned()
                .unwrap_or_else(|| serde_json::Value::Array(Vec::new()));
            app.system_notices.push(
                ChatRole::System,
                serde_json::to_string_pretty(&profiles).unwrap_or_else(|_| "[]".into()),
            );
            true
        }
        Err(error) => {
            app.app_state.last_error = Some(format!("Gateway profile query failed: {error}"));
            true
        }
    }
}

async fn typed_status(app: &mut TuiModel, session_id: String) -> bool {
    let Some(client) = app.controller.typed_gateway.as_mut() else {
        return false;
    };
    let result = client
        .query(Query::SessionSnapshot(
            gateway::protocol::SessionSnapshotQuery {
                session: SessionRef(session_id.clone()),
                after_cursor: None,
                paged: false,
            },
        ))
        .await;
    match result {
        Ok(value) => {
            super::super::response::apply_typed_projection_result(app, &session_id, value);
            app.system_notices.push(
                ChatRole::System,
                format!(
                    "会话状态来自 canonical projection：cursor={}",
                    app.app_state.cursor.sequence
                ),
            );
        }
        Err(error) => app.system_notices.push(
            ChatRole::System,
            format!("Typed Gateway status failed: {error}"),
        ),
    }
    true
}

async fn typed_memory_status(app: &mut TuiModel) -> bool {
    let Some(client) = app.controller.typed_gateway.as_mut() else {
        return false;
    };
    match client
        .query(gateway::protocol::Query::MemoryStatus(
            gateway::protocol::MemoryStatusQuery,
        ))
        .await
    {
        Ok(value) => app.system_notices.push(
            ChatRole::System,
            super::super::response::format_memory_status(&value),
        ),
        Err(error) => app.app_state.last_error = Some(format!("Memory status failed: {error}")),
    }
    true
}

async fn typed_memory_search(app: &mut TuiModel, query: String) -> bool {
    let Some(client) = app.controller.typed_gateway.as_mut() else {
        return false;
    };
    match client
        .query(gateway::protocol::Query::MemorySearch(
            gateway::protocol::MemorySearchQuery {
                query,
                session: app.app_state.session_id.clone(),
            },
        ))
        .await
    {
        Ok(value) => app.system_notices.push(
            ChatRole::System,
            super::super::response::format_memory_facts(&value),
        ),
        Err(error) => app.app_state.last_error = Some(format!("Memory search failed: {error}")),
    }
    true
}

async fn typed_memory_snapshot(app: &mut TuiModel) -> bool {
    let Some(session_id) = app.app_state.session_id.clone() else {
        return false;
    };
    let Some(client) = app.controller.typed_gateway.as_mut() else {
        return false;
    };
    match client
        .query(Query::MemorySnapshot(MemorySnapshotQuery {
            session: SessionRef(session_id),
            memory_type: "all".into(),
            limit: 20,
        }))
        .await
    {
        Ok(value) => {
            let content = value
                .get("content")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("(memory snapshot is empty)");
            app.system_notices
                .push(ChatRole::System, content.to_owned());
        }
        Err(error) => app
            .system_notices
            .push(ChatRole::System, format!("Memory snapshot failed: {error}")),
    }
    true
}

async fn typed_skill_run(app: &mut TuiModel, name: String, args: String) -> bool {
    let Some(client) = app.controller.typed_gateway.as_mut() else {
        return false;
    };
    let outcome = client
        .send(Command::InvokeSkill(
            gateway::protocol::SkillInvokeRequest {
                skill_id: name,
                user_args: args,
                session: app.app_state.session_id.clone().map(SessionRef),
                workspace: Some(app.workspace.cwd().to_string_lossy().into_owned()),
            },
        ))
        .await;
    match outcome {
        Ok(GatewayCommandOutcome::Submitted { turn }) => mark_typed_turn_submitted(app, turn),
        Ok(other) => app.app_state.last_error = Some(format!("Skill receipt invalid: {other:?}")),
        Err(error) => app.app_state.last_error = Some(format!("Skill invocation failed: {error}")),
    }
    true
}

async fn typed_cancel_active_turn(app: &mut TuiModel) -> bool {
    let Some(session_id) = app.app_state.session_id.clone() else {
        return false;
    };
    let Some(client) = app.controller.typed_gateway.as_mut() else {
        return false;
    };
    match client
        .send(Command::CancelActiveTurn(
            gateway::protocol::CancelActiveTurn {
                session: SessionRef(session_id),
            },
        ))
        .await
    {
        Ok(GatewayCommandOutcome::Cancelled) => true,
        Ok(other) => {
            app.app_state.last_error =
                Some(format!("Gateway cancellation receipt invalid: {other:?}"));
            true
        }
        Err(error) => {
            app.app_state.last_error = Some(format!("Gateway cancellation failed: {error}"));
            true
        }
    }
}

async fn typed_compact_session(app: &mut TuiModel, session_id: String) -> bool {
    let Some(client) = app.controller.typed_gateway.as_mut() else {
        return false;
    };
    match client
        .send(Command::CompactSession(SessionRef(session_id)))
        .await
    {
        Ok(GatewayCommandOutcome::SessionUpdated { session }) => {
            app.projection_target_session_id = Some(session.0.clone());
            app.projection_session_id = None;
            app.system_notices
                .push(ChatRole::System, "canonical session compacted".to_string());
        }
        Ok(other) => app.system_notices.push(
            ChatRole::System,
            format!("Gateway compact receipt invalid: {other:?}"),
        ),
        Err(error) => app
            .system_notices
            .push(ChatRole::System, format!("Gateway compact failed: {error}")),
    }
    true
}

pub(super) async fn typed_fork_session(
    app: &mut TuiModel,
    session_id: String,
    through_sequence: u64,
) -> bool {
    let Some(client) = app.controller.typed_gateway.as_mut() else {
        return false;
    };
    match client
        .send(Command::ForkSession(ForkSessionRequest {
            session: SessionRef(session_id),
            through_sequence,
        }))
        .await
    {
        Ok(GatewayCommandOutcome::Forked { session }) => {
            app.app_state.session_id = Some(session.0.clone());
            app.projection_target_session_id = Some(session.0.clone());
            app.projection_session_id = None;
            app.projection_request_in_flight = false;
            app.projection_polling = false;
            app.system_notices
                .push(ChatRole::System, format!("已创建分支会话 {}", session.0));
        }
        Ok(other) => app.system_notices.push(
            ChatRole::System,
            format!("Gateway fork receipt invalid: {other:?}"),
        ),
        Err(error) => app
            .system_notices
            .push(ChatRole::System, format!("Gateway fork failed: {error}")),
    }
    true
}

pub(super) async fn typed_checkpoint_list(
    app: &mut TuiModel,
    session_id: String,
    limit: u16,
) -> bool {
    let Some(client) = app.controller.typed_gateway.as_mut() else {
        return false;
    };
    let result = client
        .query(Query::CheckpointList(CheckpointListQuery {
            session: SessionRef(session_id),
            limit,
        }))
        .await;
    match result {
        Ok(value) => match serde_json::from_value::<::contracts::CheckpointListSnapshot>(value) {
            Ok(snapshot) => {
                match super::super::checkpoint_picker::CheckpointPicker::from_snapshot(snapshot) {
                    Ok(picker) => app.checkpoint_picker = Some(picker),
                    Err(error) => app.system_notices.push(
                        ChatRole::System,
                        format!("无法打开工作区检查点列表：{error}"),
                    ),
                }
            }
            Err(error) => app.system_notices.push(
                ChatRole::System,
                format!("无法读取工作区检查点列表：{error}"),
            ),
        },
        Err(error) => app.system_notices.push(
            ChatRole::System,
            format!("Gateway checkpoint list failed: {error}"),
        ),
    }
    true
}

pub(super) async fn typed_workspace_rewind(
    app: &mut TuiModel,
    session_id: String,
    prompt_index: u64,
    child_session_id: Option<String>,
) -> bool {
    let Some(client) = app.controller.typed_gateway.as_mut() else {
        return false;
    };
    let outcome = client
        .send(Command::RestoreWorkspaceCheckpoint(
            RestoreWorkspaceCheckpoint {
                session: SessionRef(session_id),
                prompt_index,
            },
        ))
        .await;
    match outcome {
        Ok(GatewayCommandOutcome::WorkspaceRestored {
            outcome: WorkspaceRestoreOutcome::Completed,
        }) => {
            if let Some(child_session_id) = child_session_id {
                app.app_state.session_id = Some(child_session_id.clone());
                app.projection_target_session_id = Some(child_session_id);
            }
            app.projection_session_id = None;
            app.projection_request_in_flight = false;
            app.projection_polling = false;
            app.system_notices.push(
                ChatRole::System,
                format!("工作区检查点 {prompt_index} 已恢复"),
            );
        }
        Ok(GatewayCommandOutcome::WorkspaceRestored { outcome }) => {
            app.system_notices.push(
                ChatRole::System,
                format!("工作区检查点 {prompt_index} 未恢复：{outcome:?}"),
            );
        }
        Ok(other) => app.system_notices.push(
            ChatRole::System,
            format!("Gateway rewind receipt invalid: {other:?}"),
        ),
        Err(error) => app
            .system_notices
            .push(ChatRole::System, format!("Gateway rewind failed: {error}")),
    }
    true
}

async fn typed_execute_shell(app: &mut TuiModel, command: &str) -> bool {
    let Some(client) = app.controller.typed_gateway.as_mut() else {
        return false;
    };
    let outcome = client
        .send(Command::ExecuteShell(ExecuteShellRequest {
            command: command.to_owned(),
            session: app.app_state.session_id.clone().map(SessionRef),
            workspace: Some(app.workspace.cwd().to_string_lossy().into_owned()),
            requested_permission: app.requested_permission.clone(),
        }))
        .await;
    match outcome {
        Ok(GatewayCommandOutcome::Submitted { turn }) => mark_typed_turn_submitted(app, turn),
        Ok(other) => app.system_notices.push(
            ChatRole::System,
            format!("Gateway shell receipt invalid: {other:?}"),
        ),
        Err(error) => app
            .system_notices
            .push(ChatRole::System, format!("Gateway shell failed: {error}")),
    }
    true
}

/// Submit a transaction review through the authenticated typed Gateway. The
/// legacy request remains only as a compatibility fallback for test fixtures.
pub(super) async fn typed_transaction_review(
    app: &mut TuiModel,
    action: ::contracts::TransactionReviewAction,
    acknowledge_risk: bool,
) -> bool {
    let Some(session) = app.app_state.session_id.clone().map(SessionRef) else {
        return false;
    };
    let Some(transaction) = app
        .latest_patch
        .as_ref()
        .and_then(|patch| patch.transaction_id)
        .map(|id| id.0.to_string())
    else {
        return false;
    };
    let Some(client) = app.controller.typed_gateway.as_mut() else {
        return false;
    };
    match client
        .send(Command::ReviewTransaction(ReviewTransactionRequest {
            session,
            transaction,
            action,
            acknowledge_risk,
        }))
        .await
    {
        Ok(GatewayCommandOutcome::TransactionReviewed { outcome }) => {
            app.system_notices.push(
                ChatRole::System,
                format!("transaction review settled: {outcome}"),
            );
        }
        Ok(other) => {
            app.app_state.last_error = Some(format!("Gateway review receipt invalid: {other:?}"));
        }
        Err(error) => {
            app.app_state.last_error = Some(format!("Gateway review failed: {error}"));
        }
    }
    true
}

/// Read the latest Host transaction settlement through the typed Gateway.
pub(super) async fn typed_transaction_settlement(app: &mut TuiModel) -> bool {
    let Some(session) = app.app_state.session_id.clone().map(SessionRef) else {
        return false;
    };
    let Some(transaction) = app
        .latest_patch
        .as_ref()
        .and_then(|patch| patch.transaction_id)
        .map(|id| id.0.to_string())
    else {
        return false;
    };
    let Some(client) = app.controller.typed_gateway.as_mut() else {
        return false;
    };
    match client
        .query(Query::TransactionSettlement(
            gateway::protocol::TransactionSettlementQuery {
                session,
                transaction,
            },
        ))
        .await
    {
        Ok(value) => app.system_notices.push(
            ChatRole::System,
            format!("latest transaction settlement: {value}"),
        ),
        Err(error) => {
            app.app_state.last_error = Some(format!("Gateway settlement query failed: {error}"))
        }
    }
    true
}

pub async fn submit_message(app: &mut TuiModel, text: String) {
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
            app.system_notices.push(
                ChatRole::System,
                format!(
                    "Shell confirmation\nWorkspace: {}\nPermission: {:?}\nTransaction coverage: non-rollbackable\nHost policy and approval still apply. Press Enter again to submit or Esc to cancel.",
                    app.workspace.cwd().display(),
                    ::contracts::permission::HostPermissionMode::Safe,
                ),
            );
            return;
        }
        app.pending_shell_confirmation = None;
        app.history.push(format!("!{command}"));
        app.persist_input_state();
        app.system_notices
            .push(ChatRole::User, format!("!{command}"));
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
                if typed_create_session(app, true).await {
                    return;
                }
                typed_route_required(app, "clear session");
                return;
            }
            Some(CommandType::Builtin(BuiltinCommand::New)) => {
                if typed_create_session(app, false).await {
                    return;
                }
                typed_route_required(app, "new session");
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
                        match app.caps.write_osc52_clipboard(&encoded) {
                            Ok(()) => app
                                .system_notices
                                .push(ChatRole::System, "已发送到终端剪贴板".to_string()),
                            Err(error) => {
                                let message = format!("复制到剪贴板失败: {error}");
                                app.app_state.last_error = Some(message.clone());
                                app.system_notices.push(ChatRole::System, message);
                            }
                        }
                    }
                    _ => {
                        app.system_notices
                            .push(ChatRole::System, "没有可复制的内容".to_string());
                    }
                }
                return;
            }
            Some(CommandType::Builtin(BuiltinCommand::Help)) => {
                let help = app.registry.help_text();
                let mut pager = super::super::pager::PagerOverlay::new("Aletheon 命令", help);
                pager.scroll_to_top();
                app.pager = Some(pager);
                return;
            }
            Some(CommandType::Builtin(BuiltinCommand::Status)) => {
                let Some(session_id) = app.app_state.session_id.clone() else {
                    app.system_notices.push(
                        ChatRole::System,
                        "会话仍在初始化，请稍后重试 /status".to_string(),
                    );
                    return;
                };
                if typed_status(app, session_id.clone()).await {
                    return;
                }
                typed_route_required(app, "session status");
                return;
            }
            Some(CommandType::Builtin(BuiltinCommand::Sessions)) => {
                if typed_session_list(app).await {
                    return;
                }
                typed_route_required(app, "session list");
                return;
            }
            Some(CommandType::Builtin(BuiltinCommand::Resume { id })) => {
                if id.is_empty() {
                    if typed_session_list(app).await {
                        return;
                    }
                    typed_route_required(app, "session list");
                    return;
                }
                if typed_resume_session(app, id.clone()).await {
                    return;
                }
                typed_route_required(app, "resume session");
                return;
            }
            Some(CommandType::Builtin(BuiltinCommand::Compact)) => {
                let Some(session_id) = app.app_state.session_id.clone() else {
                    app.system_notices.push(
                        ChatRole::System,
                        "会话仍在初始化，请稍后重试 /compact".to_string(),
                    );
                    return;
                };
                if typed_compact_session(app, session_id.clone()).await {
                    return;
                }
                typed_route_required(app, "compact session");
                return;
            }
            Some(CommandType::Builtin(BuiltinCommand::Fork)) => {
                let Some(session_id) = app.app_state.session_id.clone() else {
                    app.system_notices.push(
                        ChatRole::System,
                        "当前会话尚未初始化，无法创建分支".to_string(),
                    );
                    return;
                };
                if typed_fork_session(app, session_id.clone(), app.app_state.cursor.sequence).await
                {
                    return;
                }
                typed_route_required(app, "fork session");
                return;
            }
            Some(CommandType::Builtin(BuiltinCommand::Rewind { prompt_index })) => {
                let Some(session_id) = app.app_state.session_id.clone() else {
                    app.system_notices.push(
                        ChatRole::System,
                        "当前会话尚未初始化，无法恢复工作区检查点".to_string(),
                    );
                    return;
                };
                if prompt_index.trim().is_empty() {
                    if typed_checkpoint_list(app, session_id.clone(), 64).await {
                        app.system_notices
                            .push(ChatRole::System, "查询工作区检查点中…".to_string());
                        return;
                    }
                    typed_route_required(app, "checkpoint list");
                    return;
                }
                let Ok(prompt_index) = prompt_index.parse::<u64>() else {
                    app.system_notices.push(
                        ChatRole::System,
                        "用法：/rewind <prompt-index>（必须使用 daemon 提供的检查点索引）"
                            .to_string(),
                    );
                    return;
                };
                if typed_workspace_rewind(app, session_id.clone(), prompt_index, None).await {
                    return;
                }
                typed_route_required(app, "workspace rewind");
                return;
            }
            Some(CommandType::Builtin(BuiltinCommand::Permissions)) => {
                app.system_notices.push(
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
                if typed_model_catalog(app).await {
                    return;
                }
                typed_route_required(app, "model catalog");
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
                        "default" => CollaborationMode::Default,
                        "plan" => CollaborationMode::Plan,
                        "auto" => CollaborationMode::Auto,
                        "sandbox" => CollaborationMode::Sandbox,
                        _ => {
                            app.system_notices.push(
                                ChatRole::System,
                                format!(
                                    "Unknown collaboration mode '{name}'. Available modes: default, plan, auto, sandbox"
                                ),
                            );
                            return;
                        }
                    }
                };
                if typed_set_collaboration_mode(app, mode).await {
                    app.system_notices.push(
                        ChatRole::System,
                        format!("Switching mode to: {}", mode.display_name()),
                    );
                    return;
                }
                typed_route_required(app, "collaboration mode");
                app.system_notices.push(
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
                    ["general"] => Ok(::contracts::ExecutionTargetSelection::general(
                        ::contracts::ExecutionTargetSource::UserCommand,
                    )),
                    ["robot", device] | ["robot", device, "simulation"] => {
                        ::contracts::ExecutionTargetSelection::robot(
                            *device,
                            ::contracts::types::embodiment::ExecutionEnvironment::Simulation,
                            ::contracts::ExecutionTargetSource::UserCommand,
                        )
                    }
                    ["robot", device, "hil"] => ::contracts::ExecutionTargetSelection::robot(
                        *device,
                        ::contracts::types::embodiment::ExecutionEnvironment::Hil,
                        ::contracts::ExecutionTargetSource::UserCommand,
                    ),
                    ["robot", device, "real"] => ::contracts::ExecutionTargetSelection::robot(
                        *device,
                        ::contracts::types::embodiment::ExecutionEnvironment::Real,
                        ::contracts::ExecutionTargetSource::UserCommand,
                    ),
                    _ => Err(
                        "usage: /target general | /target robot <device> [simulation|hil|real]"
                            .into(),
                    ),
                };
                match next {
                    Ok(selection) => {
                        let label = match &selection.target {
                            ::contracts::ExecutionTarget::General => "general".to_string(),
                            ::contracts::ExecutionTarget::Robot {
                                device_id,
                                environment,
                            } => format!("robot:{}/{}", device_id.0, environment.as_str()),
                        };
                        app.app_state
                            .select_execution_target_for_next_turn(selection);
                        app.system_notices.push(
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
                    app.system_notices.push(
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
                    app.system_notices.push(
                        ChatRole::System,
                        format!(
                            "Agent runtime {value} is required for the next turn; completion requires its terminal receipt"
                        ),
                    );
                }
                return;
            }
            Some(CommandType::Builtin(BuiltinCommand::Agents)) => {
                request_agent_inspector(app, None).await;
                return;
            }
            Some(CommandType::Builtin(BuiltinCommand::AgentDetail { id })) => {
                request_agent_inspector(app, Some(id)).await;
                return;
            }
            Some(CommandType::Builtin(BuiltinCommand::Skills)) => {
                if refresh_typed_skill_catalog(app).await {
                    app.app_state.last_error = None;
                } else {
                    typed_route_required(app, "skill catalog");
                }
                return;
            }
            Some(CommandType::Builtin(BuiltinCommand::Memory)) => {
                if typed_memory_snapshot(app).await {
                    return;
                }
                typed_route_required(app, "memory snapshot");
                return;
            }
            Some(CommandType::Builtin(BuiltinCommand::MemorySearch { query })) => {
                if query.is_empty() {
                    app.system_notices
                        .push(ChatRole::System, "用法：/memory search <query>".to_string());
                } else if !typed_memory_search(app, query.clone()).await {
                    typed_route_required(app, "memory search");
                }
                return;
            }
            Some(CommandType::Builtin(BuiltinCommand::MemoryStatus)) => {
                if !typed_memory_status(app).await {
                    typed_route_required(app, "memory status");
                }
                return;
            }
            Some(CommandType::Builtin(BuiltinCommand::SkillRun { name, args })) => {
                if !app.registry.is_skill(&name) {
                    app.system_notices.push(
                        ChatRole::System,
                        format!("Skill 不可用：{name}。请先运行 /skills 刷新目录。"),
                    );
                    return;
                }
                if typed_skill_run(app, name.clone(), args.clone()).await {
                    return;
                }
                typed_route_required(app, "skill invocation");
                return;
            }
            Some(CommandType::Builtin(BuiltinCommand::Interrupt)) => {
                if typed_cancel_active_turn(app).await {
                    return;
                }
                typed_route_required(app, "interrupt");
                app.system_notices
                    .push(ChatRole::System, "Interrupt sent".to_string());
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
                if typed_agent_profile_catalog(app).await {
                    return;
                }
                typed_route_required(app, "agent profile catalog");
                return;
            }
            Some(CommandType::Builtin(BuiltinCommand::ProfileSet { name })) => {
                if typed_set_agent_profile(app, name.clone()).await {
                    return;
                }
                typed_route_required(app, "agent profile");
                app.system_notices.push(
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
                app.system_notices.push(
                    ChatRole::System,
                    "No authoritative checkpoint diff is available for this turn".to_string(),
                );
                return;
            }
            Some(CommandType::Builtin(BuiltinCommand::Mention { path })) => {
                if path.is_empty() {
                    app.system_notices
                        .push(ChatRole::System, "用法: /mention <path>".to_string());
                } else {
                    app.input_buf = format!("@{path} ");
                    app.cursor = app.input_buf.len();
                    app.system_notices
                        .push(ChatRole::System, format!("已将 @{path} 加入输入框"));
                }
                return;
            }
            Some(CommandType::Builtin(BuiltinCommand::Input)) => {
                app.input_buf.push('\n');
                app.cursor = app.input_buf.len();
                app.system_notices.push(
                    ChatRole::System,
                    "多行输入已开启；继续输入，使用 Alt+Enter 提交".to_string(),
                );
                return;
            }
            Some(CommandType::Skill { name, args }) => {
                if typed_skill_run(app, name.clone(), args.clone()).await {
                    return;
                }
                typed_route_required(app, "skill invocation");
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
                app.system_notices
                    .push(ChatRole::System, format!("未知命令 /{name}。{hint}"));
                return;
            }
            None => {
                app.system_notices
                    .push(ChatRole::System, "无效命令".to_string());
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
    if send_to_daemon(app, &text).await {
        app.history.push(text.clone());
        app.system_notices.push(ChatRole::User, text);
        app.persist_input_state();
        // Assistant entry created lazily on first response delta so it renders
        // after any tool/reflection logs (ordering fix).
    } else {
        app.input_buf = text;
        app.cursor = app.input_buf.len();
        app.input_literal = literal_input;
        app.check_cjk();
        app.persist_input_state();
    }
}

async fn send_shell_to_daemon(app: &mut TuiModel, command: &str) {
    if typed_execute_shell(app, command).await {
        return;
    }
    typed_route_required(app, "shell command");
}

pub async fn send_to_daemon(app: &mut TuiModel, text: &str) -> bool {
    let mut requirements = app.turn_requirements.clone();
    if let Some(runtime_id) = app.next_agent_runtime.as_ref() {
        requirements.push(::contracts::TurnRequirement::InvokeAgentRuntime {
            runtime_id: runtime_id.clone(),
        });
    }
    // An Agent runtime preference is not a task classifier.  The effective
    // task kind is selected by the authenticated Runtime/Application route;
    // the presentation edge sends only the explicit requested task kind.
    let task_kind = app.requested_task_kind;
    if let Some(client) = app.controller.typed_gateway.as_mut() {
        let Some(session_id) = app.app_state.session_id.clone() else {
            app.system_notices.push(
                ChatRole::System,
                "Gateway refused prompt: session is not initialized".to_string(),
            );
            return false;
        };
        let required_agent_runtimes = requirements
            .iter()
            .filter_map(|requirement| match requirement {
                ::contracts::TurnRequirement::InvokeAgentRuntime { runtime_id } => {
                    Some(runtime_id.clone())
                }
                _ => None,
            })
            .collect();
        let requested_task_kind = task_kind.map(|_| "coding".to_string());
        // Effective permission is authenticated and resolved by the Host;
        // TUI sends only the typed preference "inherit".
        let requested_permission = app.requested_permission.clone();
        let requested_target = match &app.app_state.execution_target_for_submission().target {
            ::contracts::ExecutionTarget::General => RequestedExecutionTarget::General,
            ::contracts::ExecutionTarget::Robot {
                device_id,
                environment,
            } => RequestedExecutionTarget::Robot {
                device_id: device_id.0.clone(),
                environment: environment.as_str().to_owned(),
            },
        };
        let outcome = client
            .send(Command::SubmitPrompt(SubmitPromptRequest {
                session: SessionRef(session_id),
                content: text.to_owned(),
                workspace: Some(app.workspace.cwd().to_string_lossy().into_owned()),
                requested_target,
                requested_permission,
                required_agent_runtimes,
                requested_task_kind,
            }))
            .await;
        match outcome {
            Ok(GatewayCommandOutcome::Submitted { turn }) => {
                app.next_agent_runtime = None;
                mark_typed_turn_submitted(app, turn);
                true
            }
            Ok(other) => {
                app.system_notices.push(
                    ChatRole::System,
                    format!("Gateway rejected prompt receipt: {other:?}"),
                );
                false
            }
            Err(error) => {
                app.system_notices.push(
                    ChatRole::System,
                    format!("Gateway prompt rejected: {error}"),
                );
                app.app_state.last_error = Some(error.to_string());
                false
            }
        }
    } else {
        typed_route_required(app, "prompt submission");
        false
    }
}

/// Record the server-assigned turn reference before any live notification can
/// arrive.  The reference is opaque at this layer: it is only used to bind a
/// later canonical projection terminal to this submission, preventing a
/// previous completed turn from clearing the spinner for a new request.
fn mark_typed_turn_submitted(app: &mut TuiModel, turn: gateway::protocol::TurnRef) {
    app.active_turn_ref = Some(turn.0);
    app.turn_active = true;
    app.app_state.turn_active = true;
    app.app_state.active_turn_id = None;
    app.turn_cancel_requested = false;
    app.streaming = true;
    app.status.waiting = true;
    app.status.begin_elapsed(app.clock.mono_now());
    app.app_state.streaming = true;
    app.projection_polling = true;
    app.projection_next_poll_at = app.clock.mono_now();
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

    #[tokio::test]
    async fn shell_intent_requires_confirmation_then_fails_closed_without_typed_gateway() {
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
        assert!(app.pending_shell_confirmation.is_none());
        assert!(app
            .app_state
            .last_error
            .as_deref()
            .is_some_and(|error| error.contains("typed Gateway")));
    }

    #[tokio::test]
    async fn pending_robot_target_survives_stale_projection_when_gateway_is_unavailable() {
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
            "test-model".into(),
            std::sync::Arc::new(ClientClock::default()),
            workspace,
            vec![],
        );
        app.app_state.cursor.sequence = 10;
        let robot = ::contracts::ExecutionTargetSelection::robot(
            "robot-1",
            ::contracts::types::embodiment::ExecutionEnvironment::Simulation,
            ::contracts::ExecutionTargetSource::UserCommand,
        )
        .unwrap();
        app.app_state
            .select_execution_target_for_next_turn(robot.clone());

        app.app_state.reconcile_projected_execution_target(
            &::contracts::ExecutionTargetSelection::default(),
            1,
        );
        send_to_daemon(&mut app, "perform a safe simulation step").await;

        assert_eq!(app.app_state.execution_target_for_submission(), &robot);
        assert!(app
            .app_state
            .last_error
            .as_deref()
            .is_some_and(|error| error.contains("typed Gateway")));
    }

    #[tokio::test]
    async fn next_runtime_is_preserved_when_gateway_is_unavailable() {
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
            "test-model".into(),
            std::sync::Arc::new(ClientClock::default()),
            workspace,
            vec![],
        );
        app.next_agent_runtime = Some("pi-rpc".into());

        send_to_daemon(&mut app, "review the repository").await;

        assert_eq!(app.next_agent_runtime.as_deref(), Some("pi-rpc"));
        assert!(app
            .app_state
            .last_error
            .as_deref()
            .is_some_and(|error| error.contains("typed Gateway")));
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
