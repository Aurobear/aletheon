use std::io;
use std::io::Write;
use std::sync::Arc;
use std::time::Duration;

use ::contracts::Clock;
use crossterm::event::Event;
use gateway::client::{
    CommandOutcome as GatewayCommandOutcome, GatewayClient, ReconnectPolicy, UnixSocketTransport,
};
use gateway::protocol::{
    CheckpointListQuery, Command, Query, RequestSessionCreation, RequestedExecutionTarget,
    RestoreWorkspaceCheckpoint, ResumeSessionReference, SessionRef, SessionSnapshotQuery,
    SubmitPromptRequest,
};
use ratatui::Terminal;

use crate::tui::host_time::ClientTimer;
use ::contracts::Timer;

use super::super::render::TuiRenderer;
use super::super::response::{format_models, format_sessions};
use super::super::term_compat::TermCaps;
use super::super::test_infra::{EventRecorder, FrameRecorder, TestConfig, TestInputReader};
use super::super::TuiModel;
use super::super::{
    command::{looks_like_command, BuiltinCommand, CommandType},
    registry::CommandRegistry,
};
use super::key_handler::{handle_key, handle_mouse};
use super::submit::{refresh_typed_skill_catalog, submit_message, typed_workspace_rewind};

fn scripted_followup_ready(
    initial_submit_pending: bool,
    submitted_script_line: bool,
    turn_active: bool,
    streaming: bool,
) -> bool {
    !initial_submit_pending && !submitted_script_line && !turn_active && !streaming
}

