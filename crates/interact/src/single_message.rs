//! One-shot client adapter for the canonical typed command protocol.
//!
//! Argument parsing belongs to the top-level `aletheon` binary. This module
//! only projects an already-parsed message launch into typed Gateway commands
//! and renders the authoritative daemon projection.

use std::path::Path;

use anyhow::Result;
use gateway::client::{CommandOutcome, GatewayClient, UnixSocketTransport};
use gateway::protocol::{
    Command, Query, RequestSessionCreation, ResumeSessionReference, SessionSnapshotQuery,
    SubmitPromptRequest,
};

use crate::tui::response::deduplicate_consecutive_text as deduplicate_response;

/// Send one already-parsed prompt intent and print its terminal response.
pub(crate) async fn run(
    socket: &Path,
    message: &str,
    workspace: &::contracts::WorkspacePolicy,
    requirements: Vec<::contracts::TurnRequirement>,
    task_kind: Option<::contracts::TaskKind>,
    session_id: Option<::contracts::SessionId>,
    requested_permission: gateway::protocol::RequestedPermissionMode,
) -> Result<()> {
    run_typed(
        socket,
        message,
        workspace,
        requirements,
        task_kind,
        session_id,
        requested_permission,
    )
    .await
}

async fn run_typed(
    socket: &Path,
    message: &str,
    workspace: &::contracts::WorkspacePolicy,
    requirements: Vec<::contracts::TurnRequirement>,
    task_kind: Option<::contracts::TaskKind>,
    session_id: Option<::contracts::SessionId>,
    requested_permission: gateway::protocol::RequestedPermissionMode,
) -> Result<()> {
    let mut client = GatewayClient::new(
        UnixSocketTransport::connect(socket)
            .await
            .map_err(|error| anyhow::anyhow!("typed Gateway connection failed: {error}"))?,
    );
    let session = if let Some(session_id) = session_id {
        match client
            .send(Command::ResumeSession(ResumeSessionReference {
                reference: session_id.0,
            }))
            .await
            .map_err(|error| anyhow::anyhow!("typed Gateway resume failed: {error}"))?
        {
            CommandOutcome::Resumed { session } => session,
            _ => {
                return Err(anyhow::anyhow!(
                    "typed Gateway did not return a resume receipt"
                ))
            }
        }
    } else {
        match client
            .send(Command::CreateSession(RequestSessionCreation {
                principal_hint: None,
                workspace: Some(workspace.cwd().to_string_lossy().into_owned()),
            }))
            .await
            .map_err(|error| anyhow::anyhow!("typed Gateway create failed: {error}"))?
        {
            CommandOutcome::Created { session } => session,
            _ => {
                return Err(anyhow::anyhow!(
                    "typed Gateway did not return a create receipt"
                ))
            }
        }
    };
    // Permission is an authenticated Gateway/Host decision. The CLI sends
    // only the typed preference `inherit`; it never derives effective policy
    // from a local environment variable.
    let requested_task_kind = task_kind.map(|kind| match kind {
        ::contracts::TaskKind::Coding => "coding".to_owned(),
    });
    let submitted_turn = match client
        .send(Command::SubmitPrompt(SubmitPromptRequest {
            session: session.clone(),
            content: message.to_owned(),
            workspace: Some(workspace.cwd().to_string_lossy().into_owned()),
            requested_target: Default::default(),
            requested_permission,
            required_agent_runtimes: requirements
                .into_iter()
                .filter_map(|requirement| match requirement {
                    ::contracts::TurnRequirement::InvokeAgentRuntime { runtime_id } => {
                        Some(runtime_id)
                    }
                    _ => None,
                })
                .collect(),
            requested_task_kind,
        }))
        .await
        .map_err(|error| anyhow::anyhow!("typed Gateway submit failed: {error}"))?
    {
        CommandOutcome::Submitted { turn } => turn,
        _ => {
            return Err(anyhow::anyhow!(
                "typed Gateway did not return a turn receipt"
            ))
        }
    };
    // Reasoning models (deepseek-v4-*[1m]) plan silently before the first
    // output token; a complex analysis turn can take ~3 minutes end to end.
    // Match the daemon's provider request/idle budget (5 min) so the CLI does
    // not abandon a healthy long-reasoning turn.
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(300);
    let output = loop {
        let value = client
            .query(Query::SessionSnapshot(SessionSnapshotQuery {
                session: session.clone(),
                after_cursor: None,
                paged: false,
            }))
            .await
            .map_err(|error| anyhow::anyhow!("typed Gateway snapshot failed: {error}"))?;
        let snapshot: ::contracts::protocol::client::SessionReadSnapshot = serde_json::from_value(
            value
                .get("snapshot")
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("typed Gateway snapshot omitted payload"))?,
        )?;
        // A Session has one stable task projection for its whole lifetime;
        // each prompt adds a step to that task.  Do not wait for a new task
        // id or for the session-level phase to become terminal (the session
        // remains Active across successful turns).  The durable step phase is
        // the per-turn terminal authority.
        // Bind completion to the exact Runtime turn receipt returned by the
        // submit command.  A workspace keeps one long-lived task and may
        // already contain completed steps; matching the server-issued turn
        // receipt avoids confusing an older step with this request.
        let terminal = snapshot.tasks.iter().find_map(|task| {
            task.steps
                .iter()
                .find(|step| {
                    step.turn_id.0.to_string() == submitted_turn.0
                        && !matches!(step.phase, ::contracts::TaskPhase::Active)
                })
                .map(|step| (task, step))
        });
        if let Some(terminal) = terminal {
            let text = snapshot
                .items
                .iter()
                .filter(|item| item.turn_id.0.to_string() == submitted_turn.0)
                .rev()
                .find_map(|item| match &item.payload {
                    ::contracts::ItemPayload::AssistantMessage { content }
                    | ::contracts::ItemPayload::TurnSettlement { content, .. }
                    | ::contracts::ItemPayload::SystemNotice { content } => Some(content.clone()),
                    _ => None,
                })
                .unwrap_or_default();
            if terminal.1.phase == ::contracts::TaskPhase::Failed {
                anyhow::bail!("typed Gateway turn failed: {text}");
            }
            break text;
        }
        if tokio::time::Instant::now() >= deadline {
            anyhow::bail!("typed Gateway turn did not reach a durable terminal projection");
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    };
    println!("{}", deduplicate_response(&output));
    Ok(())
}
