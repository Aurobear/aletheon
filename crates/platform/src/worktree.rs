//! Host-owned cleanup for managed Git worktrees.

use std::collections::BTreeMap;
use std::path::Path;
use std::time::Duration;

use application::approval::ManagedWorktreeCleaner;
use async_trait::async_trait;
use tokio_util::sync::CancellationToken;

use crate::bounded_command::{BoundedCommandRequest, BoundedCommandRunner};

#[derive(Debug, Default)]
pub struct GitManagedWorktreeCleaner;

#[async_trait]
impl ManagedWorktreeCleaner for GitManagedWorktreeCleaner {
    async fn cleanup(
        &self,
        _: application::approval::CodingJobId,
        repository_root: &Path,
        worktree: &Path,
    ) -> anyhow::Result<()> {
        if !worktree.exists() {
            return Ok(());
        }
        let git = which::which("git")?;
        let output = BoundedCommandRunner
            .run(
                BoundedCommandRequest {
                    program: git,
                    args: vec![
                        "worktree".into(),
                        "remove".into(),
                        "--force".into(),
                        "--".into(),
                        worktree.to_string_lossy().into_owned(),
                    ],
                    working_dir: repository_root.to_owned(),
                    environment: BTreeMap::from([(
                        "PATH".into(),
                        "/usr/local/bin:/usr/bin:/bin".into(),
                    )]),
                    stdin: None,
                    timeout: Duration::from_secs(60),
                    stream_cap_bytes: 64 * 1024,
                },
                CancellationToken::new(),
            )
            .await?;
        anyhow::ensure!(
            output.exit_code == Some(0) && !output.timed_out && !output.cancelled,
            "git worktree remove failed: {}",
            output.stderr.trim()
        );
        Ok(())
    }
}
