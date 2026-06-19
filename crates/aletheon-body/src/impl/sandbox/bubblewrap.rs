use anyhow::Result;
use async_trait::async_trait;
use std::time::{Duration, Instant};
use tracing::{info, warn};

use crate::r#impl::sandbox::{
    IsolationLevel, SandboxBackend, SandboxCapabilities, SandboxConfig, SandboxResult,
};

/// Bubblewrap-based sandbox backend — full namespace isolation.
/// Requires: bwrap binary, user namespace support.
pub struct BubblewrapBackend {
    bwrap_path: String,
}

impl BubblewrapBackend {
    /// Probe for bubblewrap availability.
    pub fn probe() -> Option<Self> {
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
                })
            }
            Err(e) => {
                warn!(error = %e, "Failed to run bwrap --version");
                None
            }
        }
    }

    fn build_args(&self, cmd: &str, config: &SandboxConfig) -> Vec<String> {
        let mut args = vec![
            "--die-with-parent".into(),
            "--unshare-pid".into(),
            "--unshare-ipc".into(),
            "--unshare-net".into(), // Default: no network
        ];

        // Bind entire root read-only — handles usr-merge symlinks correctly
        args.push("--ro-bind".into());
        args.push("/".into());
        args.push("/".into());

        // Proc and dev
        args.push("--proc".into());
        args.push("/proc".into());
        args.push("--dev".into());
        args.push("/dev".into());

        // Writable working directory
        args.push("--bind".into());
        args.push(config.working_dir.clone());
        args.push(config.working_dir.clone());

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
        args.push("/bin/bash".into());
        args.push("-c".into());
        args.push(cmd.to_string());

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

    async fn execute(
        &self,
        cmd: &str,
        config: &SandboxConfig,
        timeout: Duration,
    ) -> Result<SandboxResult> {
        info!(command = cmd, "Executing command in bubblewrap sandbox");

        let args = self.build_args(cmd, config);
        let start = Instant::now();

        let result = tokio::time::timeout(timeout, async {
            tokio::process::Command::new(&self.bwrap_path)
                .args(&args)
                .current_dir(&config.working_dir)
                .output()
                .await
        })
        .await;

        let elapsed = start.elapsed();

        match result {
            Ok(Ok(output)) => Ok(SandboxResult {
                stdout: String::from_utf8_lossy(&output.stdout).to_string(),
                stderr: String::from_utf8_lossy(&output.stderr).to_string(),
                exit_code: output.status.code().unwrap_or(-1),
                backend_used: "bubblewrap".to_string(),
                isolation_level: IsolationLevel::Namespace,
                elapsed_ms: elapsed.as_millis() as u64,
            }),
            Ok(Err(e)) => Err(anyhow::anyhow!("Bubblewrap execution failed: {}", e)),
            Err(_) => Ok(SandboxResult {
                stdout: String::new(),
                stderr: format!("Command timed out after {} seconds", timeout.as_secs()),
                exit_code: -1,
                backend_used: "bubblewrap".to_string(),
                isolation_level: IsolationLevel::Namespace,
                elapsed_ms: elapsed.as_millis() as u64,
            }),
        }
    }
}
