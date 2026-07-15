use anyhow::Result;
use async_trait::async_trait;
use fabric::Clock;
use fabric::Timer;
use std::sync::Arc;
use std::time::Duration;
use std::{collections::BTreeMap, path::Path, path::PathBuf};
use tracing::{info, warn};

use crate::sandbox::{
    IsolationLevel, SandboxBackend, SandboxCapabilities, SandboxCommand, SandboxConfig,
    SandboxResult,
};

/// Bubblewrap-based sandbox backend — full namespace isolation.
/// Requires: bwrap binary, user namespace support.
pub struct BubblewrapBackend {
    bwrap_path: String,
    clock: Arc<dyn Clock>,
}

impl BubblewrapBackend {
    /// Probe for bubblewrap availability.
    pub fn probe(clock: Arc<dyn Clock>) -> Option<Self> {
        let bwrap_path = which::which("bwrap").ok()?;
        let path_str = bwrap_path.to_string_lossy().to_string();

        match std::process::Command::new(&bwrap_path)
            .arg("--version")
            .output()
        {
            Ok(output) => {
                let version = String::from_utf8_lossy(&output.stdout);
                info!(version = version.trim(), path = %path_str, "Bubblewrap detected");
                Some(Self {
                    bwrap_path: path_str,
                    clock,
                })
            }
            Err(e) => {
                warn!(error = %e, "Failed to run bwrap --version");
                None
            }
        }
    }

    /// Async probe for runtime bootstrap paths; avoids blocking an executor
    /// thread while validating the configured launcher.
    pub async fn probe_async(clock: Arc<dyn Clock>) -> Option<Self> {
        let bwrap_path = which::which("bwrap").ok()?;
        let path_str = bwrap_path.to_string_lossy().to_string();
        match tokio::process::Command::new(&bwrap_path)
            .arg("--version")
            .output()
            .await
        {
            Ok(output) if output.status.success() => {
                let version = String::from_utf8_lossy(&output.stdout);
                info!(version = version.trim(), path = %path_str, "Bubblewrap detected");
                Some(Self {
                    bwrap_path: path_str,
                    clock,
                })
            }
            Ok(output) => {
                warn!(status = ?output.status.code(), "bwrap --version failed");
                None
            }
            Err(error) => {
                warn!(%error, "Failed to run bwrap --version");
                None
            }
        }
    }

    fn build_args(&self, cmd: &str, config: &SandboxConfig) -> Vec<String> {
        self.build_argv_args(Path::new("/bin/bash"), &["-c".into(), cmd.into()], config)
    }

    fn build_argv_args(
        &self,
        program: &Path,
        command_args: &[String],
        config: &SandboxConfig,
    ) -> Vec<String> {
        let mut args = vec![
            "--die-with-parent".into(),
            "--unshare-pid".into(),
            "--unshare-ipc".into(),
            "--unshare-net".into(), // Default: no network
            "--clearenv".into(),
        ];

        // Bind entire root read-only FIRST, then mount --dev and --proc
        // on top so the fresh devtmpfs is NOT overwritten by the
        // recursive --ro-bind of the host root (MS_REC crosses submount
        // boundaries and would replace a previously-mounted devtmpfs).
        args.push("--ro-bind".into());
        args.push("/".into());
        args.push("/".into());

        // Fresh devtmpfs and proc on top of the read-only root
        args.push("--dev".into());
        args.push("/dev".into());
        args.push("--proc".into());
        args.push("/proc".into());

        // bwrap's --dev creates a fresh devtmpfs, but the device nodes
        // (including /dev/null) can be unwritable for non-root users.
        // Explicitly dev-bind the host's /dev/null over the devtmpfs copy
        // so the sandboxed process can redirect output to /dev/null.
        args.push("--dev-bind".into());
        args.push("/dev/null".into());
        args.push("/dev/null".into());

        // Writable working directory
        args.push("--bind".into());
        args.push(config.working_dir.clone());
        args.push(config.working_dir.clone());

        // Re-protect repository metadata and local secret stores after the
        // working directory bind. Later bwrap mounts override earlier ones.
        for relative in [".git", ".env", ".aletheon", ".ssh"] {
            let protected = Path::new(&config.working_dir).join(relative);
            if protected.exists() {
                args.push("--ro-bind".into());
                args.push(protected.to_string_lossy().into_owned());
                args.push(protected.to_string_lossy().into_owned());
            }
        }

        // Tmpfs for /tmp
        args.push("--tmpfs".into());
        args.push("/tmp".into());

        // Environment variables
        for (key, value) in &config.env_vars {
            args.push("--setenv".into());
            args.push(key.clone());
            args.push(value.clone());
        }

        // The command to execute
        args.push("--".into());
        args.push(program.to_string_lossy().into_owned());
        args.extend(command_args.iter().cloned());

        args
    }
}

#[async_trait]
impl SandboxBackend for BubblewrapBackend {
    fn name(&self) -> &str {
        "bubblewrap"
    }

