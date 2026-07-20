use std::path::PathBuf;

use clap::Args;

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
    pub fn executive_launch(&self) -> executive::host::launcher::WorkspaceLaunch {
        executive::host::launcher::WorkspaceLaunch {
            cwd: self.cwd.clone(),
            add_dirs: self.add_dirs.clone(),
        }
    }

    pub fn interact_launch(&self) -> interact::host::WorkspaceLaunch {
        interact::host::WorkspaceLaunch {
            cwd: self.cwd.clone(),
            add_dirs: self.add_dirs.clone(),
        }
    }
}