/// Establish a Session/Turn authority through the typed Gateway. Session pick
/// is a typed projection query; it never falls back to a legacy Session RPC in
/// the production path.
async fn initialize_typed_session(
    app: &mut TuiModel,
    initial_session: crate::host::InitialSession,
) -> bool {
    let Some(client) = app.controller.typed_gateway.as_mut() else {
        return false;
    };
    if matches!(initial_session, crate::host::InitialSession::Pick) {
        let result = client.query(Query::SessionList).await;
        match result {
            Ok(value) => {
                match serde_json::from_value::<::contracts::protocol::client::SessionListSnapshot>(
                    value,
                ) {
                    Ok(list)
                        if list.schema_version
                            == ::contracts::SESSION_READ_MODEL_SCHEMA_VERSION =>
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
                            Err(error) => app.system_notices.push(
                                super::super::chat::Role::System,
                                format!("无法打开会话列表：{error}"),
                            ),
                        }
                    }
                    Ok(list) => app.system_notices.push(
                        super::super::chat::Role::System,
                        format!(
                            "无法打开会话列表：unsupported schema {}",
                            list.schema_version
                        ),
                    ),
                    Err(error) => app.system_notices.push(
                        super::super::chat::Role::System,
                        format!("无法打开会话列表：{error}"),
                    ),
                }
            }
            Err(error) => app.system_notices.push(
                super::super::chat::Role::System,
                format!("Typed Gateway session list failed: {error}"),
            ),
        }
        return true;
    }
    let command = match initial_session {
        crate::host::InitialSession::New => Command::CreateSession(RequestSessionCreation {
            principal_hint: None,
            workspace: Some(app.workspace.cwd().to_string_lossy().into_owned()),
        }),
        crate::host::InitialSession::Resume(session) => {
            Command::ResumeSession(ResumeSessionReference {
                reference: session.0,
            })
        }
        crate::host::InitialSession::Pick => unreachable!("typed Pick handled above"),
    };
    let outcome = client.send(command).await;
    let session = match outcome {
        Ok(GatewayCommandOutcome::Created { session })
        | Ok(GatewayCommandOutcome::Resumed { session }) => session,
        Ok(other) => {
            app.system_notices.push(
                super::super::chat::Role::System,
                format!("Gateway session response invalid: {other:?}"),
            );
            return false;
        }
        Err(error) => {
            app.system_notices.push(
                super::super::chat::Role::System,
                format!("Gateway session initialization failed: {error}"),
            );
            return false;
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
    true
}

pub async fn run_app<B: ratatui::backend::Backend>(
    terminal: &mut Terminal<B>,
    typed_gateway: Option<GatewayClient<UnixSocketTransport>>,
    caps: TermCaps,
    model_name: String,
    test_config: TestConfig,
    is_test_mode: bool,
    clock: Arc<dyn Clock>,
    workspace: ::contracts::WorkspacePolicy,
    turn_requirements: Vec<::contracts::TurnRequirement>,
    task_kind: Option<::contracts::TaskKind>,
    initial_session: crate::host::InitialSession,
    requested_permission: gateway::protocol::RequestedPermissionMode,
) -> anyhow::Result<()> {
    #[cfg(test)]
    let mut app = TuiModel::new_with_gateway(
        None,
        typed_gateway,
        caps,
        model_name.clone(),
        clock,
        workspace,
        turn_requirements,
        requested_permission,
    );
    #[cfg(not(test))]
    let mut app = TuiModel::new_with_gateway(
        typed_gateway,
        caps,
        model_name.clone(),
        clock,
        workspace,
        turn_requirements,
        requested_permission,
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

    // // Populate completion/help from the daemon-owned Skill catalog. The
    // registry retains its last valid catalog if a later refresh fails.
    // The top-level CLI chooses whether this terminal owns a fresh session,
    // resumes an explicit session, or opens the daemon-backed history picker.
    let typed_session_initialized =
        initialize_typed_session(&mut app, initial_session.clone()).await;
    if !typed_session_initialized {
        // Every TUI mode must fail closed rather than silently falling back to
        // the retired Session RPC branch after a typed Gateway error.
        return Err(anyhow::anyhow!(
            "typed Gateway session initialization failed; legacy Session RPC fallback is retired"
        ));
    }
    anyhow::ensure!(
        app.controller.has_typed_gateway(),
        "typed Gateway transport is required; legacy Session RPC fallback is retired"
    );
    let _ = refresh_typed_skill_catalog(&mut app).await;
    // A scripted prompt must not race session.new/session.read. Until the
    // canonical projection selects the session, omitting session_id would make
    // the Host route the prompt onto a different thread.
    let mut initial_test_submit_pending =
        test_input.as_ref().is_some_and(|reader| reader.auto_submit);

    while app.running {
        if app.input_dirty && app.clock.mono_now().0 >= app.input_persist_at.0 {
            app.persist_input_state();
        }
        // Test timeout check
        if test_input.is_some()
            && (app.clock.mono_now().0 - test_start.0) >= test_timeout.as_millis() as u64
        {
            app.running = false;
            break;
        }

        // Resize handling
        // Redraw only when state changed. This keeps idle and scroll handling
        // cheap instead of rebuilding the complete terminal frame on a timer.
        if needs_redraw {
            // Presentation preparation is local model work; the renderer only
            // consumes the resulting state and paints it.
            super::super::app::key_handler::refresh_command_completion(&mut app);
            app.sync_system_notices();
            let view = app.view();
            TuiRenderer::draw(terminal, &view, &mut frame_recorder)?;
            app.mark_frame_rendered();
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
                    app.persist_input_state();
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
                        app.mark_input_dirty();
                        needs_redraw = true;
                    }
                    Event::Paste(text) => {
                        super::key_handler::insert_paste(&mut app, &text);
                        app.mark_input_dirty();
                        needs_redraw = true;
                    }
                    Event::Resize(_w, _h) => {
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
            ClientTimer.sleep(Duration::from_millis(200)).await;
        }

        // Production and tests drain only typed Gateway events. No local
        // JSON-RPC/socket compatibility pump is part of the TUI loop.
        needs_redraw |= drive_typed_events(&mut app, &mut event_recorder).await;
        drive_deferred_checkpoint_rewind(&mut app).await;
        // A projection response is a model mutation even when no live event
        // arrived.  Mark the frame dirty so a canonical terminal settlement
        // is rendered immediately instead of leaving a stale spinner on
        // screen until the next keypress.
        needs_redraw |= drive_session_projection(&mut app).await;
        drive_agent_inspector_refresh(&mut app).await;

        // Check if a turn just completed and we should auto-submit next line
        let mut submitted_script_line = false;
        if let Some(ref mut reader) = test_input {
            if initial_test_submit_pending
                && app.app_state.session_id.is_some()
                && !app.turn_active
                && !app.streaming
            {
                if let Some(line) = reader.next_line() {
                    submit_message(&mut app, line).await;
                    submitted_script_line = true;
                    // A local command (for example `/help`) may not produce a
                    // Gateway event.  Still render its model mutation before
                    // the scripted session decides it is complete.
                    needs_redraw = true;
                }
                initial_test_submit_pending = false;
            }
            // Use turn_active (set by turn_start, cleared by turn_done) instead
            // of streaming (which is also cleared by process_response and would
            // trigger premature auto-submit before the turn actually completes).
            // The first submitted request is streaming before its turn_start
            // event arrives. Treat that transport state as in-flight too, or
            // test mode can consume every scripted line and exit before the
            // daemon has admitted the first turn.
            if scripted_followup_ready(
                initial_test_submit_pending,
                submitted_script_line,
                app.turn_active,
                app.streaming,
            ) {
                if let Some(next) = reader.on_turn_done() {
                    // Small delay to let the UI update before next turn
                    ClientTimer.sleep(Duration::from_millis(100)).await;
                    submit_message(&mut app, next).await;
                    needs_redraw = true;
                }
            }
            // All inputs consumed and last turn done
            if reader.done && !app.turn_active && !app.streaming {
                // Give a local command (such as `/help`) one final render
                // before ending scripted mode; otherwise the loop would set
                // `running = false` immediately after reducing the command
                // and the recorded frame would contain only the startup view.
                if needs_redraw {
                    continue;
                }
                app.running = false;
            }
        }

        if app.streaming {
            app.status.tick_spinner();
            needs_redraw = true;
        }
    }

    if app.input_dirty {
        app.persist_input_state();
    }
    Ok(())
}

async fn drive_agent_inspector_refresh(app: &mut TuiModel) {
    let Some(inspector) = app.agent_inspector.as_ref() else {
        return;
    };
    if app.clock.mono_now().0 < app.agent_inspector_next_refresh_at.0 {
        return;
    }
    let focus = inspector.focus_id();
    super::submit::request_agent_inspector(app, focus).await;
}

async fn drive_typed_events(app: &mut TuiModel, recorder: &mut Option<EventRecorder>) -> bool {
    let mut changed = false;
    loop {
        let result = {
            let Some(client) = app.controller.typed_gateway.as_mut() else {
                return changed;
            };
            tokio::time::timeout(Duration::from_millis(1), client.next_event()).await
        };
        let event = match result {
            Ok(Ok(event)) => event,
            Ok(Err(gateway::protocol::ProtocolError::ConnectionClosed)) | Err(_) => break,
            Ok(Err(error)) => {
                app.system_notices.push(
                    super::super::chat::Role::System,
                    format!("Typed Gateway event rejected: {error}"),
                );
                break;
            }
        };
        changed = true;
        match event {
            gateway::protocol::Event::Progress(progress) => {
                if let Some(recorder) = recorder {
                    recorder.write(
                        &progress.payload,
                        app.app_state.session_id.as_deref(),
                    );
                }
                super::super::response::handle_event(app, &progress.payload);
            }
            gateway::protocol::Event::ApprovalRequested(approval) => {
                super::super::response::handle_typed_approval(app, approval);
            }
            _ => {}
        }
    }
    changed
}

async fn drive_deferred_checkpoint_rewind(app: &mut TuiModel) {
    let Some(deferred) = app.deferred_checkpoint_rewind.take() else {
        return;
    };
    if typed_workspace_rewind(
        app,
        deferred.parent_session_id.clone(),
        deferred.prompt_index,
        Some(deferred.child_session_id.clone()),
    )
    .await
    {
        return;
    }
    app.app_state.last_error = Some(
        "typed Gateway is unavailable for workspace rewind; refusing legacy fallback".to_string(),
    );
}

async fn drive_session_projection(app: &mut TuiModel) -> bool {
    let Some(session_id) = app
        .projection_target_session_id
        .clone()
        .or_else(|| app.app_state.session_id.clone())
    else {
        app.projection_session_id = None;
        app.projection_polling = false;
        return false;
    };
    if app.projection_request_in_flight {
        return false;
    }
    if app.controller.has_typed_gateway() {
        let after_cursor =
            (app.projection_session_id.is_some()).then(|| gateway::protocol::Cursor {
                sequence: app.app_state.cursor.sequence,
                event_id: app.app_state.cursor.event_id.clone(),
            });
        let query =
            gateway::protocol::Query::SessionSnapshot(gateway::protocol::SessionSnapshotQuery {
                session: gateway::protocol::SessionRef(session_id.clone()),
                after_cursor,
            });
        let result = {
            let client = app
                .controller
                .typed_gateway
                .as_mut()
                .expect("typed Gateway checked");
            match client.query(query.clone()).await {
                Ok(value) => Ok(value),
                Err(gateway::protocol::ProtocolError::ConnectionClosed) => {
                    // Reconnect only the presentation transport and replay
                    // the authenticated projection query after the last
                    // server-issued cursor. Runtime/session authority is not
                    // recreated here and no local event replay is inferred.
                    match client.reconnect(ReconnectPolicy::default()).await {
                        Ok(()) => client.query(query).await,
                        Err(error) => Err(error),
                    }
                }
                Err(error) => Err(error),
            }
        };
        match result {
            Ok(result) => {
                super::super::response::apply_typed_projection_result(app, &session_id, result);
            }
            Err(error) => {
                app.projection_request_in_flight = false;
                app.projection_polling = false;
                app.system_notices.push(
                    super::super::chat::Role::System,
                    format!("Typed Gateway projection query failed: {error}"),
                );
            }
        }
        app.projection_request_in_flight = false;
        return true;
    }
    // A typed Gateway is mandatory for production. There is deliberately no
    // raw projection request fallback: reconnect starts from the typed cursor
    // query above and any transport failure remains visible to the user.
    app.projection_request_in_flight = false;
    app.projection_polling = false;
    false
}

/// Simple line-based mode for non-TTY (piped) input. It uses the same typed
/// Gateway command/query path as the full TUI; there is no raw socket or
/// legacy Session RPC fallback.
pub async fn simple_line_mode(
    mut typed_gateway: Option<GatewayClient<UnixSocketTransport>>,
    _caps: TermCaps,
    model_name: String,
    _clock: Arc<dyn Clock>,
    workspace: ::contracts::WorkspacePolicy,
    turn_requirements: Vec<::contracts::TurnRequirement>,
    task_kind: Option<::contracts::TaskKind>,
    requested_permission: gateway::protocol::RequestedPermissionMode,
) -> anyhow::Result<()> {
    println!("aletheon v0.1.0 (model: {model_name})");
    println!("Type your message and press Enter. /quit to exit.\n");
    let mut client = typed_gateway
        .take()
        .ok_or_else(|| anyhow::anyhow!("typed Gateway transport is required for line mode"))?;
    let registry = CommandRegistry::new();
    let mut session = match client
        .send(Command::CreateSession(RequestSessionCreation {
            principal_hint: None,
            workspace: Some(workspace.cwd().to_string_lossy().into_owned()),
        }))
        .await?
    {
        GatewayCommandOutcome::Created { session } => Some(session),
        other => anyhow::bail!("typed line session initialization returned {other:?}"),
    };
    let stdin = io::stdin();
    loop {
        print!("> ");
        io::stdout().flush()?;
        let mut input = String::new();
        if stdin.read_line(&mut input)? == 0 {
            break;
        }
        let trimmed = input.trim();
        if trimmed.is_empty() {
            continue;
        }
        if !looks_like_command(trimmed) {
            let Some(current) = session.clone() else {
                anyhow::bail!("typed line session is not initialized");
            };
            typed_line_prompt(
                &mut client,
                current,
                trimmed,
                &workspace,
                &turn_requirements,
                task_kind,
                requested_permission.clone(),
                &stdin,
            )
            .await?;
            continue;
        }
        if trimmed == "/quit" || trimmed == "/exit" {
            break;
        }
        if trimmed == "/help" {
            println!("{}", registry.help_text());
            continue;
        }
        if !typed_line_command(&mut client, &mut session, trimmed, &registry, &workspace).await? {
            println!("Command is unavailable in typed line mode: {trimmed}");
        }
    }
    Ok(())
}

/// Handle the Session/Projection command subset in non-TTY mode without
/// falling back to legacy JSON-RPC. Unsupported extension commands continue
/// through the compatibility adapter until their typed Gateway contracts are
/// introduced.
async fn typed_line_command(
    client: &mut GatewayClient<UnixSocketTransport>,
    session: &mut Option<SessionRef>,
    trimmed: &str,
    registry: &CommandRegistry,
    workspace: &::contracts::WorkspacePolicy,
) -> anyhow::Result<bool> {
    if let Some(CommandType::Skill { name, args }) = registry.parse(trimmed) {
        let outcome = client
            .send(Command::InvokeSkill(gateway::protocol::SkillInvokeRequest {
                skill_id: name,
                user_args: args,
                session: session.clone(),
                workspace: Some(workspace.cwd().to_string_lossy().into_owned()),
            }))
            .await?;
        match outcome {
            GatewayCommandOutcome::Submitted { turn } => {
                println!("\nSkill turn admitted: {}\n", turn.0);
                return Ok(true);
            }
            other => anyhow::bail!("typed Skill invocation returned {other:?}"),
        }
    }
    let Some(CommandType::Builtin(command)) = registry.parse(trimmed) else {
        return Ok(false);
    };
    let command = match command {
        BuiltinCommand::Status => {
            let Some(current) = session.as_ref() else {
                anyhow::bail!("typed line session is not initialized")
            };
            let snapshot = typed_line_snapshot(client, current).await?;
            println!("\n{}\n", format_read_snapshot(&snapshot));
            return Ok(true);
        }
        BuiltinCommand::Sessions => {
            let value = client.query(Query::SessionList).await?;
            let list: ::contracts::protocol::client::SessionListSnapshot =
                serde_json::from_value(value)?;
            let sessions = serde_json::to_value(list.sessions)?;
            println!("\n{}\n", format_sessions(&sessions));
            return Ok(true);
        }
        BuiltinCommand::Resume { id } if !id.is_empty() => {
            Command::ResumeSession(ResumeSessionReference { reference: id })
        }
        BuiltinCommand::New => Command::CreateSession(RequestSessionCreation {
            principal_hint: None,
            workspace: Some(workspace.cwd().to_string_lossy().into_owned()),
        }),
        BuiltinCommand::Clear => {
            let Some(current) = session.as_ref() else {
                anyhow::bail!("typed line session is not initialized")
            };
            Command::ClearSession(current.clone())
        }
        BuiltinCommand::Compact => {
            let Some(current) = session.as_ref() else {
                anyhow::bail!("typed line session is not initialized")
            };
            Command::CompactSession(current.clone())
        }
        BuiltinCommand::Fork => {
            let Some(current) = session.as_ref() else {
                anyhow::bail!("typed line session is not initialized")
            };
            let snapshot = typed_line_snapshot(client, current).await?;
            Command::ForkSession(gateway::protocol::ForkSessionRequest {
                session: current.clone(),
                through_sequence: snapshot.through.sequence,
            })
        }
        BuiltinCommand::Rewind { prompt_index } => {
            let Some(current) = session.as_ref() else {
                anyhow::bail!("typed line session is not initialized")
            };
            if prompt_index.trim().is_empty() {
                let value = client
                    .query(Query::CheckpointList(CheckpointListQuery {
                        session: current.clone(),
                        limit: 64,
                    }))
                    .await?;
                println!("\n{}\n", serde_json::to_string_pretty(&value)?);
                return Ok(true);
            }
            let prompt_index = prompt_index
                .parse::<u64>()
                .map_err(|_| anyhow::anyhow!("usage: /rewind <prompt-index>"))?;
            Command::RestoreWorkspaceCheckpoint(RestoreWorkspaceCheckpoint {
                session: current.clone(),
                prompt_index,
            })
        }
        BuiltinCommand::Memory => {
            let Some(current) = session.as_ref() else {
                anyhow::bail!("typed line session is not initialized")
            };
            let value = client
                .query(Query::MemorySnapshot(
                    gateway::protocol::MemorySnapshotQuery {
                        session: current.clone(),
                        memory_type: "all".into(),
                        limit: 20,
                    },
                ))
                .await?;
            println!(
                "\n{}\n",
                value
                    .get("content")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("(memory snapshot is empty)")
            );
            return Ok(true);
        }
        BuiltinCommand::Skills => {
            let value = client
                .query(Query::SkillCatalog(gateway::protocol::SkillCatalogQuery))
                .await?;
            let skills = value
                .get("skills")
                .and_then(serde_json::Value::as_array)
                .cloned()
                .unwrap_or_default();
            for skill in skills {
                let name = skill
                    .get("name")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("?");
                let description = skill
                    .get("description")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("");
                println!("/{name} {description}");
            }
            return Ok(true);
        }
        BuiltinCommand::Model => {
            let value = client
                .query(Query::ModelCatalog(gateway::protocol::ModelCatalogQuery))
                .await?;
            println!("\n{}\n", format_models(&value));
            return Ok(true);
        }
        BuiltinCommand::Mode { name } => {
            let mode = match name.as_str() {
                "" | "default" => gateway::protocol::RequestedCollaborationMode::Default,
                "plan" => gateway::protocol::RequestedCollaborationMode::Plan,
                "auto" => gateway::protocol::RequestedCollaborationMode::Auto,
                "sandbox" => gateway::protocol::RequestedCollaborationMode::Sandbox,
                other => anyhow::bail!("unsupported collaboration mode: {other}"),
            };
            Command::SetCollaborationMode(gateway::protocol::SetCollaborationModeRequest { mode })
        }
        BuiltinCommand::Profile => {
            let value = client
                .query(Query::AgentProfileCatalog(
                    gateway::protocol::AgentProfileCatalogQuery,
                ))
                .await?;
            println!(
                "\n{}\n",
                serde_json::to_string_pretty(
                    value
                        .get("profiles")
                        .unwrap_or(&serde_json::Value::Array(Vec::new()))
                )?
            );
            return Ok(true);
        }
        BuiltinCommand::ProfileSet { name } => {
            Command::SetAgentProfile(gateway::protocol::SetAgentProfileRequest { profile: name })
        }
        BuiltinCommand::SkillRun { name, args } => {
            Command::InvokeSkill(gateway::protocol::SkillInvokeRequest {
                skill_id: name,
                user_args: args,
                session: session.clone(),
                workspace: Some(workspace.cwd().to_string_lossy().into_owned()),
            })
        }
        BuiltinCommand::MemoryStatus => {
            let value = client
                .query(Query::MemoryStatus(gateway::protocol::MemoryStatusQuery))
                .await?;
            println!(
                "\n{}\n",
                super::super::response::format_memory_status(&value)
            );
            return Ok(true);
        }
        BuiltinCommand::MemorySearch { query } => {
            if query.trim().is_empty() {
                anyhow::bail!("usage: /memory search <query>");
            }
            let value = client
                .query(Query::MemorySearch(gateway::protocol::MemorySearchQuery {
                    query,
                    session: session.as_ref().map(|value| value.0.clone()),
                }))
                .await?;
            println!(
                "\n{}\n",
                super::super::response::format_memory_facts(&value)
            );
            return Ok(true);
        }
        BuiltinCommand::Interrupt => {
            let Some(current) = session.as_ref() else {
                anyhow::bail!("typed line session is not initialized");
            };
            Command::CancelActiveTurn(gateway::protocol::CancelActiveTurn {
                session: current.clone(),
            })
        }
        _ => return Ok(false),
    };
    let outcome = client.send(command).await?;
    let next = match outcome {
        GatewayCommandOutcome::Created { session }
        | GatewayCommandOutcome::Resumed { session }
        | GatewayCommandOutcome::SessionUpdated { session }
        | GatewayCommandOutcome::Forked { session } => session,
        GatewayCommandOutcome::ModelUpdated { model } => {
            println!("\nModel: {model}\n");
            return Ok(true);
        }
        GatewayCommandOutcome::CollaborationModeUpdated { mode } => {
            println!("\nMode: {mode:?}\n");
            return Ok(true);
        }
        GatewayCommandOutcome::AgentProfileUpdated { profile } => {
            println!("\nProfile: {profile}\n");
            return Ok(true);
        }
        GatewayCommandOutcome::Submitted { turn } => {
            println!("\nTurn admitted: {}\n", turn.0);
            return Ok(true);
        }
        GatewayCommandOutcome::WorkspaceRestored { outcome } => {
            println!("\nWorkspace rewind: {outcome:?}\n");
            return Ok(true);
        }
        GatewayCommandOutcome::Cancelled => {
            println!("\nCancellation requested.\n");
            return Ok(true);
        }
        other => anyhow::bail!("typed line command returned unexpected receipt: {other:?}"),
    };
    println!("\nSession: {}\n", next.0);
    *session = Some(next);
    Ok(true)
}

async fn typed_line_snapshot(
    client: &mut GatewayClient<UnixSocketTransport>,
    session: &SessionRef,
) -> anyhow::Result<::contracts::protocol::client::SessionReadSnapshot> {
    let result = client
        .query(Query::SessionSnapshot(SessionSnapshotQuery {
            session: session.clone(),
            after_cursor: None,
        }))
        .await
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;
    let snapshot = result
        .get("snapshot")
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("typed Gateway query omitted snapshot"))?;
    serde_json::from_value(snapshot).map_err(anyhow::Error::from)
}

async fn typed_line_prompt(
    client: &mut GatewayClient<UnixSocketTransport>,
    session: SessionRef,
    content: &str,
    workspace: &::contracts::WorkspacePolicy,
    turn_requirements: &[::contracts::TurnRequirement],
    task_kind: Option<::contracts::TaskKind>,
    requested_permission: gateway::protocol::RequestedPermissionMode,
    stdin: &std::io::Stdin,
) -> anyhow::Result<()> {
    let baseline = typed_line_snapshot(client, &session).await?;
    let required_agent_runtimes = turn_requirements
        .iter()
        .filter_map(|requirement| match requirement {
            ::contracts::TurnRequirement::InvokeAgentRuntime { runtime_id } => {
                Some(runtime_id.clone())
            }
            _ => None,
        })
        .collect();
    client
        .send(Command::SubmitPrompt(SubmitPromptRequest {
            session: session.clone(),
            content: content.to_owned(),
            workspace: Some(workspace.cwd().to_string_lossy().into_owned()),
            requested_target: RequestedExecutionTarget::Automatic,
            requested_permission: requested_permission.clone(),
            required_agent_runtimes,
            requested_task_kind: task_kind.map(|_| "coding".to_owned()),
        }))
        .await
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;

    let result = tokio::time::timeout(Duration::from_secs(120), async {
        loop {
            match tokio::time::timeout(Duration::from_millis(10), client.next_event()).await {
                Ok(Ok(gateway::protocol::Event::ApprovalRequested(approval))) => {
                    println!(
                        "\n⚠  Approval required [{}] {}\n   {}\n   Approve? [y]es / [N]o: ",
                        approval.risk_level, approval.tool, approval.action_summary
                    );
                    io::stdout().flush()?;
                    let mut line = String::new();
                    let approved = match stdin.read_line(&mut line) {
                        Ok(_) => matches!(line.trim().to_lowercase().as_str(), "y" | "yes"),
                        Err(_) => false,
                    };
                    client
                        .send(Command::SubmitApproval(
                            gateway::protocol::SubmitApprovalChoice {
                                session: approval.session,
                                choice_id: approval.choice_id,
                                approved,
                                version: 0,
                                reason: None,
                            },
                        ))
                        .await
                        .map_err(|error| anyhow::anyhow!(error.to_string()))?;
                }
                Ok(Ok(gateway::protocol::Event::Progress(progress))) => {
                    if let Some(message) = progress.payload.get("message").and_then(|v| v.as_str())
                    {
                        eprintln!("{message}");
                    }
                }
                Ok(Ok(_)) | Err(_) => {}
                Ok(Err(error)) => {
                    return Err(anyhow::anyhow!(error.to_string()));
                }
            }

            let snapshot = typed_line_snapshot(client, &session).await?;
            let terminal = snapshot.through.sequence > baseline.through.sequence
                && snapshot.tasks.iter().any(|task| {
                    task.active_turn_id.is_none()
                        && !matches!(task.phase, ::contracts::TaskPhase::Active)
                });
            if !terminal {
                tokio::time::sleep(Duration::from_millis(50)).await;
                continue;
            }
            if let Some(item) = snapshot
                .items
                .iter()
                .filter(|item| {
                    matches!(
                        item.payload,
                        ::contracts::ItemPayload::AssistantMessage { .. }
                    )
                })
                .max_by_key(|item| item.sequence)
            {
                if let ::contracts::ItemPayload::AssistantMessage { content } = &item.payload {
                    println!("\n{content}\n");
                }
            }
            if let Some(task) = snapshot.tasks.iter().find(|task| {
                task.active_turn_id.is_none()
                    && !matches!(task.phase, ::contracts::TaskPhase::Completed)
            }) {
                eprintln!("turn settled with {:?}", task.phase);
            }
            return Ok::<(), anyhow::Error>(());
        }
    })
    .await;
    match result {
        Ok(result) => result,
        Err(_) => Err(anyhow::anyhow!("typed Gateway prompt timed out")),
    }
}

fn format_read_snapshot(snapshot: &::contracts::protocol::client::SessionReadSnapshot) -> String {
    let mut lines = vec![format!(
        "Session {} ({} durable items)",
        snapshot.session.id.0,
        snapshot.items.len()
    )];
    for item in &snapshot.items {
        match &item.payload {
            ::contracts::ItemPayload::UserMessage { content, .. } => {
                lines.push(format!("user: {content}"));
            }
            ::contracts::ItemPayload::AssistantMessage { content } => {
                lines.push(format!("assistant: {content}"));
            }
            ::contracts::ItemPayload::SystemNotice { content } => {
                lines.push(format!("system: {content}"));
            }
            _ => {}
        }
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn typed_session_initialization_requires_the_gateway_transport() {
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
            Arc::new(crate::tui::host_time::ClientClock::default()),
            workspace,
            Vec::new(),
        );
        assert!(!initialize_typed_session(&mut app, crate::host::InitialSession::New).await);
        assert!(app.app_state.session_id.is_none());
    }

    #[test]
    fn scripted_prompt_waits_for_initial_session_projection() {
        assert!(!scripted_followup_ready(true, false, false, false));
        assert!(!scripted_followup_ready(false, true, false, false));
        assert!(!scripted_followup_ready(false, false, true, false));
        assert!(!scripted_followup_ready(false, false, false, true));
        assert!(scripted_followup_ready(false, false, false, false));
    }

    #[tokio::test]
    async fn open_agent_inspector_refreshes_automatically_without_duplicate_requests() {
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
            Arc::new(crate::tui::host_time::ClientClock::default()),
            workspace,
            Vec::new(),
        );
        app.agent_inspector = Some(
            crate::tui::agent_inspector::AgentInspector::from_json(
                &serde_json::json!([{
                    "id": "agent-1",
                    "task": "inspect repository",
                    "status": "running",
                    "runtime_id": "pi-coder",
                    "profile_id": "pi",
                    "snapshot": {
                        "handle": {
                            "agent_id": "00000000-0000-0000-0000-000000000001",
                            "root_agent_id": "00000000-0000-0000-0000-000000000002",
                            "parent_agent_id": null,
                            "process_id": "00000000-0000-0000-0000-000000000003",
                            "operation_id": "00000000-0000-0000-0000-000000000004",
                            "runtime_id": "pi-coder",
                            "profile_id": "pi"
                        },
                        "status": "running",
                        "result": null,
                        "created_at_ms": 1,
                        "started_at_ms": 2,
                        "ended_at_ms": null,
                        "last_error": null
                    },
                    "timeline": [{"sequence": 1, "kind": "started", "detail": null}]
                }]),
                None,
            )
            .unwrap(),
        );
        app.agent_inspector_next_refresh_at = ::contracts::MonoTime(0);

        drive_agent_inspector_refresh(&mut app).await;
        assert!(app
            .app_state
            .last_error
            .as_deref()
            .is_some_and(|error| error.contains("typed Gateway")));
    }

    #[test]
    fn u_tui_006_no_color_keyboard_and_text_line_mode_cover_the_core_journey() {
        let caps = TermCaps {
            color: false,
            true_color: false,
            unicode: false,
            width: 80,
            height: 24,
        };
        let theme = caps.theme();
        assert!([
            theme.accent,
            theme.text,
            theme.text_muted,
            theme.background,
            theme.error,
            theme.warning,
            theme.success,
        ]
        .iter()
        .all(|color| *color == ratatui::style::Color::Reset));

        let registry = CommandRegistry::new();
        let help = registry.help_text();
        assert!(help.contains("/sessions"));
        assert!(!help.contains('\u{1b}'));

        let text_snapshot =
            format_read_snapshot(&::contracts::protocol::client::SessionReadSnapshot {
                schema_version: ::contracts::SESSION_READ_MODEL_SCHEMA_VERSION,
                session: ::contracts::SessionRecord {
                    schema_version: ::contracts::SESSION_SCHEMA_VERSION,
                    id: ::contracts::SessionId("session-7".into()),
                    parent: None,
                    created_at_ms: 1,
                    status: ::contracts::SessionStatus::Active,
                },
                through: ::contracts::protocol::client::EventCursor::origin(),
                items: Vec::new(),
                tasks: Vec::new(),
                activities: Vec::new(),
            });
        assert_eq!(text_snapshot, "Session session-7 (0 durable items)");
    }
}
