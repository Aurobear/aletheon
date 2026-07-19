//! Linux ServiceHost — systemd detection + status (H1-05).

use crate::error::{HostError, HostErrorKind};
use crate::receipt::HostReceipt;
use crate::service::{ServiceHost, ServiceState};
use async_trait::async_trait;
use std::process::Output;
use std::time::Instant;

pub struct LinuxServiceHost;

impl LinuxServiceHost {
    pub fn new() -> Self {
        Self
    }
}

fn has_systemd() -> bool {
    std::path::Path::new("/run/systemd/system").exists()
}

fn bounded_detail(bytes: &[u8]) -> String {
    const MAX: usize = 4096;
    let text = String::from_utf8_lossy(&bytes[..bytes.len().min(MAX)]);
    text.trim().to_string()
}

fn systemctl_error(operation: &str, output: &Output) -> HostError {
    let detail = bounded_detail(&output.stderr);
    let lower = detail.to_ascii_lowercase();
    let kind = if lower.contains("access denied")
        || lower.contains("permission denied")
        || lower.contains("authentication is required")
    {
        HostErrorKind::PermissionDenied(operation.into())
    } else if lower.contains("not found")
        || lower.contains("not be found")
        || lower.contains("could not be found")
    {
        HostErrorKind::NotFound(operation.into())
    } else {
        HostErrorKind::Io(format!("systemctl exited with {}", output.status))
    };
    HostError::new(
        kind,
        if detail.is_empty() {
            operation
        } else {
            &detail
        },
    )
}

async fn run_action(action: &str, name: &str) -> Result<HostReceipt, HostError> {
    if !has_systemd() {
        return Err(HostError::unsupported("systemd not detected"));
    }
    let start = Instant::now();
    let output = tokio::process::Command::new("systemctl")
        .args([action, name])
        .output()
        .await
        .map_err(|e| HostError::new(HostErrorKind::Io(e.to_string()), action))?;
    if !output.status.success() {
        return Err(systemctl_error(action, &output));
    }
    Ok(HostReceipt::ok(
        format!("service_{action}"),
        start.elapsed().as_micros() as u64,
    ))
}

#[async_trait]
impl ServiceHost for LinuxServiceHost {
    async fn status(&self, name: &str) -> Result<ServiceState, HostError> {
        if !has_systemd() {
            return Err(HostError::unsupported("systemd not detected"));
        }
        let output = tokio::process::Command::new("systemctl")
            .args(["is-active", name])
            .output()
            .await
            .map_err(|e| HostError::new(HostErrorKind::Io(e.to_string()), "systemctl"))?;
        if !output.status.success() && output.status.code() != Some(3) {
            return Err(systemctl_error("status", &output));
        }
        let s = String::from_utf8_lossy(&output.stdout).trim().to_string();
        Ok(match s.as_str() {
            "active" => ServiceState::Running,
            "inactive" | "dead" => ServiceState::Stopped,
            "failed" => ServiceState::Failed,
            _ => ServiceState::Unknown,
        })
    }
    async fn start(&self, name: &str) -> Result<HostReceipt, HostError> {
        run_action("start", name).await
    }
    async fn stop(&self, name: &str) -> Result<HostReceipt, HostError> {
        run_action("stop", name).await
    }
    async fn restart(&self, name: &str) -> Result<HostReceipt, HostError> {
        run_action("restart", name).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::process::ExitStatusExt;

    #[tokio::test]
    async fn systemd_status_returns_state_or_unsupported() {
        let host = LinuxServiceHost::new();
        match host.status("sshd.service").await {
            Ok(state) => assert!(matches!(
                state,
                ServiceState::Running | ServiceState::Stopped | ServiceState::Unknown
            )),
            Err(_) => {} // no systemd = Unsupported, acceptable
        }
    }

    #[test]
    fn systemctl_permission_failure_is_typed_and_bounded() {
        let output = Output {
            status: std::process::ExitStatus::from_raw(1 << 8),
            stdout: vec![],
            stderr: [b"Access denied: ".as_slice(), &vec![b'x'; 8192]].concat(),
        };
        let error = systemctl_error("start", &output);
        assert!(matches!(error.kind, HostErrorKind::PermissionDenied(_)));
        assert!(error.detail.len() <= 4096);
    }

    #[test]
    fn systemctl_missing_unit_is_typed() {
        let output = Output {
            status: std::process::ExitStatus::from_raw(5 << 8),
            stdout: vec![],
            stderr: b"Unit demo.service not found".to_vec(),
        };
        let error = systemctl_error("start", &output);
        assert!(matches!(error.kind, HostErrorKind::NotFound(_)));
    }
}
