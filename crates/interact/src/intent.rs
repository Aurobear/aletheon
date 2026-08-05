//! User-entry adapters for the canonical Fabric command contract.
//!
//! Syntax parsing stays in the individual presentation surfaces. Once a
//! surface selects an application command, it must produce a [`ClientIntent`]
//! here rather than constructing daemon business requests directly.

use fabric::contract::command::{
    ClientCommand, ClientIntent, ClientSurface, ExecuteShellIntent, StatusIntent,
    SubmitPromptIntent,
};
use fabric::permission::HostPermissionMode;
use fabric::protocol::client::ClientRpcRequest;
use fabric::{PrincipalId, SessionId, TaskKind, TurnRequirement, WorkspacePolicy};
use nix::unistd::Uid;

fn local_principal() -> PrincipalId {
    PrincipalId::local_uid(Uid::effective().as_raw())
}

pub(crate) struct PromptIntent<'a> {
    pub surface: ClientSurface,
    pub correlation_id: String,
    pub content: &'a str,
    pub session_id: Option<SessionId>,
    pub workspace: &'a WorkspacePolicy,
    pub requirements: Vec<TurnRequirement>,
    pub task_kind: Option<TaskKind>,
    pub permission_mode: HostPermissionMode,
}

pub(crate) fn submit_prompt(input: PromptIntent<'_>) -> ClientIntent {
    ClientIntent::v1(
        input.surface,
        local_principal(),
        input.correlation_id,
        ClientCommand::SubmitPrompt(SubmitPromptIntent {
            content: input.content.to_owned(),
            session_id: input.session_id,
            workspace: input.workspace.clone(),
            requirements: input.requirements,
            task_kind: input.task_kind,
            permission_mode: input.permission_mode,
        }),
    )
}

pub(crate) fn execute_shell(
    correlation_id: impl Into<String>,
    command: impl Into<String>,
    session_id: Option<SessionId>,
    workspace: &WorkspacePolicy,
    permission_mode: HostPermissionMode,
) -> ClientIntent {
    ClientIntent::v1(
        ClientSurface::Tui,
        local_principal(),
        correlation_id,
        ClientCommand::ExecuteShell(ExecuteShellIntent {
            command: command.into(),
            session_id,
            workspace: workspace.clone(),
            permission_mode,
        }),
    )
}

pub(crate) fn status(
    surface: ClientSurface,
    correlation_id: impl Into<String>,
    session_id: Option<SessionId>,
) -> ClientIntent {
    ClientIntent::v1(
        surface,
        local_principal(),
        correlation_id,
        ClientCommand::Status(StatusIntent { session_id }),
    )
}

pub(crate) fn rpc(intent: ClientIntent) -> ClientRpcRequest {
    ClientRpcRequest::Intent(intent)
}
