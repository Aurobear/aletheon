use std::path::{Path, PathBuf};

use clap::Args;
use fabric::{PermissionProfileId, WorkspacePolicy, WorkspaceResolveError, WorkspaceSelection};

/// Global workspace selection shared by interactive and non-interactive modes.
#[derive(Args, Clone, Debug, Default, Eq, PartialEq)]
pub struct WorkspaceArgs {
    /// Select the primary working directory.
    #[arg(short = 'C', long = "cd", global = true, value_name = "DIR")]
    pub cwd: Option<PathBuf>,

    /// Add a writable directory without changing the primary working directory.
    #[arg(long = "add-dir", global = true, value_name = "DIR")]
    pub add_dirs: Vec<PathBuf>,
}

impl WorkspaceArgs {
    pub fn resolve(
        &self,
        process_cwd: &Path,
        profile: &PermissionProfileId,
    ) -> Result<WorkspacePolicy, WorkspaceResolveError> {
        WorkspaceSelection::new(self.cwd.clone(), self.add_dirs.clone())
            .resolve_with_profile(process_cwd, profile)
    }
}
