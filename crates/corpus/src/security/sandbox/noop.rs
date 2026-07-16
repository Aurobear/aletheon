use anyhow::Result;
use async_trait::async_trait;
use fabric::Clock;
use std::sync::Arc;
use std::time::Duration;
use tracing::warn;

use crate::sandbox::{
    IsolationLevel, SandboxBackend, SandboxCapabilities, SandboxConfig, SandboxResult,
};

/// No-op sandbox backend — executes commands directly with no isolation.
/// Always available. Used as last-resort fallback.
pub struct NoopBackend {
    pub clock: Arc<dyn Clock>,
}

#[async_trait]
impl SandboxBackend for NoopBackend {
    fn name(&self) -> &str {
        "noop"
    }

    fn isolation_level(&self) -> IsolationLevel {
        IsolationLevel::None
    }

    fn is_available(&self) -> bool {
        // NoopBackend executes commands with zero isolation.
        // It should never be selected automatically as an available backend.
        // The Forbid preference in SandboxExecutor.select_backend() selects
        // it by name (not via is_available()), so Forbid still works.
        false
    }

    fn capabilities(&self) -> SandboxCapabilities {
        SandboxCapabilities {
            filesystem_isolation: false,
            network_isolation: false,
            resource_limits: false,
            seccomp_filter: false,
            limitations: vec![
                "No filesystem isolation".into(),
                "No network isolation".into(),
                "No resource limits".into(),
                "No seccomp filter".into(),
            ],
        }
    }

    async fn execute(
        &self,
        cmd: &str,
        config: &SandboxConfig,
        _timeout: Duration,
    ) -> Result<SandboxResult> {
        warn!(
            command = cmd,
            "Executing command WITHOUT sandbox (noop backend)"
        );

        let start = self.clock.mono_now();

        let output = tokio::process::Command::new("bash")
            .arg("-c")
            .arg(cmd)
            .current_dir(config.working_dir())
            .envs(&config.environment)
            .output()
            .await?;

        let elapsed = self.clock.mono_now().0.saturating_sub(start.0);

        Ok(SandboxResult {
            stdout: String::from_utf8_lossy(&output.stdout).to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).to_string(),
            exit_code: output.status.code().unwrap_or(-1),
            backend_used: "noop".to_string(),
            isolation_level: IsolationLevel::None,
            elapsed_ms: elapsed,
        })
    }
}
