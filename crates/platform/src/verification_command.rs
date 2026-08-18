//! Bounded host execution adapter for deterministic verification commands.

use std::collections::BTreeMap;
use std::path::Path;

use application::verification::{
    TrustedVerificationCommand, VerificationCommandExecutor, VerificationCommandOutput,
    VerificationWorkspaceReader,
};
use async_trait::async_trait;
use tokio_util::sync::CancellationToken;

use crate::bounded_command::{BoundedCommandRequest, BoundedCommandRunner};

#[derive(Debug, Clone, Default)]
pub struct PlatformVerificationCommandExecutor;

#[derive(Debug, Clone, Default)]
pub struct PlatformVerificationWorkspaceReader;

impl VerificationWorkspaceReader for PlatformVerificationWorkspaceReader {
    fn read_utf8(&self, worktree: &Path, path: &Path) -> Result<String, String> {
        let absolute = worktree.join(path);
        let metadata = absolute
            .symlink_metadata()
            .map_err(|error| error.to_string())?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err("verification input is not a regular file".into());
        }
        let canonical = absolute.canonicalize().map_err(|error| error.to_string())?;
        if !canonical.starts_with(worktree) {
            return Err("verification input escapes worktree".into());
        }
        std::fs::read_to_string(canonical).map_err(|error| error.to_string())
    }
}

pub fn resolve_executable(name: &str) -> anyhow::Result<std::path::PathBuf> {
    Ok(which::which(name)?)
}

#[async_trait]
impl VerificationCommandExecutor for PlatformVerificationCommandExecutor {
    async fn execute(
        &self,
        command: &TrustedVerificationCommand,
        worktree: &Path,
        environment: &BTreeMap<String, String>,
        output_cap_bytes: usize,
        cancel: CancellationToken,
    ) -> Result<VerificationCommandOutput, String> {
        BoundedCommandRunner
            .run(
                BoundedCommandRequest {
                    program: command.program.clone(),
                    args: command.args.clone(),
                    working_dir: worktree.to_owned(),
                    environment: environment.clone(),
                    stdin: None,
                    timeout: command.timeout,
                    stream_cap_bytes: output_cap_bytes,
                },
                cancel,
            )
            .await
            .map(|output| VerificationCommandOutput {
                exit_code: output.exit_code,
                elapsed_ms: output.elapsed_ms,
                timed_out: output.timed_out,
                cancelled: output.cancelled,
                stdout: output.stdout,
                stderr: output.stderr,
                stdout_bytes: output.stdout_bytes,
                stderr_bytes: output.stderr_bytes,
                stdout_truncated: output.stdout_truncated,
                stderr_truncated: output.stderr_truncated,
            })
            .map_err(|error| error.to_string())
    }
}
