//! Explicit operator-facing Memory Gateway commands. This client never opens
//! local or supplemental persistence directly.

use std::path::{Path, PathBuf};

use fabric::protocol::memory::{
    MemoryWorkspaceBindRequestV1, MemoryWorkspaceBindingSpecV1,
    MemoryWorkspacePreviewBindRequestV1, MemoryWorkspaceUnbindRequestV1,
};

use crate::{MemoryBindingArgs, MemoryCommand, MemoryWorkspaceCommand};

pub async fn run(command: &MemoryCommand, socket: Option<PathBuf>) -> anyhow::Result<()> {
    let MemoryCommand::Workspace { sub } = command;
    let mut client = interact::memory_client::MemoryClient::connect_admin(socket).await?;
    let value = match sub {
        MemoryWorkspaceCommand::PreviewBind { binding } => {
            let request = MemoryWorkspacePreviewBindRequestV1 {
                working_dir: absolute_working_dir(&binding.working_dir)?,
                binding: binding_spec(binding),
            };
            serde_json::to_value(client.preview_bind(request).await?)?
        }
        MemoryWorkspaceCommand::Bind {
            binding,
            expected_capability_digest,
        } => {
            let request = MemoryWorkspaceBindRequestV1 {
                working_dir: absolute_working_dir(&binding.working_dir)?,
                binding: binding_spec(binding),
                expected_capability_digest: expected_capability_digest.clone(),
            };
            serde_json::to_value(client.bind(request).await?)?
        }
        MemoryWorkspaceCommand::Unbind { working_dir } => {
            let request = MemoryWorkspaceUnbindRequestV1 {
                working_dir: absolute_working_dir(working_dir)?,
            };
            serde_json::to_value(client.unbind(request).await?)?
        }
    };
    println!("{}", serde_json::to_string_pretty(&value)?);
    Ok(())
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
    let path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };
    Ok(path)
}
