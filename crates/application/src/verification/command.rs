//! Provider-neutral command contracts for deterministic verification checks.

use super::VerificationCheckKind;
use ::contracts::{VerificationCheck, VerificationSeverity};
use std::collections::BTreeMap;
use std::path::Path;
use tokio_util::sync::CancellationToken;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrustedVerificationCommand {
    pub program: std::path::PathBuf,
    pub args: Vec<String>,
    pub timeout: std::time::Duration,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerificationCommandOutput {
    pub exit_code: Option<i32>,
    pub elapsed_ms: u64,
    pub timed_out: bool,
    pub cancelled: bool,
    pub stdout: String,
    pub stderr: String,
    pub stdout_bytes: Vec<u8>,
    pub stderr_bytes: Vec<u8>,
    pub stdout_truncated: bool,
    pub stderr_truncated: bool,
}

#[async_trait::async_trait]
pub trait VerificationCommandExecutor: Send + Sync {
    async fn execute(
        &self,
        command: &TrustedVerificationCommand,
        worktree: &Path,
        environment: &BTreeMap<String, String>,
        output_cap_bytes: usize,
        cancel: CancellationToken,
    ) -> Result<VerificationCommandOutput, String>;
}

pub async fn run(
    executor: &dyn VerificationCommandExecutor,
    kind: VerificationCheckKind,
    command: &TrustedVerificationCommand,
    worktree: &Path,
    environment: &BTreeMap<String, String>,
    output_cap_bytes: usize,
    cancel: CancellationToken,
) -> VerificationCheck {
    let severity = severity(kind);
    match executor
        .execute(command, worktree, environment, output_cap_bytes, cancel)
        .await
    {
        Ok(output) => from_output(kind, severity, output),
        Err(error) => VerificationCheck {
            name: kind.as_str().into(),
            severity,
            passed: false,
            timed_out: false,
            cancelled: false,
            summary: format!("command runner error: {error}"),
            evidence: vec![],
        },
    }
}

pub fn from_output(
    kind: VerificationCheckKind,
    severity: VerificationSeverity,
    output: VerificationCommandOutput,
) -> VerificationCheck {
    let passed = output.exit_code == Some(0) && !output.timed_out && !output.cancelled;
    let summary = if output.cancelled {
        "command cancelled".into()
    } else if output.timed_out {
        format!("command timed out after {} ms", output.elapsed_ms)
    } else if passed {
        format!("command passed in {} ms", output.elapsed_ms)
    } else {
        format!("command exited with status {:?}", output.exit_code)
    };
    let mut evidence = Vec::new();
    if !output.stdout.is_empty() {
        evidence.push(format!("stdout: {}", output.stdout));
    }
    if !output.stderr.is_empty() {
        evidence.push(format!("stderr: {}", output.stderr));
    }
    if output.stdout_truncated {
        evidence.push("stdout truncated".into());
    }
    if output.stderr_truncated {
        evidence.push("stderr truncated".into());
    }
    VerificationCheck {
        name: kind.as_str().into(),
        severity,
        passed,
        timed_out: output.timed_out,
        cancelled: output.cancelled,
        summary,
        evidence,
    }
}

pub const fn severity(kind: VerificationCheckKind) -> VerificationSeverity {
    if kind.required() {
        VerificationSeverity::Required
    } else {
        VerificationSeverity::Advisory
    }
}
