//! Explicit operator-facing Memory Gateway commands. This client never opens
//! local or supplemental persistence directly.

use std::io::Read;
use std::path::{Path, PathBuf};

use ::contracts::protocol::memory::{
    MemoryObservationKindV1, MemoryObservationRequestV1, MemoryRecallRequestV1,
    MemoryReceiptGetRequestV1, MemorySensitivityV1, MemoryWorkspaceBindRequestV1,
    MemoryWorkspaceBindingSpecV1, MemoryWorkspacePreviewBindRequestV1,
    MemoryWorkspaceUnbindRequestV1, MAX_MEMORY_CONTENT_BYTES,
};

use crate::{
    MemoryBindingArgs, MemoryCommand, MemoryObservationKindArg, MemorySensitivityArg,
    MemoryWorkspaceCommand,
};

pub async fn run(command: &MemoryCommand, socket: Option<PathBuf>) -> anyhow::Result<()> {
    let value = match command {
        MemoryCommand::Observe {
            observation_id,
            working_dir,
            kind,
            content,
            session_id,
            turn_id,
            explicit_user_action,
            sensitivity,
            source_refs,
        } => {
            let content = read_content(content.as_deref())?;
            let mut client = interact::memory_client::MemoryClient::connect_gateway(socket).await?;
            serde_json::to_value(
                client
                    .observe(MemoryObservationRequestV1 {
                        observation_id: observation_id
                            .clone()
                            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
                        client_session_id: client_session_id(session_id),
                        client_turn_id: turn_id.clone(),
                        working_dir: absolute_working_dir(working_dir)?,
                        kind: (*kind).into(),
                        content,
                        occurred_at: None,
                        source_refs: source_refs.clone(),
                        sensitivity_hint: (*sensitivity).into(),
                        explicit_user_action: *explicit_user_action,
                    })
                    .await?,
            )?
        }
        MemoryCommand::Recall {
            query,
            working_dir,
            session_id,
            max_items,
            max_content_bytes,
            include_historical,
        } => {
            let mut client = interact::memory_client::MemoryClient::connect_gateway(socket).await?;
            serde_json::to_value(
                client
                    .recall(MemoryRecallRequestV1 {
                        request_id: uuid::Uuid::new_v4().to_string(),
                        client_session_id: client_session_id(session_id),
                        working_dir: absolute_working_dir(working_dir)?,
                        query: query.clone(),
                        max_items: *max_items,
                        max_content_bytes: *max_content_bytes,
                        include_historical: *include_historical,
                        requested_kinds: None,
                    })
                    .await?,
            )?
        }
        MemoryCommand::Receipt { durable_intake_id } => {
            let mut client = interact::memory_client::MemoryClient::connect_gateway(socket).await?;
            serde_json::to_value(
                client
                    .receipt(MemoryReceiptGetRequestV1 {
                        durable_intake_id: durable_intake_id.clone(),
                    })
                    .await?,
            )?
        }
        MemoryCommand::Workspace { sub } => {
            let mut client = interact::memory_client::MemoryClient::connect_admin(socket).await?;
            match sub {
                MemoryWorkspaceCommand::PreviewBind { binding } => serde_json::to_value(
                    client
                        .preview_bind(MemoryWorkspacePreviewBindRequestV1 {
                            working_dir: absolute_working_dir(&binding.working_dir)?,
                            binding: binding_spec(binding),
                        })
                        .await?,
                )?,
                MemoryWorkspaceCommand::Bind {
                    binding,
                    expected_capability_digest,
                } => serde_json::to_value(
                    client
                        .bind(MemoryWorkspaceBindRequestV1 {
                            working_dir: absolute_working_dir(&binding.working_dir)?,
                            binding: binding_spec(binding),
                            expected_capability_digest: expected_capability_digest.clone(),
                        })
                        .await?,
                )?,
                MemoryWorkspaceCommand::Unbind { working_dir } => serde_json::to_value(
                    client
                        .unbind(MemoryWorkspaceUnbindRequestV1 {
                            working_dir: absolute_working_dir(working_dir)?,
                        })
                        .await?,
                )?,
            }
        }
    };
    println!("{}", serde_json::to_string_pretty(&value)?);
    Ok(())
}

fn read_content(argument: Option<&str>) -> anyhow::Result<String> {
    if let Some(content) = argument {
        return Ok(content.to_owned());
    }
    let mut bytes = Vec::new();
    std::io::stdin()
        .take((MAX_MEMORY_CONTENT_BYTES + 1) as u64)
        .read_to_end(&mut bytes)?;
    anyhow::ensure!(
        bytes.len() <= MAX_MEMORY_CONTENT_BYTES,
        "memory observation stdin exceeds byte limit"
    );
    String::from_utf8(bytes).map_err(Into::into)
}

fn client_session_id(value: &Option<String>) -> String {
    value
        .clone()
        .unwrap_or_else(|| format!("aletheon-cli-{}", uuid::Uuid::new_v4()))
}

impl From<MemoryObservationKindArg> for MemoryObservationKindV1 {
    fn from(value: MemoryObservationKindArg) -> Self {
        match value {
            MemoryObservationKindArg::UserMessage => Self::UserMessage,
            MemoryObservationKindArg::AssistantMessage => Self::AssistantMessage,
            MemoryObservationKindArg::ToolOutcome => Self::ToolOutcome,
            MemoryObservationKindArg::TaskOutcome => Self::TaskOutcome,
            MemoryObservationKindArg::ExplicitNote => Self::ExplicitNote,
            MemoryObservationKindArg::Correction => Self::Correction,
            MemoryObservationKindArg::Feedback => Self::Feedback,
        }
    }
}

impl From<MemorySensitivityArg> for MemorySensitivityV1 {
    fn from(value: MemorySensitivityArg) -> Self {
        match value {
            MemorySensitivityArg::Public => Self::Public,
            MemorySensitivityArg::Internal => Self::Internal,
            MemorySensitivityArg::Confidential => Self::Confidential,
            MemorySensitivityArg::Restricted => Self::Restricted,
        }
    }
}

fn binding_spec(args: &MemoryBindingArgs) -> MemoryWorkspaceBindingSpecV1 {
    let read_destination_handles = if args.read_handles.is_empty() {
        vec![args.write_handle.clone()]
    } else {
        args.read_handles.clone()
    };
    MemoryWorkspaceBindingSpecV1 {
        backend_id: args.backend.clone(),
        write_destination_handle: args.write_handle.clone(),
        read_destination_handles,
        expected_write_source: args.write_source.clone(),
        expected_read_sources: args.read_sources.clone(),
        credential_ref: args
            .credential_ref
            .clone()
            .unwrap_or_else(|| format!("mcp-server:{}", args.write_handle)),
    }
}

fn absolute_working_dir(path: &Path) -> anyhow::Result<PathBuf> {
    Ok(if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    })
}