    fn isolation_level(&self) -> IsolationLevel {
        IsolationLevel::Namespace
    }

    fn is_available(&self) -> bool {
        which::which("bwrap").is_ok()
    }

    fn capabilities(&self) -> SandboxCapabilities {
        SandboxCapabilities {
            filesystem_isolation: true,
            network_isolation: true,
            resource_limits: true,
            seccomp_filter: true,
            limitations: vec![
                "Requires user namespace support".into(),
                "Some paths may not be accessible in sandbox".into(),
            ],
        }
    }

    fn wrap_argv(
        &self,
        program: &Path,
        args: &[String],
        config: &SandboxConfig,
    ) -> Result<SandboxCommand> {
        if !program.is_absolute() {
            anyhow::bail!("bubblewrap requires an absolute command path");
        }
        Ok(SandboxCommand {
            program: PathBuf::from(&self.bwrap_path),
            args: self.build_argv_args(program, args, config),
            environment: BTreeMap::new(),
        })
    }

    async fn execute(
        &self,
        cmd: &str,
        config: &SandboxConfig,
        timeout: Duration,
    ) -> Result<SandboxResult> {
        info!(command = cmd, "Executing command in bubblewrap sandbox");

        let args = self.build_args(cmd, config);
        let start = self.clock.mono_now();

        let result = aletheon_kernel::chronos::SystemTimer
            .timeout(timeout, async {
                tokio::process::Command::new(&self.bwrap_path)
                    .args(&args)
                    .current_dir(&config.working_dir)
                    .output()
                    .await
            })
            .await;

        let elapsed = self.clock.mono_now().0.saturating_sub(start.0);

        match result {
            Ok(Ok(output)) => Ok(SandboxResult {
                stdout: String::from_utf8_lossy(&output.stdout).to_string(),
                stderr: String::from_utf8_lossy(&output.stderr).to_string(),
                exit_code: output.status.code().unwrap_or(-1),
                backend_used: "bubblewrap".to_string(),
                isolation_level: IsolationLevel::Namespace,
                elapsed_ms: elapsed,
            }),
            Ok(Err(e)) => Err(anyhow::anyhow!("Bubblewrap execution failed: {}", e)),
            Err(_) => Ok(SandboxResult {
                stdout: String::new(),
                stderr: format!("Command timed out after {} seconds", timeout.as_secs()),
                exit_code: -1,
                backend_used: "bubblewrap".to_string(),
                isolation_level: IsolationLevel::Namespace,
                elapsed_ms: elapsed,
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aletheon_kernel::chronos::TestClock;
    use std::collections::HashMap;

    #[test]
    fn argv_wrapper_is_networkless_and_only_worktree_is_writable() {
        let backend = BubblewrapBackend {
            bwrap_path: "/usr/bin/bwrap".into(),
            clock: Arc::new(TestClock::default()),
        };
        let config = SandboxConfig {
            working_dir: "/managed/job-1".into(),
            env_vars: HashMap::from([("PATH".into(), "/usr/bin:/bin".into())]),
        };
        let wrapped = backend
            .wrap_argv(
                Path::new("/opt/pi/bin/pi"),
                &["--task".into(), "literal;not-shell".into()],
                &config,
            )
            .unwrap();
        assert_eq!(wrapped.program, PathBuf::from("/usr/bin/bwrap"));
        assert!(wrapped
            .args
            .windows(3)
            .any(|items| items == ["--ro-bind", "/", "/"]));
        assert!(wrapped
            .args
            .windows(3)
            .any(|items| { items == ["--bind", "/managed/job-1", "/managed/job-1"] }));
        assert!(wrapped.args.iter().any(|arg| arg == "--unshare-net"));
        assert!(wrapped.args.iter().any(|arg| arg == "--clearenv"));
        let separator = wrapped.args.iter().position(|arg| arg == "--").unwrap();
        assert_eq!(
            &wrapped.args[separator + 1..],
            ["/opt/pi/bin/pi", "--task", "literal;not-shell"]
        );
        assert!(!wrapped.args.iter().any(|arg| arg == "-c"));
    }

    #[test]
    fn protected_metadata_is_rebound_after_writable_worktree() {
        let temp = tempfile::tempdir().unwrap();
        let work = temp.path().join("project");
        std::fs::create_dir_all(work.join(".git")).unwrap();
        let backend = BubblewrapBackend {
            bwrap_path: "/usr/bin/bwrap".into(),
            clock: Arc::new(TestClock::default()),
        };
        let config = SandboxConfig {
            working_dir: work.to_string_lossy().into_owned(),
            env_vars: Default::default(),
        };
        let args = backend.build_argv_args(Path::new("/bin/true"), &[], &config);
        let writable = args
            .windows(3)
            .position(|items| items[0] == "--bind" && items[1] == config.working_dir)
            .unwrap();
        let git = work.join(".git").to_string_lossy().into_owned();
        let protected = args
            .windows(3)
            .position(|items| items[0] == "--ro-bind" && items[1] == git)
            .unwrap();
        assert!(protected > writable);
    }
}
