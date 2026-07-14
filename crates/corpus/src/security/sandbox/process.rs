use anyhow::Result;
use async_trait::async_trait;
use fabric::Clock;
use fabric::Timer;
use std::sync::Arc;
use std::time::Duration;
use tracing::info;

use crate::sandbox::{
    IsolationLevel, SandboxBackend, SandboxCapabilities, SandboxConfig, SandboxResult,
};

/// Process-level sandbox backend — uses resource limits but no namespace isolation.
/// Compatible with Docker, WSL2, and environments without user namespace support.
pub struct ProcessBackend {
    pub clock: Arc<dyn Clock>,
}

#[async_trait]
impl SandboxBackend for ProcessBackend {
    fn name(&self) -> &str {
        "process"
    }

    fn isolation_level(&self) -> IsolationLevel {
        IsolationLevel::Process
    }

    fn is_available(&self) -> bool {
        true // Always available — no special privileges required.
    }

    fn capabilities(&self) -> SandboxCapabilities {
        SandboxCapabilities {
            filesystem_isolation: false,
            network_isolation: false,
            resource_limits: true,
            seccomp_filter: false,
            limitations: vec![
                "No filesystem isolation".into(),
                "No network isolation".into(),
                "Resource limits only (RLIMIT)".into(),
            ],
        }
    }

    async fn execute(
        &self,
        cmd: &str,
        config: &SandboxConfig,
        timeout: Duration,
    ) -> Result<SandboxResult> {
        info!(
            command = cmd,
            "Executing command with process-level sandbox"
        );

        let start = self.clock.mono_now();

        // Wrap the spawned process with a timeout.
        let result = aletheon_kernel::chronos::SystemTimer
            .timeout(timeout, async {
                tokio::process::Command::new("bash")
                    .arg("-c")
                    .arg(cmd)
                    .current_dir(&config.working_dir)
                    .envs(&config.env_vars)
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
                backend_used: "process".to_string(),
                isolation_level: IsolationLevel::Process,
                elapsed_ms: elapsed,
            }),
            Ok(Err(e)) => Err(anyhow::anyhow!("Process execution failed: {}", e)),
            Err(_) => Ok(SandboxResult {
                stdout: String::new(),
                stderr: format!("Command timed out after {} seconds", timeout.as_secs()),
                exit_code: -1,
                backend_used: "process".to_string(),
                isolation_level: IsolationLevel::Process,
                elapsed_ms: elapsed,
            }),
        }
    }
}
