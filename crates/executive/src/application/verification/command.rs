//! Trusted argv command adapter for deterministic verification checks.

use super::VerificationCheckKind;
use corpus::tools::subagent::{CommandOutput, CommandRequest, CommandRunner};
use fabric::{VerificationCheck, VerificationSeverity};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio_util::sync::CancellationToken;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrustedVerificationCommand {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub timeout: Duration,
}

#[derive(Debug, Clone, Default)]
pub struct VerificationCommandRunner {
    runner: CommandRunner,
}

impl VerificationCommandRunner {
    pub async fn execute(
        &self,
        command: &TrustedVerificationCommand,
        worktree: &Path,
        environment: &BTreeMap<String, String>,
        output_cap_bytes: usize,
        cancel: CancellationToken,
    ) -> Result<CommandOutput, corpus::tools::subagent::CommandRunnerError> {
        self.runner
            .run(
                CommandRequest {
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
    }

    pub async fn run(
        &self,
        kind: VerificationCheckKind,
        command: &TrustedVerificationCommand,
        worktree: &Path,
        environment: &BTreeMap<String, String>,
        output_cap_bytes: usize,
        cancel: CancellationToken,
    ) -> VerificationCheck {
        let severity = severity(kind);
        let output = self
            .execute(command, worktree, environment, output_cap_bytes, cancel)
            .await;
        match output {
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
}

pub fn from_output(
    kind: VerificationCheckKind,
    severity: VerificationSeverity,
    output: CommandOutput,
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
